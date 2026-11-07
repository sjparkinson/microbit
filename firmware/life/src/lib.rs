//! Conway's Game of Life on the 5×5 matrix, with the edges wrapping round, so
//! the grid's a torus and a glider that walks off the right comes back on
//! the left.
//!
//! It steps a generation every [`GENERATION_MS`]. Once the grid dies out or
//! repeats one of its last [`HISTORY`] generations (a still life, a blinker,
//! a glider gone all the way round), it carries on for [`HOLD_GENERATIONS`]
//! more so you can see how it ended, then wipes across to a new start, a
//! column at a time. One that's died out wipes after a single blank
//! generation, since there's nothing to hold.
//!
//! Most random grids on a 5×5 torus die or freeze within a few generations,
//! which looks like noise, so a new start is one of two things:
//!
//! - a random soup that's been played out ahead and either lasts
//!   [`MIN_GENERATIONS`] or ends in an oscillator, or
//! - one in [`PATTERN_ONE_IN`] times, a hand-picked pattern from [`PATTERNS`],
//!   flipped, turned and moved somewhere at random.
//!
//! Button A wipes to a new start straight away. Button B pauses and resumes.
//!
//! There's no entropy source on `board` yet, so every power-on starts with
//! the same glider and the same sequence after it. Each button press stirs in
//! how many ticks it's been since power-on, which is as random as human
//! timing, so the sequence parts ways from the first press.
//!
//! Everything here is plain logic, tested on the host. `src/main.rs` reads the
//! buttons, calls [`Game::tick`] and shows what it returns, every
//! [`TICK_MS`].

#![no_std]

use board::Image;

/// How long one pass of the main loop takes, in milliseconds, which is one
/// show of the frame.
pub const TICK_MS: u32 = 10;

/// How long each generation stays up. Slow enough to see which cells made
/// which.
pub const GENERATION_MS: u32 = 650;

/// How long each column of the wipe between starts takes. Five of them is
/// 350 ms.
pub const WIPE_STEP_MS: u32 = 70;

/// How many past generations it remembers to spot a repeat. A glider on a
/// 5×5 torus takes 20 generations to get back where it started, the longest
/// cycle there is room for that's worth waiting out, so this covers it with
/// a few to spare.
pub const HISTORY: usize = 24;

/// How many generations it keeps going once it's settled, before a new
/// start: about three seconds at [`GENERATION_MS`].
pub const HOLD_GENERATIONS: u8 = 5;

/// How long a random soup has to keep changing, played out ahead, before
/// it's shown. One that settles sooner only gets in if it ends oscillating.
pub const MIN_GENERATIONS: u32 = 8;

/// How many random soups it tries before giving up and showing a pattern.
/// About half of them pass, so running out never really happens.
const SOUP_TRIES: u32 = 32;

/// How often a new start is a hand-picked pattern rather than a soup: one in
/// this many.
pub const PATTERN_ONE_IN: u32 = 4;

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

/// A glider, heading down and to the right. It moves one cell diagonally
/// every four generations, so it's home again in 20.
pub const GLIDER: Grid = Grid::from_rows([
    0b01000, //
    0b00100, //
    0b11100, //
    0b00000, //
    0b00000, //
]);

/// A blinker: three in a row, flipping between across and down.
pub const BLINKER: Grid = Grid::from_rows([
    0b00000, //
    0b00000, //
    0b01110, //
    0b00000, //
    0b00000, //
]);

/// A toad: two offset rows of three that rock back and forth.
pub const TOAD: Grid = Grid::from_rows([
    0b00000, //
    0b01110, //
    0b11100, //
    0b00000, //
    0b00000, //
]);

/// The hand-picked starts. Each keeps going on the 5×5 torus however it's
/// placed, which the tests check. A beacon would be the obvious next one,
/// but it's four wide, and on a torus five wide its blocks feel each other
/// round the back and it dies.
pub const PATTERNS: [Grid; 3] = [GLIDER, BLINKER, TOAD];

