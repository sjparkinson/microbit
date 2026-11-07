//! The board itself, on top of the nrf-rs BSP for whichever micro:bit the
//! feature picks. Only built for a real target.

// Both BSP packages call their library `microbit`, and only one is ever a
// dependency.
use microbit as bsp;

use bsp::display::blocking;
use bsp::gpio::{BTN_A, BTN_B};
use bsp::hal::{self, pac};
use embedded_hal::delay::DelayNs;
use embedded_hal::digital::InputPin;

use crate::Image;

/// The parts of the micro:bit that both versions have.
///
/// Taking it takes every peripheral, so anything not listed here (the radio,
/// the edge connector, the v2's speaker, microphone and touch logo) is out of
/// reach until it's added.
pub struct Board {
    /// The 5×5 LED matrix.
    pub display: Display,
    /// Buttons A (left) and B (right).
    pub buttons: Buttons,
    /// A microsecond timer for delays, on TIMER0.
    pub timer: Timer,
}

impl Board {
    /// The board, the first time it's called. After that, `None`.
    pub fn take() -> Option<Self> {
        let board = bsp::Board::take()?;
        Some(Self {
            display: Display(blocking::Display::new(board.display_pins)),
            buttons: Buttons {
                a: board.buttons.button_a,
                b: board.buttons.button_b,
            },
            timer: Timer(hal::Timer::new(board.TIMER0)),
        })
    }
}

/// The LED matrix, driven by the CPU a row at a time.
///
/// Nothing lights unless [`Display::show`] is running: it scans the rows
/// itself, so the CPU's busy for as long as the picture's up. The v1 wires
/// its 25 LEDs as a 3×9 grid rather than 5×5; the BSP hides that.
pub struct Display(blocking::Display);

/// How long the BSP's blocking display lights each row, in milliseconds. It's
/// the BSP's default.
const ROW_MS: u32 = 2;

/// How many rows the display scans: 5 on the v2, and 3 on the v1, which
/// wires its 25 LEDs as a 3×9 grid.
const ROWS: u32 = bsp::gpio::NUM_ROWS as u32;

impl Display {
    /// Shows `image` for `duration_ms` milliseconds, on either board, then
    /// turns every LED off.
    ///
    /// The BSP only lights whole scans of the matrix at 2 ms a row, which is
    /// 10 ms on the v2 and 6 ms on the v1, and on its own it returns early
    /// by whatever's left over. This fills that remainder with one quicker
    /// scan at 1 ms a row where it fits, so every row still gets the same
    /// time lit, then waits out the rest (under a row's worth) dark. So a
    /// 10 ms show on the v1 is 9 ms lit and 1 ms dark, and anything under
    /// a quick scan (5 ms on the v2, 3 ms on the v1) is only a wait.
    pub fn show(&mut self, timer: &mut Timer, image: &Image, duration_ms: u32) {
        let scan_ms = ROWS * ROW_MS;
        let mut rest_ms = duration_ms % scan_ms;
        self.0.show(&mut timer.0, *image, duration_ms - rest_ms);
        // A whole scan at ROW_MS doesn't fit in `rest_ms`, so at most one at
        // 1 ms a row does.
        if rest_ms >= ROWS {
            self.0.set_delay_ms(1);
            self.0.show(&mut timer.0, *image, ROWS);
            self.0.set_delay_ms(ROW_MS);
            rest_ms -= ROWS;
        }
        if rest_ms > 0 {
            timer.0.delay_ms(rest_ms);
        }
    }

    /// Turns every LED off.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// Buttons A and B. Both pull up to 3 V and read low when pressed, on both
/// boards.
///
/// These are raw readings, so a press can bounce for a few milliseconds.
pub struct Buttons {
    a: BTN_A,
    b: BTN_B,
}

impl Buttons {
    /// Whether button A (left) is held down right now.
    pub fn a_pressed(&mut self) -> bool {
        // Reading a GPIO input can't fail on the nRF; the error type is
        // `Infallible`.
        self.a.is_low().unwrap_or(false)
    }

    /// Whether button B (right) is held down right now.
    pub fn b_pressed(&mut self) -> bool {
        self.b.is_low().unwrap_or(false)
    }
}

/// TIMER0, running at 1 MHz, for blocking delays.
pub struct Timer(hal::Timer<pac::TIMER0>);

impl DelayNs for Timer {
    /// Rounds down to a whole microsecond, which is the timer's tick, so
    /// anything under 1000 ns doesn't wait at all.
    fn delay_ns(&mut self, ns: u32) {
        self.0.delay_ns(ns);
    }

    fn delay_us(&mut self, us: u32) {
        self.0.delay_us(us);
    }

    fn delay_ms(&mut self, ms: u32) {
        self.0.delay_ms(ms);
    }
}
