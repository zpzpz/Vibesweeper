//! Game engine — a faithful port of the logic in `winmine.exe` (Windows Minesweeper 5.1).
//!
//! The original stores the board as a byte array (stride 0x20) with the following cell
//! encoding, which is reproduced here so behaviour is identical:
//!
//! * bit `0x80` — the cell contains a mine
//! * bit `0x40` — the cell has been revealed
//! * low 5 bits — value/state (see [`cell`])
//!
//! A one-cell border of `0x10` surrounds the playfield. The playfield is addressed with
//! 1-based coordinates: `x` in `1..=width`, `y` in `1..=height`.

/// Maximum playfield width accepted by the (custom) dialog.
pub const MAX_WIDTH: usize = 30;
/// Maximum playfield height accepted by the (custom) dialog.
pub const MAX_HEIGHT: usize = 24;

/// Cell byte constants, mirroring the original encoding.
pub mod cell {
    /// Cell contains a mine.
    pub const MINE: u8 = 0x80;
    /// Cell has been revealed.
    pub const REVEALED: u8 = 0x40;
    /// Border / out-of-play sentinel value.
    pub const BORDER: u8 = 0x10;

    /// Revealed, zero adjacent mines.
    pub const V_EMPTY: u8 = 0x00;
    /// Pressed (highlighted) question-marked cell — transient UI state only.
    pub const V_PRESSED_Q: u8 = 0x09;
    /// Mine revealed on loss.
    pub const V_MINE: u8 = 0x0a;
    /// Flag planted on a cell that is *not* a mine (revealed on loss).
    pub const V_WRONG: u8 = 0x0b;
    /// The mine that was clicked / an incorrectly chorded mine (revealed on loss).
    pub const V_EXPLODE: u8 = 0x0c;
    /// Question mark.
    pub const V_QUESTION: u8 = 0x0d;
    /// Flag.
    pub const V_FLAG: u8 = 0x0e;
    /// Covered / unmarked.
    pub const V_COVERED: u8 = 0x0f;
}

/// A difficulty preset.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Preset {
    pub width: usize,
    pub height: usize,
    pub mines: usize,
}

/// The three standard difficulty presets.
pub const BEGINNER: Preset = Preset { width: 9, height: 9, mines: 10 };
pub const INTERMEDIATE: Preset = Preset { width: 16, height: 16, mines: 40 };
pub const EXPERT: Preset = Preset { width: 30, height: 16, mines: 99 };

/// Which preset (or a custom board) is currently selected.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Difficulty {
    Beginner,
    Intermediate,
    Expert,
    Custom,
}

impl Difficulty {
    pub fn preset(self, custom: Preset) -> Preset {
        match self {
            Difficulty::Beginner => BEGINNER,
            Difficulty::Intermediate => INTERMEDIATE,
            Difficulty::Expert => EXPERT,
            Difficulty::Custom => custom,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Difficulty::Beginner => 0,
            Difficulty::Intermediate => 1,
            Difficulty::Expert => 2,
            Difficulty::Custom => 3,
        }
    }

    /// The window title string for the level, used in the "fastest time" dialog.
    pub fn name(self) -> &'static str {
        match self {
            Difficulty::Beginner => "beginner",
            Difficulty::Intermediate => "intermediate",
            Difficulty::Expert => "expert",
            Difficulty::Custom => "custom",
        }
    }
}

/// Overall game outcome.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameState {
    Playing,
    Won,
    Lost,
}

/// Microsoft C runtime `rand()` — a 32-bit linear congruential generator.
///
/// This is the exact generator used by `winmine.exe` (`srand`/`rand` from msvcrt).
pub struct MsvcRand {
    state: u32,
}

impl MsvcRand {
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    /// Equivalent to `rand()`: returns a value in `0..=0x7fff`.
    pub fn next_u32(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(214013).wrapping_add(2531011);
        (self.state >> 16) & 0x7fff
    }

    /// Equivalent to `rand() % n`.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u32() as usize) % n
    }
}

/// One high-score entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighScore {
    pub time: i32,
    pub name: String,
}

impl Default for HighScore {
    fn default() -> Self {
        Self { time: 999, name: "Anonymous".to_string() }
    }
}