impl Grid {
    /// Every cell dead.
    pub const EMPTY: Self = Self(0);

    /// The grid from 25 bits, in the layout above. Anything past bit 24 is
    /// dropped.
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits & ALL)
    }

    /// The grid from five rows, top first, each with its leftmost cell in
    /// bit 4, so they read the right way round written in binary.
    pub const fn from_rows(rows: [u8; SIZE]) -> Self {
        let mut bits = 0;
        let mut y = 0;
        while y < SIZE {
            let mut x = 0;
            while x < SIZE {
                if rows[y] >> (SIZE - 1 - x) & 1 != 0 {
                    bits |= bit(x, y);
                }
                x += 1;
            }
            y += 1;
        }
        Self(bits)
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

    /// Each cell moved to wherever `to` says, from its column and row.
    fn mapped(self, to: impl Fn(usize, usize) -> (usize, usize)) -> Self {
        let mut bits = 0;
        for y in 0..SIZE {
            for x in 0..SIZE {
                if self.alive(x, y) {
                    let (nx, ny) = to(x, y);
                    bits |= bit(nx, ny);
                }
            }
        }
        Self(bits)
    }

    /// The grid moved `dx` right and `dy` down, wrapping.
    pub fn shifted(self, dx: usize, dy: usize) -> Self {
        self.mapped(|x, y| ((x + dx) % SIZE, (y + dy) % SIZE))
    }

    /// The grid mirrored left to right.
    pub fn flipped_across(self) -> Self {
        self.mapped(|x, y| (SIZE - 1 - x, y))
    }

    /// The grid mirrored top to bottom.
    pub fn flipped_down(self) -> Self {
        self.mapped(|x, y| (x, SIZE - 1 - y))
    }

    /// The grid mirrored along the diagonal from top left to bottom right.
    pub fn transposed(self) -> Self {
        self.mapped(|x, y| (y, x))
    }
}

/// The bit for column `x`, row `y`.
const fn bit(x: usize, y: usize) -> u32 {
    1 << (y * SIZE + x)
}

/// Every cell in the columns left of `column`.
const fn columns_before(column: usize) -> u32 {
    let mut bits = 0;
    let mut x = 0;
    while x < column {
        bits |= column_cells(x);
        x += 1;
    }
    bits
}

/// Every cell in `column`.
const fn column_cells(column: usize) -> u32 {
    let mut bits = 0;
    let mut y = 0;
    while y < SIZE {
        bits |= bit(column, y);
        y += 1;
    }
    bits
}

/// Generations of one start, and enough of what came before to spot it
/// repeating.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Run {
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
}

impl Run {
    const fn new(grid: Grid) -> Self {
        Self {
            grid,
            history: [Grid::EMPTY; HISTORY],
            remembered: 0,
            slot: 0,
            settled: None,
        }
    }

    /// One generation on, noting when it's settled and counting from there.
    fn step(&mut self) {
        let next = self.grid.step();
        self.history[self.slot] = self.grid;
        self.slot = (self.slot + 1) % HISTORY;
        self.remembered = (self.remembered + 1).min(HISTORY);
        self.grid = next;
        self.settled = match self.settled {
            Some(held) => Some(held.saturating_add(1)),
            None if next.is_empty() || self.seen(next) => Some(0),
            None => None,
        };
    }

    /// Whether `grid` is one of the remembered generations.
    fn seen(&self, grid: Grid) -> bool {
        self.history[..self.remembered].contains(&grid)
    }
}

