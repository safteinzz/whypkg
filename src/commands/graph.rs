//! An in-terminal dependency graph view (prototype).
//!
//! Drawn with ratatui's `Canvas` (braille "pixels"), so it works in every
//! terminal - including tmux - with no image protocol. It shows the *ego graph*
//! of one package: the package in the centre, the things that need it on the
//! left, the things it needs on the right. Arrow keys move the selection around
//! the ring; Enter re-centres on the selected node (expanding outward); Esc
//! closes it. Node colour matches the list tags: green = manual, yellow = auto,
//! blue = flatpak.

use crate::model::{Source, World};
use ratatui::prelude::*;
use ratatui::symbols::Marker;
use ratatui::widgets::canvas::{Canvas, Circle, Line as CanvasLine, Points};
use ratatui::widgets::{Block, Borders, Paragraph};

/// Which column a node lives in - drives the spatial navigation.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Side {
    /// Left column: packages that need the centre (reverse deps).
    Left,
    /// The centre package itself.
    Center,
    /// Right column: packages the centre depends on.
    Right,
}

/// One node in the ego graph, with a laid-out canvas position.
struct Node {
    name: String,
    x: f64,
    y: f64,
    color: Color,
    center: bool,
    side: Side,
}

/// The graph view state: an ego graph laid out radially around `center`.
pub struct GraphView {
    nodes: Vec<Node>,
    /// `(from, to)` indices into `nodes`.
    edges: Vec<(usize, usize)>,
    /// Index of the currently selected node.
    selected: usize,
    /// Counts trimmed off each side (so we can tell the user "+N more").
    hidden_needed: usize,
    hidden_deps: usize,
    /// Centres visited before this one, so Esc can walk back out the way you
    /// dug in (Enter pushes, Esc pops).
    history: Vec<String>,
}

/// How many neighbours to show per side before it turns into a hairball.
const PER_SIDE: usize = 16;
const RADIUS: f64 = 48.0;

impl GraphView {
    /// Build the ego graph for `center`: reverse-deps on the left arc, deps on
    /// the right arc, the package itself in the middle.
    pub fn build(world: &World, center: &str) -> GraphView {
        let color_of = |name: &str| -> Color {
            match world.packages.get(name) {
                Some(p) if p.source == Source::Flatpak => Color::Blue,
                _ if world.is_manual(name) => Color::Green,
                _ => Color::Yellow,
            }
        };

        let needed: Vec<String> = dedup(world.rdeps_of(center));
        let deps: Vec<String> = dedup(world.deps_of(center));
        let hidden_needed = needed.len().saturating_sub(PER_SIDE);
        let hidden_deps = deps.len().saturating_sub(PER_SIDE);

        let mut nodes = vec![Node {
            name: center.to_string(),
            x: 0.0,
            y: 0.0,
            color: color_of(center),
            center: true,
            side: Side::Center,
        }];
        let mut edges = Vec::new();

        // Left side (needed by) and right side (depends on), on a bowed arc.
        place_side(&needed, PER_SIDE, Side::Left, &color_of, &mut nodes, &mut edges);
        place_side(&deps, PER_SIDE, Side::Right, &color_of, &mut nodes, &mut edges);

        // Start on the centre - the package you're actually inspecting - so
        // h/l steps out into either side from there.
        let selected = 0;
        GraphView {
            nodes,
            edges,
            selected,
            hidden_needed,
            hidden_deps,
            history: Vec::new(),
        }
    }

