//! A tilt maze: roll a marble from the top left of a maze to the bottom
//! right by tipping the board. The maze is 15×15 LEDs' worth, so the 5×5
//! display is a window that follows the marble around.
//!
//! Walls are lit, the marble blinks slowly, and the goal flickers. Reaching
//! the goal sends rings out from the middle, then a new maze starts. A puts
//! the marble back at the start; B swaps in a new maze.

#![no_std]

use board::{Image, Tilt};

/// How long each tick shows for, in milliseconds. Each tick reads the
/// accelerometer once and moves the marble once.
pub const TICK_MS: u32 = 20;

/// The maze is this many cells a side. Each cell is a floor pixel with a
/// wall pixel or a gap to its right and below, plus the border.
pub const CELLS: usize = 7;

/// The maze's size in pixels, walls and all.
pub const SIZE: usize = 2 * CELLS + 1;

/// The display's size in pixels.
const VIEW: usize = 5;

/// One pixel in the marble's fixed-point position and speed.
const ONE: i32 = 256;

/// Tilts under this many milli-g don't push the marble, so it sits still on
/// a board that's nearly flat.
const DEAD_ZONE_MG: i32 = 60;

/// Each milli-g past the dead zone adds 1/`PUSH_DIV` of a fixed-point step
/// to the speed each tick.
const PUSH_DIV: i32 = 160;

/// The marble's top speed in fixed point, a tick. It has to stay under
/// [`ONE`] so the marble can't jump a wall between ticks; this is about
/// 30 pixels a second.
const MAX_SPEED: i32 = 160;

/// How many ticks the marble's blink takes: 400 ms, lit for three quarters.
const BLINK_TICKS: u32 = 20;

/// How many ticks the rings play for after reaching the goal: 1.2 s.
const WIN_TICKS: u32 = 60;

/// How many ticks each ring of the win shows for.
const RING_TICKS: u32 = 5;

/// How many ticks a button has to read the same before it counts.
const DEBOUNCE_TICKS: u8 = 2;

/// Where xorshift starts when given zero, the same as life's.
const SEED: u32 = 0x2545_f491;

/// The marble's starting pixel, the top-left cell.
const START: (usize, usize) = (1, 1);

/// The goal's pixel, the bottom-right cell.
pub const GOAL: (usize, usize) = (SIZE - 2, SIZE - 2);

/// What shows when there's no accelerometer to read: a cross.
pub const NO_SENSOR: Image = [
    [1, 0, 0, 0, 1],
    [0, 1, 0, 1, 0],
    [0, 0, 1, 0, 0],
    [0, 1, 0, 1, 0],
    [1, 0, 0, 0, 1],
];

/// The maze, as a bit per pixel: set is wall. Row `y`'s bit `x` is the pixel
/// `x` along and `y` down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Maze {
    walls: [u16; SIZE],
}

impl Maze {
    /// A new maze from `rng`, carved by a depth-first backtracker: every cell
    /// joins up, and there's exactly one way between any two.
    pub fn generate(rng: &mut Rng) -> Self {
        let mut maze = Self {
            walls: [(1 << SIZE) - 1; SIZE],
        };
        // Cells are numbered across then down. Every cell goes on the stack
        // once at most, so it never overflows.
        let mut stack = [0u8; CELLS * CELLS];
        let mut depth = 1;
        let mut visited: u64 = 1;
        maze.open(1, 1);
        while depth > 0 {
            let cell = stack[depth - 1] as usize;
            let (cx, cy) = (cell % CELLS, cell / CELLS);
            let mut next = [(0, 0); 4];
            let mut count = 0;
            for (dx, dy) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                let (nx, ny) = (cx as isize + dx, cy as isize + dy);
                if !(0..CELLS as isize).contains(&nx) || !(0..CELLS as isize).contains(&ny) {
                    continue;
                }
                let n = ny as usize * CELLS + nx as usize;
                if visited & 1 << n == 0 {
                    next[count] = (nx as usize, ny as usize);
                    count += 1;
                }
            }
            if count == 0 {
                depth -= 1;
                continue;
            }
            // `count` is at most four, so this always fits.
            let (nx, ny) = next[(rng.next() % count as u32) as usize];
            // The wall between two cells sits halfway between their pixels.
            maze.open(cx + nx + 1, cy + ny + 1);
            maze.open(2 * nx + 1, 2 * ny + 1);
            let n = ny * CELLS + nx;
            visited |= 1 << n;
            stack[depth] = n as u8;
            depth += 1;
        }
        maze
    }

    /// Whether the pixel at `x` along and `y` down is wall. Everything
    /// outside the maze is.
    pub fn is_wall(&self, x: usize, y: usize) -> bool {
        x >= SIZE || y >= SIZE || self.walls[y] & 1 << x != 0
    }

    fn open(&mut self, x: usize, y: usize) {
        self.walls[y] &= !(1 << x);
    }
}

