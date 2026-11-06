//! `whypkg` (default): the interactive investigator.
//!
//! Every installed package sits in a tab per installer (`all` first), `↵`
//! opens a package's dossier, and from there every package it needs or is
//! needed by opens in turn, with `esc` walking back the way you came. The
//! footer names the keys a screen answers to, and `?` lists every one.
//!
//! Nothing is read while you browse: the `World` is in memory and every hop is
//! a lookup. Only `r`, and a source turned on in the settings that was never
//! read, reads the machine again, on a thread of its own while the screen
//! stays usable.

mod boot;
pub mod graph;
mod input;
mod line_edit;
mod options;
mod render;
mod widgets;

pub use boot::while_loading;

use crate::engine::{Why, why};
use crate::load::{Progress, Sources};
use crate::model::{Source, World};
use crossterm::event::{
    self, Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{self, Stdout};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// Which packages the lists show, stepped through with `s`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Show {
    All,
    /// Only what you installed yourself.
    Manual,
    /// Only what something pulled in.
    Auto,
    /// Only what has an upgrade waiting.
    Upgradable,
}

impl Show {
    pub fn word(self) -> &'static str {
        match self {
            Show::All => "all",
            Show::Manual => "manual",
            Show::Auto => "auto",
            Show::Upgradable => "upgradable",
        }
    }

    fn next(self) -> Show {
        match self {
            Show::All => Show::Manual,
            Show::Manual => Show::Auto,
            Show::Auto => Show::Upgradable,
            Show::Upgradable => Show::All,
        }
    }

    fn keeps(self, world: &World, key: &str) -> bool {
        match self {
            Show::All => true,
            Show::Manual => world.is_manual(key),
            Show::Auto => !world.is_manual(key),
            Show::Upgradable => world.is_upgradable(key),
        }
    }
}

/// The two lists under a dossier, `h`/`l` between them.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Side {
    NeededBy,
    DependsOn,
}

impl Side {
    fn index(self) -> usize {
        match self {
            Side::NeededBy => 0,
            Side::DependsOn => 1,
        }
    }
}

/// One row of a dossier list: a package, and whether the edge is a `uses`
/// link (a library it loads, the interpreter it runs on) rather than a
/// dependency its installer recorded.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub key: String,
    pub uses: bool,
}

/// A package's dossier, open in one tab.
pub struct Dossier {
    pub pkg: String,
    pub side: Side,
    /// `[needed by, depends on]`.
    pub lists: [Vec<Link>; 2],
    /// Rows into each list as the filter shows it.
    pub cursor: [usize; 2],
    pub top: [Cell<usize>; 2],
    /// Packages installed in the same session, cached because finding them
    /// scans the whole install log.
    pub alongside: Vec<String>,
    /// The dossiers `↵` came through, for `esc`, with the side and cursor each
    /// was left on.
    pub back: Vec<(String, Side, [usize; 2])>,
}

impl Dossier {
    fn new(world: &World, pkg: &str) -> Dossier {
        let side = |edges: Vec<String>, links: &[String]| -> Vec<Link> {
            edges
                .into_iter()
                .map(|key| Link {
                    uses: links.contains(&key),
                    key,
                })
                .collect()
        };
        // An edge that is both (a library the package manager also recorded)
        // counts as a dependency.
        let used_by: Vec<String> = world
            .used_by_of(pkg)
            .iter()
            .filter(|k| !world.rdeps_of(pkg).contains(k))
            .cloned()
            .collect();
        let uses: Vec<String> = world
            .uses_of(pkg)
            .iter()
            .filter(|k| !world.deps_of(pkg).contains(k))
            .cloned()
            .collect();
        Dossier {
            pkg: pkg.to_string(),
            side: Side::NeededBy,
            lists: [
                side(world.needed_by_all(pkg), &used_by),
                side(world.depends_on_all(pkg), &uses),
            ],
            cursor: [0, 0],
            top: [Cell::new(0), Cell::new(0)],
            alongside: crate::engine::same_session(world, pkg),
            back: Vec::new(),
        }
    }

    /// Replace what is shown with `pkg`, keeping the way back.
    fn show(&mut self, world: &World, pkg: &str, side: Side, cursor: [usize; 2]) {
        let back = std::mem::take(&mut self.back);
        *self = Dossier::new(world, pkg);
        self.back = back;
        self.side = side;
        for ((at, want), list) in self.cursor.iter_mut().zip(cursor).zip(&self.lists) {
            *at = want.min(list.len().saturating_sub(1));
        }
    }
}