    /// Node indices in one column, ordered top (higher y) to bottom.
    fn column(&self, side: Side) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.nodes.len())
            .filter(|&i| self.nodes[i].side == side)
            .collect();
        v.sort_by(|&a, &b| {
            self.nodes[b]
                .y
                .partial_cmp(&self.nodes[a].y)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        v
    }

    /// Move up/down *within* the current column (`dir` -1 = up, +1 = down),
    /// clamped so you never jump columns by accident.
    pub fn move_vertical(&mut self, dir: isize) {
        let col = self.column(self.nodes[self.selected].side);
        if col.is_empty() {
            return;
        }
        let pos = col.iter().position(|&i| i == self.selected).unwrap_or(0);
        let next = (pos as isize + dir).clamp(0, col.len() as isize - 1) as usize;
        self.selected = col[next];
    }

    /// Move to the next non-empty column left/right (`dir` -1 = left, +1 =
    /// right), landing on the node closest in height to the current one.
    pub fn move_horizontal(&mut self, dir: isize) {
        let order = |s: Side| match s {
            Side::Left => 0isize,
            Side::Center => 1,
            Side::Right => 2,
        };
        let y = self.nodes[self.selected].y;
        let mut target = order(self.nodes[self.selected].side) + dir;
        while (0..=2).contains(&target) {
            let side = match target {
                0 => Side::Left,
                1 => Side::Center,
                _ => Side::Right,
            };
            let col = self.column(side);
            if let Some(&best) = col.iter().min_by(|&&a, &&b| {
                (self.nodes[a].y - y)
                    .abs()
                    .partial_cmp(&(self.nodes[b].y - y).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) {
                self.selected = best;
                return;
            }
            target += dir;
        }
    }

    /// The package the graph is currently centred on.
    pub fn center_name(&self) -> &str {
        self.nodes
            .iter()
            .find(|n| n.center)
            .map(|n| n.name.as_str())
            .unwrap_or("")
    }

    /// Re-centre on the selected node, remembering where we came from so Esc
    /// can walk back out.
    pub fn recenter(&mut self, world: &World) {
        let name = self.nodes[self.selected].name.clone();
        if name == self.center_name() {
            return; // already here; don't stack a pointless history entry
        }
        let mut history = std::mem::take(&mut self.history);
        history.push(self.center_name().to_string());
        *self = GraphView::build(world, &name);
        self.history = history;
    }

    /// Step back to the previous centre. Returns `false` when there's no
    /// history left, so the caller can close the graph instead.
    pub fn back(&mut self, world: &World) -> bool {
        let Some(prev) = self.history.pop() else {
            return false;
        };
        let history = std::mem::take(&mut self.history);
        *self = GraphView::build(world, &prev);
        self.history = history;
        true
    }

    /// The name of the node under the cursor (for the title bar).
    pub fn selected_name(&self) -> &str {
        &self.nodes[self.selected].name
    }

    /// Draw the graph into `area`.
    pub fn render(&self, f: &mut ratatui::Frame, area: Rect) {
        let center_name = self.center_name();

        // Zone tints: a cool half for "needed by", a warm half for "depends on".
        let left_bg = Color::Rgb(14, 20, 34);
        let right_bg = Color::Rgb(32, 23, 14);

        let chunks = Layout::vertical([
            Constraint::Length(1), // title
            Constraint::Length(1), // zone headers
            Constraint::Min(3),    // the graph canvas
            Constraint::Length(1), // legend (colour key)
            Constraint::Length(1), // controls
        ])
        .split(area);

        // Title: the package we're centred on, and the node under the cursor.
        let title = Line::from(vec![
            Span::styled("  graph: ", Style::new().dim()),
            Span::styled(center_name.to_string(), Style::new().bold().cyan()),
            Span::styled("    now: ", Style::new().dim()),
            Span::styled(self.selected_name().to_string(), Style::new().bold().white()),
        ]);
        f.render_widget(Paragraph::new(title), chunks[0]);

        // Zone headers, centred over each half and sitting on its zone tint.
        let halves =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1]);
        f.render_widget(
            Paragraph::new("needed by")
                .alignment(Alignment::Center)
                .style(Style::new().fg(Color::Rgb(150, 175, 225)).bold().bg(left_bg)),
            halves[0],
        );
        f.render_widget(
            Paragraph::new("depends on")
                .alignment(Alignment::Center)
                .style(Style::new().fg(Color::Rgb(225, 180, 140)).bold().bg(right_bg)),
            halves[1],
        );

        // Approx. width of one character in canvas x-units, so we can place
        // left-side labels *leftward* into the otherwise-empty outer zone.
        let ca = chunks[2];
        const X_SPAN: f64 = 72.0;
        const Y_SPAN: f64 = 52.0;
        let char_w = (2.0 * X_SPAN) / (ca.width.max(1) as f64);
        // Height of one terminal row in canvas units, so labels can be nudged
        // exactly one row off their planet.
        let row_h = (2.0 * Y_SPAN) / (ca.height.max(1) as f64);

        let canvas = Canvas::default()
            .block(Block::default().borders(Borders::ALL))
            .marker(Marker::Braille)
            .x_bounds([-X_SPAN, X_SPAN])
            .y_bounds([-Y_SPAN, Y_SPAN])
            .paint(|ctx| {
                // Edges first, under the nodes.
                for &(a, b) in &self.edges {
                    ctx.draw(&CanvasLine {
                        x1: self.nodes[a].x,
                        y1: self.nodes[a].y,
                        x2: self.nodes[b].x,
                        y2: self.nodes[b].y,
                        color: Color::DarkGray,
                    });
                }
                ctx.layer();
                // Node markers. A selected node is drawn purely white (no origin
                // colour underneath, so no yellow/green dots peek through).
                for (i, node) in self.nodes.iter().enumerate() {
                    if i == self.selected {
                        ctx.draw(&Circle {
                            x: node.x,
                            y: node.y,
                            radius: if node.center { 3.0 } else { 2.4 },
                            color: Color::White,
                        });
                        ctx.draw(&Points {
                            coords: &[(node.x, node.y)],
                            color: Color::White,
                        });
                    } else {
                        ctx.draw(&Circle {
                            x: node.x,
                            y: node.y,
                            radius: if node.center { 2.6 } else { 1.7 },
                            color: node.color,
                        });
                    }
                }
                ctx.layer();
                // Labels: left-zone labels grow leftward (into the empty side),
                // right-zone rightward, centre centred - so nothing crowds the
                // middle and the wide sides get used.
                for (i, node) in self.nodes.iter().enumerate() {
                    let style = if i == self.selected {
                        Style::new().fg(Color::White).bold()
                    } else {
                        Style::new().fg(node.color)
                    };
                    // Labels hug their planet, just clear of the dot, and fan
                    // diagonally *away* from the middle: nodes in the top half
                    // get their name a row above, bottom half a row below. That
                    // splays the names outward instead of stacking them, so
                    // neighbours drift apart rather than collide.
                    const GAP: f64 = 1.9; // hugs the node marker
                    let room = match node.side {
                        Side::Left => node.x - GAP + X_SPAN,
                        Side::Right => X_SPAN - (node.x + GAP),
                        Side::Center => X_SPAN,
                    };
                    let fits = ((room / char_w).floor() as usize).clamp(6, 26);
                    let label = truncate(&node.name, fits);
                    let width = label.chars().count() as f64 * char_w;
                    // Away from the centre line; nodes near it stay level.
                    let lift = if node.y > row_h {
                        row_h
                    } else if node.y < -row_h {
                        -row_h
                    } else {
                        0.0
                    };
                    let (lx, ly) = match node.side {
                        Side::Left => (node.x - GAP - width, node.y + lift),
                        Side::Right => (node.x + GAP, node.y + lift),
                        Side::Center => (node.x - width / 2.0, node.y + 4.0),
                    };
                    ctx.print(lx, ly, Span::styled(label, style));
                }
            });
        f.render_widget(canvas, chunks[2]);

        // Tint the two zones, leaving a neutral lane down the middle for the
        // package itself (it's neither "needed by" nor "depends on"). Bg only,
        // after the canvas renders, so the braille graph stays visible on top.
        let mid = ca.x + ca.width / 2;
        let band = ca.width / 24; // half-width of the neutral centre lane
        let buf = f.buffer_mut();
        for y in (ca.y + 1)..(ca.y + ca.height.saturating_sub(1)) {
            for x in (ca.x + 1)..(ca.x + ca.width.saturating_sub(1)) {
                let bg = if x + band < mid {
                    Some(left_bg)
                } else if x > mid + band {
                    Some(right_bg)
                } else {
                    None // neutral centre - the package we're inspecting
                };
                if let Some(bg) = bg
                    && let Some(cell) = buf.cell_mut((x, y))
                {
                    cell.set_bg(bg);
                }
            }
        }

        // Legend: colour key only (the zones now show which side is which).
        let legend = Line::from(vec![
            Span::raw("  "),
            Span::styled("● manual", Style::new().green()),
            Span::raw("    "),
            Span::styled("● auto", Style::new().yellow()),
            Span::raw("    "),
            Span::styled("● flatpak", Style::new().blue()),
        ]);
        f.render_widget(Paragraph::new(legend), chunks[3]);

        // Controls, plus a note if some neighbours were trimmed.
        let mut controls =
            "  ←↓↑→ / hjkl move · Enter dig in · Esc back · q quit graph".to_string();
        if self.hidden_needed > 0 || self.hidden_deps > 0 {
            controls.push_str(&format!(
                " · +{} needed, +{} deps hidden",
                self.hidden_needed, self.hidden_deps
            ));
        }
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(controls, Style::new().dim()))),
            chunks[4],
        );
    }
}

