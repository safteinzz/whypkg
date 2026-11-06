//! Drawing the browser: the tab line, a list or a dossier, the status line,
//! the footer, and whatever box is open over them.

use super::widgets::{
    BACK, CENTRE, FIND, GRAPH, HELP, KEEP, OPEN, OPTIONS_KEYS, QUIT, READER_KEYS, REFRESH, SEP,
    SHOW, box_area, box_block, box_height, box_hint, box_width, key_footer, vscrollbar,
    wrapped_line_count,
};
use super::*;
use crate::engine::{format_size, is_kernel_pkg, relative_time};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs, Wrap};

/// The cursor row of the focused list.
const CURSOR: Color = Color::Indexed(54);
/// The cursor row of a list that is not focused, so it is still findable.
const RESTING: Color = Color::Indexed(236);
/// The status line's background.
const STATUS: Color = Color::Indexed(236);

/// The list's footer.
const LIST_KEYS: &[&str] = &[OPEN, SHOW, GRAPH, FIND, REFRESH, QUIT];
/// A package's footer: the list's, one level down.
const DOSSIER_KEYS: &[&str] = &[OPEN, GRAPH, BACK, FIND, REFRESH, QUIT];
/// The graph's footer. `ctrl-g` closes it onto the selected package.
const GRAPH_KEYS: &[&str] = &[CENTRE, GRAPH, BACK, REFRESH, QUIT];

/// Truncate to at most `max` characters, never splitting one.
pub(super) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

/// `s` cut or padded to exactly `width` characters.
fn fit(s: &str, width: usize) -> String {
    format!("{:<width$}", truncate(s, width))
}

/// The mark a package wears in every list and in the help panel.
fn mark(world: &World, key: &str) -> Span<'static> {
    if world.is_manual(key) {
        Span::styled("●", Style::new().green())
    } else {
        Span::styled("○", Style::new().yellow())
    }
}

/// Keep the cursor on screen: move `top` the least it takes.
fn follow(top: &Cell<usize>, cursor: usize, len: usize, view: usize) -> usize {
    let view = view.max(1);
    let mut t = top.get().min(len.saturating_sub(view));
    if cursor < t {
        t = cursor;
    } else if cursor >= t + view {
        t = cursor + 1 - view;
    }
    top.set(t);
    t
}

impl App {
    pub(crate) fn render(&self, f: &mut ratatui::Frame) {
        let area = f.area();
        let rows = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
        self.render_tabline(f, rows[0]);
        if let Some(g) = &self.graph {
            g.render(f, rows[1]);
        } else {
            let tab = self.current();
            match &tab.dossier {
                Some(d) => self.render_dossier(f, rows[1], d),
                None => self.render_list(f, rows[1], tab),
            }
        }
        self.render_status(f, rows[2]);
        self.render_footer(f, rows[3]);

        // A box stops above the status line, so what a change in it did
        // stays in sight.
        let under = Rect {
            height: area.height.saturating_sub(2),
            ..area
        };
        match &self.overlay {
            Some(Overlay::Help { scroll, page }) => render_help(f, under, scroll, page),
            Some(Overlay::Options(opts)) => self.render_options(f, under, opts),
            Some(Overlay::Alert { title, body }) => render_alert(f, under, title, body),
            None => {}
        }
    }

    /// The bar on top: the app's name on its border, and a tab per installer.
    fn render_tabline(&self, f: &mut ratatui::Frame, area: Rect) {
        let names: Vec<&str> = self.tabs.iter().map(Tab::name).collect();
        let tabs = Tabs::new(names)
            .select(self.tab)
            .block(Block::default().borders(Borders::ALL).title(" whypkg "))
            .divider("│")
            .highlight_style(Style::new().cyan().bold());
        f.render_widget(tabs, area);
    }