/// One tab: every shown package of one source (`None` is `all`), and the
/// dossier open in it, if any. Each tab keeps its own place.
pub struct Tab {
    pub source: Option<Source>,
    pub rows: Vec<String>,
    /// A row into `rows` as the filter shows it.
    pub cursor: usize,
    /// The first row on screen, kept by the renderer.
    pub top: Cell<usize>,
    pub dossier: Option<Dossier>,
}

impl Tab {
    pub fn name(&self) -> &'static str {
        self.source.map_or("all", Source::word)
    }
}

/// The `/` filter on the list in front of you. It belongs to that list and
/// is dropped whenever the screen changes.
pub struct Filter {
    pub query: String,
    /// The cursor in `query`, as characters after it (`line_edit::edit`).
    pub back: usize,
    /// Still being typed, rather than kept with `↵`.
    pub typing: bool,
}

/// Whether a message reports something that worked or something that did
/// not; it decides the colour, never red.
#[derive(Clone, Copy, PartialEq)]
pub enum Tone {
    Done,
    Failed,
}

/// The transient line on the status row.
pub struct Message {
    pub text: String,
    pub tone: Tone,
    pub at: Instant,
}

/// How long a message stays on the status row.
pub const MESSAGE_TTL: Duration = Duration::from_secs(3);

/// A box drawn over the screen that takes every key until it closes.
pub enum Overlay {
    /// `page` is the rows the body shows, kept by the renderer.
    Help {
        scroll: Cell<usize>,
        page: Cell<usize>,
    },
    Options(options::Options),
    Alert {
        title: String,
        body: String,
    },
}

/// A re-read of the machine running on its own thread.
pub struct Job {
    /// What it reads, which becomes `App::read` when it lands.
    pub sources: Sources,
    pub progress: Arc<Progress>,
    pub rx: Receiver<Result<World, String>>,
    pub started: Instant,
}

pub struct App {
    pub world: World,
    /// The sources this session shows.
    pub sources: Sources,
    /// The sources `world` was read from.
    pub read: Sources,
    pub show: Show,
    pub tabs: Vec<Tab>,
    pub tab: usize,
    pub graph: Option<graph::GraphView>,
    pub filter: Option<Filter>,
    pub message: Option<Message>,
    pub overlay: Option<Overlay>,
    pub job: Option<Job>,
    /// A failed re-read waiting for the open box to close before its alert.
    held: Option<String>,
    /// "Why is this here" for each row drawn so far, since tracing one walks
    /// the graph.
    why_cache: RefCell<HashMap<String, Why>>,
    pub quit: bool,
    /// `ctrl-l` asks for the whole screen to be drawn again.
    pub redraw: bool,
    /// Rows the focused list shows, kept by the renderer for the keys that
    /// move by a screen.
    pub view: Cell<usize>,
}

impl App {
    pub fn new(world: World, read: Sources, show: Show) -> App {
        let mut app = App {
            world,
            sources: read.clone(),
            read,
            show,
            tabs: Vec::new(),
            tab: 0,
            graph: None,
            filter: None,
            message: None,
            overlay: None,
            job: None,
            held: None,
            why_cache: RefCell::new(HashMap::new()),
            quit: false,
            redraw: false,
            view: Cell::new(1),
        };
        app.rebuild_tabs();
        app
    }

    pub fn run_ui(&mut self) -> io::Result<()> {
        let mut terminal = setup_terminal()?;
        let result = self.event_loop(&mut terminal);
        restore_terminal(&mut terminal)?;
        result
    }