/// Place up to `cap` names down one side, spaced *evenly in height* so no two
/// share a terminal row, on two alternating orbits (like planets at different
/// distances). Neighbouring nodes therefore sit at different distances from the
/// centre, so their labels start at different x - which is what stops long
/// names running into each other while keeping the radial bow.
fn place_side(
    names: &[String],
    cap: usize,
    side: Side,
    color_of: &dyn Fn(&str) -> Color,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<(usize, usize)>,
) {
    let shown = names.len().min(cap);
    if shown == 0 {
        return;
    }
    // Vertical extent, kept shy of the poles so the extremes keep some bow.
    const SWEEP_DEG: f64 = 80.0;
    let y_top = RADIUS * SWEEP_DEG.to_radians().sin();
    // Alternate between the outer orbit and an inner one at 60% the distance.
    let inner_radius = RADIUS * 0.6;

    for (i, name) in names.iter().take(shown).enumerate() {
        let frac = (i as f64 + 1.0) / (shown as f64 + 1.0); // 0..1, excludes ends
        let y = y_top - 2.0 * y_top * frac; // evenly from +y_top down to -y_top

        // Only stagger when there are enough nodes for crowding to matter.
        let inner = shown > 6 && i % 2 == 1;
        let radius = if inner { inner_radius } else { RADIUS };

        // Bow by height: full radius at the middle, easing in towards the poles
        // but never collapsing onto the centre line - otherwise the top and
        // bottom planets stack up vertically and sit on each other's trails.
        const POLE_FRAC: f64 = 0.6; // how far out the extremes stay
        let t = (y / y_top).clamp(-1.0, 1.0);
        let bow = (t * SWEEP_DEG.to_radians()).cos();
        let x_mag = radius * (POLE_FRAC + (1.0 - POLE_FRAC) * bow);
        let x = if side == Side::Left { -x_mag } else { x_mag };

        let idx = nodes.len();
        nodes.push(Node {
            name: name.clone(),
            x,
            y,
            color: color_of(name),
            center: false,
            side,
        });
        edges.push((0, idx));
    }
}

