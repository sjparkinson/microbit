//! The firmware: read the accelerometer and show where the dot goes, over
//! and over. A cross means there's no LSM303AGR to read.
//!
//! On the host this builds to a stub, so `cargo test` and `cargo clippy`
//! work across the workspace.

#![cfg_attr(target_os = "none", no_std, no_main)]

#[cfg(target_os = "none")]
use panic_halt as _;

// cortex-m-rt's reset handler calls this once memory's set up.
#[cfg(target_os = "none")]
#[cortex_m_rt::entry]
fn main() -> ! {
    use level::{FRAME_MS, STARTING, error_frame, frame};

    // `take` only fails on a second call, and this is the first.
    let Some(board::Board {
        mut display,
        mut timer,
        motion,
        ..
    }) = board::Board::take()
    else {
        panic!("board taken twice");
    };

    display.show(&mut timer, &STARTING, 500);
    let mut accelerometer = match motion.accelerometer(&mut timer) {
        Ok(accelerometer) => accelerometer,
        Err(error) => loop {
            display.show(&mut timer, &error_frame(error), 1000);
        },
    };

    loop {
        // A failed read shows the error for a frame rather than a stale dot.
        let image = accelerometer.read().map_or_else(error_frame, frame);
        display.show(&mut timer, &image, FRAME_MS);
    }
}

#[cfg(not(target_os = "none"))]
fn main() {
    eprintln!(
        "level is firmware for the micro:bit; build it with --target and --features v1 or v2"
    );
    std::process::exit(1);
}
