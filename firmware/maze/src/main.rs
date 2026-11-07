//! The firmware: read the accelerometer and buttons, step the game, show
//! the window, every 20 ms. A cross means there's no LSM303AGR to read.
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
    use board::Tilt;
    use maze::{Game, NO_SENSOR, Rng, TICK_MS, noise};

    // `take` only fails on a second call, and this is the first.
    let Some(board::Board {
        mut display,
        mut buttons,
        mut timer,
        motion,
    }) = board::Board::take()
    else {
        panic!("board taken twice");
    };

    let Ok(mut accelerometer) = motion.accelerometer() else {
        loop {
            display.show(&mut timer, &NO_SENSOR, 1000);
        }
    };

    // The sensor takes 70 ms to settle, then its jitter seeds the first
    // maze, so each power-on gets a different one.
    display.show(&mut timer, &[[0; 5]; 5], 100);
    let mut rng = Rng::new(0);
    for _ in 0..8 {
        rng.stir(noise(accelerometer.read().unwrap_or_default()));
        display.show(&mut timer, &[[0; 5]; 5], 10);
    }

    let mut game = Game::new(rng.next());
    loop {
        // A failed read is a flat board for a tick.
        let tilt: Tilt = accelerometer.read().unwrap_or_default();
        let frame = game.tick(tilt, buttons.a_pressed(), buttons.b_pressed());
        // The show is the tick's timing, so the tick runs a hair over
        // TICK_MS with the I2C read.
        display.show_greyscale(&mut timer, &frame, TICK_MS);
    }
}

#[cfg(not(target_os = "none"))]
fn main() {
    eprintln!("maze is firmware for the micro:bit; build it with --target and --features v1 or v2");
    std::process::exit(1);
}