/// Whether a soup's worth showing: played out ahead, it's still changing
/// after [`MIN_GENERATIONS`], or it's settled into something that moves.
fn worth_watching(grid: Grid) -> bool {
    let mut run = Run::new(grid);
    for _ in 0..MIN_GENERATIONS {
        run.step();
        if run.settled.is_some() {
            return !run.grid.is_empty() && run.grid.step() != run.grid;
        }
    }
    true
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
    /// about nine cells. Never empty, but often over in a few generations.
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

    /// A soup that's [`worth_watching`], or `None` if [`SOUP_TRIES`] in a
    /// row weren't.
    pub fn watchable_soup(&mut self) -> Option<Grid> {
        (0..SOUP_TRIES)
            .map(|_| self.soup())
            .find(|&s| worth_watching(s))
    }

    /// One of [`PATTERNS`], in any of its eight orientations, anywhere.
    pub fn pattern(&mut self) -> Grid {
        let r = self.next();
        // The pattern count is tiny, so this always fits.
        let mut grid = PATTERNS[(r % PATTERNS.len() as u32) as usize];
        if r & 1 << 4 != 0 {
            grid = grid.flipped_across();
        }
        if r & 1 << 5 != 0 {
            grid = grid.flipped_down();
        }
        if r & 1 << 6 != 0 {
            grid = grid.transposed();
        }
        let dx = (r >> 8) as usize % SIZE;
        let dy = (r >> 16) as usize % SIZE;
        grid.shifted(dx, dy)
    }

    /// A new start: a pattern one in [`PATTERN_ONE_IN`] times, a watchable
    /// soup otherwise.
    pub fn start(&mut self) -> Grid {
        if self.next().is_multiple_of(PATTERN_ONE_IN) {
            return self.pattern();
        }
        match self.watchable_soup() {
            Some(soup) => soup,
            None => self.pattern(),
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

/// The wipe from the last start to a new one: a lit bar sweeps left to
/// right, with the new grid behind it and the old one ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Wipe {
    from: Grid,
    /// Where the bar is, 0 to 4.
    column: usize,
}

/// The game: the current run, the wipe if there's one going, and the
/// buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    run: Run,
    wipe: Option<Wipe>,
    paused: bool,
    /// Milliseconds since the last generation or wipe step.
    since_ms: u32,
    /// Ticks since power-on, for stirring into `rng`.
    ticks: u32,
    rng: Rng,
    a: Debounce,
    b: Debounce,
}

impl Game {
    /// A game at power-on, which always starts with a glider.
    pub const fn new() -> Self {
        Self::starting(GLIDER)
    }

    /// A game starting from `grid`, with the power-on random numbers for
    /// the starts after it.
    pub const fn starting(grid: Grid) -> Self {
        Self {
            run: Run::new(grid),
            wipe: None,
            paused: false,
            since_ms: 0,
            ticks: 0,
            rng: Rng::new(SEED),
            a: Debounce::new(),
            b: Debounce::new(),
        }
    }

    /// The grid as it is now. During a wipe, that's the new one.
    pub const fn grid(&self) -> Grid {
        self.run.grid
    }

    /// Whether it's died out or started repeating.
    pub const fn settled(&self) -> bool {
        self.run.settled.is_some()
    }

    /// Whether it's wiping across to a new start.
    pub const fn wiping(&self) -> bool {
        self.wipe.is_some()
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
            self.restart();
        }
        if self.b.update(b_raw) {
            self.rng.stir(self.ticks);
            self.paused = !self.paused;
        }
        if !self.paused {
            self.since_ms += TICK_MS;
            if let Some(wipe) = &mut self.wipe {
                if self.since_ms >= WIPE_STEP_MS {
                    self.since_ms = 0;
                    wipe.column += 1;
                    if wipe.column >= SIZE {
                        self.wipe = None;
                    }
                }
            } else if self.since_ms >= GENERATION_MS {
                self.since_ms = 0;
                self.advance();
            }
        }
        self.frame()
    }

    /// What's on the matrix: the grid, or partway through a wipe.
    pub fn frame(&self) -> Image {
        let Some(wipe) = self.wipe else {
            return self.run.grid.image();
        };
        let behind = columns_before(wipe.column);
        let bar = column_cells(wipe.column);
        let ahead = ALL & !behind & !bar;
        Grid(self.run.grid.0 & behind | bar | wipe.from.0 & ahead).image()
    }

    /// One generation on, or a wipe to a new start once it's been settled
    /// for [`HOLD_GENERATIONS`] (or straight after one blank generation, if
    /// it died out).
    pub fn advance(&mut self) {
        if let Some(held) = self.run.settled {
            let hold = if self.run.grid.is_empty() {
                0
            } else {
                HOLD_GENERATIONS
            };
            if held >= hold {
                self.restart();
                return;
            }
        }
        self.run.step();
    }

    /// Starts over on a new start, forgetting the old history, and wipes
    /// across to it.
    fn restart(&mut self) {
        let from = self.run.grid;
        self.run = Run::new(self.rng.start());
        self.wipe = Some(Wipe { from, column: 0 });
        self.since_ms = 0;
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
    use std::collections::BTreeSet;
    use std::string::String;
    use std::vec::Vec;

    use super::*;

    const TICKS_PER_GENERATION: u32 = GENERATION_MS / TICK_MS;
    const TICKS_PER_WIPE_STEP: u32 = WIPE_STEP_MS / TICK_MS;

    const BLOCK: &str = "...../.##../.##../...../.....";

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

    /// `grid` after `n` generations.
    fn after(grid: Grid, n: usize) -> Grid {
        (0..n).fold(grid, |g, _| g.step())
    }

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

    /// Advances a still life until it wipes to a new start.
    fn settle_and_restart(game: &mut Game) {
        while !game.wiping() {
            game.advance();
        }
    }

    #[test]
    fn text_and_images_round_trip() {
        let glider = grid(".#.../..#../###../...../.....");
        assert_eq!(glider, GLIDER);
        assert_eq!(draw(GLIDER), ".#.../..#../###../...../.....");
        assert_eq!(Grid::from_image(&GLIDER.image()), GLIDER);
        assert_eq!(GLIDER.population(), 5);
    }

    #[test]
    fn patterns_read_the_way_they_are_written() {
        assert_eq!(draw(BLINKER), "...../...../.###./...../.....");
        assert_eq!(draw(TOAD), "...../.###./###../...../.....");
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
        assert_eq!(grid(BLOCK).step(), grid(BLOCK));
    }

    #[test]
    fn a_blinker_flips_every_generation() {
        let down = grid("...../..#../..#../..#../.....");
        assert_eq!(BLINKER.step(), down);
        assert_eq!(down.step(), BLINKER);
    }

    #[test]
    fn a_blinker_on_the_edge_wraps() {
        let across = grid("...../...../##..#/...../.....");
        let down = grid("...../#..../#..../#..../.....");
        assert_eq!(across.step(), down);
    }

    #[test]
    fn a_glider_moves_one_down_and_right_every_four() {
        assert_eq!(after(GLIDER, 4), GLIDER.shifted(1, 1));
    }

    #[test]
    fn a_glider_gets_home_in_twenty() {
        for generation in 1..=20 {
            let g = after(GLIDER, generation);
            assert_eq!(g.population(), 5);
            assert_eq!(g == GLIDER, generation == 20, "generation {generation}");
        }
    }

    #[test]
    fn shifts_and_flips_move_cells_where_they_should() {
        let corner = grid("#..../...../...../...../.....");
        assert_eq!(draw(corner.shifted(4, 1)), "...../....#/...../...../.....");
        assert_eq!(
            draw(corner.flipped_across()),
            "....#/...../...../...../....."
        );
        assert_eq!(draw(corner.flipped_down()), "...../...../...../...../#....");
        let top = grid(".#.../...../...../...../.....");
        assert_eq!(draw(top.transposed()), "...../#..../...../...../.....");
        assert_eq!(GLIDER.transposed().transposed(), GLIDER);
    }

    /// Every orientation and position of `pattern`.
    fn placements(pattern: Grid) -> Vec<Grid> {
        let mut all = Vec::new();
        for turn in 0..8 {
            let mut g = pattern;
            if turn & 1 != 0 {
                g = g.flipped_across();
            }
            if turn & 2 != 0 {
                g = g.flipped_down();
            }
            if turn & 4 != 0 {
                g = g.transposed();
            }
            for dy in 0..5 {
                for dx in 0..5 {
                    all.push(g.shifted(dx, dy));
                }
            }
        }
        all
    }

    #[test]
    fn every_pattern_keeps_moving_however_its_placed() {
        for pattern in PATTERNS {
            for placed in placements(pattern) {
                for n in 0..40 {
                    let g = after(placed, n);
                    assert!(!g.is_empty(), "{} died", draw(placed));
                    assert_ne!(g.step(), g, "{} froze", draw(placed));
                }
            }
        }
    }

    #[test]
    fn oscillators_repeat_every_two() {
        for pattern in [BLINKER, TOAD] {
            assert_eq!(after(pattern, 2), pattern, "{}", draw(pattern));
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
        assert!(soups.windows(2).all(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn plenty_of_raw_soups_are_not_worth_watching() {
        let mut rng = Rng::new(SEED);
        let dull = (0..1000).filter(|_| !worth_watching(rng.soup())).count();
        assert!((200..800).contains(&dull), "{dull} dull");
    }

    #[test]
    fn watchable_soups_last_or_end_moving() {
        let mut rng = Rng::new(SEED);
        for _ in 0..1000 {
            let soup = rng.watchable_soup().expect("32 dull soups in a row");
            let mut run = Run::new(soup);
            for _ in 0..MIN_GENERATIONS {
                run.step();
                if run.settled.is_some() {
                    assert!(!run.grid.is_empty(), "{} died", draw(soup));
                    assert_ne!(run.grid.step(), run.grid, "{} froze", draw(soup));
                    break;
                }
            }
        }
    }

    #[test]
    fn placed_gliders_head_all_four_ways() {
        let mut rng = Rng::new(SEED);
        let mut headings = BTreeSet::new();
        for _ in 0..2000 {
            let g = rng.pattern();
            if g.population() != 5 {
                continue;
            }
            let later = after(g, 4);
            for (dx, dy) in [(1, 1), (4, 1), (1, 4), (4, 4)] {
                if g.shifted(dx, dy) == later {
                    headings.insert((dx, dy));
                }
            }
        }
        assert_eq!(headings.len(), 4, "{headings:?}");
    }

    #[test]
    fn starts_mix_patterns_and_soups() {
        let mut rng = Rng::new(SEED);
        let mut patterns = 0;
        for _ in 0..1000 {
            let start = rng.start();
            if PATTERNS.iter().any(|&p| placements(p).contains(&start)) {
                patterns += 1;
            }
        }
        // One in four, give or take, plus the odd soup that happens to be a
        // blinker.
        assert!((180..330).contains(&patterns), "{patterns} patterns");
    }

    #[test]
    fn power_on_is_a_glider() {
        assert_eq!(Game::new().grid(), GLIDER);
        assert!(!Game::new().wiping());
    }

    #[test]
    fn a_generation_every_generation_ms() {
        let mut game = Game::new();
        idle(&mut game, TICKS_PER_GENERATION - 1);
        assert_eq!(game.grid(), GLIDER);
        game.tick(false, false);
        assert_eq!(game.grid(), GLIDER.step());
    }

    #[test]
    fn tick_shows_the_grid() {
        let mut game = Game::new();
        assert_eq!(game.tick(false, false), GLIDER.image());
    }

    #[test]
    fn a_still_life_settles_holds_then_wipes() {
        let block = grid(BLOCK);
        let mut game = Game::starting(block);
        game.advance();
        assert!(game.settled());
        for _ in 0..HOLD_GENERATIONS {
            game.advance();
            assert_eq!(game.grid(), block);
            assert!(!game.wiping());
        }
        game.advance();
        assert!(game.wiping());
        assert_ne!(game.grid(), block);
        assert!(!game.settled());
        assert_eq!(game.run.remembered, 0);
    }

    #[test]
    fn dying_out_wipes_after_one_blank_generation() {
        let mut game = Game::starting(grid("...../..##./...../...../....."));
        game.advance();
        assert!(game.grid().is_empty());
        assert!(game.settled());
        assert!(!game.wiping());
        game.advance();
        assert!(game.wiping());
    }

    #[test]
    fn a_blinker_settles_after_two() {
        let mut game = Game::starting(BLINKER);
        game.advance();
        assert!(!game.settled());
        game.advance();
        assert!(game.settled());
    }

    #[test]
    fn a_glider_runs_all_the_way_round_before_settling() {
        let mut game = Game::new();
        for _ in 0..19 {
            game.advance();
            assert!(!game.settled());
        }
        game.advance();
        assert!(game.settled());
        assert_eq!(game.grid(), GLIDER);
    }

    #[test]
    fn the_wipe_sweeps_a_bar_from_old_to_new() {
        let block = grid(BLOCK);
        let mut game = Game::starting(block);
        settle_and_restart(&mut game);
        let new = game.grid();
        assert_eq!(
            draw(Grid::from_image(&game.frame())),
            "#..../###../###../#..../#...."
        );
        for column in 1..5 {
            idle(&mut game, TICKS_PER_WIPE_STEP);
            let frame = Grid::from_image(&game.frame());
            for y in 0..5 {
                for x in 0..5 {
                    let want = match x.cmp(&column) {
                        core::cmp::Ordering::Less => new.alive(x, y),
                        core::cmp::Ordering::Equal => true,
                        core::cmp::Ordering::Greater => block.alive(x, y),
                    };
                    assert_eq!(frame.alive(x, y), want, "column {column}, {x},{y}");
                }
            }
        }
        idle(&mut game, TICKS_PER_WIPE_STEP);
        assert!(!game.wiping());
        assert_eq!(game.frame(), new.image());
    }

    #[test]
    fn the_new_start_gets_a_whole_generation_after_the_wipe() {
        let mut game = Game::starting(grid(BLOCK));
        settle_and_restart(&mut game);
        let new = game.grid();
        idle(&mut game, 5 * TICKS_PER_WIPE_STEP);
        assert!(!game.wiping());
        idle(&mut game, TICKS_PER_GENERATION - 1);
        assert_eq!(game.grid(), new);
        game.tick(false, false);
        assert_eq!(game.grid(), new.step());
    }

    #[test]
    fn a_wipes_to_a_new_start() {
        let block = grid(BLOCK);
        let mut game = Game::starting(block);
        press(&mut game, true, false);
        assert!(game.wiping());
        assert_ne!(game.grid(), block);
        assert!(!game.grid().is_empty());
    }

    #[test]
    fn presses_at_different_times_give_different_starts() {
        let mut early = Game::new();
        let mut late = Game::new();
        idle(&mut late, 7);
        press(&mut early, true, false);
        press(&mut late, true, false);
        assert_ne!(early.grid(), late.grid());
    }

    #[test]
    fn b_pauses_and_resumes() {
        let mut game = Game::new();
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
    fn pausing_holds_the_wipe_too() {
        let mut game = Game::starting(grid(BLOCK));
        press(&mut game, true, false);
        press(&mut game, false, true);
        let frame = game.frame();
        idle(&mut game, 10 * TICKS_PER_WIPE_STEP);
        assert!(game.wiping());
        assert_eq!(game.frame(), frame);
    }

    #[test]
    fn holding_a_is_one_restart() {
        let mut game = Game::new();
        for _ in 0..DEBOUNCE_TICKS {
            game.tick(true, false);
        }
        let start = game.grid();
        for _ in 0..TICKS_PER_GENERATION {
            game.tick(true, false);
        }
        assert_eq!(game.grid(), start);
    }
}
