use std::collections::VecDeque;

use crate::board::Board;
use crate::movegen::{Placement, Spin};
use crate::piece::Piece;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gravity {
    Zero,
    TwentyG,
}

/// Mode-specific physics. Rotation system and randomizer are fixed to SRS + 7-bag
/// until a second game needs something else.
#[derive(Clone, Copy, Debug)]
pub struct Rules {
    pub gravity: Gravity,
    pub spawn: (i8, i8),
    pub preview: usize,
    pub hold: bool,
}

impl Rules {
    /// MonochromeRush `Game.updateGravity`: levels 1-10 fall at 50-16 ms/row, 11+ are 20G.
    pub fn monochrome_rush(level: u32) -> Self {
        // ponytail: levels 1-10 modelled as zero gravity (bot inputs outpace 50 ms/row); model the fall rate if tucks fail in-game.
        let gravity = if level > 10 { Gravity::TwentyG } else { Gravity::Zero };
        Rules { gravity, spawn: (4, 18), preview: 5, hold: true }
    }

    /// Stackfall versus: same board and spawn, low gravity the bot treats as zero.
    pub fn versus() -> Self {
        Rules { gravity: Gravity::Zero, spawn: (4, 18), preview: 5, hold: true }
    }
}

/// Extra lines sent for 0, 1, 2, ... consecutive clears.
const COMBO_ATTACK: [u32; 13] = [0, 0, 1, 1, 1, 2, 2, 3, 3, 4, 4, 4, 5];

/// Garbage lines a clear sends in Stackfall versus (guideline-style table). Updates `b2b` and
/// `combo`: any lock without a clear resets combo; a non-spin, non-quad clear breaks back-to-back.
pub fn attack(lines: u32, spin: Spin, b2b: &mut bool, combo: &mut i32, perfect_clear: bool) -> u32 {
    if lines == 0 {
        *combo = -1;
        return 0;
    }
    let base = match (spin, lines) {
        (Spin::Full, n) => 2 * n,
        (Spin::Mini, 2) => 1,
        (Spin::Mini, _) => 0,
        (Spin::None, 4) => 4,
        (Spin::None, n) => n - 1,
    };
    let difficult = lines == 4 || spin != Spin::None;
    let b2b_bonus = (difficult && *b2b) as u32;
    *b2b = difficult;
    *combo += 1;
    base + b2b_bonus + COMBO_ATTACK[(*combo as usize).min(12)] + if perfect_clear { 10 } else { 0 }
}

/// MonochromeRush `calculateScore` before the level multiplier: base table, x1.5 when
/// back-to-back, plus 50 per combo step. Updates `b2b` and `combo` like the game does.
pub fn points(lines: u32, spin: Spin, b2b: &mut bool, combo: &mut i32) -> f32 {
    if lines == 0 && spin == Spin::None {
        *combo = -1;
        return 0.0;
    }
    let (base, b2b_clear) = match (spin, lines) {
        (Spin::Full, 0) => (400.0, false),
        (Spin::Full, n) => (400.0 * (n + 1) as f32, true),
        (Spin::Mini, 0) => (100.0, false),
        (Spin::Mini, 1) => (200.0, true),
        (Spin::Mini, _) => (400.0, true),
        (Spin::None, 1) => (100.0, false),
        (Spin::None, 2) => (300.0, false),
        (Spin::None, 3) => (500.0, false),
        (Spin::None, _) => (800.0, true),
    };
    let mut pts = base;
    if b2b_clear {
        if *b2b {
            pts *= 1.5;
        }
        *b2b = true;
    } else if lines > 0 {
        *b2b = false;
    }
    if lines > 0 {
        if *combo >= 0 {
            pts += 50.0 * *combo as f32;
        }
        *combo += 1;
    }
    pts
}

#[derive(Debug)]
pub struct LockInfo {
    pub lines: u32,
    pub spin: Spin,
    pub perfect_clear: bool,
    /// Game points before the level multiplier.
    pub points: f32,
}

pub struct Game {
    pub rules: Rules,
    pub board: Board,
    pub current: Piece,
    pub hold: Option<Piece>,
    pub queue: VecDeque<Piece>,
    pub lines: u32,
    pub b2b: bool,
    pub combo: i32,
    pub over: bool,
    bag: Bag,
}