/// The marble, in fixed point: [`ONE`] to a pixel, with pixel `n` running
/// from `n * ONE` to just under `(n + 1) * ONE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marble {
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
}

impl Marble {
    /// A marble at rest in the middle of pixel `(x, y)`.
    pub const fn at((x, y): (usize, usize)) -> Self {
        Self {
            x: x as i32 * ONE + ONE / 2,
            y: y as i32 * ONE + ONE / 2,
            vx: 0,
            vy: 0,
        }
    }

    /// The pixel it's in.
    pub const fn pixel(&self) -> (usize, usize) {
        // The marble never leaves the maze, so these are never negative.
        ((self.x / ONE) as usize, (self.y / ONE) as usize)
    }

    /// Rolls one tick's worth under `tilt`, stopping at walls.
    pub fn step(&mut self, maze: &Maze, tilt: Tilt) {
        self.vx = speed(self.vx, tilt.x);
        self.vy = speed(self.vy, tilt.y);

        let (_, py) = self.pixel();
        if let Some((x, vx)) = roll(self.x, self.vx, |px| maze.is_wall(px, py)) {
            (self.x, self.vx) = (x, vx);
        } else {
            self.x += self.vx;
        }
        let (px, _) = self.pixel();
        if let Some((y, vy)) = roll(self.y, self.vy, |py| maze.is_wall(px, py)) {
            (self.y, self.vy) = (y, vy);
        } else {
            self.y += self.vy;
        }
    }
}

/// The speed along one axis after a tick of `mg` tilt: pushed by the tilt
/// past the dead zone, slowed by friction, and capped.
fn speed(v: i32, mg: i16) -> i32 {
    let mg = i32::from(mg);
    let push = if mg.abs() < DEAD_ZONE_MG {
        0
    } else {
        (mg - DEAD_ZONE_MG * mg.signum()) / PUSH_DIV
    };
    // A sixteenth off, and one more so it comes to a stop rather than
    // creeping forever.
    let v = v + push;
    (v - v / 16 - v.signum()).clamp(-MAX_SPEED, MAX_SPEED)
}

/// Where a move of `v` from `at` along one axis hits a wall, if it does: the
/// position against the wall, and the speed bounced back at half. `wall`
/// says whether the pixel at an index along this axis is wall.
fn roll(at: i32, v: i32, wall: impl Fn(usize) -> bool) -> Option<(i32, i32)> {
    let to = at + v;
    // Speeds stay under a pixel a tick, so a move only reaches the next
    // pixel along.
    let pixel = (to / ONE) as usize;
    if pixel == (at / ONE) as usize || !wall(pixel) {
        return None;
    }
    let against = if v > 0 {
        pixel as i32 * ONE - 1
    } else {
        (pixel as i32 + 1) * ONE
    };
    Some((against, -v / 2))
}

/// A xorshift32 generator, the same as life's.
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
}

