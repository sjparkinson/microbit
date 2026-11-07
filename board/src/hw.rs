//! The board itself, on top of the nrf-rs BSP for whichever micro:bit the
//! feature picks. Only built for a real target.

// Both BSP packages call their library `microbit`, and only one is ever a
// dependency.
use microbit as bsp;

use bsp::display::blocking;
use bsp::gpio::{BTN_A, BTN_B};
use bsp::hal::{self, pac};
use embedded_hal::delay::DelayNs;
use embedded_hal::digital::{InputPin, OutputPin};

use crate::{Image, SensorError, Tilt};

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
    /// The motion sensor, not yet spoken to. [`MotionSensor::accelerometer`]
    /// turns it on.
    pub motion: MotionSensor,
}

impl Board {
    /// The board, the first time it's called. After that, `None`.
    ///
    /// On a v2 it also keeps the debug port open across resets, on the
    /// nRF52833s that need telling (see `approtect.rs`). The v1 doesn't need
    /// it.
    pub fn take() -> Option<Self> {
        let board = bsp::Board::take()?;
        #[cfg(feature = "v2")]
        crate::approtect::keep_debug_port_open();
        let mut timer = hal::Timer::new(board.TIMER0);
        // The v1 has one I2C bus, shared with the edge connector, and the v2
        // has the sensor on an internal bus of its own.
        #[cfg(feature = "v1")]
        let bus = {
            let hal::twi::Pins { scl, sda } = board.i2c.into();
            let (scl, sda) = free_bus(scl, sda, &mut timer);
            hal::Twi::new(
                board.TWI0,
                hal::twi::Pins { scl, sda },
                hal::twi::Frequency::K100,
            )
        };
        #[cfg(feature = "v2")]
        let bus = {
            let hal::twim::Pins { scl, sda } = board.i2c_internal.into();
            let (scl, sda) = free_bus(scl, sda, &mut timer);
            hal::Twim::new(
                board.TWIM0,
                hal::twim::Pins { scl, sda },
                hal::twim::Frequency::K100,
            )
        };
        Some(Self {
            display: Display(blocking::Display::new(board.display_pins)),
            buttons: Buttons {
                a: board.buttons.button_a,
                b: board.buttons.button_b,
            },
            timer: Timer(timer),
            motion: MotionSensor(bus),
        })
    }
}

type BusPin = hal::gpio::Pin<hal::gpio::Input<hal::gpio::Floating>>;

/// Lets go of a sensor left stuck part-way through an I2C read.
///
/// A reset restarts the nRF but not the sensor, which keeps its power. If
/// the reset lands while the sensor's sending a byte, it can be left holding
/// SDA low, waiting for clocks that never come, and then nothing can start a
/// transfer: the nRF51's TWI waits for the bus forever. So this clocks SCL
/// nine times, which is enough to finish any byte and its acknowledge, then
/// sends a STOP. With the bus already idle, it's a few harmless pulses that
/// nothing answers.
fn free_bus(scl: BusPin, sda: BusPin, timer: &mut impl DelayNs) -> (BusPin, BusPin) {
    use hal::gpio::{Level, OpenDrainConfig::Standard0Disconnect1};
    // Driving an nRF output can't fail; the error type is `Infallible`.
    let mut scl = scl.into_open_drain_output(Standard0Disconnect1, Level::High);
    let mut sda = sda.into_open_drain_output(Standard0Disconnect1, Level::High);
    // 5 µs a half-clock is 100 kHz, the speed the bus runs at anyway.
    for _ in 0..9 {
        scl.set_low().ok();
        timer.delay_us(5);
        scl.set_high().ok();
        timer.delay_us(5);
    }
    // A STOP is SDA rising while SCL is high.
    scl.set_low().ok();
    sda.set_low().ok();
    timer.delay_us(5);
    scl.set_high().ok();
    timer.delay_us(5);
    sda.set_high().ok();
    timer.delay_us(5);
    (scl.into_floating_input(), sda.into_floating_input())
}