impl Game {
    pub fn new(rules: Rules, seed: u64) -> Self {
        let mut bag = Bag::new(seed);
        let current = bag.next();
        let queue = (0..rules.preview).map(|_| bag.next()).collect();
        Game { rules, board: Board::default(), current, hold: None, queue, lines: 0, b2b: false, combo: -1, over: false, bag }
    }

    /// Plays a placement, holding first if it's for the hold piece (or the next piece when hold is empty).
    pub fn play(&mut self, p: &Placement) -> LockInfo {
        if p.piece != self.current {
            assert!(self.rules.hold, "placement needs hold, which these rules don't allow");
            let held = self.current;
            self.current = match self.hold {
                Some(h) => h,
                None => self.next_piece(),
            };
            self.hold = Some(held);
            assert_eq!(p.piece, self.current, "placement is for neither current nor hold piece");
        }
        let lines = self.board.lock(p.piece, p.x, p.y, p.rot);
        self.lines += lines;
        let points = points(lines, p.spin, &mut self.b2b, &mut self.combo);
        let info = LockInfo { lines, spin: p.spin, perfect_clear: lines > 0 && self.board.is_empty(), points };
        self.current = self.next_piece();
        let (x, y) = self.rules.spawn;
        self.over = self.board.collides(self.current, x, y, 0);
        info
    }

    /// What can come after the visible queue, for `Root::bag`.
    pub fn bag_mask(&self) -> u8 {
        self.bag.remaining_mask()
    }

    fn next_piece(&mut self) -> Piece {
        let p = self.queue.pop_front().unwrap();
        self.queue.push_back(self.bag.next());
        p
    }
}

/// 7-bag with Fisher-Yates over a xorshift RNG, so games replay from a seed.
pub(crate) struct Bag {
    rng: u64,
    pieces: Vec<Piece>,
}

impl Bag {
    pub(crate) fn new(seed: u64) -> Self {
        Bag { rng: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1, pieces: vec![] }
    }

    /// Piece types the current bag still holds (all seven once it's used up).
    pub(crate) fn remaining_mask(&self) -> u8 {
        if self.pieces.is_empty() { 0x7F } else { self.pieces.iter().fold(0, |m, &p| m | 1 << p as u8) }
    }

    pub(crate) fn next(&mut self) -> Piece {
        if self.pieces.is_empty() {
            self.pieces = Piece::ALL.to_vec();
            for i in (1..7).rev() {
                self.rng ^= self.rng << 13;
                self.rng ^= self.rng >> 7;
                self.rng ^= self.rng << 17;
                self.pieces.swap(i, (self.rng % (i as u64 + 1)) as usize);
            }
        }
        self.pieces.pop().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bag_deals_permutations() {
        let mut bag = Bag::new(42);
        for _ in 0..10 {
            let mut seven: Vec<_> = (0..7).map(|_| bag.next() as u8).collect();
            seven.sort();
            assert_eq!(seven, [0, 1, 2, 3, 4, 5, 6]);
        }
    }

    #[test]
    fn points_match_monochrome_rush() {
        let (mut b2b, mut combo) = (false, -1);
        assert_eq!(points(4, Spin::None, &mut b2b, &mut combo), 800.0); // quad, combo 0
        assert_eq!(points(2, Spin::Full, &mut b2b, &mut combo), 1200.0 * 1.5); // B2B TSD, combo bonus 50 x 0
        assert_eq!(points(1, Spin::None, &mut b2b, &mut combo), 100.0 + 50.0); // single breaks B2B, combo bonus 50 x 1
        assert!(!b2b);
        assert_eq!(points(0, Spin::None, &mut b2b, &mut combo), 0.0);
        assert_eq!(combo, -1);
    }

    /// The bot survives 300 pieces in both gravities, exercising hold and line clears.
    #[test]
    fn bot_survives_both_gravities() {
        use crate::bot::{Config, Root, suggest};
        let cfg = Config { width: 8, depth: 3, ..Config::default() };
        for level in [1, 11] {
            let mut game = Game::new(Rules::monochrome_rush(level), 7);
            for _ in 0..300 {
                let queue: Vec<_> = game.queue.iter().copied().collect();
                let root = Root { board: game.board, current: game.current, hold: game.hold, can_hold: true, queue: &queue, b2b: game.b2b, combo: game.combo, pending: 0, bag: game.bag_mask() };
                game.play(&suggest(&root, &game.rules, &cfg).unwrap());
                assert!(!game.over, "level {level}: topped out after {} lines", game.lines);
            }
        }
    }
}