fn dedup(src: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    src.iter()
        .filter(|s| seen.insert((*s).clone()))
        .cloned()
        .collect()
}

/// Truncate a label to `max` chars (UTF-8 safe).
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world where `center` is needed by 12 packages and depends on 5.
    fn busy_world() -> (World, Vec<String>, Vec<String>) {
        let needed: Vec<String> = (0..12).map(|i| format!("needs-me-{i:02}")).collect();
        let deps: Vec<String> = (0..5).map(|i| format!("i-need-{i:02}")).collect();
        let mut edges: Vec<(&str, &str)> = Vec::new();
        for n in &needed {
            edges.push((n, "center"));
        }
        for d in &deps {
            edges.push(("center", d));
        }
        (World::from_edges(&edges, &["center"]), needed, deps)
    }

    fn side_of(g: &GraphView, name: &str) -> Side {
        g.nodes.iter().find(|n| n.name == name).unwrap().side
    }

    // ── layout invariants (these are the visual bugs, caught in code) ────────

    #[test]
    fn no_two_nodes_on_a_side_share_a_height() {
        // Two planets at the same y means their labels land on the same
        // terminal row and overwrite each other.
        let (w, _, _) = busy_world();
        let g = GraphView::build(&w, "center");
        for side in [Side::Left, Side::Right] {
            let ys: Vec<f64> = g
                .nodes
                .iter()
                .filter(|n| n.side == side)
                .map(|n| n.y)
                .collect();
            for (i, a) in ys.iter().enumerate() {
                for b in ys.iter().skip(i + 1) {
                    assert!((a - b).abs() > 1.0, "two nodes share a row: {a} vs {b}");
                }
            }
        }
    }

    #[test]
    fn planets_never_collapse_onto_the_centre_line() {
        // If the arc pinches to x~0 at the poles, top/bottom planets stack up
        // and sit on each other's trails to the centre.
        let (w, _, _) = busy_world();
        let g = GraphView::build(&w, "center");
        // The inner orbit sits at 60% of the radius, so measure against that:
        // even its extremes must stay at least half an inner-orbit out.
        let floor = RADIUS * 0.6 * 0.5;
        for n in g.nodes.iter().filter(|n| !n.center) {
            assert!(
                n.x.abs() > floor,
                "{} pinches toward the centre line (x={}, floor={floor})",
                n.name,
                n.x
            );
        }
    }

    #[test]
    fn sides_are_on_the_side_they_claim() {
        let (w, needed, deps) = busy_world();
        let g = GraphView::build(&w, "center");
        for n in &needed {
            assert_eq!(side_of(&g, n), Side::Left, "{n} should be a 'needed by'");
        }
        for d in &deps {
            assert_eq!(side_of(&g, d), Side::Right, "{d} should be a 'depends on'");
        }
        for n in g.nodes.iter().filter(|n| n.side == Side::Left) {
            assert!(n.x < 0.0, "left-side node has positive x");
        }
        for n in g.nodes.iter().filter(|n| n.side == Side::Right) {
            assert!(n.x > 0.0, "right-side node has negative x");
        }
    }

    #[test]
    fn labels_stay_inside_the_canvas() {
        // The furthest-out planet plus its label must still fit, or names clip.
        let (w, _, _) = busy_world();
        let g = GraphView::build(&w, "center");
        let max_x = g.nodes.iter().map(|n| n.x.abs()).fold(0.0, f64::max);
        assert!(max_x < 72.0, "a planet is drawn off-canvas (x={max_x})");
    }

    #[test]
    fn caps_each_side_and_reports_what_it_hid() {
        let needed: Vec<String> = (0..30).map(|i| format!("n{i:02}")).collect();
        let edges: Vec<(&str, &str)> = needed.iter().map(|n| (n.as_str(), "center")).collect();
        let w = World::from_edges(&edges, &["center"]);
        let g = GraphView::build(&w, "center");
        assert_eq!(g.nodes.iter().filter(|n| n.side == Side::Left).count(), PER_SIDE);
        assert_eq!(g.hidden_needed, 30 - PER_SIDE);
    }

    // ── navigation ──────────────────────────────────────────────────────────

    #[test]
    fn starts_on_the_package_you_are_inspecting() {
        let (w, _, _) = busy_world();
        let g = GraphView::build(&w, "center");
        assert_eq!(g.selected_name(), "center");
    }

    #[test]
    fn vertical_movement_never_leaves_its_column() {
        // The bug that made j/k jump from "depends on" to "needed by".
        let (w, _, _) = busy_world();
        let mut g = GraphView::build(&w, "center");
        g.move_horizontal(-1); // into the left column
        assert_eq!(side_of(&g, g.selected_name()), Side::Left);
        for _ in 0..40 {
            g.move_vertical(1);
            assert_eq!(side_of(&g, g.selected_name()), Side::Left);
        }
        for _ in 0..40 {
            g.move_vertical(-1);
            assert_eq!(side_of(&g, g.selected_name()), Side::Left);
        }
    }

    #[test]
    fn horizontal_movement_walks_left_centre_right() {
        let (w, _, _) = busy_world();
        let mut g = GraphView::build(&w, "center");
        g.move_horizontal(-1);
        assert_eq!(side_of(&g, g.selected_name()), Side::Left);
        g.move_horizontal(1);
        assert_eq!(side_of(&g, g.selected_name()), Side::Center);
        g.move_horizontal(1);
        assert_eq!(side_of(&g, g.selected_name()), Side::Right);
        g.move_horizontal(1); // already at the far side: stays put
        assert_eq!(side_of(&g, g.selected_name()), Side::Right);
    }

    // ── history ─────────────────────────────────────────────────────────────

    #[test]
    fn digging_in_and_backing_out_retraces_your_steps() {
        let (w, _, _) = busy_world();
        let mut g = GraphView::build(&w, "center");
        g.move_horizontal(-1); // pick a neighbour
        let first_hop = g.selected_name().to_string();

        g.recenter(&w);
        assert_eq!(g.center_name(), first_hop, "Enter re-centres on the node");

        assert!(g.back(&w), "Esc unwinds one step");
        assert_eq!(g.center_name(), "center");

        assert!(!g.back(&w), "no history left, so the caller closes the graph");
    }

    #[test]
    fn recentring_on_the_current_centre_is_a_no_op() {
        let (w, _, _) = busy_world();
        let mut g = GraphView::build(&w, "center");
        g.recenter(&w); // selection starts on the centre
        assert_eq!(g.center_name(), "center");
        assert!(!g.back(&w), "should not have pushed a pointless history entry");
    }

    // ── labels ──────────────────────────────────────────────────────────────

    #[test]
    fn truncation_never_splits_a_multibyte_character() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("0123456789", 5), "0123…");
        // Would panic if we sliced by bytes instead of chars.
        let wide = "日本語パッケージ名";
        assert_eq!(truncate(wide, 4).chars().count(), 4);
        assert_eq!(truncate("café-utils", 5), "café…");
    }
}