    fn event_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        while !self.quit {
            self.poll_job();
            if std::mem::take(&mut self.redraw) {
                terminal.clear()?;
            }
            terminal.draw(|f| self.render(f))?;
            // Wake up on our own while something on screen moves or fades:
            // the reading mark, and a message's three seconds.
            let wait = if self.job.is_some() {
                Duration::from_millis(100)
            } else if let Some(m) = &self.message {
                MESSAGE_TTL.saturating_sub(m.at.elapsed()) + Duration::from_millis(20)
            } else {
                Duration::from_secs(3600)
            };
            if !event::poll(wait)? {
                continue;
            }
            if let Event::Key(key) = event::read()?
                // The enhanced keyboard protocol also reports releases.
                && key.kind != KeyEventKind::Release
            {
                self.key(key);
            }
        }
        Ok(())
    }

    pub fn current(&self) -> &Tab {
        &self.tabs[self.tab]
    }

    pub fn current_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.tab]
    }

    pub fn set_status(&mut self, text: impl Into<String>) {
        self.say(text.into(), Tone::Done);
    }

    pub fn set_failed(&mut self, text: impl Into<String>) {
        self.say(text.into(), Tone::Failed);
    }

    fn say(&mut self, text: String, tone: Tone) {
        self.message = Some(Message {
            text,
            tone,
            at: Instant::now(),
        });
    }

    /// Why `key` is here, traced once and remembered.
    pub fn why_of(&self, key: &str) -> Why {
        if let Some(w) = self.why_cache.borrow().get(key) {
            return w.clone();
        }
        let w = why(&self.world, key);
        self.why_cache
            .borrow_mut()
            .insert(key.to_string(), w.clone());
        w
    }

    /// The sources that are both read and wanted.
    fn visible(&self, source: Source) -> bool {
        self.sources.has(source) && self.read.has(source)
    }

    /// Whether the world was read from other sources than this session
    /// shows, so it has to be read again.
    pub fn stale(&self) -> bool {
        Source::ALL
            .into_iter()
            .any(|s| self.sources.has(s) != self.read.has(s))
    }

    /// Whether `key` is kept by the filter: every word of the query appears,
    /// in any case, in its name, key, description or details.
    fn kept(&self, key: &str) -> bool {
        let Some(filter) = self.filter.as_ref().filter(|f| !f.query.is_empty()) else {
            return true;
        };
        let Some(p) = self.world.packages.get(key) else {
            return false;
        };
        let haystack = format!(
            "{} {} {} {}",
            p.label(),
            key,
            p.description,
            p.details.as_deref().unwrap_or_default()
        )
        .to_lowercase();
        filter
            .query
            .to_lowercase()
            .split_whitespace()
            .all(|word| haystack.contains(word))
    }

    /// The list in front of you, unfiltered: a dossier's focused side, or the
    /// tab's packages.
    fn focus_all(&self) -> Vec<&str> {
        let tab = self.current();
        match &tab.dossier {
            Some(d) => d.lists[d.side.index()]
                .iter()
                .map(|l| l.key.as_str())
                .collect(),
            None => tab.rows.iter().map(String::as_str).collect(),
        }
    }

    /// The rows of the list in front of you as the filter shows them, in the
    /// list's own order; the cursor is an index into these.
    pub fn focus_rows(&self) -> Vec<String> {
        self.focus_all()
            .into_iter()
            .filter(|k| self.kept(k))
            .map(String::from)
            .collect()
    }

    /// Build the tabs again after the world, `show` or the sources changed,
    /// keeping each tab's cursor on the package it was on and its dossier open
    /// when that package is still there.
    pub fn rebuild_tabs(&mut self) {
        // The cursor indexes the filtered rows, so it is put back on its
        // package in the full list before the rows change under it.
        if !self.tabs.is_empty() {
            self.drop_filter();
        }
        let mut keys: Vec<&String> = self
            .world
            .packages
            .iter()
            .filter(|(k, p)| self.visible(p.source) && self.show.keeps(&self.world, k))
            .map(|(k, _)| k)
            .collect();
        keys.sort_by(|a, b| {
            let (la, lb) = (self.world.label(a), self.world.label(b));
            la.to_lowercase()
                .cmp(&lb.to_lowercase())
                .then_with(|| a.cmp(b))
        });

        let mut old: Vec<Tab> = std::mem::take(&mut self.tabs);
        let current = old.get(self.tab).map(|t| t.source);
        let mut tabs = vec![Tab {
            source: None,
            rows: keys.iter().map(|k| (*k).clone()).collect(),
            cursor: 0,
            top: Cell::new(0),
            dossier: None,
        }];
        for source in Source::ALL {
            let rows: Vec<String> = keys
                .iter()
                .filter(|k| self.world.source(k) == source)
                .map(|k| (*k).clone())
                .collect();
            if !rows.is_empty() {
                tabs.push(Tab {
                    source: Some(source),
                    rows,
                    cursor: 0,
                    top: Cell::new(0),
                    dossier: None,
                });
            }
        }
        for tab in &mut tabs {
            let Some(was) = old.iter_mut().find(|t| t.source == tab.source) else {
                continue;
            };
            let on = was.rows.get(was.cursor);
            tab.cursor = on
                .and_then(|k| tab.rows.iter().position(|r| r == k))
                .unwrap_or_else(|| was.cursor.min(tab.rows.len().saturating_sub(1)));
            if let Some(d) = was.dossier.take()
                && self.world.packages.contains_key(&d.pkg)
            {
                let mut fresh = Dossier::new(&self.world, &d.pkg);
                fresh.back = d.back;
                fresh.side = d.side;
                for ((at, was), list) in fresh.cursor.iter_mut().zip(d.cursor).zip(&fresh.lists) {
                    *at = was.min(list.len().saturating_sub(1));
                }
                tab.dossier = Some(fresh);
            }
        }
        self.tab = tabs
            .iter()
            .position(|t| Some(t.source) == current)
            .unwrap_or(0);
        self.tabs = tabs;
        self.why_cache.borrow_mut().clear();
    }

    /// Open `pkg`'s dossier in this tab, one level below what is on screen.
    pub fn open(&mut self, pkg: &str) {
        self.drop_filter();
        let world = &self.world;
        let tab = &mut self.tabs[self.tab];
        match &mut tab.dossier {
            Some(d) => {
                let left = (d.pkg.clone(), d.side, d.cursor);
                d.back.push(left);
                d.show(world, pkg, Side::NeededBy, [0, 0]);
            }
            None => tab.dossier = Some(Dossier::new(world, pkg)),
        }
    }

    /// The package the cursor is on, in the list in front of you.
    pub fn under_cursor(&self) -> Option<String> {
        self.focus_rows().get(self.focus_cursor()).cloned()
    }

    pub fn focus_cursor(&self) -> usize {
        let tab = self.current();
        match &tab.dossier {
            Some(d) => d.cursor[d.side.index()],
            None => tab.cursor,
        }
    }

    /// Re-read the machine with `sources` on a thread of its own. Asking again
    /// while one runs does nothing.
    pub fn reload(&mut self, sources: Sources) {
        if self.job.is_some() {
            return;
        }
        let progress = Arc::new(Progress::default());
        let (tx, rx) = mpsc::channel();
        let shared = Arc::clone(&progress);
        let reading = sources.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::load::read_world(&reading, &shared));
        });
        self.job = Some(Job {
            sources,
            progress,
            rx,
            started: Instant::now(),
        });
    }

    /// Take in a finished re-read: the new world on success, an alert that
    /// says why on failure, held back until any open box is closed.
    fn poll_job(&mut self) {
        if self.overlay.is_none()
            && let Some(body) = self.held.take()
        {
            self.overlay = Some(Overlay::Alert {
                title: "could not read".into(),
                body,
            });
        }
        let Some(job) = &self.job else {
            return;
        };
        let result = match job.rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("the reading thread stopped".into()),
        };
        let read = job.sources.clone();
        self.job = None;
        match result {
            Ok(world) => {
                self.world = world;
                self.read = read;
                // The graph stays open on the same centre when that package
                // is still there.
                let centre = self.graph.as_ref().map(|g| g.center_name().to_string());
                self.graph = centre
                    .filter(|c| self.world.packages.contains_key(c))
                    .map(|c| graph::GraphView::build(&self.world, &c));
                self.rebuild_tabs();
                let n = self.world.packages.len();
                self.set_status(format!("read {}", crate::engine::count(n, "package")));
                // The settings changed a source while this read ran.
                if self.stale() {
                    self.reload(self.sources.clone());
                }
            }
            Err(e) => {
                self.held = Some(format!(
                    "{e}\n\nThe packages on screen are from the last read."
                ));
                self.poll_job();
            }
        }
    }
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    // Where supported, ask the terminal to report keys unambiguously, which is
    // what tells `ctrl-[` from a bare `esc` byte sequence (and gives
    // key-repeat events). Unsupported terminals simply ignore it.
    if matches!(
        crossterm::terminal::supports_keyboard_enhancement(),
        Ok(true)
    ) {
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    if matches!(
        crossterm::terminal::supports_keyboard_enhancement(),
        Ok(true)
    ) {
        let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()
}
