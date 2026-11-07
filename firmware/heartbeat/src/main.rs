//! The firmware: read the buttons, step the heart, show its frame, every
//! 10 ms.
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
    use heartbeat::{Heart, TICK_MS};

    // `take` only fails on a second call, and this is the first.
    let Some(board::Board {
        mut display,
        mut buttons,
        mut timer,
        ..
    }) = board::Board::take()
    else {
        panic!("board taken twice");
    };

    let mut heart = Heart::new();
    loop {
        let frame = heart.tick(buttons.a_pressed(), buttons.b_pressed());
        // The show is the tick's timing. Reading the buttons and stepping the
        // heart add a few microseconds a pass, so the beat runs a hair slow.
        display.show(&mut timer, frame, TICK_MS);
    }
}

#[cfg(not(target_os = "none"))]
fn main() {
    eprintln!(
        "heartbeat is firmware for the micro:bit; build it with --target and --features v1 or v2"
    );
    std::process::exit(1);
}
