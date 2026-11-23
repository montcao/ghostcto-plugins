//! Ghost sprites: a 10x10 pixel ghost holding a 5x5 shield, drawn with half
//! blocks so each terminal row carries two pixel rows (16 columns, 5 rows).
//!
//! The body colour and eyes say the state (resting, working, needs you); the
//! shield fill says the category, matching the GhostCTO catalog's shields. The
//! card always prints the state in words as well, so colour is never the only
//! signal.

use ratatui::buffer::Buffer;
use ratatui::style::Color;

use crate::model::GhostState;
use crate::roster::Category;

pub const WIDTH: u16 = 16;
pub const HEIGHT: u16 = 5;

const PAPER: Color = Color::Rgb(0xff, 0xfa, 0xf2);
const INK: Color = Color::Rgb(0x18, 0x11, 0x20);
const VIOLET: Color = Color::Rgb(0x75, 0x38, 0xf5);
const LAVENDER: Color = Color::Rgb(0xc9, 0xb4, 0xff);
const MUTED: Color = Color::Rgb(0x5e, 0x55, 0x65);

const BODY: [&str; 9] = [
    "...BBBB...",
    ".BBBBBBBB.",
    "BBBBBBBBBB",
    "BBeeBBeeBB",
    "BBeeBBeeBB",
    "BBBBBBBBBB",
    "BBBBBBBBBB",
    "BBBBBBBBBB",
    "BBBBBBBBBB",
];
const HEM: [&str; 2] = ["B.BB..BB.B", ".BB.BB.BB."];
const SHIELD: [&str; 5] = ["OOOOO", "OFFFO", "OFFFO", ".OFO.", "..O.."];
const SHIELD_X: usize = 11;
const SHIELD_Y: usize = 5;

/// Eye rows 3 and 4 for each state: working looks toward the shield, needs you
/// looks up at the person, resting has its eyes shut.
fn eyes(state: GhostState) -> [&'static str; 2] {
    match state {
        GhostState::Working => ["WW", "WP"],
        GhostState::NeedsYou => ["WP", "WW"],
        GhostState::Idle => ["BB", "LL"],
    }
}

/// The sprite as a grid of pixel codes: `.` clear, `B` body, `W` eye, `P`
/// pupil, `L` closed lid, `O` shield outline, `F` shield fill.
pub fn pixels(state: GhostState, frame: usize) -> Vec<Vec<char>> {
    let mut grid = vec![vec!['.'; WIDTH as usize]; (HEIGHT * 2) as usize];
    let [eye_top, eye_bottom] = eyes(state);
    for (y, row) in BODY.iter().enumerate() {
        let eye = match y {
            3 => Some(eye_top),
            4 => Some(eye_bottom),
            _ => None,
        };
        let mut eye_cells = eye.map(|e| e.chars().cycle());
        for (x, c) in row.chars().enumerate() {
            grid[y][x] = match (c, eye_cells.as_mut()) {
                ('e', Some(cells)) => cells.next().unwrap_or('B'),
                (c, _) => c,
            };
        }
    }
    let hem = if state == GhostState::Working {
        HEM[frame % 2]
    } else {
        HEM[0]
    };
    for (x, c) in hem.chars().enumerate() {
        grid[9][x] = c;
    }
    for (dy, row) in SHIELD.iter().enumerate() {
        for (dx, c) in row.chars().enumerate() {
            if c != '.' {
                grid[SHIELD_Y + dy][SHIELD_X + dx] = c;
            }
        }
    }
    grid
}

pub fn body_colour(state: GhostState) -> Color {
    match state {
        GhostState::Idle => MUTED,
        GhostState::Working => LAVENDER,
        GhostState::NeedsYou => PAPER,
    }
}

fn shield_fill(category: Category) -> Color {
    match category {
        Category::Build => VIOLET,
        Category::Quality => INK,
        Category::Communication => PAPER,
        Category::Unlisted => MUTED,
    }
}

fn colour(code: char, state: GhostState, category: Category) -> Option<Color> {
    match code {
        'B' => Some(body_colour(state)),
        'W' => Some(PAPER),
        'P' | 'L' => Some(INK),
        'O' => Some(LAVENDER),
        'F' => Some(shield_fill(category)),
        _ => None,
    }
}

/// Draws the large sprite (16x5 cells) with its top-left corner at (x, y).
pub fn draw(buf: &mut Buffer, x: u16, y: u16, state: GhostState, category: Category, frame: usize) {
    paint(buf, x, y, &pixels(state, frame), |code| {
        colour(code, state, category)
    });
}

pub const MINI_WIDTH: u16 = 6;
pub const MINI_HEIGHT: u16 = 3;

