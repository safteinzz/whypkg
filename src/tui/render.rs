//! Drawing the browser: the package list, the dossier and its relation panes.

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use super::widgets::{
    BACK, FIND, HELP, KEEP, QUIT, READER_KEYS, box_area, box_block, box_height, box_hint,
    box_width, key_footer, vscrollbar,
};
use super::*;

/// The root list's footer.
const LIST_KEYS: &[&str] = &["↵ open", "tab filter", GRAPH_KEY, FIND, QUIT];
/// A package's footer: the list's, one level down.
const DOSSIER_KEYS: &[&str] = &["↵ open", "tab filter", GRAPH_KEY, BACK, FIND, QUIT];

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
        // The list has no border, so a scrollbar takes its last column rather
        // than drawing over the end of a description.
        let view = chunks[2].height as usize;
        let list_area = if visible.len() > view {
            Rect {
                width: chunks[2].width.saturating_sub(1),
                ..chunks[2]
            }
        } else {
            chunks[2]
        };
        f.render_stateful_widget(list, list_area, &mut state);
        // One row taller at each end, since `vscrollbar` keeps off a border
        // row there and this list has none.
        let edge = Rect {
            y: chunks[2].y.saturating_sub(1),
            height: chunks[2].height + 2,
            ..chunks[2]
        };
        vscrollbar(f, edge, visible.len(), state.offset(), view);

        // Query input line, with the active manual/auto filter shown on the right.
        let mode_span = if self.filter == FilterMode::All {
            Span::styled("  showing: all", Style::new().dim())
        } else {
            Span::styled(
                format!("  showing: {}", self.filter.label()),
                Style::new().cyan(),
            )
        };
        // The `/` query: with its cursor and the match count while it is
        // typed, as plain text while it is kept, and nothing until then.
        let mut query = Vec::new();
        if self.typing {
            query.push(Span::styled("  /", Style::new().cyan()));
            query.extend(line_edit::with_cursor(
                &frame.query,
                frame.query_back,
                Style::new(),
            ));
            query.push(Span::styled(
                format!("   {} match", visible.len()),
                Style::new().dim(),
            ));
        } else if !frame.query.is_empty() {
            query.push(Span::raw(format!("  /{}", frame.query)));
        }
        query.push(mode_span);
        f.render_widget(Paragraph::new(Line::from(query)), chunks[3]);

        // While the query is typed, its own two keys; while it is kept, it
        // leads the footer with the `esc` that drops it.
        let kept = format!("/{}", frame.query);
        let keys: Vec<&str> = if self.typing {
            vec![KEEP, BACK]
        } else {
            let base = if has_dossier { DOSSIER_KEYS } else { LIST_KEYS };
            let mut keys = Vec::new();
            if !frame.query.is_empty() {
                keys.push(kept.as_str());
                keys.push(BACK);
            }
            keys.extend(
                base.iter()
                    .filter(|k| frame.query.is_empty() || **k != BACK)
                    .copied(),
            );
            keys
        };
        f.render_widget(
            Paragraph::new(key_footer(&keys, HELP, chunks[4].width)),
            chunks[4],
        );

        if self.help {
            render_help(f, f.area(), self);
        }
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

/// One group of the help panel: a heading, then `(keys, what they do)` rows,
/// where a row with no keys is a note about the group.
type HelpSection = (&'static str, &'static [(&'static str, &'static str)]);

/// Every key the browser and the graph answer to, grouped by where it works.
/// The panel scrolls, so a new row costs nothing but its line.
const HELP_ROWS: &[HelpSection] = &[
    (
        "list",
        &[
            ("j/k ↑↓", "move"),
            ("↵", "open the package"),
            ("/", "filter by name and description"),
            ("tab", "the next filter: all, manual, auto, flatpak"),
            ("ctrl-g", "graph"),
            ("esc", "drop the filter"),
            ("q ctrl-c", "quit"),
            ("?", "this help"),
            (
                "",
                "[M] yours · [A] pulled in · [F] flatpak · ↑ upgrade waiting",
            ),
        ],
    ),
    (
        "package",
        &[
            ("h/l ←→", "needed by or depends on"),
            ("esc", "back, or drop the filter first"),
            ("", "the rest as the list"),
        ],
    ),
    (
        "typing a filter",
        &[
            ("←→ ctrl-a/e", "move in it, like a shell line"),
            (
                "ctrl-w ctrl-u",
                "erase a word, everything before the cursor",
            ),
            ("↑↓", "move in the list beneath"),
            ("↵", "keep it"),
            ("esc ctrl-c", "drop it"),
        ],
    ),
    (
        "graph",
        &[
            ("h/l ←→", "change side"),
            ("j/k ↑↓", "move in a side, paging past its end"),
            ("↵", "centre on it"),
            ("esc", "one step back"),
            ("ctrl-g", "open its package"),
            ("q", "quit"),
            ("?", "this help"),
            ("ctrl-c", "quit"),
        ],
    ),
    (
        "in this help",
        &[
            ("j/k ↑↓", "scroll"),
            ("ctrl-d ctrl-u", "half a page down, up"),
            ("g G", "the top, the bottom"),
            ("esc q ?", "close"),
        ],
    ),
];

/// The width of the key column, so every description starts in one place.
const HELP_KEYS: usize = 16;

pub(super) fn help_lines() -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (section, entries) in HELP_ROWS {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(*section, Style::new().cyan().bold()));
        for (keys, what) in *entries {
            if keys.is_empty() {
                lines.push(Line::styled(format!("  {what}"), Style::new().dim()));
                continue;
            }
            lines.push(Line::from(vec![
                Span::styled(format!("  {keys:<HELP_KEYS$}"), Style::new().yellow()),
                Span::raw(*what),
            ]));
        }
    }
    lines
}

/// The help reader over `area`: the body scrolls under a key row that never
/// moves, with a scrollbar on the right border once it is taller than the box.
/// Both screens end in a footer row, so the box stops above it.
pub(super) fn render_help(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let area = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    let lines = help_lines();
    let width = box_width(area.width);
    // The body, then a blank and the key row.
    let rect = box_area(area, width, box_height(lines.len() as u16 + 2, area.height));
    f.render_widget(Clear, rect);
    let block = box_block(Color::Cyan, "help");
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let shown = inner.height.saturating_sub(2) as usize;
    // Clamped here, where the height is known, so scrolling past the end never
    // piles up presses that then take as many to undo.
    let top = app.help_scroll.get().min(lines.len().saturating_sub(shown));
    app.help_scroll.set(top);
    let body = Rect {
        height: shown as u16,
        ..inner
    };
    f.render_widget(Paragraph::new(lines[top..].to_vec()), body);
    let keys = Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: 1,
        ..inner
    };
    f.render_widget(Paragraph::new(box_hint(READER_KEYS)), keys);
    if lines.len() > shown {
        vscrollbar(f, rect, lines.len(), top, shown);
    }
}
