//! The LED matrix as data: how the v1 wires its 25 LEDs, and how long each
//! brightness keeps an LED lit. Plain functions, so the host tests them;
//! `hw.rs` does the scanning. Each board uses only its own layout, and the
//! host neither, hence the `dead_code` allowances.

use crate::Image;

/// The brightest level [`Display::show_greyscale`](crate::Display) takes.
/// Anything above shows as this.
pub const MAX_BRIGHTNESS: u8 = 9;

/// How long each row is driven for in one scan, in microseconds. A scan is
/// 3 ms on the v1 and 5 ms on the v2, so 333 and 200 a second, which is
/// quick enough not to flicker.
#[cfg_attr(not(target_os = "none"), allow(dead_code))]
pub(crate) const ROW_US: u32 = 1000;

/// How long an LED at each brightness stays lit in its row's
/// [`ROW_US`], in microseconds. Eyes see brightness on a log scale, so
/// each step roughly doubles; level 9 is the whole slot, as bright as
/// `show` lights everything.
#[cfg_attr(not(target_os = "none"), allow(dead_code))]
pub(crate) const ON_US: [u32; MAX_BRIGHTNESS as usize + 1] =
    [0, 8, 16, 30, 55, 100, 180, 320, 560, ROW_US];

/// Where each LED of the 5×5 picture is in the v1's 3×9 matrix, as (row,
/// column). Copied from the BSP's `blocking.rs`, which keeps it private.
/// Row 2's columns 8 and 9 aren't wired to anything.
const V1_LAYOUT: [[(usize, usize); 5]; 5] = [
    [(0, 0), (1, 3), (0, 1), (1, 4), (0, 2)],
    [(2, 3), (2, 4), (2, 5), (2, 6), (2, 7)],
    [(1, 1), (0, 8), (1, 2), (2, 8), (1, 0)],
    [(0, 7), (0, 6), (0, 5), (0, 4), (0, 3)],
    [(2, 2), (1, 6), (2, 0), (1, 5), (2, 1)],
];

/// `image` laid out as the v1 wires it, brightnesses capped at
/// [`MAX_BRIGHTNESS`].
#[cfg_attr(not(feature = "v1"), allow(dead_code))]
pub(crate) fn v1_matrix(image: &Image) -> [[u8; 9]; 3] {
    let mut matrix = [[0; 9]; 3];
    for (line, layout) in image.iter().zip(&V1_LAYOUT) {
        for (&level, &(row, col)) in line.iter().zip(layout) {
            matrix[row][col] = level.min(MAX_BRIGHTNESS);
        }
    }
    matrix
}

/// `image` as the v2 wires it, which is as it's drawn, brightnesses capped
/// at [`MAX_BRIGHTNESS`].
#[cfg_attr(not(feature = "v2"), allow(dead_code))]
pub(crate) fn v2_matrix(image: &Image) -> [[u8; 5]; 5] {
    image.map(|line| line.map(|level| level.min(MAX_BRIGHTNESS)))
}

/// `image` with everything that's lit at full brightness, for `show`.
#[cfg_attr(not(target_os = "none"), allow(dead_code))]
pub(crate) fn full(image: &Image) -> Image {
    image.map(|line| line.map(|level| if level > 0 { MAX_BRIGHTNESS } else { 0 }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_times_grow_and_end_at_the_whole_row() {
        assert_eq!(ON_US[0], 0);
        assert_eq!(ON_US[MAX_BRIGHTNESS as usize], ROW_US);
        assert!(ON_US.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn the_v1_layout_uses_25_different_leds_and_skips_the_unwired_two() {
        let mut used = [[false; 9]; 3];
        for &(row, col) in V1_LAYOUT.iter().flatten() {
            assert!(!used[row][col], "({row}, {col}) twice");
            used[row][col] = true;
        }
        assert!(!used[1][7] && !used[1][8]);
    }

    #[test]
    fn the_v1_matrix_moves_each_led_to_its_wire() {
        let mut image = [[0; 5]; 5];
        image[0][1] = 3;
        image[4][4] = 7;
        let matrix = v1_matrix(&image);
        assert_eq!(matrix[1][3], 3);
        assert_eq!(matrix[2][1], 7);
        assert_eq!(matrix.iter().flatten().filter(|&&l| l > 0).count(), 2);
    }

    #[test]
    fn levels_above_the_brightest_are_capped() {
        let image = [[200; 5]; 5];
        assert_eq!(v2_matrix(&image), [[MAX_BRIGHTNESS; 5]; 5]);
        assert!(
            v1_matrix(&image)
                .iter()
                .flatten()
                .all(|&l| l <= MAX_BRIGHTNESS)
        );
    }

    #[test]
    fn full_lights_anything_nonzero_at_the_brightest() {
        let image = [[0, 1, 5, 9, 255]; 5];
        assert_eq!(full(&image), [[0, 9, 9, 9, 9]; 5]);
    }
}
