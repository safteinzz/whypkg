//! Every key the browser answers to.

use super::options::Options;
use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    pub(super) fn key(&mut self, key: KeyEvent) {
        if self.overlay.is_some() {
            self.overlay_key(key);
            return;
        }
        if self.filter.as_ref().is_some_and(|f| f.typing) {
            self.filter_key(key);
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            _ if is_ctrl(key, 'c') => self.quit = true,
            _ if is_ctrl(key, 'l') => self.redraw = true,
            KeyCode::Char('q') if !ctrl => self.quit = true,
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char('r') if !ctrl => self.reload(self.sources.clone()),
            KeyCode::Char('o') if !ctrl => self.open_options(),
            _ if self.graph.is_some() => self.graph_key(key),
            _ => self.list_key(key),
        }
    }

    // ── the lists ────────────────────────────────────────────────────────────

    /// Put the cursor on row `at` of the list in front of you, clamped to it.
    fn goto(&mut self, at: usize) {
        let last = self.focus_rows().len().saturating_sub(1);
        let at = at.min(last);
        let tab = self.current_mut();
        match &mut tab.dossier {
            Some(d) => d.cursor[d.side.index()] = at,
            None => tab.cursor = at,
        }
    }

    fn move_by(&mut self, delta: isize) {
        let at = (self.focus_cursor() as isize + delta).max(0) as usize;
        self.goto(at);
    }

    fn list_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let view = self.view.get().max(1) as isize;
        let in_dossier = self.current().dossier.is_some();
        match key.code {
            _ if is_escape(key) => self.back(),
            KeyCode::Char('j') | KeyCode::Down if !ctrl => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up if !ctrl => self.move_by(-1),
            KeyCode::Char('d') if ctrl => self.move_by(view / 2),
            KeyCode::Char('u') if ctrl => self.move_by(-view / 2),
            KeyCode::PageDown => self.move_by(view),
            KeyCode::PageUp => self.move_by(-view),
            KeyCode::Char('g') | KeyCode::Home if !ctrl => self.goto(0),
            KeyCode::Char('G') | KeyCode::End => self.goto(usize::MAX),
            KeyCode::Tab => self.tab_step(1),
            KeyCode::BackTab => self.tab_step(-1),
            KeyCode::Enter => {
                if let Some(pkg) = self.under_cursor() {
                    self.open(&pkg);
                }
            }
            KeyCode::Char('/') => {
                self.filter = Some(Filter {
                    query: String::new(),
                    back: 0,
                    typing: true,
                });
                self.goto(0);
            }
            KeyCode::Char('g') if ctrl => self.open_graph(),
            KeyCode::Char('s') if !ctrl && !in_dossier => {
                self.show = self.show.next();
                self.rebuild_tabs();
            }
            KeyCode::Char('h') | KeyCode::Left if in_dossier => self.flip(Side::NeededBy),
            KeyCode::Char('l') | KeyCode::Right if in_dossier => self.flip(Side::DependsOn),
            _ => {}
        }
    }

    /// `esc`: out one step. A kept filter goes first, then the dossier `↵`
    /// came from, then the dossier itself. The list is the top, so `esc` never
    /// quits there.
    fn back(&mut self) {
        if self.filter.is_some() {
            self.drop_filter();
            return;
        }
        let world = &self.world;
        let tab = &mut self.tabs[self.tab];
        let Some(d) = &mut tab.dossier else {
            return;
        };
        match d.back.pop() {
            Some((pkg, side, cursor)) => d.show(world, &pkg, side, cursor),
            None => tab.dossier = None,
        }
    }

    /// Drop the filter, leaving the cursor on the package it was on.
    pub(super) fn drop_filter(&mut self) {
        let on = self.under_cursor();
        self.filter = None;
        let at = on
            .and_then(|k| self.focus_all().iter().position(|r| *r == k))
            .unwrap_or(0);
        self.goto(at);
    }

    fn flip(&mut self, side: Side) {
        self.drop_filter();
        if let Some(d) = &mut self.current_mut().dossier {
            d.side = side;
        }
    }

    fn tab_step(&mut self, by: isize) {
        self.drop_filter();
        let len = self.tabs.len() as isize;
        self.tab = (self.tab as isize + by).rem_euclid(len) as usize;
    }

    /// The `/` line has the keys while it is typed: they edit the query like a
    /// shell line, `↵` keeps it, and `esc` or `ctrl-c` drops it. `?` still
    /// opens help, since no package name holds one, and the arrows still move
    /// in the list beneath.
    fn filter_key(&mut self, key: KeyEvent) {
        if is_escape(key) || is_ctrl(key, 'c') {
            self.drop_filter();
            return;
        }
        match key.code {
            KeyCode::Enter => {
                if let Some(f) = &mut self.filter {
                    f.typing = false;
                    if f.query.is_empty() {
                        self.drop_filter();
                    }
                }
            }
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Down => self.move_by(1),
            KeyCode::Up => self.move_by(-1),
            _ => {
                let Some(f) = &mut self.filter else {
                    return;
                };
                let before = f.query.clone();
                if line_edit::edit(&mut f.query, &mut f.back, key) && f.query != before {
                    self.goto(0);
                }
            }
        }
    }

    // ── the graph ────────────────────────────────────────────────────────────

    fn graph_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(g) = &mut self.graph else {
            return;
        };
        match key.code {
            // Back out through the centres; once there is nothing left to
            // unwind, close the graph.
            _ if is_escape(key) => {
                if !g.back(&self.world) {
                    self.graph = None;
                }
            }
            // Back to the dossier, of whichever package is selected now.
            KeyCode::Char('g') if ctrl => {
                let pkg = g.selected_name().to_string();
                self.graph = None;
                if self.current().dossier.as_ref().map(|d| d.pkg.as_str()) != Some(pkg.as_str()) {
                    self.open(&pkg);
                }
            }
            KeyCode::Char('h') | KeyCode::Left => g.move_horizontal(-1),
            KeyCode::Char('l') | KeyCode::Right => g.move_horizontal(1),
            KeyCode::Char('k') | KeyCode::Up if !ctrl => g.move_vertical(-1),
            KeyCode::Char('j') | KeyCode::Down if !ctrl => g.move_vertical(1),
            KeyCode::Enter => g.recenter(&self.world),
            _ => {}
        }
    }

    /// The graph of the open dossier's package, or of the row under the
    /// cursor.
    fn open_graph(&mut self) {
        let target = match &self.current().dossier {
            Some(d) => Some(d.pkg.clone()),
            None => self.under_cursor(),
        };
        if let Some(t) = target {
            self.drop_filter();
            self.graph = Some(graph::GraphView::build(&self.world, &t));
        }
    }

    // ── boxes ────────────────────────────────────────────────────────────────

    fn open_help(&mut self) {
        self.overlay = Some(Overlay::Help {
            scroll: Cell::new(0),
            page: Cell::new(1),
        });
    }

    fn open_options(&mut self) {
        self.overlay = Some(match crate::config::load() {
            Ok(config) => Overlay::Options(Options::open(&config)),
            Err(e) => Overlay::Alert {
                title: "settings".into(),
                body: e,
            },
        });
    }

    /// A box owns every key while it is up: `esc` and `ctrl-c` close it, and
    /// a key that means nothing to it is swallowed.
    fn overlay_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let close = is_escape(key) || is_ctrl(key, 'c');
        match &mut self.overlay {
            Some(Overlay::Help { scroll, page }) => {
                if close || matches!(key.code, KeyCode::Char('?' | 'q')) {
                    self.overlay = None;
                    return;
                }
                let top = scroll.get();
                let page = page.get().max(1);
                let to = match key.code {
                    KeyCode::Char('d') if ctrl => top.saturating_add(page / 2),
                    KeyCode::Char('u') if ctrl => top.saturating_sub(page / 2),
                    KeyCode::PageDown => top.saturating_add(page),
                    KeyCode::PageUp => top.saturating_sub(page),
                    KeyCode::Char('j') | KeyCode::Down => top.saturating_add(1),
                    KeyCode::Char('k') | KeyCode::Up => top.saturating_sub(1),
                    KeyCode::Char('g') | KeyCode::Home => 0,
                    // `render_help` clamps it to the last screenful.
                    KeyCode::Char('G') | KeyCode::End => usize::MAX,
                    _ => top,
                };
                scroll.set(to);
            }
            Some(Overlay::Options(opts)) => match key.code {
                _ if close => self.close_options(),
                KeyCode::Char('j') | KeyCode::Down => opts.move_by(1),
                KeyCode::Char('k') | KeyCode::Up => opts.move_by(-1),
                KeyCode::Char('g') | KeyCode::Home => opts.move_by(isize::MIN / 2),
                KeyCode::Char('G') | KeyCode::End => opts.move_by(isize::MAX / 2),
                KeyCode::Enter => self.change_option(Options::toggle),
                KeyCode::Char('d') if !ctrl => self.change_option(Options::default),
                _ => {}
            },
            Some(Overlay::Alert { .. }) if close || key.code == KeyCode::Char('q') => {
                self.overlay = None;
            }
            Some(Overlay::Alert { .. }) | None => {}
        }
    }

    /// Apply `change` to the settings box and write the file, putting it back
    /// when the file cannot be written.
    fn change_option(&mut self, change: fn(&mut Options)) {
        let Some(Overlay::Options(opts)) = &mut self.overlay else {
            return;
        };
        let before = opts.skip.clone();
        change(opts);
        if opts.skip == before {
            return;
        }
        match crate::config::save(&opts.config()) {
            Ok(path) => self.set_status(format!("wrote {}", crate::config::shown(&path))),
            Err(e) => {
                opts.skip = before;
                self.set_failed(e);
            }
        }
    }

    /// Close the settings box. Only the sources turned on or off in it
    /// change for this session, so a run started with `--only` keeps the rest
    /// as it was, and the machine is read again so that a source turned off
    /// leaves the dossiers and the graph too.
    fn close_options(&mut self) {
        let Some(Overlay::Options(opts)) = self.overlay.take() else {
            return;
        };
        if !opts.changed() {
            return;
        }
        for source in Source::ALL {
            let was = !opts.opened.contains(&source);
            let now = !opts.skip.contains(&source);
            if was != now {
                self.sources.set(source, now);
            }
        }
        self.rebuild_tabs();
        self.reload(self.sources.clone());
    }
}

fn is_ctrl(key: KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(c)
}

/// `esc`, or `ctrl-[`: the same control byte historically, but the enhanced
/// keyboard protocol reports them separately.
fn is_escape(key: KeyEvent) -> bool {
    key.code == KeyCode::Esc || is_ctrl(key, '[')
}
