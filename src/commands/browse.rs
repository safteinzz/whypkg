//! `whypkg` (default) — the interactive investigator.
//!
//! This is the soul of the tool, ported from `apt-why`: fuzzy-find a package,
//! open its dossier (manual/auto, install date, size, upgrade), then *navigate*
//! — every package it's needed by and everything it depends on is itself
//! selectable, so you follow the thread inward and outward. Esc pops back up a
//! level; a breadcrumb shows the trail you've drilled.
//!
//! Unlike the bash original it does no work while you browse: the `World` is in
//! memory, the fuzzy matching is `nucleo` compiled in (no external `fzf`), and
//! every hop is a `HashMap` lookup. The deeper you go, the more obvious the win.

use crate::engine::{format_size, same_session};
use crate::model::World;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::execute;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use std::io::{self, Stdout};

pub struct Args {
    pub upgradable: bool,
}

/// Which packages the list shows, toggled with Tab. The fuzzy query filters by
/// *name*; this filters by *origin* (the `[M]`/`[A]` axis) — the two compose.
#[derive(Clone, Copy, PartialEq)]
enum FilterMode {
    All,
    Manual,
    Auto,
}

impl FilterMode {
    fn matches(self, world: &World, name: &str) -> bool {
        match self {
            FilterMode::All => true,
            FilterMode::Manual => world.is_manual(name),
            FilterMode::Auto => !world.is_manual(name),
        }
    }
    fn next(self) -> Self {
        match self {
            FilterMode::All => FilterMode::Manual,
            FilterMode::Manual => FilterMode::Auto,
            FilterMode::Auto => FilterMode::All,
        }
    }
    fn label(self) -> &'static str {
        match self {
            FilterMode::All => "all",
            FilterMode::Manual => "manual [M]",
            FilterMode::Auto => "auto [A]",
        }
    }
}

/// Which side of the dependency relation the dossier's navigation list shows,
/// toggled with ←/→. Kept separate so the two are never mixed in one list.
#[derive(Clone, Copy, PartialEq)]
enum Relation {
    /// Packages that depend on the focused one (reverse deps).
    NeededBy,
    /// Packages the focused one depends on.
    DependsOn,
}

impl Relation {
    fn toggle(self) -> Self {
        match self {
            Relation::NeededBy => Relation::DependsOn,
            Relation::DependsOn => Relation::NeededBy,
        }
    }
}

/// One level of the navigation stack: either the root package list, or a
/// package's dossier.
struct Frame {
    /// `None` at the root list; `Some(pkg)` when viewing a dossier.
    focus: Option<String>,
    /// Root-list package names (empty for dossier frames, which use the two
    /// relation lists below instead).
    pool: Vec<String>,
    /// Dossier: packages that depend on `focus` (reverse deps).
    needed_by: Vec<String>,
    /// Dossier: packages `focus` depends on.
    depends_on: Vec<String>,
    /// Current fuzzy query.
    query: String,
    /// Selected position within the *filtered* list.
    selected: usize,
    /// Packages installed in the same session as `focus`, cached at open time
    /// (computing this scans the whole install log, so we don't redo it every
    /// render — only when the dossier is first opened).
    alongside: Vec<String>,
    /// Where `focus` came from, cached at open time.
    origin: Origin,
}

/// Why a focused package is on the system — the headline the tool exists to
/// answer. Computed once when a dossier opens.
enum Origin {
    /// Root list frame, or otherwise not applicable.
    None,
    /// The user installed this package directly.
    Manual,
    /// Auto-installed; reverse-dep BFS traced it back to this manual package.
    PulledIn(String),
    /// Auto-installed, but no manual ancestor was found.
    Untraced,
}

