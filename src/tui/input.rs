//! The event loop: every key the browser answers to.

use super::*;

impl App {
    pub(crate) fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> io::Result<()> {
        loop {
            // The graph view, when open, takes over the whole screen and its
            // own keys until Esc.
            if self.graph.is_some() {
                terminal.draw(|f| {
                    if let Some(g) = &self.graph {
                        g.render(f, f.area());
                    }
                })?;
                let Event::Key(key) = event::read()? else {
                    continue;
                };
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('c'))
                {
                    return Ok(());
                }
                // Ctrl+[ is Esc; treat both as "step back".
                let graph_back = matches!(key.code, KeyCode::Esc)
                    || (key.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(key.code, KeyCode::Char('[')));

                // Ctrl+G flips back to the dossier - for whatever package you
                // navigated to in the graph, not the one you came in on.
                let to_dossier = key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('g'));

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
                    // q always leaves the graph outright.
                    KeyCode::Char('q') => self.graph = None,
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

            // Ctrl-C always quits.
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c'))
            {
                return Ok(());
            }

            // Esc, or Ctrl+[ - the same control byte historically, but the
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
                    // Cycle all → manual → auto → flatpak; skip the flatpak
                    // bucket entirely on machines with no flatpak apps.
                    self.filter = self.filter.next();
                    if self.filter == FilterMode::Flatpak && !self.has_flatpak() {
                        self.filter = self.filter.next();
                    }
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
                    // vim-style: Ctrl-j/k move, Ctrl-h/l flip the relation
                    // (Ctrl-l works everywhere; Ctrl-h only where the terminal
                    // sends it distinct from Backspace, e.g. kitty protocol).
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        match c {
                            'j' | 'n' => self.move_selection(1, visible.len()),
                            'k' | 'p' => self.move_selection(-1, visible.len()),
                            'h' | 'l' if self.frame().focus.is_some() => {
                                self.relation = self.relation.toggle();
                                self.frame_mut().selected = 0;
                            }
                            'g' => {
                                // Open the graph view centred on the focused
                                // package, or the highlighted row at the root.
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
                    } else {
                        self.frame_mut().query.push(c);
                        self.frame_mut().selected = 0;
                    }
                }
                _ => {}
            }
        }
    }
}