/// The jitter in a reading as 32 bits, to stir into an [`Rng`]. An
/// accelerometer's last few bits wobble even on a still board.
pub fn noise(tilt: Tilt) -> u32 {
    let bits = |mg: i16| u32::from(mg as u16);
    bits(tilt.x) | bits(tilt.y) << 11 | bits(tilt.z) << 22
}

/// Turns raw button readings, one a tick, into presses. The same as life's.
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

/// The whole game: a maze, the marble in it, and the buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    rng: Rng,
    maze: Maze,
    marble: Marble,
    ticks: u32,
    /// Ticks left of the win's rings, or `None` while playing.
    won: Option<u32>,
    a: Debounce,
    b: Debounce,
}

impl Game {
    /// A game with a maze from `seed`.
    pub fn new(seed: u32) -> Self {
        let mut rng = Rng::new(seed);
        Self {
            maze: Maze::generate(&mut rng),
            rng,
            marble: Marble::at(START),
            ticks: 0,
            won: None,
            a: Debounce::new(),
            b: Debounce::new(),
        }
    }

    /// One tick: feed in this tick's tilt and raw button readings, get back
    /// the frame to show for [`TICK_MS`].
    pub fn tick(&mut self, tilt: Tilt, a: bool, b: bool) -> Image {
        self.ticks = self.ticks.wrapping_add(1);
        self.rng.stir(noise(tilt));
        let restart = self.a.update(a);
        let new_maze = self.b.update(b);

        if let Some(left) = self.won {
            if left == 0 {
                self.next_maze();
            } else {
                self.won = Some(left - 1);
                return rings(WIN_TICKS - left);
            }
        }
        if new_maze {
            self.next_maze();
        } else if restart {
            self.marble = Marble::at(START);
        }

        self.marble.step(&self.maze, tilt);
        if self.marble.pixel() == GOAL {
            self.won = Some(WIN_TICKS);
        }
        self.frame()
    }

    fn next_maze(&mut self) {
        self.maze = Maze::generate(&mut self.rng);
        self.marble = Marble::at(START);
        self.won = None;
    }

    /// The window around the marble: walls lit, the marble blinking slowly,
    /// the goal flickering.
    fn frame(&self) -> Image {
        let (mx, my) = self.marble.pixel();
        let ox = window(mx);
        let oy = window(my);
        let mut image = [[0; VIEW]; VIEW];
        for (row, line) in image.iter_mut().enumerate() {
            for (col, led) in line.iter_mut().enumerate() {
                *led = u8::from(self.maze.is_wall(ox + col, oy + row));
            }
        }
        let (gx, gy) = GOAL;
        if (ox..ox + VIEW).contains(&gx) && (oy..oy + VIEW).contains(&gy) {
            image[gy - oy][gx - ox] = u8::from((self.ticks / 2).is_multiple_of(2));
        }
        image[my - oy][mx - ox] = u8::from(self.ticks % BLINK_TICKS < BLINK_TICKS * 3 / 4);
        image
    }

    /// The maze, for tests.
    #[cfg(test)]
    fn maze(&self) -> &Maze {
        &self.maze
    }
}

/// Where the window starts along one axis to keep `at` in the middle,
/// without running off the maze.
fn window(at: usize) -> usize {
    at.saturating_sub(VIEW / 2).min(SIZE - VIEW)
}