    fn render_list(&self, f: &mut ratatui::Frame, area: Rect, tab: &Tab) {
        let rows = self.focus_rows();
        let (shown, total) = (rows.len(), tab.rows.len());
        let title = if shown == total {
            format!(" {} ({total}) ", tab.name())
        } else {
            format!(" {} ({shown}/{total}) ", tab.name())
        };
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let view = inner.height as usize;
        self.view.set(view);
        if rows.is_empty() {
            let text = if let Some(filter) = self.filter.as_ref().filter(|f| !f.query.is_empty()) {
                format!("nothing matches /{}", filter.query)
            } else if self.show == Show::All {
                "nothing installed here".to_string()
            } else {
                format!("nothing {} here: s steps on to the rest", self.show.word())
            };
            f.render_widget(
                Paragraph::new(Span::styled(format!(" {text}"), Style::new().dim())),
                inner,
            );
            return;
        }
        let top = follow(&tab.top, tab.cursor, rows.len(), view);
        let width = inner.width as usize;
        let lines: Vec<Line> = rows
            .iter()
            .enumerate()
            .skip(top)
            .take(view)
            .map(|(i, key)| {
                let line = self.row(key, width, tab.source.is_none());
                if i == tab.cursor {
                    line.style(Style::new().bg(CURSOR).bold())
                } else {
                    line
                }
            })
            .collect();
        f.render_widget(Paragraph::new(lines), inner);
        vscrollbar(f, area, rows.len(), top, view);
    }

