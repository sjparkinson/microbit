//! A spirit level: one lit LED that sits on whichever side of the matrix is
//! lowest, and in the middle when the board's flat.
//!
//! It's also the check that `board`'s accelerometer axes point the way its
//! docs say: tip the right edge down and the dot should go right, tip the
//! edge connector down and it should go to the bottom row.

#![no_std]

use board::{Image, SensorError, Tilt};

/// How many milli-g of tilt move the dot one LED from the middle. At 250 mg
/// a step, about 15° moves it one LED and 30° takes it to the edge.
pub const MG_PER_STEP: i16 = 250;

/// How long each frame shows for, in milliseconds.
pub const FRAME_MS: u32 = 20;

/// Where the dot goes for `tilt`, as (row, column), top left first.
pub fn dot(tilt: Tilt) -> (usize, usize) {
    // Integer division rounds towards zero, so the middle LED covers a band
    // twice as wide as the others: the dot stays put until the board's
    // clearly tilted.
    let step = |mg: i16| (2 + (mg / MG_PER_STEP).clamp(-2, 2)) as usize;
    (step(tilt.y), step(tilt.x))
}

/// The frame for `tilt`: one LED lit.
pub fn frame(tilt: Tilt) -> Image {
    let (row, col) = dot(tilt);
    let mut image = [[0; 5]; 5];
    image[row][col] = 1;
    image
}

/// What shows while the sensor's being turned on: the four corners. If it
/// flashes up and then the display goes dark, turning it on hung.
pub const STARTING: Image = [
    [1, 0, 0, 0, 1],
    [0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0],
    [1, 0, 0, 0, 1],
];

/// What shows when an I2C transfer fails: a cross.
pub const BUS_ERROR: Image = [
    [1, 0, 0, 0, 1],
    [0, 1, 0, 1, 0],
    [0, 0, 1, 0, 0],
    [0, 1, 0, 1, 0],
    [1, 0, 0, 0, 1],
];

/// What shows for `error`. A bus error is [`BUS_ERROR`]. A chip that isn't
/// an LSM303AGR shows the ID it gave in binary, most significant bit first:
/// the high four bits along the second row and the low four along the
/// fourth, with the last column lit on both to tell it from a dot.
pub fn error_frame(error: SensorError) -> Image {
    match error {
        SensorError::Bus => BUS_ERROR,
        SensorError::WrongChip(id) => {
            // Bit 7 first, then 6, and so on, with the marker on the end.
            let row = |high: u8| -> [u8; 5] {
                let bits = id >> high;
                [
                    (bits >> 3) & 1,
                    (bits >> 2) & 1,
                    (bits >> 1) & 1,
                    bits & 1,
                    1,
                ]
            };
            let mut image = [[0; 5]; 5];
            image[1] = row(4);
            image[3] = row(0);
            image
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tilt(x: i16, y: i16) -> Tilt {
        Tilt { x, y, z: -1000 }
    }

    #[test]
    fn sits_in_the_middle_when_flat() {
        assert_eq!(dot(tilt(0, 0)), (2, 2));
        assert_eq!(dot(tilt(249, -249)), (2, 2));
    }

    #[test]
    fn moves_right_as_x_grows_and_down_as_y_does() {
        assert_eq!(dot(tilt(250, 0)), (2, 3));
        assert_eq!(dot(tilt(0, 500)), (4, 2));
        assert_eq!(dot(tilt(-250, -250)), (1, 1));
    }

    #[test]
    fn stops_at_the_edge() {
        assert_eq!(dot(tilt(i16::MAX, i16::MIN)), (0, 4));
        assert_eq!(dot(tilt(-1000, 1000)), (4, 0));
    }

    #[test]
    fn lights_exactly_one_led() {
        let lit: u8 = frame(tilt(300, -600)).iter().flatten().sum();
        assert_eq!(lit, 1);
        assert_eq!(frame(tilt(300, -600))[0][3], 1);
    }

    #[test]
    fn shows_a_wrong_chips_id_in_binary() {
        // 0x5A, the MMA8653's: 0101 1010.
        let image = error_frame(SensorError::WrongChip(0x5a));
        assert_eq!(image[1], [0, 1, 0, 1, 1]);
        assert_eq!(image[3], [1, 0, 1, 0, 1]);
        assert_eq!(image[0], [0; 5]);
        assert_eq!(image[2], [0; 5]);
        assert_eq!(image[4], [0; 5]);
    }

    #[test]
    fn shows_a_cross_for_a_bus_error() {
        assert_eq!(error_frame(SensorError::Bus), BUS_ERROR);
    }
}