/// The small sprite used for every profile in the grid: 6x6 pixels, three
/// terminal rows. Eyes open while lit, shut while idle.
pub fn mini_pixels(state: GhostState, frame: usize) -> Vec<Vec<char>> {
    let lit = state != GhostState::Idle;
    let hem = if state == GhostState::Working && frame % 2 == 1 {
        ".BB.BB"
    } else {
        "B.BB.B"
    };
    [
        ".BBBB.",
        "BBBBBB",
        if lit { "BWBBWB" } else { "BBBBBB" },
        if lit { "BBBBBB" } else { "BLBBLB" },
        "BBBBBB",
        hem,
    ]
    .iter()
    .map(|r| r.chars().collect())
    .collect()
}

pub fn draw_mini(buf: &mut Buffer, x: u16, y: u16, state: GhostState, frame: usize) {
    paint(buf, x, y, &mini_pixels(state, frame), |code| {
        colour(code, state, Category::Unlisted)
    });
}

/// Paints a pixel grid two rows per cell with half blocks, clipped to the buffer.
fn paint(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    grid: &[Vec<char>],
    colour: impl Fn(char) -> Option<Color>,
) {
    let area = buf.area;
    for row in 0..grid.len().div_ceil(2) {
        for (col, &code) in grid[row * 2].iter().enumerate() {
            let (cx, cy) = (x + col as u16, y + row as u16);
            if cx >= area.right() || cy >= area.bottom() {
                continue;
            }
            let top = colour(code);
            let bottom = grid
                .get(row * 2 + 1)
                .and_then(|r| r.get(col))
                .and_then(|&c| colour(c));
            let cell = &mut buf[(cx, cy)];
            match (top, bottom) {
                (None, None) => {}
                (Some(t), None) => {
                    cell.set_symbol("▀").set_fg(t).set_bg(Color::Reset);
                }
                (None, Some(b)) => {
                    cell.set_symbol("▄").set_fg(b).set_bg(Color::Reset);
                }
                (Some(t), Some(b)) => {
                    cell.set_symbol("▀").set_fg(t).set_bg(b);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn rows(state: GhostState, frame: usize) -> Vec<String> {
        pixels(state, frame)
            .into_iter()
            .map(|r| r.into_iter().collect())
            .collect()
    }

    #[test]
    fn working_ghost_looks_at_its_shield() {
        assert_eq!(
            rows(GhostState::Working, 0),
            vec![
                "...BBBB.........",
                ".BBBBBBBB.......",
                "BBBBBBBBBB......",
                "BBWWBBWWBB......",
                "BBWPBBWPBB......",
                "BBBBBBBBBB.OOOOO",
                "BBBBBBBBBB.OFFFO",
                "BBBBBBBBBB.OFFFO",
                "BBBBBBBBBB..OFO.",
                "B.BB..BB.B...O..",
            ]
        );
    }

    #[test]
    fn only_a_working_ghost_wiggles() {
        assert_ne!(
            rows(GhostState::Working, 0)[9],
            rows(GhostState::Working, 1)[9]
        );
        assert_eq!(rows(GhostState::Idle, 0), rows(GhostState::Idle, 1));
        assert_eq!(rows(GhostState::NeedsYou, 0), rows(GhostState::NeedsYou, 1));
    }

    #[test]
    fn resting_ghost_has_shut_eyes_and_needs_you_looks_up() {
        let idle = rows(GhostState::Idle, 0);
        assert_eq!(&idle[3][..10], "BBBBBBBBBB");
        assert_eq!(&idle[4][..10], "BBLLBBLLBB");
        let needs = rows(GhostState::NeedsYou, 0);
        assert_eq!(&needs[3][..10], "BBWPBBWPBB");
    }

    #[test]
    fn mini_ghost_is_three_rows_and_shuts_its_eyes_when_idle() {
        let idle: Vec<String> = mini_pixels(GhostState::Idle, 0)
            .into_iter()
            .map(|r| r.into_iter().collect())
            .collect();
        assert_eq!(
            idle,
            vec![".BBBB.", "BBBBBB", "BBBBBB", "BLBBLB", "BBBBBB", "B.BB.B"]
        );
        assert_eq!(
            mini_pixels(GhostState::Working, 0)[2]
                .iter()
                .collect::<String>(),
            "BWBBWB"
        );
        assert_ne!(
            mini_pixels(GhostState::Working, 0)[5],
            mini_pixels(GhostState::Working, 1)[5]
        );
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 3));
        draw_mini(&mut buf, 0, 0, GhostState::Idle, 0);
        let rows: Vec<String> = (0..3)
            .map(|y| (0..6).map(|x| buf[(x, y)].symbol().to_string()).collect())
            .collect();
        assert_eq!(rows, vec!["▄▀▀▀▀▄", "▀▀▀▀▀▀", "▀▀▀▀▀▀"]);
    }

    #[test]
    fn draws_half_blocks_and_clips() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 3));
        draw(&mut buf, 0, 0, GhostState::Working, Category::Build, 0);
        let line: String = (0..12).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert_eq!(line, " ▄▄▀▀▀▀▄▄   ");
        assert_eq!(buf[(1, 0)].fg, LAVENDER);
    }
}
