//! Furniture with no idea what it is drawing: the shared key words every footer
//! is built from, the footer itself, and the house box an overlay is drawn in.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Padding, Scrollbar, ScrollbarOrientation, ScrollbarState};

/// Narrowest a box may be, so a two-word message still reads as a box.
pub(super) const BOX_MIN_W: u16 = 24;
/// Widest, so one long line does not stretch a box across a wide screen.
pub(super) const BOX_MAX_W: u16 = 88;
/// Rows the chrome costs: two borders plus the single top padding row.
pub(super) const BOX_CHROME_H: u16 = 3;

/// How wide a box is on a screen this wide.
pub(super) fn box_width(screen_w: u16) -> u16 {
    screen_w.saturating_sub(4).clamp(BOX_MIN_W, BOX_MAX_W)
}

/// How tall a box holding `body_rows` *wrapped* rows is, capped at `screen_h`,
/// which is the area the box is drawn in and not the terminal: a box sized for
/// the whole terminal and drawn above the footer loses its key row.
pub(super) fn box_height(body_rows: u16, screen_h: u16) -> u16 {
    let floor = BOX_CHROME_H + 1;
    body_rows
        .saturating_add(BOX_CHROME_H)
        .clamp(floor, screen_h.max(floor))
}

/// A box of at most `w` by `h`, centered in `area`.
pub(super) fn box_area(area: Rect, w: u16, h: u16) -> Rect {
    let (w, h) = (w.min(area.width), h.min(area.height));
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

/// The bordered block every box wears: a spaced title on the top border and
/// nothing else on it, in the colour that says what kind of box it is. Keys go
/// in the body, through `box_hint`.
pub(super) fn box_block(colour: Color, title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colour))
        .padding(Padding::new(2, 2, 1, 0))
        .title(format!(" {} ", title.trim()))
}

// The words every footer and key row is built from, so the same key reads the
// same on every screen and a hand-typed legend stands out.
pub(super) const QUIT: &str = "q quit";
pub(super) const HELP: &str = "? help";
pub(super) const BACK: &str = "esc back";
pub(super) const CLOSE: &str = "esc close";
pub(super) const SEP: &str = " · ";
pub(super) const FIND: &str = "/ find";
pub(super) const KEEP: &str = "↵ keep";

/// The key row of a reader, drawn by the help panel.
pub(super) const READER_KEYS: &[&str] = &[CLOSE];

/// The line of keys a box ends with, as the last row of its body, a step
/// quieter than anything else in the box.
pub(super) fn box_hint(keys: &[&str]) -> Line<'static> {
    Line::from(Span::styled(
        keys.join(SEP),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    ))
}

/// The row under a view: as many of `keys` as fit in `width`, whole and in
/// order, with `help` pinned at the
/// right edge however narrow it gets, since help is the way to every key that
/// fell off.
pub(super) fn key_footer(keys: &[&str], help: &str, width: u16) -> Line<'static> {
    let width = width as usize;
    let help_w = help.chars().count();
    // One column of margin at each end, and a separator's width before `help`.
    let room = width.saturating_sub(help_w + 2 + SEP.chars().count());
    let mut left = String::new();
    for entry in keys {
        let next = if left.is_empty() {
            entry.to_string()
        } else {
            format!("{left}{SEP}{entry}")
        };
        if next.chars().count() > room {
            break;
        }
        left = next;
    }
    let pad = width.saturating_sub(left.chars().count() + help_w + 2);
    let dim = Style::default().add_modifier(Modifier::DIM);
    Line::from(vec![
        Span::styled(format!(" {left}"), dim),
        Span::raw(" ".repeat(pad.max(1))),
        Span::styled(format!("{help} "), dim),
    ])
}

/// A scrollbar on `area`'s right border for `total` rows, `view` of them on
/// screen from `top`, where `area` is the bordered rect it sits on. Nothing is
/// drawn when every row fits.
pub(super) fn vscrollbar(f: &mut Frame, area: Rect, total: usize, top: usize, view: usize) {
    if total <= view {
        return;
    }
    let mut state = ScrollbarState::new(total - view).position(top);
    let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None);
    f.render_stateful_widget(bar, area.inner(Margin::new(0, 1)), &mut state);
}