/// The complete game state.
pub struct Game {
    pub width: usize,
    pub height: usize,
    pub mines: usize,
    /// Stride of the board buffer (`width + 2`).
    stride: usize,
    /// Board buffer, one byte per cell (see module docs).
    pub board: Vec<u8>,
    pub state: GameState,
    /// Number of revealed (non-mine) cells.
    pub opened: usize,
    /// Number of cells that must be revealed to win (`width*height - mines`).
    pub total_safe: usize,
    /// Displayed mine counter: `mines - flags` (may be negative).
    pub counter: i32,
    /// Elapsed seconds (0..=999).
    pub time: i32,
    /// True while the clock is running.
    pub timer_running: bool,
    /// Face state: 0 = normal, 1 = pressed, 2 = dead, 3 = sunglasses.
    pub face: u8,
    /// Whether right-click cycles through the question mark.
    pub marks_enabled: bool,
    rng: MsvcRand,
}

impl Game {
    /// Create a game with the given dimensions and place mines using `seed`.
    pub fn new(width: usize, height: usize, mines: usize, seed: u32, marks_enabled: bool) -> Self {
        let width = width.clamp(9, MAX_WIDTH);
        let height = height.clamp(9, MAX_HEIGHT);
        let max_mines = ((width - 1) * (height - 1)).min(999);
        let mines = mines.clamp(10, max_mines.max(10));
        let stride = width + 2;
        let mut g = Game {
            width,
            height,
            mines,
            stride,
            board: Vec::new(),
            state: GameState::Playing,
            opened: 0,
            total_safe: width * height - mines,
            counter: mines as i32,
            time: 0,
            timer_running: false,
            face: 0,
            marks_enabled,
            rng: MsvcRand::new(seed),
        };
        g.new_game(seed);
        g
    }

    #[inline]
    fn idx(&self, x: usize, y: usize) -> usize {
        y * self.stride + x
    }

    #[inline]
    fn in_bounds(&self, x: usize, y: usize) -> bool {
        x >= 1 && y >= 1 && x <= self.width && y <= self.height
    }

    /// Read the raw board byte for a playfield cell (1-based).
    pub fn cell(&self, x: usize, y: usize) -> u8 {
        self.board[self.idx(x, y)]
    }

    /// Sprite index (0..=15) for a playfield cell; this is what the original blits.
    pub fn sprite(&self, x: usize, y: usize) -> u8 {
        self.board[self.idx(x, y)] & 0x1f
    }

    /// Start a fresh game, placing mines with `seed` (mirrors `FUN_0100367a`).
    pub fn new_game(&mut self, seed: u32) {
        self.timer_running = false;
        self.rng = MsvcRand::new(seed);

        // Clear the board and install the 0x10 border.
        let stride = self.stride;
        self.board = vec![cell::BORDER; stride * (self.height + 2)];
        for y in 1..=self.height {
            for x in 1..=self.width {
                let i = y * stride + x;
                self.board[i] = cell::V_COVERED;
            }
        }

        self.face = 0;

        // Place mines exactly like the original: pick a random cell, retry while occupied.
        let mut to_place = self.mines;
        while to_place > 0 {
            let (x, y) = loop {
                let x = self.rng.below(self.width);
                let y = self.rng.below(self.height);
                if self.board[(y + 1) * stride + (x + 1)] & cell::MINE == 0 {
                    break (x, y);
                }
            };
            self.board[(y + 1) * stride + (x + 1)] |= cell::MINE;
            to_place -= 1;
        }

        self.total_safe = self.width * self.height - self.mines;
        self.time = 0;
        self.counter = self.mines as i32;
        self.opened = 0;
        self.state = GameState::Playing;
    }

    /// Advance the clock by one second (mirrors `FUN_01002fe0`).
    pub fn on_second(&mut self) {
        if self.timer_running && self.time < 999 {
            self.time += 1;
        }
    }

