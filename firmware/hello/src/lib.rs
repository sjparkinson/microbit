//! HELLO WORLD! scrolling right to left across the 5×5 matrix.
//!
//! The message is laid out as one long strip of columns: five blank ones so
//! it scrolls in from the right, each letter's columns with a blank one
//! after it, then five more blank ones so it scrolls all the way off. Each
//! frame is the five columns starting at an offset, and the offset goes up
//! by one every [`STEP_MS`].
//!
//! The font is my own, drawn for this, four columns wide for most letters.

#![no_std]

use board::Image;

/// What scrolls past.
pub const MESSAGE: &str = "HELLO WORLD!";

/// How long each one-column step stays up.
pub const STEP_MS: u32 = 120;

/// Blank columns before and after the message, a whole screen's worth.
const MARGIN: usize = 5;

/// A letter: how many columns wide it is, and its five rows, top first, with
/// the leftmost column in the highest of the `width` bits.
#[derive(Clone, Copy)]
struct Glyph {
    width: u8,
    rows: [u8; 5],
}

const SPACE: Glyph = Glyph {
    width: 2,
    rows: [0; 5],
};

/// The letters `MESSAGE` uses, or `None`.
fn lookup(c: char) -> Option<Glyph> {
    let (width, rows) = match c {
        ' ' => (SPACE.width, SPACE.rows),
        'H' => (4, [0b1001, 0b1001, 0b1111, 0b1001, 0b1001]),
        'E' => (4, [0b1111, 0b1000, 0b1110, 0b1000, 0b1111]),
        'L' => (4, [0b1000, 0b1000, 0b1000, 0b1000, 0b1111]),
        'O' => (4, [0b0110, 0b1001, 0b1001, 0b1001, 0b0110]),
        'W' => (5, [0b10001, 0b10001, 0b10101, 0b10101, 0b01010]),
        'R' => (4, [0b1110, 0b1001, 0b1110, 0b1010, 0b1001]),
        'D' => (4, [0b1110, 0b1001, 0b1001, 0b1001, 0b1110]),
        '!' => (1, [0b1, 0b1, 0b1, 0b0, 0b1]),
        _ => return None,
    };
    Some(Glyph { width, rows })
}

/// `c`'s glyph. Anything without one shows as a space.
fn glyph(c: char) -> Glyph {
    lookup(c).unwrap_or(SPACE)
}

/// The strip's length in columns: every glyph plus its gap, and both
/// margins.
pub fn length(message: &str) -> usize {
    let letters: usize = message
        .chars()
        .map(|c| usize::from(glyph(c).width) + 1)
        .sum();
    MARGIN + letters + MARGIN
}

/// Column `index` of the strip, as five bits, top row in bit 0.
fn column(message: &str, index: usize) -> u8 {
    let Some(mut at) = index.checked_sub(MARGIN) else {
        return 0;
    };
    for c in message.chars() {
        let glyph = glyph(c);
        let width = usize::from(glyph.width);
        if at < width {
            let shift = width - 1 - at;
            let mut bits = 0;
            for (row, pattern) in glyph.rows.iter().enumerate() {
                bits |= ((pattern >> shift) & 1) << row;
            }
            return bits;
        }
        // The blank column after the glyph, or on to the next one.
        if at == width {
            return 0;
        }
        at -= width + 1;
    }
    0
}

/// The frame with the strip's column `offset` at the left edge.
pub fn frame(message: &str, offset: usize) -> Image {
    let mut image = [[0; 5]; 5];
    for x in 0..5 {
        let bits = column(message, offset + x);
        for (y, row) in image.iter_mut().enumerate() {
            row[x] = (bits >> y) & 1;
        }
    }
    image
}

/// Every frame, in order, once round. The firmware loops over it forever.
pub fn frames(message: &str) -> impl Iterator<Item = Image> + '_ {
    // The last frame worth showing is the one that's just gone blank.
    (0..=length(message) - 5).map(move |offset| frame(message, offset))
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::string::String;
    use std::vec::Vec;

    use super::*;

    const BLANK: Image = [[0; 5]; 5];

    /// The image as text, `#` for lit, rows joined with `/`.
    fn draw(image: &Image) -> String {
        image
            .iter()
            .map(|row| {
                row.iter()
                    .map(|&v| if v != 0 { '#' } else { '.' })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    #[test]
    fn every_letter_in_the_message_has_a_glyph() {
        for c in MESSAGE.chars() {
            assert!(lookup(c).is_some(), "no glyph for {c:?}");
        }
        assert!(lookup('Z').is_none());
    }

    #[test]
    fn glyphs_fit_their_width() {
        for c in MESSAGE.chars() {
            let g = glyph(c);
            for row in g.rows {
                assert!(row >> g.width == 0, "{c:?} spills past {} columns", g.width);
            }
        }
    }

    #[test]
    fn starts_and_ends_blank() {
        let frames: Vec<Image> = frames(MESSAGE).collect();
        assert_eq!(frames.first(), Some(&BLANK));
        assert_eq!(frames.last(), Some(&BLANK));
        assert!(frames.iter().filter(|f| **f != BLANK).count() > 40);
    }

    #[test]
    fn scrolls_in_from_the_right() {
        // One step in, the H's left edge is in the rightmost column.
        assert_eq!(draw(&frame(MESSAGE, 1)), "....#/....#/....#/....#/....#");
        assert_eq!(draw(&frame(MESSAGE, 2)), "...#./...#./...##/...#./...#.");
    }

    #[test]
    fn shows_a_whole_letter_with_its_gap() {
        assert_eq!(
            draw(&frame(MESSAGE, MARGIN)),
            "#..#./#..#./####./#..#./#..#."
        );
        // E starts after H's four columns and one gap.
        assert_eq!(
            draw(&frame(MESSAGE, MARGIN + 5)),
            "####./#..../###../#..../####."
        );
    }

    #[test]
    fn spaces_and_the_bang_come_out_right() {
        let text = "O !";
        // O, gap, two blank columns for the space, gap, then the bang.
        assert_eq!(column(text, MARGIN + 4), 0);
        assert_eq!(column(text, MARGIN + 5), 0);
        assert_eq!(column(text, MARGIN + 6), 0);
        assert_eq!(column(text, MARGIN + 7), 0);
        assert_eq!(column(text, MARGIN + 8), 0b10111);
        assert_eq!(column(text, MARGIN + 9), 0);
    }

    #[test]
    fn length_counts_glyphs_gaps_and_margins() {
        // H E L L O (4 each) space (2) W (5) O R L D (4 each) ! (1), each
        // plus a gap.
        let letters = 5 * 5 + 3 + 6 + 4 * 5 + 2;
        assert_eq!(length(MESSAGE), 2 * MARGIN + letters);
        assert_eq!(frames(MESSAGE).count(), length(MESSAGE) - 4);
    }
}
