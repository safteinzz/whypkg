//! `whypkg` (default) - the interactive investigator.
//!
//! This is the soul of the tool, ported from `apt-why`: fuzzy-find a package,
//! open its dossier (manual/auto, install date, size, upgrade), then *navigate*:
//! every package it's needed by and everything it depends on is itself
//! selectable, so you follow the thread inward and outward. Esc pops back up a
//! level; a breadcrumb shows the trail you've drilled.
//!
//! Unlike the bash original it does no work while you browse: the `World` is in
//! memory, the fuzzy matching is `nucleo` compiled in (no external `fzf`), and
//! every hop is a `HashMap` lookup. The deeper you go, the more obvious the win.

pub mod filter;
pub mod graph;
mod input;
mod render;

use filter::FilterMode;

use crate::engine::{format_size, same_session};
use crate::model::World;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32Str};
use ratatui::prelude::*;
use std::io::{self, Stdout};

/// Which side of the dependency relation the dossier's navigation list shows,
/// toggled with ←/→. Kept separate so the two are never mixed in one list.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Relation {
    /// Packages that depend on the focused one (reverse deps).
    NeededBy,
    /// Packages the focused one depends on.
    DependsOn,
}

impl Relation {
    pub(crate) fn toggle(self) -> Self {
        match self {
            Relation::NeededBy => Relation::DependsOn,
            Relation::DependsOn => Relation::NeededBy,
        }
    }
}

/// One level of the navigation stack: either the root package list, or a
/// package's dossier.
pub(crate) struct Frame {
    /// `None` at the root list; `Some(pkg)` when viewing a dossier.
    pub(crate) focus: Option<String>,
    /// Root-list package names (empty for dossier frames, which use the two
    /// relation lists below instead).
    pub(crate) pool: Vec<String>,
    /// Dossier: packages that depend on `focus` (reverse deps).
    pub(crate) needed_by: Vec<String>,
    /// Dossier: packages `focus` depends on.
    pub(crate) depends_on: Vec<String>,
    /// Current fuzzy query.
    pub(crate) query: String,
    /// Selected position within the *filtered* list.
    pub(crate) selected: usize,
    /// Packages installed in the same session as `focus`, cached at open time
    /// (computing this scans the whole install log, so we don't redo it every
    /// render - only when the dossier is first opened).
    pub(crate) alongside: Vec<String>,
    /// Where `focus` came from, cached at open time.
    pub(crate) origin: Origin,
}

/// Why a focused package is on the system - the headline the tool exists to
/// answer. Computed once when a dossier opens.
pub(crate) enum Origin {
    /// Root list frame, or otherwise not applicable.
    None,
    /// The user installed this package directly.
    Manual,
    /// Auto-installed; reverse-dep BFS traced it back to this manual package.
    PulledIn(String),
    /// Auto-installed, but no manual ancestor was found.
    Untraced,
}

pub(crate) struct App {
    pub(crate) world: World,
    pub(crate) matcher: Matcher,
    /// Manual/auto filter, shared across levels and toggled with Tab.
    pub(crate) filter: FilterMode,
    /// Which dependency side the dossier list shows, toggled with ←/→.
    pub(crate) relation: Relation,
    pub(crate) stack: Vec<Frame>,
    /// When `Some`, the graph view is open over everything else (Ctrl+G).
    pub(crate) graph: Option<graph::GraphView>,
}

impl App {
    pub(crate) fn run_ui(&mut self) -> io::Result<()> {
        let mut terminal = setup_terminal()?;
        let result = self.event_loop(&mut terminal);
        restore_terminal(&mut terminal)?;
        result
    }

    /// Whether any flatpak apps are present (gates the flatpak Tab bucket).
    pub(crate) fn has_flatpak(&self) -> bool {
        self.world
            .packages
            .values()
            .any(|p| p.source == crate::model::Source::Flatpak)
    }

    pub(crate) fn frame(&self) -> &Frame {
        self.stack.last().unwrap()
    }
    pub(crate) fn frame_mut(&mut self) -> &mut Frame {
        self.stack.last_mut().unwrap()
    }

    /// Push a new dossier frame for `pkg`. The two relation lists are kept
    /// separate (never mixed) and toggled with ←/→; we reset to "what needs it"
    /// so every dossier opens in the same, origin-oriented direction.
    pub(crate) fn open(&mut self, pkg: String) {
        let dedup = |src: &[String]| -> Vec<String> {
            let mut seen = std::collections::HashSet::new();
            src.iter()
                .filter(|p| seen.insert((*p).clone()))
                .cloned()
                .collect()
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

    /// The active base list for the current frame: the root pool, or - in a
    /// dossier - whichever relation side is currently selected.
    pub(crate) fn base_list(&self) -> &[String] {
        let frame = self.stack.last().unwrap();
        if frame.focus.is_none() {
            &frame.pool
        } else if self.relation == Relation::NeededBy {
            &frame.needed_by
        } else {
            &frame.depends_on
        }
    }

    pub(crate) fn clamp_selection(&mut self, len: usize) {
        let sel = &mut self.frame_mut().selected;
        if len == 0 {
            *sel = 0;
        } else if *sel >= len {
            *sel = len - 1;
        }
    }

    pub(crate) fn move_selection(&mut self, delta: isize, len: usize) {
        if len == 0 {
            return;
        }
        let cur = self.frame().selected as isize;
        let next = (cur + delta).clamp(0, len as isize - 1);
        self.frame_mut().selected = next as usize;
    }

    // ── rendering ─────────────────────────────────────────────────────────────
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    // Where supported, ask the terminal to report keys unambiguously - this is
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
