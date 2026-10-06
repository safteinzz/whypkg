//! The event loop: every key the browser answers to.

use super::*;
use crossterm::event::KeyEvent;

impl App {
    pub(crate) fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> io::Result<()> {
        loop {
            // The graph view, when open, takes over the whole screen and its
            // own keys until `esc`.
            if self.graph.is_some() {
                terminal.draw(|f| {
                    if let Some(g) = &self.graph {
                        g.render(f, f.area());
                    }
                    if self.help {
                        render::render_help(f, f.area(), self);
                    }
                })?;
                let Event::Key(key) = event::read()? else {
                    continue;
                };
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                if self.help {
                    self.help_key(key);
                    continue;
                }
                if is_ctrl(key, 'c') {
                    return Ok(());
                }
                let graph_back = is_escape(key);

                // `ctrl-g` flips back to the dossier, for whatever package you
                // navigated to in the graph, not the one you came in on.
                let to_dossier = is_ctrl(key, 'g');

                match key.code {
                    // Back out through the graph history; once there's nothing
                    // left to unwind, close the graph.
                    _ if graph_back => {
                        let unwound = self
                            .graph
                            .as_mut()
                            .map(|g| g.back(&self.world))
                            .unwrap_or(false);
                        if !unwound {
                            self.graph = None;
                        }
                    }
                    _ if to_dossier => {
                        let target = self.graph.as_ref().map(|g| g.selected_name().to_string());
                        self.graph = None;
                        // If we dug somewhere new, open that package's dossier;
                        // if we never moved, closing already lands us on it.
                        if let Some(t) = target
                            && self.frame().focus.as_deref() != Some(t.as_str())
                        {
                            self.open(t);
                        }
                    }
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('?') => self.open_help(),
                    KeyCode::Enter => {
                        if let Some(g) = &mut self.graph {
                            g.recenter(&self.world);
                        }
                    }
                    // Column-aware nav: h/l (and ←/→) cross columns, j/k (and
                    // ↓/↑) move within a column.
                    KeyCode::Left | KeyCode::Char('h') => {
                        if let Some(g) = &mut self.graph {
                            g.move_horizontal(-1);
                        }
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        if let Some(g) = &mut self.graph {
                            g.move_horizontal(1);
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if let Some(g) = &mut self.graph {
                            g.move_vertical(-1);
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if let Some(g) = &mut self.graph {
                            g.move_vertical(1);
                        }
                    }
                    _ => {}
                }
                continue;
            }

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

            if self.help {
                self.help_key(key);
                continue;
            }

            if self.typing {
                self.query_key(key, visible.len());
                continue;
            }
            if is_ctrl(key, 'c') {
                return Ok(());
            }

            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                // Out one step: a kept filter first, then the dossier. The list
                // is the top, so `esc` never quits; `q` does.
                _ if is_escape(key) => {
                    if !self.frame().query.is_empty() {
                        let frame = self.frame_mut();
                        frame.query.clear();
                        frame.query_back = 0;
                        frame.selected = 0;
                    } else if self.stack.len() > 1 {
                        self.stack.pop();
                    }
                }
                KeyCode::Char('q') if !ctrl => return Ok(()),
                KeyCode::Char('?') => self.open_help(),
                // A new search, as in every other app: the kept query goes.
                KeyCode::Char('/') => {
                    self.typing = true;
                    let frame = self.frame_mut();
                    frame.query.clear();
                    frame.query_back = 0;
                    frame.selected = 0;
                }
                KeyCode::Enter => {
                    if let Some(pkg) = visible.get(self.frame().selected).cloned() {
                        self.open(pkg);
                    }
                }
                KeyCode::Tab => self.step_filter(),
                KeyCode::Char('j') | KeyCode::Down => self.move_selection(1, visible.len()),
                KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1, visible.len()),
                // In a dossier, flip the list between "what needs it" and "what
                // it needs", the way the arrows switch tabs everywhere else.
                KeyCode::Char('h' | 'l') | KeyCode::Left | KeyCode::Right
                    if self.frame().focus.is_some() =>
                {
                    self.relation = self.relation.toggle();
                    self.frame_mut().selected = 0;
                }
                KeyCode::Char('g') if ctrl => {
                    // Open the graph view centred on the focused package, or
                    // the highlighted row at the root.
                    let target = self
                        .frame()
                        .focus
                        .clone()
                        .or_else(|| visible.get(self.frame().selected).cloned());
                    if let Some(t) = target {
                        self.graph = Some(graph::GraphView::build(&self.world, &t));
                    }
                }
                _ => {}
            }
        }
    }

    /// The `/` line has the keys: they edit the query like a shell line, `↵`
    /// keeps it, and `esc` or ctrl-c drops it. `?` still opens help, since no
    /// package name holds one, and the arrows still move the list beneath.
    fn query_key(&mut self, key: KeyEvent, len: usize) {
        if is_escape(key) || is_ctrl(key, 'c') {
            self.typing = false;
            let frame = self.frame_mut();
            frame.query.clear();
            frame.query_back = 0;
            frame.selected = 0;
            return;
        }
        match key.code {
            KeyCode::Enter => self.typing = false,
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Down => self.move_selection(1, len),
            KeyCode::Up => self.move_selection(-1, len),
            _ => {
                let frame = self.frame_mut();
                let before = frame.query.clone();
                if line_edit::edit(&mut frame.query, &mut frame.query_back, key)
                    && frame.query != before
                {
                    frame.selected = 0;
                }
            }
        }
    }

    /// All, manual, auto, flatpak and round again, skipping flatpak on a
    /// machine with no flatpak apps.
    fn step_filter(&mut self) {
        self.filter = self.filter.next();
        if self.filter == FilterMode::Flatpak && !self.has_flatpak() {
            self.filter = self.filter.next();
        }
        self.frame_mut().selected = 0;
    }

    fn open_help(&mut self) {
        self.help = true;
        self.help_scroll.set(0);
    }

    /// The help panel owns every key while it is up: it scrolls, `esc`,
    /// `ctrl-c` and the keys that open it close it, and anything else is
    /// swallowed.
    fn help_key(&mut self, key: KeyEvent) {
        if is_escape(key)
            || is_ctrl(key, 'c')
            || matches!(key.code, KeyCode::Char('?') | KeyCode::Char('q'))
        {
            self.help = false;
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let half = 10;
        let top = self.help_scroll.get();
        let to = match key.code {
            KeyCode::Char('d') if ctrl => top.saturating_add(half),
            KeyCode::Char('u') if ctrl => top.saturating_sub(half),
            KeyCode::PageDown => top.saturating_add(half),
            KeyCode::PageUp => top.saturating_sub(half),
            KeyCode::Char('j') | KeyCode::Down => top.saturating_add(1),
            KeyCode::Char('k') | KeyCode::Up => top.saturating_sub(1),
            KeyCode::Char('g') | KeyCode::Home => 0,
            // `render_help` clamps this to the last screenful.
            KeyCode::Char('G') | KeyCode::End => usize::MAX,
            _ => top,
        };
        self.help_scroll.set(to);
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
