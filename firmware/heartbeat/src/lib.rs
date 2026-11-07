//! A heart that beats on the LED matrix. Button A slows it down and button B
//! speeds it up, a step per press, between [`MIN_BPM`] and [`MAX_BPM`].
//!
//! Everything here is plain logic, tested on the host. `src/main.rs` reads the
//! buttons, calls [`Heart::tick`] and shows what it returns, every
//! [`TICK_MS`].

#![no_std]

use board::Image;

/// How long one pass of the main loop takes, in milliseconds. `board`'s
/// display shows a frame for exactly this long on both boards; it's one full
/// scan of the v2's matrix, and a scan and a bit on the v1's.
pub const TICK_MS: u32 = 10;

/// The slowest the heart goes, in beats a minute.
pub const MIN_BPM: u32 = 40;
/// The fastest. The beat's shape takes [`BEAT_MS`], and at 150 that leaves
/// 100 ms of rest before the next one, which is about as short as still
/// looks like a heartbeat.
pub const MAX_BPM: u32 = 150;
/// Where it starts.
pub const DEFAULT_BPM: u32 = 70;
/// How far one press moves it.
pub const STEP_BPM: u32 = 10;

/// How long each of the big, small, big frames of a beat stays up.
const PULSE_MS: u32 = 100;
/// The whole "lub-dub": big, small, big. The heart sits small for the rest of
/// the period.
pub const BEAT_MS: u32 = 3 * PULSE_MS;

/// How many ticks in a row a button has to read the same before it counts,
/// which is 20 ms at [`TICK_MS`].
const DEBOUNCE_TICKS: u8 = 2;

/// The heart at full size.
pub const BIG: Image = [
    [0, 1, 0, 1, 0],
    [1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1],
    [0, 1, 1, 1, 0],
    [0, 0, 1, 0, 0],
];

/// The heart between beats.
pub const SMALL: Image = [
    [0, 0, 0, 0, 0],
    [0, 1, 0, 1, 0],
    [0, 1, 1, 1, 0],
    [0, 0, 1, 0, 0],
    [0, 0, 0, 0, 0],
];

/// How long one beat lasts at `bpm`, in milliseconds, rounded down.
pub const fn period_ms(bpm: u32) -> u32 {
    60_000 / bpm
}

/// Which frame shows `phase_ms` into a beat.
pub const fn frame_at(phase_ms: u32) -> &'static Image {
    if phase_ms < PULSE_MS {
        &BIG
    } else if phase_ms < 2 * PULSE_MS {
        &SMALL
    } else if phase_ms < BEAT_MS {
        &BIG
    } else {
        &SMALL
    }
}

/// The new rate after a tick's presses. A press on A slows by [`STEP_BPM`],
/// a press on B speeds up by the same, and it stops at the limits rather than
/// wrapping. Fresh presses on both in the same tick cancel out.
pub const fn adjust(bpm: u32, slower: bool, faster: bool) -> u32 {
    match (slower, faster) {
        (true, false) if bpm >= MIN_BPM + STEP_BPM => bpm - STEP_BPM,
        (true, false) => MIN_BPM,
        (false, true) if bpm + STEP_BPM <= MAX_BPM => bpm + STEP_BPM,
        (false, true) => MAX_BPM,
        _ => bpm,
    }
}

/// Turns raw button readings, one a tick, into presses.
///
/// The button only changes state once it's read the new way for
/// [`DEBOUNCE_TICKS`] ticks running, so a bounce on press or release doesn't
/// count twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Debounce {
    pressed: bool,
    disagreeing: u8,
}

impl Debounce {
    /// A button that starts released.
    pub const fn new() -> Self {
        Self {
            pressed: false,
            disagreeing: 0,
        }
    }

    /// Feeds in this tick's reading. True on the tick a press is accepted,
    /// and never again until it's been released.
    pub const fn update(&mut self, raw: bool) -> bool {
        if raw == self.pressed {
            self.disagreeing = 0;
            return false;
        }
        self.disagreeing += 1;
        if self.disagreeing < DEBOUNCE_TICKS {
            return false;
        }
        self.pressed = raw;
        self.disagreeing = 0;
        raw
    }

    /// Whether the button counts as held down.
    pub const fn pressed(&self) -> bool {
        self.pressed
    }
}

impl Default for Debounce {
    fn default() -> Self {
        Self::new()
    }
}

/// The heart: its rate, where it is in the current beat, and the buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heart {
    bpm: u32,
    phase_ms: u32,
    a: Debounce,
    b: Debounce,
}

impl Heart {
    /// A heart at [`DEFAULT_BPM`], at the start of a beat.
    pub const fn new() -> Self {
        Self {
            bpm: DEFAULT_BPM,
            phase_ms: 0,
            a: Debounce::new(),
            b: Debounce::new(),
        }
    }

