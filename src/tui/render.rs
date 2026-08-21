//! Drawing the browser: the package list, the dossier and its relation panes.

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use super::*;

/// Truncate to at most `max` characters (UTF-8 safe - never splits a char,
/// unlike the byte-based `substr`/`:0:n` the bash version used).
/// Naive English pluralization for counts: `plural(1, "package")` -> "package",
/// `plural(3, "package")` -> "packages".
pub(super) fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        word.to_string()
    } else {
        format!("{word}s")
    }
}

pub(super) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

// ── terminal lifecycle ────────────────────────────────────────────────────────

impl App {
    pub(crate) fn render(&self, f: &mut ratatui::Frame, visible: &[String]) {
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
                Constraint::Length(1), // breadcrumb
                Constraint::Length(if has_dossier {
                    dossier_lines.len() as u16 + 2
                } else {
                    0
                }),
                Constraint::Min(3),    // list
                Constraint::Length(1), // query input
                Constraint::Length(1), // help
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
            "Enter open · Esc back · ←/→ needs-it / it-needs · Tab filter · Ctrl-G graph · Ctrl-C quit"
        } else {
            "type to filter · Tab filter · Enter open · Ctrl-G graph · Esc quit"
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(help, Style::new().dim()))),
            chunks[4],
        );
    }

    /// The styled info block shown above a package's navigation list. Reads the
    /// frame's cached `origin`/`alongside` so it's cheap to redraw every frame.
    pub(crate) fn dossier_lines(&self, frame: &Frame) -> Vec<Line<'static>> {
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

        // A "needed by: nothing" package is normally safe to remove - but never
        // say that about kernel/firmware, which nothing "depends on" yet must
        // not be touched.
        let needed_by_text = if needed_by == 0 {
            if crate::engine::is_kernel_pkg(pkg) {
                "nothing - but kernel/firmware, do not remove".to_string()
            } else {
                "nothing - safe to remove".to_string()
            }
        } else {
            format!("{needed_by} {}", plural(needed_by, "package"))
        };

        let kv = |k: &str, v: Span<'static>| -> Line<'static> {
            Line::from(vec![
                Span::styled(format!("  {k:<12}"), Style::new().dim()),
                v,
            ])
        };
        // A key/value line whose value is several styled spans (e.g. the origin).
        let kv_spans = |k: &str, mut spans: Vec<Span<'static>>| -> Line<'static> {
            let mut out = vec![Span::styled(format!("  {k:<12}"), Style::new().dim())];
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
                vec![Span::styled(
                    "auto-installed (origin untraced)",
                    Style::new().yellow(),
                )]
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

        // A trimmed extended description under the synopsis, when the manager
        // has one - this is what tells you `code` is actually Visual Studio Code.
        if let Some(d) = p.and_then(|p| p.details.as_deref()) {
            let text = truncate(d, 100);
            lines.insert(2, Line::from(Span::styled(format!("  {text}"), dim)));
        }

        // "alongside" sits high - it's context for *why here*: a few example
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

        // Where it came from: a repo, a sideloaded local file, or an orphan
        // whose repo is gone. (`crate::model::Origin` by full path - this
        // module has its own `Origin` for the "why here" trace.)
        let source = if p.map(|p| p.source) == Some(crate::model::Source::Flatpak) {
            // Show the remote it came from, e.g. "flatpak (flathub)".
            let text = match p.and_then(|p| p.remote.as_deref()) {
                Some(remote) => format!("flatpak ({remote})"),
                None => "flatpak app".to_string(),
            };
            Some(Span::styled(text, Style::new().blue()))
        } else {
            match p.map(|p| p.origin) {
                Some(crate::model::Origin::Repo) => {
                    Some(Span::styled("from a repository", Style::new().green()))
                }
                Some(crate::model::Origin::Local) => Some(Span::styled(
                    "installed from a local file",
                    Style::new().yellow(),
                )),
                Some(crate::model::Origin::Orphaned) => Some(Span::styled(
                    "not in any repo (repo removed?)",
                    Style::new().red(),
                )),
                _ => None,
            }
        };
        if let Some(span) = source {
            lines.push(kv("source", span));
        }

        // The two relations are separate lists, one shown at a time (toggle with
        // ←/→). Mark whichever is active so it's always clear which packages the
        // list below holds - even if that side happens to be empty.
        let showing = || Span::styled("  ← showing below", Style::new().bold().cyan());
        let mut needed = vec![Span::raw(needed_by_text)];
        if self.relation == Relation::NeededBy {
            needed.push(showing());
        }
        lines.push(kv_spans("needed by", needed));

        let mut depends = vec![Span::raw(format!(
            "{depends_on} {}",
            plural(depends_on, "package")
        ))];
        if self.relation == Relation::DependsOn {
            depends.push(showing());
        }
        lines.push(kv_spans("depends on", depends));
        lines
    }

    /// One styled row in the package list: tag, name, upgrade arrow, size, desc.
    pub(crate) fn pkg_line(&self, name: &str) -> Line<'static> {
        let manual = self.world.is_manual(name);
        let is_flatpak =
            self.world.packages.get(name).map(|p| p.source) == Some(crate::model::Source::Flatpak);
        // Flatpak apps get their own tag; system packages show manual/auto.
        let tag = if is_flatpak {
            Span::styled("[F]", Style::new().blue())
        } else if manual {
            Span::styled("[M]", Style::new().green())
        } else {
            Span::styled("[A]", Style::new().yellow())
        };
        let up = if self.world.is_upgradable(name) {
            Span::styled("↑", Style::new().cyan())
        } else {
            Span::raw(" ")
        };
        let size = format_size(
            self.world
                .packages
                .get(name)
                .map(|p| p.installed_size)
                .unwrap_or(0),
        );
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