/// The I2C bus the motion sensor's on.
#[cfg(feature = "v1")]
type SensorBus = hal::Twi<pac::TWI0>;
#[cfg(feature = "v2")]
type SensorBus = hal::Twim<pac::TWIM0>;

/// The motion sensor, on its I2C bus, before anything's been sent to it.
pub struct MotionSensor(SensorBus);

impl MotionSensor {
    /// Checks the chip is an LSM303AGR and turns its accelerometer on.
    ///
    /// It talks to the chip, so it blocks for the length of three short I2C
    /// transfers. [`Board::take`] leaves it to the firmware so that a
    /// firmware can have something on the display first.
    pub fn accelerometer(self) -> Result<Accelerometer, SensorError> {
        Accelerometer::new(self.0)
    }
}

/// The LSM303AGR's accelerometer, at 7-bit I2C address 0x19.
const LSM303AGR_A: u8 = 0x19;
/// `WHO_AM_I_A`, which reads 0x33 on an LSM303AGR.
const WHO_AM_I_A: u8 = 0x0f;
/// `CTRL_REG1_A`: data rate and which axes are on.
const CTRL_REG1_A: u8 = 0x20;
/// `CTRL_REG4_A`: range, resolution and block data update.
const CTRL_REG4_A: u8 = 0x23;
/// `OUT_X_L_A`, the first of the six output registers.
const OUT_X_L_A: u8 = 0x28;
/// Set in a register address to read on through the next ones.
const AUTO_INCREMENT: u8 = 0x80;

/// The accelerometer in an ST LSM303AGR, which every v1.5 has and some v2s
/// do.
///
/// The v1.3x has an NXP MMA8653FC instead, and a v2 may have an NXP
/// FXOS8700CQ. Neither is supported, so those boards get no accelerometer.
/// It's set to 100 readings a second at ±2 g in high-resolution mode, and
/// takes 70 ms after it's turned on to settle, so the first few readings can
/// be off.
pub struct Accelerometer(SensorBus);

impl Accelerometer {
    fn new(bus: SensorBus) -> Result<Self, SensorError> {
        let mut sensor = Self(bus);
        let mut id = [0];
        sensor.read_registers(WHO_AM_I_A, &mut id)?;
        if id[0] != 0x33 {
            return Err(SensorError::WrongChip(id[0]));
        }
        // Block data update (so a reading's two bytes always match),
        // ±2 g, high resolution.
        sensor.write_register(CTRL_REG4_A, 0x88)?;
        // 100 Hz, normal power, X, Y and Z on.
        sensor.write_register(CTRL_REG1_A, 0x57)?;
        Ok(sensor)
    }

    /// The latest reading. Reading faster than 100 times a second returns
    /// the same one again.
    pub fn read(&mut self) -> Result<Tilt, SensorError> {
        let mut raw = [0; 6];
        self.read_registers(OUT_X_L_A | AUTO_INCREMENT, &mut raw)?;
        Ok(Tilt::from_lsm303agr(raw))
    }

    // These call the HALs' own methods rather than embedded-hal's `I2c`.
    // nrf-hal's `I2c` for the nRF51's TWI starts a read without the
    // shortcuts that pause the bus after each byte, which the nRF51 needs,
    // and hangs. The buffers are locals, never `&[CONST]`, because the v2's
    // TWIM can only send from RAM and a promoted constant lives in flash.

    fn read_registers(&mut self, register: u8, buffer: &mut [u8]) -> Result<(), SensorError> {
        let tx = [register];
        self.0
            .write_then_read(LSM303AGR_A, &tx, buffer)
            .map_err(|_| SensorError::Bus)
    }

    fn write_register(&mut self, register: u8, value: u8) -> Result<(), SensorError> {
        let tx = [register, value];
        self.0.write(LSM303AGR_A, &tx).map_err(|_| SensorError::Bus)
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
