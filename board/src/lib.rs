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
//! buttons A and B, and a timer for delays. It sits on the nrf-rs BSPs
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
/// LED first. Anything other than 0 is lit.
///
/// The blocking display underneath has no brightness levels, so 1 and 9 look
/// the same.
pub type Image = [[u8; 5]; 5];

#[cfg(any(
    all(feature = "v1", target_os = "none", target_abi = "eabi"),
    all(feature = "v2", target_os = "none", target_abi = "eabihf"),
))]
mod hw;

#[cfg(any(
    all(feature = "v1", target_os = "none", target_abi = "eabi"),
    all(feature = "v2", target_os = "none", target_abi = "eabihf"),
))]
pub use hw::{Board, Buttons, Display, Timer};