    /// Count mines in the 3x3 neighbourhood (including the centre), mirroring `FUN_01002f3b`.
    fn count_adjacent_mines(&self, x: usize, y: usize) -> u8 {
        let mut n = 0u8;
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let nx = (x as i32 + dx) as usize;
                let ny = (y as i32 + dy) as usize;
                if self.board[self.idx(nx, ny)] & cell::MINE != 0 {
                    n += 1;
                }
            }
        }
        n
    }

    /// Count flags in the 3x3 neighbourhood (including the centre), mirroring `FUN_01003119`.
    fn count_adjacent_flags(&self, x: usize, y: usize) -> u8 {
        let mut n = 0u8;
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let nx = (x as i32 + dx) as usize;
                let ny = (y as i32 + dy) as usize;
                if self.board[self.idx(nx, ny)] & 0x1f == cell::V_FLAG {
                    n += 1;
                }
            }
        }
        n
    }

    /// Set the low bits of a cell while preserving the mine/revealed bits (`FUN_01002eab`).
    fn set_cell(&mut self, x: usize, y: usize, value: u8) {
        let i = self.idx(x, y);
        self.board[i] = (self.board[i] & 0xe0) | value;
    }

    /// Reveal a single cell during a flood fill (`FUN_01003008`).
    fn step_flood(&mut self, x: usize, y: usize, queue: &mut Vec<(usize, usize)>) {
        let i = self.idx(x, y);
        let c = self.board[i];
        if c & cell::REVEALED != 0 {
            return;
        }
        let v = c & 0x1f;
        if v == cell::BORDER || v == cell::V_FLAG {
            return;
        }
        self.opened += 1;
        let n = self.count_adjacent_mines(x, y);
        self.board[i] = n | cell::REVEALED;
        if n == 0 {
            queue.push((x, y));
        }
    }

    /// Iterative flood fill starting at `(x, y)` (`FUN_01003084`).
    fn flood(&mut self, x: usize, y: usize) {
        let mut queue: Vec<(usize, usize)> = Vec::with_capacity(128);
        self.step_flood(x, y, &mut queue);
        let mut head = 0;
        while head < queue.len() {
            let (cx, cy) = queue[head];
            head += 1;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nx = (cx as i32 + dx) as usize;
                    let ny = (cy as i32 + dy) as usize;
                    self.step_flood(nx, ny, &mut queue);
                }
            }
        }
    }

    /// Reveal a cell, with the original's first-click mine relocation (`FUN_01003512`).
    fn reveal(&mut self, x: usize, y: usize) {
        let i = self.idx(x, y);
        if self.board[i] & cell::MINE == 0 {
            self.flood(x, y);
            if self.opened != self.total_safe {
                return;
            }
            self.game_over(true);
            return;
        }

        // Clicked a mine as the very first action: move the mine elsewhere.
        if self.opened == 0 {
            if self.height < 2 {
                return;
            }
            // The original scans rows 1..height-1 and cols 1..width-1.
            for ry in 1..self.height {
                for rx in 1..self.width {
                    let j = self.idx(rx, ry);
                    if self.board[j] & cell::MINE == 0 {
                        self.board[i] = cell::V_COVERED;
                        self.board[j] |= cell::MINE;
                        self.flood(x, y);
                        return;
                    }
                }
            }
            return;
        }

        self.set_cell(x, y, cell::REVEALED | cell::V_EXPLODE);
        self.game_over(false);
    }

    /// Chord: reveal all unflagged neighbours when the flag count matches (`FUN_010035b7`).
    fn chord(&mut self, x: usize, y: usize) {
        let i = self.idx(x, y);
        let c = self.board[i];
        if c & cell::REVEALED == 0 {
            return;
        }
        let flags = self.count_adjacent_flags(x, y);
        if (c & 0x1f) != flags {
            return;
        }

        let mut hit_mine = false;
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let nx = (x as i32 + dx) as usize;
                let ny = (y as i32 + dy) as usize;
                let j = self.idx(nx, ny);
                let nc = self.board[j];
                if (nc & 0x1f) == cell::V_FLAG || (nc as i8) >= 0 {
                    self.flood(nx, ny);
                } else {
                    hit_mine = true;
                    self.set_cell(nx, ny, cell::REVEALED | cell::V_EXPLODE);
                }
            }
        }

        if hit_mine {
            self.game_over(false);
        } else if self.opened == self.total_safe {
            self.game_over(true);
        }
    }

    /// Cycle the mark on a cell (`FUN_0100374f`).
    pub fn mark(&mut self, x: usize, y: usize) {
        if self.state != GameState::Playing || !self.in_bounds(x, y) {
            return;
        }
        let i = self.idx(x, y);
        let c = self.board[i];
        if c & cell::REVEALED != 0 {
            return;
        }
        let v = c & 0x1f;
        let (new_v, delta) = if v == cell::V_FLAG {
            let nv = if self.marks_enabled { cell::V_QUESTION } else { cell::V_COVERED };
            (nv, 1)
        } else if v == cell::V_QUESTION {
            (cell::V_COVERED, 0)
        } else {
            (cell::V_FLAG, -1)
        };
        self.counter += delta;
        self.set_cell(x, y, new_v);

        if self.board[i] & 0x1f == cell::V_FLAG && self.opened == self.total_safe {
            self.game_over(true);
        }
    }

    /// Primary click on a cell: reveal, or chord when `chord` is set (`FUN_010037e1`/`01003512`.
    ///
    /// `chord` corresponds to the original's "both buttons pressed" mode.
    pub fn click(&mut self, x: usize, y: usize, chord: bool) {
        if self.state != GameState::Playing || !self.in_bounds(x, y) {
            return;
        }

        // Starting the clock also happens on the first reveal action.
        if self.opened == 0 && self.time == 0 {
            self.time = 1;
            self.timer_running = true;
        }

        if chord {
            self.chord(x, y);
        } else {
            let i = self.idx(x, y);
            if self.board[i] & cell::REVEALED == 0 && self.board[i] & 0x1f != cell::V_FLAG {
                self.reveal(x, y);
            }
        }
    }

    /// Shared end-of-game logic (`FUN_0100347c`).
    fn game_over(&mut self, won: bool) {
        self.timer_running = false;
        self.face = if won { 3 } else { 2 };
        self.reveal_mines(if won { cell::V_FLAG } else { cell::V_MINE });
        if won && self.counter != 0 {
            self.counter = 0;
        }
        self.state = if won { GameState::Won } else { GameState::Lost };
    }

    /// Reveal all mines / mark incorrect flags at the end of a game (`FUN_01002f80`).
    fn reveal_mines(&mut self, value: u8) {
        for y in 1..=self.height {
            for x in 1..=self.width {
                let i = self.idx(x, y);
                let b = self.board[i];
                if b & cell::REVEALED != 0 {
                    continue;
                }
                if b & cell::MINE != 0 {
                    if b & 0x1f != cell::V_FLAG {
                        self.board[i] = (b & 0xe0) | value;
                    }
                } else if b & 0x1f == cell::V_FLAG {
                    self.board[i] = (b & 0xeb) | cell::V_WRONG;
                }
            }
        }
    }

    /// True while cells can still be played.
    pub fn is_playing(&self) -> bool {
        self.state == GameState::Playing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place_mines_manual(g: &mut Game, mines: &[(usize, usize)]) {
        let stride = g.stride;
        for y in 1..=g.height {
            for x in 1..=g.width {
                g.board[y * stride + x] = cell::V_COVERED;
            }
        }
        for &(x, y) in mines {
            g.board[y * stride + x] |= cell::MINE;
        }
        g.total_safe = g.width * g.height - mines.len();
        g.mines = mines.len();
        g.counter = mines.len() as i32;
        g.opened = 0;
        g.state = GameState::Playing;
        g.time = 0;
        g.timer_running = false;
    }

    // MSVC rand() sequence for seed 0 (well-known values).
    #[test]
    fn msvc_rand_seed_zero() {
        let mut r = MsvcRand::new(0);
        // state = 0*214013 + 2531011 = 2531011; (>>16)&0x7fff = 38
        assert_eq!(r.next_u32(), 38);
        assert_eq!(r.next_u32(), 7719);
        assert_eq!(r.next_u32(), 21238);
        assert_eq!(r.next_u32(), 2437);
    }

    #[test]
    fn board_dimensions_and_border() {
        let g = Game::new(9, 9, 10, 12345, true);
        assert_eq!(g.board.len(), (9 + 2) * (9 + 2));
        // Border cells are 0x10.
        assert_eq!(g.cell(0, 0), cell::BORDER);
        assert_eq!(g.cell(10, 5), cell::BORDER);
        // Interior cells are covered and (mostly) not mines.
        let mut mine_count = 0;
        for y in 1..=9 {
            for x in 1..=9 {
                let c = g.cell(x, y);
                assert_eq!(c & 0x1f, cell::V_COVERED);
                if c & cell::MINE != 0 {
                    mine_count += 1;
                }
            }
        }
        assert_eq!(mine_count, 10);
        assert_eq!(g.total_safe, 81 - 10);
        assert_eq!(g.counter, 10);
    }

    #[test]
    fn deterministic_placement_for_seed() {
        let a = Game::new(9, 9, 10, 42, true);
        let b = Game::new(9, 9, 10, 42, true);
        assert_eq!(a.board, b.board);
        let c = Game::new(9, 9, 10, 43, true);
        assert_ne!(a.board, c.board);
    }

    #[test]
    fn first_click_on_mine_relocates_never_loses() {
        // Build a tiny-ish board fully controlled: 9x9 with a single mine at (1,1).
        let mut g = Game::new(9, 9, 10, 7, true);
        place_mines_manual(&mut g, &[(1, 1)]);
        // Reveal cell (1,1) which is a mine; first click must relocate, not lose.
        g.click(1, 1, false);
        assert_eq!(g.state, GameState::Playing);
        assert!(g.opened > 0);
        // The clicked cell must now be revealed.
        assert_ne!(g.cell(1, 1) & cell::REVEALED, 0);
        // The mine must have moved somewhere within rows 1..height-1, cols 1..width-1.
        let mut found = 0;
        for y in 1..g.height {
            for x in 1..g.width {
                if g.cell(x, y) & cell::MINE != 0 {
                    found += 1;
                }
            }
        }
        assert_eq!(found, 1);
    }

    #[test]
    fn reveal_number_counts_neighbours() {
        let mut g = Game::new(9, 9, 10, 1, true);
        // Mines around (2,2): (1,1),(3,1),(1,3) => 3 neighbours.
        place_mines_manual(&mut g, &[(1, 1), (3, 1), (1, 3)]);
        g.click(2, 2, false);
        assert_eq!(g.cell(2, 2), cell::REVEALED | 3);
        assert_eq!(g.opened, 1);
    }

    #[test]
    fn zero_cell_floods_and_reveals_region() {
        let mut g = Game::new(9, 9, 10, 1, true);
        place_mines_manual(&mut g, &[(9, 9)]);
        // Clicking (1,1) is a zero cell -> floods the whole board except near the mine.
        g.click(1, 1, false);
        // Everything except the mine cell should be revealed.
        assert_eq!(g.opened, 9 * 9 - 1);
        assert_eq!(g.state, GameState::Won);
    }

    #[test]
    fn mark_cycle_with_marks_enabled() {
        let mut g = Game::new(9, 9, 10, 1, true);
        // Full board except one safe cell so the game does not auto-win toggling.
        place_mines_manual(&mut g, &[(5, 5), (6, 6)]);
        let before = g.counter;
        g.mark(1, 1);
        assert_eq!(g.cell(1, 1) & 0x1f, cell::V_FLAG);
        assert_eq!(g.counter, before - 1);
        g.mark(1, 1);
        assert_eq!(g.cell(1, 1) & 0x1f, cell::V_QUESTION);
        assert_eq!(g.counter, before);
        g.mark(1, 1);
        assert_eq!(g.cell(1, 1) & 0x1f, cell::V_COVERED);
        assert_eq!(g.counter, before);
    }

    #[test]
    fn mark_cycle_with_marks_disabled_skips_question() {
        let mut g = Game::new(9, 9, 10, 1, false);
        place_mines_manual(&mut g, &[(5, 5), (6, 6)]);
        g.mark(1, 1);
        assert_eq!(g.cell(1, 1) & 0x1f, cell::V_FLAG);
        g.mark(1, 1);
        assert_eq!(g.cell(1, 1) & 0x1f, cell::V_COVERED);
    }

    #[test]
    fn chord_reveals_when_flags_match_and_loses_on_wrong_flag() {
        let mut g = Game::new(9, 9, 10, 1, true);
        // Mines at (1,1),(3,1),(1,3); number cell (2,2)=3.
        place_mines_manual(&mut g, &[(1, 1), (3, 1), (1, 3), (9, 9)]);
        g.click(2, 2, false);
        assert_eq!(g.cell(2, 2) & 0x1f, 3);
        // Correctly flag the three mines, then chord.
        g.mark(1, 1);
        g.mark(3, 1);
        g.mark(1, 3);
        g.click(2, 2, true);
        // Non-flagged neighbours are revealed by the chord.
        assert_eq!(g.cell(2, 1) & cell::REVEALED, cell::REVEALED);
        assert_eq!(g.cell(1, 2) & cell::REVEALED, cell::REVEALED);
    }

    #[test]
    fn chord_on_wrong_flag_explodes_and_loses() {
        let mut g = Game::new(9, 9, 10, 1, true);
        place_mines_manual(&mut g, &[(1, 1), (3, 1), (1, 3), (9, 9)]);
        g.click(2, 2, false);
        // Flag a safe cell as if it were a mine, and leave a real mine unflagged.
        g.mark(2, 1);
        g.mark(3, 1);
        g.mark(1, 3);
        g.click(2, 2, true);
        assert_eq!(g.state, GameState::Lost);
        // The first unflagged mine among the neighbours should show the exploded sprite.
        assert_eq!(g.cell(1, 1) & 0x1f, cell::V_EXPLODE);
    }

    #[test]
    fn win_when_all_safe_revealed() {
        let mut g = Game::new(9, 9, 10, 1, true);
        place_mines_manual(&mut g, &[(5, 5)]);
        g.click(1, 1, false);
        assert_eq!(g.state, GameState::Won);
        assert_eq!(g.face, 3);
        assert_eq!(g.counter, 0);
        // All mines are shown as flags on a win.
        assert_eq!(g.cell(5, 5) & 0x1f, cell::V_FLAG);
    }

    #[test]
    fn lose_shows_mines_and_wrong_flags() {
        let mut g = Game::new(9, 9, 10, 1, true);
        // A mine next to (1,1) keeps the first reveal to a single numbered cell.
        place_mines_manual(&mut g, &[(2, 2), (9, 9)]);
        g.click(1, 1, false);
        assert_eq!(g.cell(1, 1), cell::REVEALED | 1);
        assert_eq!(g.state, GameState::Playing);
        // Flag a safe cell, then click a mine.
        g.mark(5, 5);
        g.click(2, 2, false);
        assert_eq!(g.state, GameState::Lost);
        assert_eq!(g.face, 2);
        assert_eq!(g.cell(9, 9) & 0x1f, cell::V_MINE);
        assert_eq!(g.cell(5, 5) & 0x1f, cell::V_WRONG);
    }

    #[test]
    fn timer_semantics() {
        let mut g = Game::new(9, 9, 10, 1, true);
        // Mine at (2,2): clicking (1,1) reveals one numbered cell and does not win.
        place_mines_manual(&mut g, &[(2, 2), (9, 9)]);
        assert_eq!(g.time, 0);
        assert!(!g.timer_running);
        g.click(1, 1, false);
        assert_eq!(g.time, 1);
        assert!(g.timer_running);
        g.on_second();
        g.on_second();
        assert_eq!(g.time, 3);
    }

    #[test]
    fn winning_stops_the_clock() {
        let mut g = Game::new(9, 9, 10, 1, true);
        place_mines_manual(&mut g, &[(9, 9)]);
        g.click(1, 1, false);
        assert_eq!(g.state, GameState::Won);
        assert!(!g.timer_running);
    }

    #[test]
    fn custom_mine_cap() {
        // 9x9 -> max mines (8*8)=64.
        let g = Game::new(9, 9, 999, 1, true);
        assert_eq!(g.mines, 64);
        // 30x24 -> max mines 29*23=667 (<999).
        let g = Game::new(30, 24, 999, 1, true);
        assert_eq!(g.mines, 29 * 23);
    }
}