    /// The short answer to "why is this here" a row shows: `you`, `← root`
    /// for what pulled it in, or `auto` when nothing you installed needs it.
    fn why_word(&self, key: &str) -> Span<'static> {
        match self.why_of(key) {
            Why::You => Span::styled("you", Style::new().green()),
            Why::PulledIn(chain) => {
                let root = chain
                    .first()
                    .map(|r| self.world.label(r))
                    .unwrap_or_default();
                Span::styled(format!("← {root}"), Style::new().yellow())
            }
            Why::Untraced => Span::styled("auto", Style::new().yellow().dim()),
        }
    }

    /// One list row: mark, name, (source), why, version, size, age, then the
    /// description in whatever width is left.
    fn row(&self, key: &str, width: usize, with_source: bool) -> Line<'static> {
        let Some(p) = self.world.packages.get(key) else {
            return Line::raw(key.to_string());
        };
        let why = self.why_word(key);
        let why_text = truncate(&why.content, 22);
        let mut spans = vec![
            mark(&self.world, key),
            Span::raw(" "),
            Span::raw(fit(p.label(), 28)),
            Span::raw("  "),
        ];
        if with_source {
            spans.push(Span::styled(fit(p.source.word(), 7), Style::new().blue()));
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(format!("{why_text:<22}"), why.style));
        spans.push(Span::raw("  "));
        spans.push(Span::styled(fit(&p.version, 14), Style::new().dim()));
        spans.push(if p.candidate.is_some() {
            Span::styled("↑", Style::new().cyan())
        } else {
            Span::raw(" ")
        });
        spans.push(Span::raw(format!(" {:>9}", format_size(p.installed_size))));
        let age = p.install_epoch.map(relative_time).unwrap_or_default();
        spans.push(Span::styled(
            format!("  {}", fit(&age, 14)),
            Style::new().dim(),
        ));
        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        if width > used + 8 && !p.description.is_empty() {
            spans.push(Span::styled(
                format!("  {}", truncate(&p.description, width - used - 2)),
                Style::new().dim(),
            ));
        }
        Line::from(spans)
    }

    fn render_dossier(&self, f: &mut ratatui::Frame, area: Rect, d: &Dossier) {
        let lines = self.dossier_lines(d);
        let parts = Layout::vertical([
            Constraint::Length((lines.len() as u16 + 2).min(area.height.saturating_sub(5))),
            Constraint::Min(3),
        ])
        .split(area);
        let title = self.world.label(&d.pkg).to_string();
        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().dark_gray())
                    .title(format!(" {title} ")),
            ),
            parts[0],
        );

        let halves = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(parts[1]);
        let titles = ["needed by", "depends on"];
        for side in [Side::NeededBy, Side::DependsOn] {
            let i = side.index();
            let focused = d.side == side;
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(if focused {
                    Style::new().cyan()
                } else {
                    Style::new().dark_gray()
                })
                .title(format!(" {} {} ", titles[i], d.lists[i].len()));
            let inner = block.inner(halves[i]);
            f.render_widget(block, halves[i]);
            let view = inner.height as usize;
            if focused {
                self.view.set(view);
            }
            let filtered: Vec<Link>;
            let list: &[Link] = if focused && self.filter.is_some() {
                filtered = d.lists[i]
                    .iter()
                    .filter(|l| self.kept(&l.key))
                    .cloned()
                    .collect();
                &filtered
            } else {
                &d.lists[i]
            };
            if list.is_empty() {
                let note = if !d.lists[i].is_empty() {
                    "nothing matches"
                } else if side == Side::NeededBy && is_kernel_pkg(&d.pkg) {
                    "nothing, but it is kernel or firmware"
                } else {
                    "nothing"
                };
                f.render_widget(
                    Paragraph::new(Span::styled(format!(" {note}"), Style::new().dim())),
                    inner,
                );
                continue;
            }
            let top = follow(&d.top[i], d.cursor[i], list.len(), view);
            let width = inner.width as usize;
            let rows: Vec<Line> = list
                .iter()
                .enumerate()
                .skip(top)
                .take(view)
                .map(|(row, link)| {
                    let line = self.link_row(link, side, width);
                    if row != d.cursor[i] {
                        line
                    } else if focused {
                        line.style(Style::new().bg(CURSOR).bold())
                    } else {
                        line.style(Style::new().bg(RESTING))
                    }
                })
                .collect();
            f.render_widget(Paragraph::new(rows), inner);
            vscrollbar(f, halves[i], list.len(), top, view);
        }
    }

    /// One row of a dossier list: mark, name, its installer when that is not
    /// the system, and `(uses)` on a link that is not a dependency.
    fn link_row(&self, link: &Link, side: Side, width: usize) -> Line<'static> {
        let p = self.world.packages.get(&link.key);
        let mut spans = vec![
            Span::raw(" "),
            mark(&self.world, &link.key),
            Span::raw(" "),
            Span::raw(self.world.label(&link.key).to_string()),
        ];
        if let Some(p) = p
            && p.source != Source::System
        {
            spans.push(Span::styled(
                format!("  {}", p.source.word()),
                Style::new().blue(),
            ));
        }
        if link.uses {
            let tag = match side {
                Side::NeededBy => "  uses it",
                Side::DependsOn => "  uses",
            };
            spans.push(Span::styled(tag, Style::new().dim()));
        }
        if p.is_some_and(|p| p.candidate.is_some()) {
            spans.push(Span::styled("  ↑", Style::new().cyan()));
        }
        let line = Line::from(spans);
        if line.width() > width {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            Line::raw(truncate(&text, width))
        } else {
            line
        }
    }

    /// The facts above a dossier's two lists.
    fn dossier_lines(&self, d: &Dossier) -> Vec<Line<'static>> {
        let pkg = d.pkg.as_str();
        let Some(p) = self.world.packages.get(pkg) else {
            return Vec::new();
        };
        let dim = Style::new().dim();
        let kv = |k: &str, spans: Vec<Span<'static>>| -> Line<'static> {
            let mut out = vec![Span::styled(format!(" {k:<11}"), dim)];
            out.extend(spans);
            Line::from(out)
        };

        let mut head = vec![
            Span::styled(format!(" {}", p.label()), Style::new().bold().white()),
            Span::styled(format!("  {}", p.version), dim),
        ];
        if let Some(c) = &p.candidate {
            head.push(Span::styled(format!("  ↑ {c}"), Style::new().cyan()));
        }
        if p.label() != pkg {
            head.push(Span::styled(format!("   {pkg}"), dim));
        }
        let mut lines = vec![Line::from(head)];
        if !p.description.is_empty() {
            lines.push(Line::from(Span::styled(format!(" {}", p.description), dim)));
        }
        if let Some(details) = &p.details {
            lines.push(Line::from(Span::styled(
                format!(" {}", truncate(details, 100)),
                dim,
            )));
        }

        match self.why_of(pkg) {
            Why::You => lines.push(kv(
                "why here",
                vec![Span::styled("you installed this", Style::new().green())],
            )),
            Why::PulledIn(chain) => {
                let root = chain
                    .first()
                    .map(|r| self.world.label(r))
                    .unwrap_or_default();
                lines.push(kv(
                    "why here",
                    vec![
                        Span::styled("pulled in by ", Style::new().yellow()),
                        Span::styled(root.to_string(), Style::new().bold().yellow()),
                    ],
                ));
                if chain.len() > 2 {
                    let names: Vec<&str> = chain.iter().map(|k| self.world.label(k)).collect();
                    lines.push(kv("chain", vec![Span::raw(names.join(" → "))]));
                }
            }
            Why::Untraced => lines.push(kv(
                "why here",
                vec![Span::styled(
                    "auto-installed, and nothing you installed needs it",
                    Style::new().yellow(),
                )],
            )),
        }
        if !d.alongside.is_empty() {
            let shown: Vec<&str> = d.alongside.iter().take(3).map(String::as_str).collect();
            let mut text = shown.join(", ");
            if d.alongside.len() > shown.len() {
                text.push_str(&format!(", +{} more", d.alongside.len() - shown.len()));
            }
            lines.push(kv("alongside", vec![Span::raw(text)]));
        }
        let installed = match (&p.install_date, p.install_epoch) {
            (Some(date), Some(epoch)) => format!("{date} ({})", relative_time(epoch)),
            (Some(date), None) => date.clone(),
            _ => "unknown".to_string(),
        };
        lines.push(kv("installed", vec![Span::raw(installed)]));
        lines.push(kv("size", vec![Span::raw(format_size(p.installed_size))]));
        if let Some(text) = crate::commands::why::source_text(&self.world, pkg) {
            let style = match (p.source, p.origin) {
                (Source::System, crate::model::Origin::Repo) => Style::new().green(),
                (Source::System, crate::model::Origin::Orphaned) => Style::new().red(),
                (_, crate::model::Origin::Local) => Style::new().yellow(),
                _ => Style::new().blue(),
            };
            lines.push(kv("source", vec![Span::styled(text, style)]));
        }
        let used_by = self.world.used_by_of(pkg);
        if !used_by.is_empty() {
            let mut names: Vec<String> = used_by
                .iter()
                .map(|k| format!("{} ({})", self.world.label(k), self.world.source(k).word()))
                .collect();
            names.sort();
            lines.push(kv(
                "used by",
                vec![Span::styled(
                    format!("{}, so removing it breaks them", names.join(", ")),
                    Style::new().yellow(),
                )],
            ));
        }
        if self.world.rdeps_of(pkg).is_empty()
            && !is_kernel_pkg(pkg)
            && let Some(cmd) = self.world.remove_command(pkg)
        {
            lines.push(kv("remove", vec![Span::styled(cmd, dim)]));
        }
        lines
    }

    fn render_status(&self, f: &mut ratatui::Frame, area: Rect) {
        let tab = self.current();
        let message = self
            .message
            .as_ref()
            .filter(|m| m.at.elapsed() < MESSAGE_TTL);
        let (left, style) = match message {
            Some(m) => {
                let colour = match m.tone {
                    Tone::Done => Color::Green,
                    Tone::Failed => Color::Yellow,
                };
                (format!(" {}", m.text), Style::new().bg(STATUS).fg(colour))
            }
            None => {
                let mut crumb = format!(" {}", tab.name());
                if let Some(d) = &tab.dossier {
                    for (pkg, _, _) in &d.back {
                        crumb.push_str(&format!(" › {}", self.world.label(pkg)));
                    }
                    crumb.push_str(&format!(" › {}", self.world.label(&d.pkg)));
                }
                if let Some(g) = &self.graph {
                    crumb.push_str(&format!(" › graph › {}", self.world.label(g.center_name())));
                }
                (crumb, Style::new().bg(STATUS).fg(Color::White))
            }
        };

        let mut right = Vec::new();
        if self.show != Show::All && tab.dossier.is_none() && self.graph.is_none() {
            right.push(format!("show {}", self.show.word()));
        }
        if self.graph.is_none() {
            let len = self.focus_rows().len();
            let at = if len == 0 { 0 } else { self.focus_cursor() + 1 };
            right.push(format!("{at}/{len}"));
        }
        let right = format!("{} ", right.join("   "));
        let pad =
            (area.width as usize).saturating_sub(left.chars().count() + right.chars().count());
        let line = Line::from(vec![
            Span::raw(format!("{left}{}", " ".repeat(pad.max(1)))),
            Span::styled(right, Style::new().fg(Color::White)),
        ]);
        f.render_widget(Paragraph::new(line).style(style), area);
    }

    /// The bottom row: the `/` query while it is typed, else the keys this
    /// screen answers to, led by the reading mark and a kept filter.
    fn render_footer(&self, f: &mut ratatui::Frame, area: Rect) {
        if let Some(filter) = self.filter.as_ref().filter(|f| f.typing) {
            let mut spans = vec![Span::styled(" /", Style::new().cyan())];
            spans.extend(line_edit::with_cursor(
                &filter.query,
                filter.back,
                Style::new(),
            ));
            spans.push(Span::styled(
                format!("   {} match   {KEEP}{SEP}{BACK}", self.focus_rows().len()),
                Style::new().dim(),
            ));
            f.render_widget(Paragraph::new(Line::from(spans)), area);
            return;
        }
        let mut keys: Vec<String> = Vec::new();
        if let Some(job) = &self.job {
            const SPIN: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
            let frame = (job.started.elapsed().as_millis() / 100) as usize % SPIN.len();
            let (_, done, all) = job.progress.snapshot();
            keys.push(format!("{} reading {done}/{all}", SPIN[frame]));
        }
        let screen = if self.graph.is_some() {
            GRAPH_KEYS
        } else if self.current().dossier.is_some() {
            DOSSIER_KEYS
        } else {
            LIST_KEYS
        };
        let kept = self.filter.is_some() && self.graph.is_none();
        if let Some(filter) = self.filter.as_ref().filter(|_| kept) {
            keys.push(format!("/{}", filter.query));
            keys.push(BACK.to_string());
        }
        keys.extend(
            screen
                .iter()
                .filter(|k| !kept || **k != BACK)
                .map(|k| k.to_string()),
        );
        f.render_widget(Paragraph::new(key_footer(&keys, HELP, area.width)), area);
    }

    fn render_options(&self, f: &mut ratatui::Frame, area: Rect, opts: &options::Options) {
        let mut lines = vec![Line::raw("Sources whypkg reads:"), Line::raw("")];
        for (i, source) in Source::ALL.into_iter().enumerate() {
            let on = !opts.skip.contains(&source);
            let count = self
                .world
                .packages
                .values()
                .filter(|p| p.source == source)
                .count();
            let note = if !on {
                "skipped".to_string()
            } else if self.read.has(source) {
                crate::engine::count(count, "package")
            } else {
                "read when this closes".to_string()
            };
            let mark = if on {
                Span::styled("●", Style::new().green())
            } else {
                Span::styled("○", Style::new().dim())
            };
            let line = Line::from(vec![
                mark,
                Span::raw(format!(" {:<10}", source.word())),
                Span::styled(note, Style::new().dim()),
            ]);
            lines.push(if i == opts.cursor {
                line.style(Style::new().bg(CURSOR).bold())
            } else {
                line
            });
        }
        lines.push(Line::raw(""));
        let file = crate::config::path()
            .map(|p| crate::config::shown(&p))
            .unwrap_or_default();
        lines.push(Line::from(Span::styled(
            format!("written to {file} as you change it"),
            Style::new().dim(),
        )));
        draw_box(f, area, Color::Cyan, "settings", lines, OPTIONS_KEYS);
    }
}

