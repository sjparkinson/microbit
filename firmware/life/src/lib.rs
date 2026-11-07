//! Conway's Game of Life on the 5×5 matrix, with the edges wrapping round, so
//! the grid's a torus and a glider that walks off the right comes back on
//! the left.
//!
//! It starts from a random soup and steps a generation every
//! [`GENERATION_MS`]. Once the grid dies out or repeats one of its last
//! [`HISTORY`] generations (a still life, a blinker, a glider gone all the way
//! round), it carries on for [`HOLD_GENERATIONS`] more so you can see how it
//! ended, then starts a new soup.
//!
//! Button A throws down a new soup straight away. Button B pauses and
//! resumes.
//!
//! There's no entropy source on `board` yet, so every power-on starts from
//! the same soup and the same sequence after it. Each button press stirs in
//! how many ticks it's been since power-on, which is as random as human
//! timing, so the soups part ways from the first press.
//!
//! Everything here is plain logic, tested on the host. `src/main.rs` reads the
//! buttons, calls [`Game::tick`] and shows what it returns, every
//! [`TICK_MS`].

#![no_std]

use board::Image;

/// How long one pass of the main loop takes, in milliseconds, which is one
/// show of the frame.
pub const TICK_MS: u32 = 10;

/// How long each generation stays up.
pub const GENERATION_MS: u32 = 400;

/// How many past generations it remembers to spot a repeat. A glider on a
/// 5×5 torus takes 20 generations to get back where it started, the longest
/// cycle there is room for that's worth waiting out, so this covers it with
/// a few to spare.
pub const HISTORY: usize = 24;

/// How many generations it keeps going once it's settled, before a new
/// soup: two seconds at [`GENERATION_MS`].
pub const HOLD_GENERATIONS: u8 = 5;

/// How many ticks in a row a button has to read the same before it counts,
/// which is 20 ms at [`TICK_MS`].
const DEBOUNCE_TICKS: u8 = 2;

/// Where the random numbers start on every power-on. Any non-zero value
/// works.
const SEED: u32 = 0x2545_f491;

/// The side of the grid.
const SIZE: usize = 5;

/// All 25 cells lit.
const ALL: u32 = (1 << (SIZE * SIZE)) - 1;

/// The 5×5 grid, one bit a cell: the cell at column `x` and row `y` (top
/// left is 0, 0) is bit `y * 5 + x`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Grid(u32);

impl Grid {
    /// Every cell dead.
    pub const EMPTY: Self = Self(0);

    /// The grid from 25 bits, in the layout above. Anything past bit 24 is
    /// dropped.
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits & ALL)
    }

    /// The grid as 25 bits.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The grid from a picture, any non-zero LED counting as alive.
    pub fn from_image(image: &Image) -> Self {
        let mut bits = 0;
        for (y, row) in image.iter().enumerate() {
            for (x, &led) in row.iter().enumerate() {
                if led != 0 {
                    bits |= bit(x, y);
                }
            }
        }
        Self(bits)
    }

    /// The grid as a picture for the display.
    pub fn image(self) -> Image {
        let mut image = [[0; SIZE]; SIZE];
        for (y, row) in image.iter_mut().enumerate() {
            for (x, led) in row.iter_mut().enumerate() {
                *led = u8::from(self.alive(x, y));
            }
        }
        image
    }

    /// Whether the cell at column `x`, row `y` is alive.
    pub const fn alive(self, x: usize, y: usize) -> bool {
        self.0 & bit(x, y) != 0
    }

    /// Whether every cell's dead.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// How many cells are alive.
    pub const fn population(self) -> u32 {
        self.0.count_ones()
    }

    /// How many of the eight cells round `x`, `y` are alive, wrapping at the
    /// edges.
    pub fn neighbours(self, x: usize, y: usize) -> u32 {
        // SIZE - 1 is one step back, once it's wrapped.
        let steps = [SIZE - 1, 0, 1];
        let mut count = 0;
        for dy in steps {
            for dx in steps {
                if (dx, dy) != (0, 0) && self.alive((x + dx) % SIZE, (y + dy) % SIZE) {
                    count += 1;
                }
            }
        }
        count
    }

    /// The next generation: a live cell with two or three live neighbours
    /// stays alive, a dead one with exactly three comes alive, and
    /// everything else is dead.
    pub fn step(self) -> Self {
        let mut next = 0;
        for y in 0..SIZE {
            for x in 0..SIZE {
                let n = self.neighbours(x, y);
                if n == 3 || (n == 2 && self.alive(x, y)) {
                    next |= bit(x, y);
                }
            }
        }
        Self(next)
    }
}

/// The bit for column `x`, row `y`.
const fn bit(x: usize, y: usize) -> u32 {
    1 << (y * SIZE + x)
}

