//! The firmware: scroll the message, over and over.
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
    use hello::{MESSAGE, STEP_MS, frames};

    // `take` only fails on a second call, and this is the first.
    let Some(board::Board {
        mut display,
        mut timer,
        ..
    }) = board::Board::take()
    else {
        panic!("board taken twice");
    };

    loop {
        for frame in frames(MESSAGE) {
            display.show(&mut timer, &frame, STEP_MS);
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {
    eprintln!(
        "hello is firmware for the micro:bit; build it with --target and --features v1 or v2"
    );
    std::process::exit(1);
}