pub fn run(args: Args) {
    let world = load_world_with_notice();

    let pool = if args.upgradable {
        world.upgradable_names_sorted()
    } else {
        world.all_names_sorted()
    };

    if pool.is_empty() {
        if args.upgradable {
            println!("\n  Nothing to upgrade — system is up to date.\n");
        } else {
            println!("\n  No packages found.\n");
        }
        return;
    }

    let mut app = App {
        world,
        matcher: Matcher::new(Config::DEFAULT),
        filter: FilterMode::All,
        relation: Relation::NeededBy,
        stack: vec![Frame {
            focus: None,
            pool,
            needed_by: Vec::new(),
            depends_on: Vec::new(),
            query: String::new(),
            selected: 0,
            alongside: Vec::new(),
            origin: Origin::None,
        }],
    };

    if let Err(e) = app.run_ui() {
        eprintln!("whypkg: terminal error: {e}");
        std::process::exit(1);
    }
}

/// Load the world, but since this can take ~½ s on a big system, print a one
/// line notice first so the user isn't staring at a blank terminal.
fn load_world_with_notice() -> World {
    eprint!("  loading package data…\r");
    let world = crate::commands::load_world();
    eprint!("                        \r");
    world
}

struct App {
    world: World,
    matcher: Matcher,
    /// Manual/auto filter, shared across levels and toggled with Tab.
    filter: FilterMode,
    /// Which dependency side the dossier list shows, toggled with ←/→.
    relation: Relation,
    stack: Vec<Frame>,
}

impl App {
    fn run_ui(&mut self) -> io::Result<()> {
        let mut terminal = setup_terminal()?;
        let result = self.event_loop(&mut terminal);
        restore_terminal(&mut terminal)?;
        result
    }

    fn event_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        loop {
            // Recompute the visible (filtered) list for the current frame.
            let visible = self.filtered();
            self.clamp_selection(visible.len());

            terminal.draw(|f| self.render(f, &visible))?;

            let Event::Key(key) = event::read()? else {
                continue;
            };
            // Accept Press and Repeat (so a held Ctrl+J scrolls); ignore Release,
            // which the enhanced keyboard protocol also reports.
            if key.kind == KeyEventKind::Release {
                continue;
            }

            // Ctrl-C always quits.
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c'))
            {
                return Ok(());
            }

            // Esc, or Ctrl+[ — the same control byte historically, but the
            // enhanced keyboard protocol reports them separately, so treat both
            // as escape.
            let escape = matches!(key.code, KeyCode::Esc)
                || (key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('[')));