/// A xorshift32 generator: small, quick on a Cortex-M0, and plenty random
/// for picking soups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rng(u32);

impl Rng {
    /// A generator from `seed`. Xorshift sticks at zero, so a zero seed
    /// starts from [`SEED`] instead.
    pub const fn new(seed: u32) -> Self {
        Self(if seed == 0 { SEED } else { seed })
    }

    /// The next 32 random bits.
    pub const fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Mixes `value` into the state, keeping it off zero.
    pub const fn stir(&mut self, value: u32) {
        *self = Self::new(self.0 ^ value);
        self.next();
    }

    /// A random grid with each cell alive three times in eight, which is
    /// about nine cells. Never empty.
    pub const fn soup(&mut self) -> Grid {
        loop {
            let a = self.next();
            let b = self.next();
            let c = self.next();
            let grid = Grid::from_bits(a & (b | c));
            if !grid.is_empty() {
                return grid;
            }
        }
    }
}

/// Turns raw button readings, one a tick, into presses. The same as
/// heartbeat's.
///
/// The button only changes state once it's read the new way for
/// [`DEBOUNCE_TICKS`] ticks running, so a bounce on press or release doesn't
/// count twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
}

/// The game: the grid, what it's been lately, and the buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    grid: Grid,
    /// The generations before this one, oldest overwritten first. Only the
    /// first `remembered` are real.
    history: [Grid; HISTORY],
    remembered: usize,
    /// Where the next one goes in `history`.
    slot: usize,
    /// Generations since it died out or repeated, or `None` while it's
    /// still going.
    settled: Option<u8>,
    paused: bool,
    /// Milliseconds since the last generation.
    since_ms: u32,
    /// Ticks since power-on, for stirring into `rng`.
    ticks: u32,
    rng: Rng,
    a: Debounce,
    b: Debounce,
}

impl Game {
    /// A game on the power-on soup.
    pub const fn new() -> Self {
        let mut rng = Rng::new(SEED);
        let grid = rng.soup();
        Self::with(grid, rng)
    }

    /// A game starting from `grid`, with the power-on random numbers for
    /// the soups after it.
    pub const fn starting(grid: Grid) -> Self {
        Self::with(grid, Rng::new(SEED))
    }

    const fn with(grid: Grid, rng: Rng) -> Self {
        Self {
            grid,
            history: [Grid::EMPTY; HISTORY],
            remembered: 0,
            slot: 0,
            settled: None,
            paused: false,
            since_ms: 0,
            ticks: 0,
            rng,
            a: Debounce::new(),
            b: Debounce::new(),
        }
    }

    /// The grid as it is now.
    pub const fn grid(&self) -> Grid {
        self.grid
    }

    /// Whether it's died out or started repeating.
    pub const fn settled(&self) -> bool {
        self.settled.is_some()
    }

    /// Whether B has it paused.
    pub const fn paused(&self) -> bool {
        self.paused
    }

    /// One tick: takes whether A and B read pressed right now, and returns
    /// the frame to show for the next [`TICK_MS`].
    pub fn tick(&mut self, a_raw: bool, b_raw: bool) -> Image {
        self.ticks = self.ticks.wrapping_add(1);
        if self.a.update(a_raw) {
            self.rng.stir(self.ticks);
            self.reseed();
        }
        if self.b.update(b_raw) {
            self.rng.stir(self.ticks);
            self.paused = !self.paused;
        }
        if !self.paused {
            self.since_ms += TICK_MS;
            if self.since_ms >= GENERATION_MS {
                self.since_ms = 0;
                self.advance();
            }
        }
        self.grid.image()
    }

    /// One generation on, or a new soup once it's been settled for
    /// [`HOLD_GENERATIONS`].
    pub fn advance(&mut self) {
        if let Some(held) = self.settled {
            if held >= HOLD_GENERATIONS {
                self.reseed();
                return;
            }
            self.settled = Some(held + 1);
        }

        let next = self.grid.step();
        self.remember(self.grid);
        self.grid = next;
        if self.settled.is_none() && (next.is_empty() || self.seen(next)) {
            self.settled = Some(0);
        }
    }

    /// Starts over on a new soup, forgetting the old history.
    fn reseed(&mut self) {
        let grid = self.rng.soup();
        let rng = self.rng;
        *self = Self {
            paused: self.paused,
            ticks: self.ticks,
            a: self.a,
            b: self.b,
            ..Self::with(grid, rng)
        };
    }

    fn remember(&mut self, grid: Grid) {
        self.history[self.slot] = grid;
        self.slot = (self.slot + 1) % HISTORY;
        if self.remembered < HISTORY {
            self.remembered += 1;
        }
    }

