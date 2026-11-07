//! One API over the micro:bit v1 (nRF51822) and v2 (nRF52833).
//!
//! Turn on exactly one of the `v1` and `v2` features, and build for that
//! board's target:
//!
//! | feature | target                  | chip     |
//! |---------|-------------------------|----------|
//! | `v1`    | `thumbv6m-none-eabi`    | nRF51822 |
//! | `v2`    | `thumbv7em-none-eabihf` | nRF52833 |
//!
//! With neither, which is how the host build and `cargo test` see it, the
//! crate is only [`Image`], so firmware libraries can name frames without
//! pulling in a HAL.
//!
//! On a board you get [`Board::take`], which hands out the 5×5 LED matrix,
//! buttons A and B, a timer for delays, and the motion sensor, whose
//! accelerometer works where the board has an LSM303AGR. It sits on the nrf-rs BSPs
//! (`microbit` and `microbit-v2`), and covers only what both boards have.
//! The v2 also has a speaker, a microphone and a touch-sensitive logo; none
//! of those are wrapped here yet.

#![no_std]

#[cfg(all(feature = "v1", feature = "v2"))]
compile_error!("turn on one of the `v1` and `v2` features, not both");

#[cfg(all(
    feature = "v1",
    not(feature = "v2"),
    not(all(target_os = "none", target_abi = "eabi"))
))]
compile_error!(
    "the `v1` feature (micro:bit v1, nRF51822) builds for `--target thumbv6m-none-eabi`"
);

#[cfg(all(
    feature = "v2",
    not(feature = "v1"),
    not(all(target_os = "none", target_abi = "eabihf"))
))]
compile_error!(
    "the `v2` feature (micro:bit v2, nRF52833) builds for `--target thumbv7em-none-eabihf`"
);

#[cfg(all(target_os = "none", not(any(feature = "v1", feature = "v2"))))]
compile_error!(
    "pick a board: `--features v1` for thumbv6m-none-eabi, `--features v2` for thumbv7em-none-eabihf"
);

/// A picture for the LED matrix: five rows of five, top row first, leftmost
/// LED first.
///
/// `Display::show` lights anything other than 0 at full brightness, so 1
/// and 9 look the same there. `Display::show_greyscale` reads each as a
/// brightness from 0 (off) to [`MAX_BRIGHTNESS`].
pub type Image = [[u8; 5]; 5];

mod matrix;
pub use matrix::MAX_BRIGHTNESS;

/// One reading from the accelerometer, in milli-g (1000 is one g) on each of
/// the board's axes.
///
/// The signs are MakeCode's: they read like gravity, pointing towards
/// whichever side is lowest. x and y are checked on a v1.5. z and the v2 come
/// from the micro:bit runtime's source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tilt {
    /// Grows as the right edge (button B's) tips down.
    pub x: i16,
    /// Grows as the bottom edge (the edge connector) tips down.
    pub y: i16,
    /// About -1000 lying flat with the LEDs up, and +1000 face down.
    pub z: i16,
}

/// Why the motion sensor couldn't be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensorError {
    /// An I2C transfer failed: nothing answered, or the bus errored.
    Bus,
    /// Something answered, but its `WHO_AM_I_A` wasn't the LSM303AGR's
    /// 0x33. This is what it said instead.
    WrongChip(u8),
}

impl Tilt {
    /// A reading from the LSM303AGR's six output registers (`OUT_X_L_A` to
    /// `OUT_Z_H_A`, low byte first), set up as `board` sets it: high
    /// resolution and ±2 g, where each 12-bit step is about 1 mg (0.98 in
    /// the datasheet). That makes the left-justified value over 16 the
    /// reading in milli-g, 2% low.
    pub fn from_lsm303agr(raw: [u8; 6]) -> Self {
        let axis = |lo: u8, hi: u8| i16::from_le_bytes([lo, hi]) >> 4;
        let (x, y, z) = (
            axis(raw[0], raw[1]),
            axis(raw[2], raw[3]),
            axis(raw[4], raw[5]),
        );
        // The runtime swaps and negates the chip's axes twice over, which
        // comes out as only y flipped. A 12-bit value can't be -32768, so
        // negating can't overflow.
        Self { x, y: -y, z }
    }
}

#[cfg(test)]
mod tests {
    use super::Tilt;

    /// Packs chip-axis milli-g the way the LSM303AGR would report them.
    fn raw(x: i16, y: i16, z: i16) -> [u8; 6] {
        let [x0, x1] = (x << 4).to_le_bytes();
        let [y0, y1] = (y << 4).to_le_bytes();
        let [z0, z1] = (z << 4).to_le_bytes();
        [x0, x1, y0, y1, z0, z1]
    }

    #[test]
    fn flips_only_the_chips_y() {
        let tilt = Tilt::from_lsm303agr(raw(100, -250, 1000));
        assert_eq!(
            tilt,
            Tilt {
                x: 100,
                y: 250,
                z: 1000
            }
        );
    }

    #[test]
    fn keeps_the_full_range() {
        let tilt = Tilt::from_lsm303agr(raw(-2048, -2048, 2047));
        assert_eq!(
            tilt,
            Tilt {
                x: -2048,
                y: 2048,
                z: 2047
            }
        );
    }

    #[test]
    fn drops_the_four_unused_low_bits() {
        let tilt = Tilt::from_lsm303agr([0x1f, 0x00, 0xff, 0xff, 0x0f, 0x00]);
        assert_eq!(tilt, Tilt { x: 1, y: 1, z: 0 });
    }
}

#[cfg(any(
    all(feature = "v1", target_os = "none", target_abi = "eabi"),
    all(feature = "v2", target_os = "none", target_abi = "eabihf"),
))]
mod hw;

// The v1's nRF51822 has no such thing: its protection is UICR.RBPCONF alone,
// which nothing here sets.
#[cfg(all(feature = "v2", target_os = "none", target_abi = "eabihf"))]
mod approtect;

#[cfg(any(
    all(feature = "v1", target_os = "none", target_abi = "eabi"),
    all(feature = "v2", target_os = "none", target_abi = "eabihf"),
))]
pub use hw::{Accelerometer, Board, Buttons, Display, MotionSensor, Timer};