            match key.code {
                _ if escape => {
                    // Pop a level; quit if we're already at the root.
                    if self.stack.len() == 1 {
                        return Ok(());
                    }
                    self.stack.pop();
                }
                KeyCode::Enter => {
                    if let Some(pkg) = visible.get(self.frame().selected).cloned() {
                        self.open(pkg);
                    }
                }
                KeyCode::Tab => {
                    // Cycle all → manual → auto; reset selection to the top.
                    self.filter = self.filter.next();
                    self.frame_mut().selected = 0;
                }
                KeyCode::Up => self.move_selection(-1, visible.len()),
                KeyCode::Down => self.move_selection(1, visible.len()),
                KeyCode::Left | KeyCode::Right => {
                    // In a dossier, flip the list between "what needs it" and
                    // "what it needs". No-op on the root list.
                    if self.frame().focus.is_some() {
                        self.relation = self.relation.toggle();
                        self.frame_mut().selected = 0;
                    }
                }
                KeyCode::Backspace => {
                    self.frame_mut().query.pop();
                    self.frame_mut().selected = 0;
                }
                KeyCode::Char(c) => {
                    // Ctrl-j/Ctrl-n move down, Ctrl-k/Ctrl-p move up.
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        match c {
                            'j' | 'n' => self.move_selection(1, visible.len()),
                            'k' | 'p' => self.move_selection(-1, visible.len()),
                            _ => {}
                        }
                    } else {
                        self.frame_mut().query.push(c);
                        self.frame_mut().selected = 0;
                    }
                }
                _ => {}
            }
        }
    }

    fn frame(&self) -> &Frame {
        self.stack.last().unwrap()
    }
    fn frame_mut(&mut self) -> &mut Frame {
        self.stack.last_mut().unwrap()
    }

    /// Push a new dossier frame for `pkg`. The two relation lists are kept
    /// separate (never mixed) and toggled with ←/→; we reset to "what needs it"
    /// so every dossier opens in the same, origin-oriented direction.
    fn open(&mut self, pkg: String) {
        let dedup = |src: &[String]| -> Vec<String> {
            let mut seen = std::collections::HashSet::new();
            src.iter().filter(|p| seen.insert((*p).clone())).cloned().collect()
        };
        let needed_by = dedup(self.world.rdeps_of(&pkg));
        let depends_on = dedup(self.world.deps_of(&pkg));

        // Compute the "why is this here" answer and same-session context once,
        // now, so rendering the dossier stays a cheap lookup.
        let origin = if self.world.is_manual(&pkg) {
            Origin::Manual
        } else {
            match crate::engine::bfs_root(&self.world, &pkg) {
                Some(path) => Origin::PulledIn(path.last().cloned().unwrap_or_default()),
                None => Origin::Untraced,
            }
        };
        let alongside = same_session(&self.world, &pkg);

        self.relation = Relation::NeededBy;
        self.stack.push(Frame {
            focus: Some(pkg),
            pool: Vec::new(),
            needed_by,
            depends_on,
            query: String::new(),
            selected: 0,
            alongside,
            origin,
        });
    }

    /// The active base list for the current frame: the root pool, or — in a
    /// dossier — whichever relation side is currently selected.
    fn base_list(&self) -> &[String] {
        let frame = self.stack.last().unwrap();
        if frame.focus.is_none() {
            &frame.pool
        } else if self.relation == Relation::NeededBy {
            &frame.needed_by
        } else {
            &frame.depends_on
        }
    }

    /// The current base list, filtered by manual/auto and ranked by the fuzzy
    /// query. An empty query keeps the natural (sorted) order.
    fn filtered(&mut self) -> Vec<String> {
        let (query, base) = {
            let base: Vec<String> = self
                .base_list()
                .iter()
                .filter(|n| self.filter.matches(&self.world, n))
                .cloned()
                .collect();
            (self.stack.last().unwrap().query.clone(), base)
        };
        if query.is_empty() {
            return base;
        }
        let pattern = Pattern::parse(&query, CaseMatching::Ignore, Normalization::Smart);
        pattern
            .match_list(base.into_iter(), &mut self.matcher)
            .into_iter()
            .map(|(name, _score)| name)
            .collect()
    }

    fn clamp_selection(&mut self, len: usize) {
        let sel = &mut self.frame_mut().selected;
        if len == 0 {
            *sel = 0;
        } else if *sel >= len {
            *sel = len - 1;
        }
    }

    fn move_selection(&mut self, delta: isize, len: usize) {
        if len == 0 {
            return;
        }
        let cur = self.frame().selected as isize;
        let next = (cur + delta).clamp(0, len as isize - 1);
        self.frame_mut().selected = next as usize;
    }

    // ── rendering ─────────────────────────────────────────────────────────────

    fn render(&self, f: &mut ratatui::Frame, visible: &[String]) {
        let frame = self.frame();
        let has_dossier = frame.focus.is_some();

        let dossier_lines = if frame.focus.is_some() {
            self.dossier_lines(frame)
        } else {
            Vec::new()
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),                                   // breadcrumb
                Constraint::Length(if has_dossier { dossier_lines.len() as u16 + 2 } else { 0 }),
                Constraint::Min(3),                                      // list
                Constraint::Length(1),                                   // query input
                Constraint::Length(1),                                   // help
            ])
            .split(f.area());

        // Breadcrumb: whypkg › firefox › libnss3 …
        let mut crumb = vec![Span::styled("whypkg", Style::new().bold().cyan())];
        for fr in &self.stack {
            if let Some(p) = &fr.focus {
                crumb.push(Span::raw(" › "));
                crumb.push(Span::styled(p.clone(), Style::new().bold()));
            }
        }
        f.render_widget(Paragraph::new(Line::from(crumb)), chunks[0]);

        // Dossier info panel (only on a focused frame).
        if has_dossier {
            f.render_widget(
                Paragraph::new(dossier_lines).block(Block::default().borders(Borders::ALL)),
                chunks[1],
            );
        }

        // The navigable package list.
        let items: Vec<ListItem> = visible
            .iter()
            .map(|name| ListItem::new(self.pkg_line(name)))
            .collect();
        let mut state = ListState::default();
        state.select(if visible.is_empty() {
            None
        } else {
            Some(frame.selected)
        });
        let list = List::new(items)
            .highlight_style(Style::new().bg(Color::Indexed(54)).bold())
            .highlight_symbol("› ");
        f.render_stateful_widget(list, chunks[2], &mut state);

        // Query input line, with the active manual/auto filter shown on the right.
        let mode_span = if self.filter == FilterMode::All {
            Span::styled("  showing: all", Style::new().dim())
        } else {
            Span::styled(
                format!("  showing: {}", self.filter.label()),
                Style::new().cyan(),
            )
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  filter ", Style::new().dim()),
                Span::raw(frame.query.clone()),
                Span::styled("▏", Style::new().cyan()),
                mode_span,
            ])),
            chunks[3],
        );

        // Contextual help footer.
        let help = if has_dossier {
            "Enter open  Esc back  ←/→ needs-it / it-needs  Tab manual/auto  Ctrl-C quit"
        } else {
            "type to filter  │  Tab manual/auto  Enter open  Esc quit"
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(help, Style::new().dim()))),
            chunks[4],
        );
    }

    /// The styled info block shown above a package's navigation list. Reads the
    /// frame's cached `origin`/`alongside` so it's cheap to redraw every frame.
    fn dossier_lines(&self, frame: &Frame) -> Vec<Line<'static>> {
        let pkg = frame.focus.as_deref().unwrap_or_default();
        let p = self.world.packages.get(pkg);
        let dim = Style::new().dim();

        let version = match p {
            Some(p) => match &p.candidate {
                Some(c) => format!("{}  →  {}", p.version, c),
                None => p.version.clone(),
            },
            None => "unknown".into(),
        };
        let size = format_size(p.map(|p| p.installed_size).unwrap_or(0));
        // Absolute date plus a complementary relative hint: "2024-06-01 (3 months ago)".
        let installed = match (
            p.and_then(|p| p.install_date.clone()),
            p.and_then(|p| p.install_epoch),
        ) {
            (Some(date), Some(epoch)) => {
                format!("{date} ({})", crate::engine::relative_time(epoch))
            }
            (Some(date), None) => date,
            _ => "unknown".to_string(),
        };
        let description = p.map(|p| p.description.clone()).unwrap_or_default();

        let needed_by = self.world.rdep_count(pkg);
        let depends_on = self.world.deps_of(pkg).len();

        // A "needed by: nothing" package is normally safe to remove — but never
        // say that about kernel/firmware, which nothing "depends on" yet must
        // not be touched.
        let needed_by_text = if needed_by == 0 {
            if crate::engine::is_kernel_pkg(pkg) {
                "nothing — but kernel/firmware, do not remove".to_string()
            } else {
                "nothing — safe to remove".to_string()
            }
        } else {
            format!("{needed_by} packages")
        };

        let kv = |k: &str, v: Span<'static>| -> Line<'static> {
            Line::from(vec![Span::styled(format!("  {k:<10}"), Style::new().dim()), v])
        };
        // A key/value line whose value is several styled spans (e.g. the origin).
        let kv_spans = |k: &str, mut spans: Vec<Span<'static>>| -> Line<'static> {
            let mut out = vec![Span::styled(format!("  {k:<10}"), Style::new().dim())];
            out.append(&mut spans);
            Line::from(out)
        };

        let origin_spans: Vec<Span<'static>> = match &frame.origin {
            Origin::Manual => vec![Span::styled("you installed this", Style::new().green())],
            Origin::PulledIn(root) => vec![
                Span::styled("pulled in by ", Style::new().yellow()),
                Span::styled(root.clone(), Style::new().bold().yellow()),
            ],
            Origin::Untraced => {
                vec![Span::styled("auto-installed (origin untraced)", Style::new().yellow())]
            }
            Origin::None => vec![Span::raw("")],
        };

        let mut lines = vec![
            Line::from(vec![
                Span::styled(format!("  {pkg}"), Style::new().bold().white()),
                if self.world.is_upgradable(pkg) {
                    Span::styled("   ↑ upgrade available", Style::new().cyan())
                } else {
                    Span::raw("")
                },
            ]),
            Line::from(Span::styled(format!("  {description}"), dim)),
            kv_spans("why here", origin_spans),
        ];

        // "alongside" sits high — it's context for *why here*: a few example
        // packages installed in the same session, not just a count.
        if !frame.alongside.is_empty() {
            let preview: Vec<&str> = frame.alongside.iter().take(3).map(String::as_str).collect();
            let more = frame.alongside.len().saturating_sub(preview.len());
            let mut text = preview.join(", ");
            if more > 0 {
                text.push_str(&format!(", +{more} more"));
            }
            lines.push(kv("alongside", Span::raw(text)));
        }

        lines.push(kv("version", Span::raw(version)));
        lines.push(kv("size", Span::raw(size)));
        lines.push(kv("installed", Span::raw(installed)));

        // The two relations are separate lists, one shown at a time (toggle with
        // ←/→). Mark whichever is active so it's always clear which packages the
        // list below holds — even if that side happens to be empty.
        let showing = || Span::styled("  ← showing below", Style::new().bold().cyan());
        let mut needed = vec![Span::raw(needed_by_text)];
        if self.relation == Relation::NeededBy {
            needed.push(showing());
        }
        lines.push(kv_spans("needed by", needed));

        let mut depends = vec![Span::raw(format!("{depends_on} packages"))];
        if self.relation == Relation::DependsOn {
            depends.push(showing());
        }
        lines.push(kv_spans("depends on", depends));
        lines
    }

    /// One styled row in the package list: tag, name, upgrade arrow, size, desc.
    fn pkg_line(&self, name: &str) -> Line<'static> {
        let manual = self.world.is_manual(name);
        let tag = if manual {
            Span::styled("[M]", Style::new().green())
        } else {
            Span::styled("[A]", Style::new().yellow())
        };
        let up = if self.world.is_upgradable(name) {
            Span::styled("↑", Style::new().cyan())
        } else {
            Span::raw(" ")
        };
        let size = format_size(self.world.packages.get(name).map(|p| p.installed_size).unwrap_or(0));
        let desc = truncate(
            &self
                .world
                .packages
                .get(name)
                .map(|p| p.description.clone())
                .unwrap_or_default(),
            55,
        );
        Line::from(vec![
            tag,
            Span::raw(" "),
            Span::raw(format!("{name:<34}")),
            Span::raw(" "),
            up,
            Span::raw(format!(" {size:>9}  ")),
            Span::styled(desc, Style::new().dim()),
        ])
    }
}

/// Truncate to at most `max` characters (UTF-8 safe — never splits a char,
/// unlike the byte-based `substr`/`:0:n` the bash version used).
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

// ── terminal lifecycle ────────────────────────────────────────────────────────

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    // Where supported, ask the terminal to report keys unambiguously — this is
    // what makes Ctrl+J distinct from Enter (and gives key-repeat events).
    // Unsupported terminals simply ignore it.
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