/// The win's frame `elapsed` ticks in: a square ring around the middle LED,
/// growing outwards and starting over.
fn rings(elapsed: u32) -> Image {
    let ring = (elapsed / RING_TICKS % 3) as usize;
    let mut image = [[0; VIEW]; VIEW];
    for (row, line) in image.iter_mut().enumerate() {
        for (col, led) in line.iter_mut().enumerate() {
            *led = u8::from(row.abs_diff(2).max(col.abs_diff(2)) == ring);
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLAT: Tilt = Tilt {
        x: 0,
        y: 0,
        z: -1000,
    };

    fn tilt(x: i16, y: i16) -> Tilt {
        Tilt { x, y, z: -1000 }
    }

    /// A maze with no inside walls, only the border.
    fn room() -> Maze {
        let mut walls = [(1 << SIZE) - 1; SIZE];
        for row in &mut walls[1..SIZE - 1] {
            *row = 1 | 1 << (SIZE - 1);
        }
        Maze { walls }
    }

    fn floor(maze: &Maze) -> usize {
        (0..SIZE)
            .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
            .filter(|&(x, y)| !maze.is_wall(x, y))
            .count()
    }

    /// How many floor pixels a flood from the start reaches.
    fn reachable(maze: &Maze) -> usize {
        let mut seen = [[false; SIZE]; SIZE];
        let mut todo = [(0, 0); SIZE * SIZE];
        todo[0] = START;
        seen[START.1][START.0] = true;
        let (mut head, mut tail) = (0, 1);
        while head < tail {
            let (x, y) = todo[head];
            head += 1;
            for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                if !maze.is_wall(nx, ny) && !seen[ny][nx] {
                    seen[ny][nx] = true;
                    todo[tail] = (nx, ny);
                    tail += 1;
                }
            }
        }
        tail
    }

    #[test]
    fn a_maze_joins_every_cell_with_no_loops() {
        for seed in 1..50 {
            let maze = Maze::generate(&mut Rng::new(seed));
            // A cell per floor pixel, and one gap fewer than cells between
            // them: a tree, so one path between any two.
            assert_eq!(floor(&maze), CELLS * CELLS + CELLS * CELLS - 1);
            assert_eq!(reachable(&maze), floor(&maze));
        }
    }

    #[test]
    fn a_maze_keeps_its_border() {
        let maze = Maze::generate(&mut Rng::new(7));
        for i in 0..SIZE {
            assert!(maze.is_wall(i, 0) && maze.is_wall(i, SIZE - 1));
            assert!(maze.is_wall(0, i) && maze.is_wall(SIZE - 1, i));
        }
        assert!(maze.is_wall(SIZE, 1) && maze.is_wall(1, SIZE));
    }

    #[test]
    fn different_seeds_make_different_mazes() {
        let a = Maze::generate(&mut Rng::new(1));
        let b = Maze::generate(&mut Rng::new(2));
        assert_ne!(a, b);
    }

    #[test]
    fn the_marble_sits_still_on_a_flat_board() {
        let mut marble = Marble::at(START);
        for _ in 0..100 {
            marble.step(&room(), tilt(50, -50));
        }
        assert_eq!(marble, Marble::at(START));
    }

    #[test]
    fn the_marble_rolls_downhill_and_stops_at_the_wall() {
        let room = room();
        let mut marble = Marble::at(START);
        for _ in 0..200 {
            marble.step(&room, tilt(700, 0));
            let (x, y) = marble.pixel();
            assert!(!room.is_wall(x, y));
        }
        assert_eq!(marble.pixel(), (SIZE - 2, 1));
    }

    #[test]
    fn the_marble_rolls_the_other_way_too() {
        let room = room();
        let mut marble = Marble::at((SIZE - 2, SIZE - 2));
        for _ in 0..200 {
            marble.step(&room, tilt(-700, -700));
        }
        assert_eq!(marble.pixel(), START);
    }

    #[test]
    fn the_marble_never_goes_through_a_wall() {
        let maze = Maze::generate(&mut Rng::new(3));
        let mut marble = Marble::at(START);
        let mut rng = Rng::new(9);
        for _ in 0..5000 {
            let r = rng.next();
            let x = (r & 0x7ff) as i16 - 1024;
            let y = (r >> 11 & 0x7ff) as i16 - 1024;
            marble.step(&maze, tilt(x, y));
            let (px, py) = marble.pixel();
            assert!(!maze.is_wall(px, py), "in a wall at {px}, {py}");
        }
    }

    #[test]
    fn friction_brings_the_marble_to_rest() {
        let room = room();
        let mut marble = Marble::at((7, 7));
        for _ in 0..10 {
            marble.step(&room, tilt(500, 0));
        }
        for _ in 0..100 {
            marble.step(&room, FLAT);
        }
        let still = marble;
        marble.step(&room, FLAT);
        assert_eq!(marble, still);
        assert_eq!((marble.vx, marble.vy), (0, 0));
    }

    #[test]
    fn its_speed_stays_under_a_pixel_a_tick() {
        let mut v = 0;
        for _ in 0..100 {
            v = speed(v, i16::MAX);
        }
        assert_eq!(v, MAX_SPEED);
        const { assert!(MAX_SPEED < ONE) };
    }

    #[test]
    fn the_window_follows_the_marble_without_leaving_the_maze() {
        assert_eq!(window(1), 0);
        assert_eq!(window(2), 0);
        assert_eq!(window(7), 5);
        assert_eq!(window(SIZE - 2), SIZE - VIEW);
    }

    #[test]
    fn the_first_frame_shows_the_top_left_corner_and_the_marble() {
        let mut game = Game::new(5);
        let frame = game.tick(FLAT, false, false);
        assert_eq!(frame[0], [1; 5]);
        assert_eq!(frame[1][0], 1);
        // Lit, as the blink starts on.
        assert_eq!(frame[1][1], 1);
        let maze = *game.maze();
        for (row, line) in frame.iter().enumerate() {
            for (col, &led) in line.iter().enumerate() {
                if (col, row) != START {
                    assert_eq!(led, u8::from(maze.is_wall(col, row)));
                }
            }
        }
    }

    #[test]
    fn the_marble_blinks() {
        let mut game = Game::new(5);
        let lit: u32 = (0..BLINK_TICKS)
            .map(|_| u32::from(game.tick(FLAT, false, false)[1][1]))
            .sum();
        assert_eq!(lit, BLINK_TICKS * 3 / 4);
    }

    #[test]
    fn reaching_the_goal_plays_rings_then_starts_a_new_maze() {
        let mut game = Game::new(5);
        let first = *game.maze();
        game.maze = room();
        game.marble = Marble::at((GOAL.0 - 1, GOAL.1));
        for _ in 0..20 {
            game.tick(tilt(1000, 0), false, false);
            if game.won.is_some() {
                break;
            }
        }
        assert!(game.won.is_some());
        assert_eq!(game.tick(FLAT, false, false), rings(0));
        for _ in 0..WIN_TICKS {
            game.tick(FLAT, false, false);
        }
        assert_eq!(game.won, None);
        assert_eq!(game.marble.pixel(), START);
        assert_ne!(*game.maze(), room());
        assert_ne!(*game.maze(), first);
    }

    #[test]
    fn a_puts_the_marble_back_and_b_makes_a_new_maze() {
        let mut game = Game::new(5);
        game.maze = room();
        for _ in 0..30 {
            game.tick(tilt(800, 800), false, false);
        }
        assert_ne!(game.marble.pixel(), START);
        game.tick(FLAT, true, false);
        game.tick(FLAT, true, false);
        assert_eq!(game.marble.pixel(), START);
        assert_eq!(*game.maze(), room());

        game.tick(FLAT, false, true);
        game.tick(FLAT, false, true);
        assert_ne!(*game.maze(), room());
    }

    #[test]
    fn rings_grow_from_the_middle() {
        assert_eq!(rings(0)[2], [0, 0, 1, 0, 0]);
        assert_eq!(rings(RING_TICKS)[1], [0, 1, 1, 1, 0]);
        assert_eq!(rings(2 * RING_TICKS)[0], [1; 5]);
        assert_eq!(rings(3 * RING_TICKS), rings(0));
    }

    #[test]
    fn noise_uses_each_axis() {
        assert_ne!(noise(tilt(1, 0)), noise(tilt(0, 1)));
        assert_ne!(noise(tilt(0, 0)), noise(Tilt { x: 0, y: 0, z: 1 }));
    }
}