/// A box measured from its content: the body, a blank row and the key row.
fn draw_box(
    f: &mut ratatui::Frame,
    area: Rect,
    colour: Color,
    title: &str,
    mut lines: Vec<Line<'static>>,
    keys: &[&str],
) {
    lines.push(Line::raw(""));
    lines.push(box_hint(keys));
    let width = box_width(area.width);
    let rows = wrapped_line_count(&lines, width.saturating_sub(6)) as u16;
    let rect = box_area(area, width, box_height(rows, area.height));
    f.render_widget(Clear, rect);
    let block = box_block(colour, title);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_alert(f: &mut ratatui::Frame, area: Rect, title: &str, body: &str) {
    let lines: Vec<Line<'static>> = body.lines().map(|l| Line::raw(l.to_string())).collect();
    draw_box(f, area, Color::Yellow, title, lines, READER_KEYS);
}

/// One group of the help panel: a heading, then `(keys, what they do)` rows,
/// where a row with no keys is a note about the group.
type HelpSection = (&'static str, &'static [(&'static str, &'static str)]);

/// Every key the browser answers to, grouped by where it works. The panel
/// scrolls, so a new row costs nothing but its line.
const HELP_ROWS: &[HelpSection] = &[
    (
        "the list",
        &[
            ("j/k ↑↓", "move"),
            ("ctrl-d ctrl-u", "half a screen down, up"),
            ("pgdn pgup", "a screen down, up"),
            ("g G home end", "the first row, the last"),
            ("tab shift-tab", "the next installer's tab, the previous"),
            ("↵", "open the package"),
            ("s", "show all, then only manual, auto or upgradable"),
            ("ctrl-g", "the dependency graph of the package"),
            ("/", "filter by name, description or binary"),
            ("esc", "drop the filter"),
            ("r", "read the machine again"),
            ("q ctrl-c", "quit"),
            ("?", "this help"),
        ],
    ),
    (
        "a package",
        &[
            ("h/l ←→", "the needed by list, the depends on list"),
            ("↵", "open the package under the cursor"),
            ("esc", "back the way you came, or drop the filter first"),
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
        "the graph",
        &[
            ("h/l ←→", "change side"),
            ("j/k ↑↓", "move in a side, paging past its end"),
            ("↵", "centre on it"),
            ("esc", "the centre before, then close the graph"),
            ("ctrl-g", "open the selected package"),
        ],
    ),
    (
        "everywhere",
        &[
            ("o", "settings: which installers whypkg reads"),
            ("ctrl-l", "draw the screen again"),
        ],
    ),
    (
        "marks",
        &[
            ("●", "you installed it"),
            ("○", "something pulled it in"),
            (
                "← clang",
                "pulled in by clang; auto: nothing you installed needs it",
            ),
            ("↑", "an upgrade is waiting"),
            (
                "uses",
                "a library it loads or the Python or node it runs on",
            ),
        ],
    ),
    (
        "in this help",
        &[
            ("j/k ↑↓", "scroll"),
            ("ctrl-d ctrl-u", "half a page down, up"),
            ("pgdn pgup", "a page down, up"),
            ("g G home end", "the top, the bottom"),
            ("esc q ?", "close"),
        ],
    ),
];

/// The width of the key column, so every description starts in one place.
const HELP_KEYS: usize = 15;

fn help_lines() -> Vec<Line<'static>> {
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

/// The help reader: the body scrolls under a key row that never moves, with a
/// scrollbar on the right border once it is taller than the box.
fn render_help(f: &mut ratatui::Frame, area: Rect, scroll: &Cell<usize>, page: &Cell<usize>) {
    let lines = help_lines();
    let width = box_width(area.width);
    // The body, then a blank and the key row.
    let rect = box_area(area, width, box_height(lines.len() as u16 + 2, area.height));
    f.render_widget(Clear, rect);
    let block = box_block(Color::Cyan, "help");
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let shown = inner.height.saturating_sub(2) as usize;
    page.set(shown.max(1));
    // Clamped here, where the height is known, so scrolling past the end never
    // piles up presses that then take as many to undo.
    let top = scroll.get().min(lines.len().saturating_sub(shown));
    scroll.set(top);
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