    /// The current rate, in beats a minute.
    pub const fn bpm(&self) -> u32 {
        self.bpm
    }

    /// How far into the current beat it is, in milliseconds.
    pub const fn phase_ms(&self) -> u32 {
        self.phase_ms
    }

    /// One tick: takes whether A and B read pressed right now, and returns
    /// the frame to show for the next [`TICK_MS`].
    ///
    /// A new rate takes effect straight away. If the beat's already past the
    /// end of the new, shorter period, the next beat starts on the next tick.
    pub const fn tick(&mut self, a_raw: bool, b_raw: bool) -> &'static Image {
        let slower = self.a.update(a_raw);
        let faster = self.b.update(b_raw);
        self.bpm = adjust(self.bpm, slower, faster);

        let frame = frame_at(self.phase_ms);
        let period = period_ms(self.bpm);
        self.phase_ms = if self.phase_ms >= period {
            // Only after a speed-up: the phase is already past the new
            // period's end, and wrapping it would land mid-beat.
            0
        } else {
            // The remainder carries into the next beat, so a period that
            // isn't a whole number of ticks still averages out to the right
            // rate.
            (self.phase_ms + TICK_MS) % period
        };
        frame
    }
}

impl Default for Heart {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ticks with both buttons up `n` times.
    fn idle(heart: &mut Heart, n: u32) {
        for _ in 0..n {
            heart.tick(false, false);
        }
    }

    /// A clean press and release of one button, long enough to debounce.
    fn press(heart: &mut Heart, a: bool, b: bool) {
        for _ in 0..DEBOUNCE_TICKS {
            heart.tick(a, b);
        }
        idle(heart, u32::from(DEBOUNCE_TICKS));
    }

    #[test]
    fn frames_light_the_right_number_of_leds() {
        let lit = |image: &Image| image.iter().flatten().filter(|&&led| led != 0).count();
        assert_eq!(lit(&BIG), 16);
        assert_eq!(lit(&SMALL), 6);
    }

    #[test]
    fn small_heart_fits_inside_big_one() {
        for (big, small) in BIG.iter().flatten().zip(SMALL.iter().flatten()) {
            assert!(*small == 0 || *big != 0);
        }
    }

    #[test]
    fn beat_goes_big_small_big_then_rests() {
        assert_eq!(frame_at(0), &BIG);
        assert_eq!(frame_at(PULSE_MS - 1), &BIG);
        assert_eq!(frame_at(PULSE_MS), &SMALL);
        assert_eq!(frame_at(2 * PULSE_MS - 1), &SMALL);
        assert_eq!(frame_at(2 * PULSE_MS), &BIG);
        assert_eq!(frame_at(BEAT_MS - 1), &BIG);
        assert_eq!(frame_at(BEAT_MS), &SMALL);
        assert_eq!(frame_at(period_ms(MIN_BPM) - 1), &SMALL);
    }

    #[test]
    fn beat_fits_in_the_fastest_period_with_rest_to_spare() {
        assert!(BEAT_MS + 100 <= period_ms(MAX_BPM));
    }

    #[test]
    fn limits_are_whole_steps_from_the_default() {
        assert_eq!((DEFAULT_BPM - MIN_BPM) % STEP_BPM, 0);
        assert_eq!((MAX_BPM - DEFAULT_BPM) % STEP_BPM, 0);
    }

    #[test]
    fn period_is_sixty_seconds_over_bpm() {
        assert_eq!(period_ms(60), 1000);
        assert_eq!(period_ms(MIN_BPM), 1500);
        assert_eq!(period_ms(MAX_BPM), 400);
        assert_eq!(period_ms(70), 857);
    }

    #[test]
    fn adjust_steps_and_stops_at_the_limits() {
        assert_eq!(adjust(70, true, false), 60);
        assert_eq!(adjust(70, false, true), 80);
        assert_eq!(adjust(MIN_BPM, true, false), MIN_BPM);
        assert_eq!(adjust(MAX_BPM, false, true), MAX_BPM);
        assert_eq!(adjust(MIN_BPM + 5, true, false), MIN_BPM);
        assert_eq!(adjust(MAX_BPM - 5, false, true), MAX_BPM);
    }

    #[test]
    fn adjust_ignores_both_or_neither() {
        assert_eq!(adjust(70, false, false), 70);
        assert_eq!(adjust(70, true, true), 70);
        assert_eq!(adjust(MIN_BPM, true, true), MIN_BPM);
    }

    #[test]
    fn debounce_needs_two_ticks_to_press() {
        let mut button = Debounce::new();
        assert!(!button.update(true));
        assert!(!button.pressed());
        assert!(button.update(true));
        assert!(button.pressed());
    }

    #[test]
    fn debounce_reports_a_press_once_while_held() {
        let mut button = Debounce::new();
        button.update(true);
        button.update(true);
        for _ in 0..50 {
            assert!(!button.update(true));
        }
        assert!(button.pressed());
    }