    /// Whether `grid` is one of the remembered generations.
    fn seen(&self, grid: Grid) -> bool {
        self.history[..self.remembered].contains(&grid)
    }
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::string::String;
    use std::vec::Vec;

    use super::*;

    const TICKS_PER_GENERATION: u32 = GENERATION_MS / TICK_MS;

    /// A grid from text: `#` alive, anything else dead, rows split on `/`.
    fn grid(text: &str) -> Grid {
        let mut image = [[0; 5]; 5];
        for (y, row) in text.split('/').enumerate() {
            for (x, c) in row.chars().enumerate() {
                image[y][x] = u8::from(c == '#');
            }
        }
        Grid::from_image(&image)
    }

    /// The grid as text, the other way.
    fn draw(grid: Grid) -> String {
        (0..5)
            .map(|y| {
                (0..5)
                    .map(|x| if grid.alive(x, y) { '#' } else { '.' })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    /// `grid` moved `dx` right and `dy` down, wrapping.
    fn shifted(grid: Grid, dx: usize, dy: usize) -> Grid {
        let mut bits = 0;
        for y in 0..5 {
            for x in 0..5 {
                if grid.alive(x, y) {
                    bits |= bit((x + dx) % 5, (y + dy) % 5);
                }
            }
        }
        Grid(bits)
    }

    const GLIDER: &str = ".#.../..#../###../...../.....";

    /// Ticks with both buttons up `n` times.
    fn idle(game: &mut Game, n: u32) {
        for _ in 0..n {
            game.tick(false, false);
        }
    }

    /// A clean press and release, long enough to debounce.
    fn press(game: &mut Game, a: bool, b: bool) {
        for _ in 0..DEBOUNCE_TICKS {
            game.tick(a, b);
        }
        for _ in 0..DEBOUNCE_TICKS {
            game.tick(false, false);
        }
    }

    #[test]
    fn text_and_images_round_trip() {
        let glider = grid(GLIDER);
        assert_eq!(draw(glider), GLIDER);
        assert_eq!(Grid::from_image(&glider.image()), glider);
        assert_eq!(glider.population(), 5);
        assert!(glider.alive(1, 0));
        assert!(!glider.alive(0, 0));
    }

    #[test]
    fn from_bits_drops_anything_past_the_grid() {
        assert_eq!(Grid::from_bits(u32::MAX).bits(), ALL);
        assert_eq!(Grid::from_bits(u32::MAX).population(), 25);
    }

    #[test]
    fn neighbours_wrap_round_the_edges() {
        // Each of the top left corner's eight neighbours, most of them on the
        // far side.
        let ring = grid("##..#/##..#/...../...../##..#");
        assert_eq!(ring.neighbours(0, 0), 8);
        let lone = grid("#..../...../...../...../.....");
        assert_eq!(lone.neighbours(4, 4), 1);
        assert_eq!(lone.neighbours(0, 0), 0);
        assert_eq!(lone.neighbours(2, 2), 0);
    }

    #[test]
    fn empty_stays_empty() {
        assert_eq!(Grid::EMPTY.step(), Grid::EMPTY);
    }

    #[test]
    fn lone_cells_and_pairs_die() {
        assert!(grid("...../..#../...../...../.....").step().is_empty());
        assert!(grid("...../..##./...../...../.....").step().is_empty());
    }

    #[test]
    fn a_block_stays_put() {
        let block = grid("...../.##../.##../...../.....");
        assert_eq!(block.step(), block);
    }

    #[test]
    fn a_blinker_flips_every_generation() {
        let across = grid("...../...../.###./...../.....");
        let down = grid("...../..#../..#../..#../.....");
        assert_eq!(across.step(), down);
        assert_eq!(down.step(), across);
    }

    #[test]
    fn a_blinker_on_the_edge_wraps() {
        let across = grid("...../...../##..#/...../.....");
        let down = grid("...../#..../#..../#..../.....");
        assert_eq!(across.step(), down);
    }

    #[test]
    fn a_glider_moves_one_down_and_right_every_four() {
        let glider = grid(GLIDER);
        let mut g = glider;
        for _ in 0..4 {
            g = g.step();
        }
        assert_eq!(g, shifted(glider, 1, 1));
    }

    #[test]
    fn a_glider_gets_home_in_twenty() {
        let glider = grid(GLIDER);
        let mut g = glider;
        for generation in 1..=20 {
            g = g.step();
            assert_eq!(g.population(), 5);
            assert_eq!(g == glider, generation == 20, "generation {generation}");
        }
    }

    #[test]
    fn rng_never_sticks_at_zero() {
        assert_ne!(Rng::new(0).next(), 0);
        let mut rng = Rng::new(SEED);
        let state = rng.0;
        // Stirring in the state itself would zero it.
        rng.stir(state);
        assert_ne!(rng.next(), 0);
    }

    #[test]
    fn soups_are_never_empty_and_about_nine_cells() {
        let mut rng = Rng::new(SEED);
        let soups: Vec<Grid> = (0..1000).map(|_| rng.soup()).collect();
        assert!(soups.iter().all(|g| !g.is_empty() && g.bits() & !ALL == 0));
        let total: u32 = soups.iter().map(|g| g.population()).sum();
        let mean = total / 1000;
        assert!((8..=10).contains(&mean), "mean population {mean}");
        // And they're not all the same one.
        assert!(soups.windows(2).all(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn a_generation_every_generation_ms() {
        let mut game = Game::starting(grid(GLIDER));
        idle(&mut game, TICKS_PER_GENERATION - 1);
        assert_eq!(game.grid(), grid(GLIDER));
        game.tick(false, false);
        assert_eq!(game.grid(), grid(GLIDER).step());
    }

    #[test]
    fn tick_shows_the_grid() {
        let mut game = Game::starting(grid(GLIDER));
        assert_eq!(game.tick(false, false), grid(GLIDER).image());
    }

    #[test]
    fn a_still_life_settles_holds_then_reseeds() {
        let block = grid("...../.##../.##../...../.....");
        let mut game = Game::starting(block);
        game.advance();
        assert!(game.settled());
        for _ in 0..HOLD_GENERATIONS {
            game.advance();
            assert_eq!(game.grid(), block);
        }
        game.advance();
        assert_ne!(game.grid(), block);
        assert!(!game.settled());
    }

    #[test]
    fn dying_out_settles() {
        let mut game = Game::starting(grid("...../..##./...../...../....."));
        game.advance();
        assert!(game.grid().is_empty());
        assert!(game.settled());
    }

    #[test]
    fn a_blinker_settles_after_two() {
        let mut game = Game::starting(grid("...../...../.###./...../....."));
        game.advance();
        assert!(!game.settled());
        game.advance();
        assert!(game.settled());
    }

    #[test]
    fn a_glider_runs_all_the_way_round_before_settling() {
        let mut game = Game::starting(grid(GLIDER));
        for _ in 0..19 {
            game.advance();
            assert!(!game.settled());
        }
        game.advance();
        assert!(game.settled());
        assert_eq!(game.grid(), grid(GLIDER));
    }

    #[test]
    fn a_reseed_forgets_the_old_history() {
        let block = grid("...../.##../.##../...../.....");
        let mut game = Game::starting(block);
        for _ in 0..=HOLD_GENERATIONS + 1 {
            game.advance();
        }
        assert_eq!(game.remembered, 0);
        assert!(!game.settled());
    }

    #[test]
    fn a_throws_down_a_new_soup() {
        let block = grid("...../.##../.##../...../.....");
        let mut game = Game::starting(block);
        press(&mut game, true, false);
        assert_ne!(game.grid(), block);
        assert!(!game.grid().is_empty());
    }

    #[test]
    fn presses_at_different_times_give_different_soups() {
        let mut early = Game::new();
        let mut late = Game::new();
        idle(&mut late, 7);
        press(&mut early, true, false);
        press(&mut late, true, false);
        assert_ne!(early.grid(), late.grid());
    }

    #[test]
    fn b_pauses_and_resumes() {
        let mut game = Game::starting(grid(GLIDER));
        press(&mut game, false, true);
        assert!(game.paused());
        let held = game.grid();
        idle(&mut game, 10 * TICKS_PER_GENERATION);
        assert_eq!(game.grid(), held);
        press(&mut game, false, true);
        assert!(!game.paused());
        idle(&mut game, TICKS_PER_GENERATION);
        assert_ne!(game.grid(), held);
    }

    #[test]
    fn a_while_paused_stays_paused() {
        let mut game = Game::starting(grid(GLIDER));
        press(&mut game, false, true);
        press(&mut game, true, false);
        assert!(game.paused());
        let soup = game.grid();
        idle(&mut game, 10 * TICKS_PER_GENERATION);
        assert_eq!(game.grid(), soup);
    }

    #[test]
    fn holding_a_is_one_soup() {
        let mut game = Game::starting(grid(GLIDER));
        for _ in 0..DEBOUNCE_TICKS {
            game.tick(true, false);
        }
        let soup = game.grid();
        // Fewer ticks than a generation, so only another press could change it.
        for _ in 0..TICKS_PER_GENERATION - u32::from(DEBOUNCE_TICKS) - 1 {
            game.tick(true, false);
        }
        assert_eq!(game.grid(), soup);
    }
}