    #[test]
    fn debounce_ignores_a_one_tick_blip() {
        let mut button = Debounce::new();
        assert!(!button.update(true));
        assert!(!button.update(false));
        assert!(!button.update(true));
        assert!(!button.update(false));
        assert!(!button.pressed());
    }

    #[test]
    fn debounce_ignores_a_bounce_on_release() {
        let mut button = Debounce::new();
        button.update(true);
        button.update(true);
        // One tick up, then down again: still the same press.
        assert!(!button.update(false));
        assert!(!button.update(true));
        assert!(button.pressed());
        // Released for real, then pressed again: a second press.
        button.update(false);
        button.update(false);
        assert!(!button.pressed());
        button.update(true);
        assert!(button.update(true));
    }

    #[test]
    fn heart_starts_at_the_default_on_a_big_frame() {
        let mut heart = Heart::new();
        assert_eq!(heart.bpm(), DEFAULT_BPM);
        assert_eq!(heart.tick(false, false), &BIG);
    }

    #[test]
    fn a_slows_and_b_speeds_up() {
        let mut heart = Heart::new();
        press(&mut heart, true, false);
        assert_eq!(heart.bpm(), DEFAULT_BPM - STEP_BPM);
        press(&mut heart, false, true);
        press(&mut heart, false, true);
        assert_eq!(heart.bpm(), DEFAULT_BPM + STEP_BPM);
    }

    #[test]
    fn holding_a_button_is_one_press() {
        let mut heart = Heart::new();
        for _ in 0..500 {
            heart.tick(false, true);
        }
        assert_eq!(heart.bpm(), DEFAULT_BPM + STEP_BPM);
    }

    #[test]
    fn presses_stop_at_the_limits() {
        let mut heart = Heart::new();
        for _ in 0..30 {
            press(&mut heart, true, false);
        }
        assert_eq!(heart.bpm(), MIN_BPM);
        for _ in 0..30 {
            press(&mut heart, false, true);
        }
        assert_eq!(heart.bpm(), MAX_BPM);
    }

    #[test]
    fn beats_repeat_every_period() {
        let mut heart = Heart::new();
        press(&mut heart, true, false);
        press(&mut heart, true, false);
        assert_eq!(heart.bpm(), 50);
        // 1200 ms is 120 ticks exactly, so each beat lines up with the last.
        assert_eq!(period_ms(50), 120 * TICK_MS);
        let start = heart.phase_ms();
        let first: [&Image; 120] = core::array::from_fn(|_| heart.tick(false, false));
        let second: [&Image; 120] = core::array::from_fn(|_| heart.tick(false, false));
        assert_eq!(first, second);
        assert_eq!(heart.phase_ms(), start);
    }

    #[test]
    fn one_beat_has_two_big_pulses() {
        let mut heart = Heart::new();
        // Start of a beat at 60 bpm, so a beat is 100 ticks.
        press(&mut heart, true, false);
        while heart.phase_ms() != 0 {
            heart.tick(false, false);
        }
        let frames: [&Image; 100] = core::array::from_fn(|_| heart.tick(false, false));
        let rising = frames
            .windows(2)
            .filter(|pair| pair[0] == &SMALL && pair[1] == &BIG)
            .count();
        // Big at the start, then one more rise from small to big for the
        // "dub".
        assert_eq!(frames[0], &BIG);
        assert_eq!(rising, 1);
        assert_eq!(frames.iter().filter(|&&f| f == &BIG).count(), 20);
    }

    #[test]
    fn a_shorter_period_behind_the_phase_starts_a_new_beat() {
        let mut heart = Heart::new();
        // 1300 ms into a 40 bpm beat (1500 ms), then up to 50 (1200 ms),
        // which ends before where the beat already is.
        for _ in 0..3 {
            press(&mut heart, true, false);
        }
        assert_eq!(heart.bpm(), MIN_BPM);
        while heart.phase_ms() != 0 {
            heart.tick(false, false);
        }
        idle(&mut heart, 130);
        assert_eq!(heart.phase_ms(), 1300);
        // The press lands on its second tick, which still shows the rest.
        assert_eq!(heart.tick(false, true), &SMALL);
        assert_eq!(heart.tick(false, true), &SMALL);
        assert_eq!(heart.bpm(), 50);
        // Wrapping 1320 into 1200 would give 120, on the small frame
        // between the pulses. It starts the beat over instead.
        assert_eq!(heart.phase_ms(), 0);
        assert_eq!(heart.tick(false, false), &BIG);
    }

    #[test]
    fn phase_carries_the_remainder_across_beats() {
        let mut heart = Heart::new();
        // 70 bpm is 857 ms, not a whole number of 10 ms ticks: after 86 ticks
        // (860 ms) it's 3 ms into the next beat.
        idle(&mut heart, 86);
        assert_eq!(heart.phase_ms(), 3);
    }
}
