//! Beam search over the visible queue (with hold). Nodes are scored by MonochromeRush
//! points earned along the way plus a hand-weighted board evaluation at the leaf.

use std::time::{Duration, Instant};

use crate::board::{Board, HEIGHT, WIDTH};
use crate::game::{Game, Rules, attack, points};
use crate::movegen::{Placement, Spin, find_placements};
use crate::piece::Piece;

/// Board evaluation weights, in units of 100 game points (a single = 1, a quad = 8).
/// Defaults come from `examples/tune.rs` (cross-entropy, 15 generations, MonochromeRush scoring).
#[derive(Clone, Debug)]
pub struct Weights {
    pub holes: f32,
    pub bumpiness: f32,
    pub height: f32,
    /// Per row above 10 in each column; 20G movement gets cramped up there.
    pub danger: f32,
    /// Per row of the deepest well, capped at 4 (a quad's worth).
    pub well: f32,
    pub tsd: f32,
    pub tst: f32,
    pub b2b: f32,
}

impl Default for Weights {
    fn default() -> Self {
        Weights { holes: -5.41, bumpiness: -0.76, height: 0.19, danger: -2.42, well: 1.03, tsd: 6.81, tst: 3.45, b2b: 2.82 }
    }
}

impl Weights {
    /// For Stackfall versus. The tuned defaults learned to *reward* height (MonochromeRush
    /// multiplies score by level), which makes a versus bot stack up without clearing.
    /// Hand-picked until a versus tuner (bot-vs-bot win rate) exists.
    pub fn versus() -> Self {
        Weights { holes: -4.0, bumpiness: -0.4, height: -0.05, danger: -1.0, well: 0.7, tsd: 4.0, tst: 6.0, b2b: 1.0 }
    }

    pub fn to_vec(&self) -> Vec<f32> {
        vec![self.holes, self.bumpiness, self.height, self.danger, self.well, self.tsd, self.tst, self.b2b]
    }

    pub fn from_slice(v: &[f32]) -> Self {
        Weights { holes: v[0], bumpiness: v[1], height: v[2], danger: v[3], well: v[4], tsd: v[5], tst: v[6], b2b: v[7] }
    }
}

/// What a clear is worth to the search.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Objective {
    /// MonochromeRush score (points / 100).
    Points,
    /// Stackfall versus garbage sent (x2, so a quad is worth 8 like in `Points`).
    Attack,
}

#[derive(Clone, Debug)]
pub struct Config {
    /// Nodes kept per depth.
    pub width: usize,
    /// Pieces looked ahead, capped by the visible queue.
    pub depth: usize,
    pub weights: Weights,
    pub objective: Objective,
    /// Random 0..noise added to every node's value; makes weaker levels misjudge like a human.
    pub noise: f32,
    /// Seeds the noise, so two bots at the same level make different mistakes.
    pub salt: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config { width: 64, depth: 6, weights: Weights::default(), objective: Objective::Points, noise: 0.0, salt: 0 }
    }
}

impl Config {
    /// Versus bot strength, 1 (beginner) ..= 10 (full search, no mistakes). Skills unlock with
    /// level: low levels don't plan quad wells or T-spins at all and misjudge a lot.
    /// `objective` picks the weights: `Attack` (versus) or `Points` (solo score modes).
    pub fn difficulty(level: u32, objective: Objective) -> Self {
        let i = level.clamp(1, 10) as usize - 1;
        let spin_skill = [0.0, 0.0, 0.0, 0.2, 0.4, 0.6, 0.8, 1.0, 1.0, 1.0][i];
        let well_skill = [0.0, 0.0, 0.3, 0.6, 0.8, 1.0, 1.0, 1.0, 1.0, 1.0][i];
        let base = match objective {
            Objective::Attack => Weights::versus(),
            Objective::Points => Weights::default(),
        };
        Config {
            width: [1, 1, 1, 2, 4, 8, 16, 24, 48, 64][i],
            depth: [1, 1, 1, 2, 2, 3, 4, 5, 6, 6][i],
            noise: [14.0, 10.0, 7.0, 4.5, 3.0, 1.8, 1.0, 0.4, 0.15, 0.0][i],
            objective,
            weights: Weights {
                tsd: base.tsd * spin_skill,
                tst: base.tst * spin_skill,
                b2b: base.b2b * spin_skill,
                well: base.well * well_skill,
                ..base
            },
            salt: 0,
        }
    }
}

/// What the player can see. `queue` is only the visible preview, so the bot doesn't peek at the bag.
pub struct Root<'a> {
    pub board: Board,
    pub current: Piece,
    pub hold: Option<Piece>,
    pub can_hold: bool,
    pub queue: &'a [Piece],
    pub b2b: bool,
    pub combo: i32,
}

#[derive(Clone, Copy)]
struct Node {
    board: Board,
    hold: Option<Piece>,
    /// Index into [current, queue..] of the next piece to play.
    next: usize,
    b2b: bool,
    combo: i32,
    reward: f32,
    value: f32,
    /// Index of the root placement this line started with.
    first: usize,
}

pub fn suggest(root: &Root, rules: &Rules, cfg: &Config) -> Option<Placement> {
    let seq: Vec<Piece> = std::iter::once(root.current).chain(root.queue.iter().copied()).collect();
    let mut first_moves = vec![];
    let mut layer = vec![Node {
        board: root.board,
        hold: root.hold,
        next: 0,
        b2b: root.b2b,
        combo: root.combo,
        reward: 0.0,
        value: 0.0,
        first: 0,
    }];
    let mut best = None;

    for depth in 0..cfg.depth {
        let mut children = vec![];
        for node in &layer {
            for (piece, hold, next) in options(node, &seq, depth > 0 || root.can_hold) {
                for p in find_placements(&node.board, piece, rules) {
                    let mut board = node.board;
                    let lines = board.lock(p.piece, p.x, p.y, p.rot);
                    if tops_out(&board, rules) {
                        continue;
                    }
                    let (mut b2b, mut combo) = (node.b2b, node.combo);
                    let gain = match cfg.objective {
                        Objective::Points => points(lines, p.spin, &mut b2b, &mut combo) / 100.0,
                        Objective::Attack => {
                            let pc = lines > 0 && board.is_empty();
                            2.0 * attack(lines, p.spin, &mut b2b, &mut combo, pc) as f32
                        }
                    };
                    let reward = node.reward + gain;
                    let mut value = reward + eval(&board, b2b, &cfg.weights);
                    if cfg.noise > 0.0 {
                        value += cfg.noise * jitter(&board, cfg.salt);
                    }
                    let first = if depth == 0 {
                        first_moves.push(p);
                        first_moves.len() - 1
                    } else {
                        node.first
                    };
                    children.push(Node { board, hold, next, b2b, combo, reward, value, first });
                }
            }
        }
        if children.is_empty() {
            break;
        }
        // ponytail: full sort per layer; select_nth_unstable if profiling says so.
        children.sort_unstable_by(|a, b| b.value.total_cmp(&a.value));
        children.truncate(cfg.width);
        best = Some(children[0].first);
        layer = children;
    }
    best.map(|i| first_moves.swap_remove(i))
}

/// (piece to play, hold afterwards, next index) for each choice at this node.
fn options(n: &Node, seq: &[Piece], can_hold: bool) -> Vec<(Piece, Option<Piece>, usize)> {
    let Some(&cur) = seq.get(n.next) else { return vec![] };
    let mut out = vec![(cur, n.hold, n.next + 1)];
    if can_hold {
        match n.hold {
            Some(h) if h != cur => out.push((h, Some(cur), n.next + 1)),
            None => {
                if let Some(&after) = seq.get(n.next + 1) {
                    out.push((after, Some(cur), n.next + 2));
                }
            }
            _ => {}
        }
    }
    out
}

/// Deterministic pseudo-random 0..1 from the board, so noisy play stays reproducible.
fn jitter(board: &Board, salt: u64) -> f32 {
    let h = board.rows.iter().fold(0xCBF2_9CE4_8422_2325u64 ^ salt, |h, &r| (h ^ r as u64).wrapping_mul(0x100_0000_01B3));
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Anything in the spawn area means the next piece can't appear.
fn tops_out(board: &Board, rules: &Rules) -> bool {
    let (x, y) = rules.spawn;
    let mask = 0b1111 << (x - 1);
    (y - 1..=y).any(|r| board.rows[r as usize] & mask != 0)
}

fn eval(board: &Board, b2b: bool, w: &Weights) -> f32 {
    // Cutout (as in Cold Clear): if a T-spin slot is ready, score the board as if the T-spin
    // happened. Otherwise the slot's covered cells count as holes and setups get punished.
    let slot = tslot(board);
    let mut cut = *board;
    if let Some((x, y, rot, _)) = slot {
        cut.lock(Piece::T, x, y, rot);
    }
    let b = &cut;

    let h: [i32; WIDTH as usize] =
        std::array::from_fn(|x| (0..HEIGHT).find(|&y| b.occupied(x as i8, y)).map_or(0, |y| (HEIGHT - y) as i32));
    let holes: usize =
        (0..WIDTH as usize).map(|x| (HEIGHT - h[x] as i8..HEIGHT).filter(|&y| !b.occupied(x as i8, y)).count()).sum();
    let well = (0..WIDTH as usize).min_by_key(|&x| h[x]).unwrap();
    let bumpiness: i32 =
        (0..WIDTH as usize - 1).filter(|&x| x != well && x + 1 != well).map(|x| (h[x] - h[x + 1]).abs()).sum();
    let rim = [well.checked_sub(1), Some(well + 1).filter(|&x| x < WIDTH as usize)]
        .into_iter()
        .flatten()
        .map(|x| h[x])
        .min()
        .unwrap();
    let well_depth = (rim - h[well]).min(4);
    let danger: i32 = h.iter().map(|&v| (v - 10).max(0)).sum();
    let spin = match slot {
        Some((.., 3)) => w.tst,
        Some((.., 2)) => w.tsd,
        _ => 0.0,
    };

    w.holes * holes as f32
        + w.bumpiness * bumpiness as f32
        + w.height * h.iter().sum::<i32>() as f32
        + w.danger * danger as f32
        + w.well * well_depth as f32
        + spin
        + w.b2b * b2b as u8 as f32
}

/// The best ready T-spin slot as (x, y, rot, lines): a TST if any, else a TSD.
/// Corners are implied by the rows being full; full reachability isn't checked.
/// TSD: nose-down T at (x, y) under an overhang completes rows y and y+1.
/// TST: upright T at (x, y), nose in a side notch, completes rows y-1..=y+1; needs a roof over
/// its column, since the T only gets in through the SRS triple-twist kick.
fn tslot(b: &Board) -> Option<(i8, i8, u8, u32)> {
    const FULL: u16 = 0x3FF;
    // Only `bits` are missing from row y.
    let only = |y: i8, bits: u16| b.rows[y as usize] | bits == FULL && b.rows[y as usize] & bits == 0;
    let mut tsd = None;
    for x in 1..WIDTH - 1 {
        let (t, stem) = (0b111u16 << (x - 1), 1u16 << x);
        for y in 1..HEIGHT - 1 {
            if only(y - 1, stem) && only(y + 1, stem) && (0..y - 1).any(|r| b.occupied(x, r)) {
                for (n, rot) in [(x - 1, 3), (x + 1, 1)] {
                    if only(y, stem | 1 << n) {
                        return Some((x, y, rot, 3));
                    }
                }
            }
            if tsd.is_none() && only(y, t) && only(y + 1, stem) && (b.occupied(x - 1, y - 1) || b.occupied(x + 1, y - 1)) {
                tsd = Some((x, y, 2, 2));
            }
        }
    }
    tsd
}

/// Totals from one simulated MonochromeRush game.
#[derive(Default, Clone, Debug)]
pub struct GameStats {
    pub score: f64,
    pub lines: u32,
    pub pieces: u32,
    pub quads: u32,
    pub tsds: u32,
    pub tsts: u32,
    pub think: Duration,
    pub worst: Duration,
}

/// Plays MonochromeRush from level 1 (level = 1 + lines/10, 20G from 11) until 300 lines or top-out.
/// Score excludes the game's soft/hard drop points.
pub fn play_game(cfg: &Config, seed: u64) -> GameStats {
    let mut game = Game::new(Rules::monochrome_rush(1), seed);
    let mut s = GameStats::default();
    while game.lines < 300 && !game.over {
        let queue: Vec<_> = game.queue.iter().copied().collect();
        let root = Root {
            board: game.board,
            current: game.current,
            hold: game.hold,
            can_hold: true,
            queue: &queue,
            b2b: game.b2b,
            combo: game.combo,
        };
        let t = Instant::now();
        let Some(p) = suggest(&root, &game.rules, cfg) else { break };
        let dt = t.elapsed();
        s.think += dt;
        s.worst = s.worst.max(dt);
        let level = 1 + game.lines / 10;
        let info = game.play(&p);
        s.score += (info.points * level as f32) as f64;
        s.pieces += 1;
        s.quads += (info.lines == 4) as u32;
        if info.spin == Spin::Full {
            s.tsds += (info.lines == 2) as u32;
            s.tsts += (info.lines == 3) as u32;
        }
        game.rules = Rules::monochrome_rush(1 + game.lines / 10);
    }
    s.lines = game.lines.min(300);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_tsd() {
        let mut board = Board::default();
        board.rows[39] = 0x3FF & !(1 << 1);
        board.rows[38] = 0x3FF & !0b111;
        board.rows[37] = 1;
        assert_eq!(tslot(&board).map(|s| s.3), Some(2));
        let root = Root { board, current: Piece::T, hold: None, can_hold: true, queue: &[Piece::I, Piece::O], b2b: false, combo: -1 };
        let p = suggest(&root, &Rules::monochrome_rush(1), &Config::default()).unwrap();
        assert_eq!((p.piece, p.spin), (Piece::T, Spin::Full));
    }

    /// Versus bots keep their stack low (garbage is coming); T-spin setups cost about half a row.
    /// Score-tuned weights average ~6 rows
    /// at level 7+ and ~7.5 at level 4, so they fail this.
    #[test]
    fn versus_levels_stack_low() {
        for (level, max_avg) in [(4, 7.0), (7, 5.5), (10, 5.5)] {
            let cfg = Config::difficulty(level, Objective::Attack);
            let (mut sum, mut n) = (0, 0);
            for seed in 0..2 {
                let mut game = Game::new(Rules::versus(), seed);
                for _ in 0..200 {
                    let queue: Vec<_> = game.queue.iter().copied().collect();
                    let root = Root { board: game.board, current: game.current, hold: game.hold, can_hold: true, queue: &queue, b2b: game.b2b, combo: game.combo };
                    game.play(&suggest(&root, &game.rules, &cfg).unwrap());
                    assert!(!game.over, "level {level} topped out");
                    sum += 40 - game.board.rows.iter().position(|&r| r != 0).unwrap_or(40);
                    n += 1;
                }
            }
            let avg = sum as f32 / n as f32;
            assert!(avg <= max_avg, "level {level} averaged {avg:.1} rows high");
        }
    }

    /// Needs the SRS triple-twist kick (0, +2) under a two-block overhang.
    #[test]
    fn takes_the_tst() {
        let mut board = Board::default();
        board.rows[39] = 0x3FF & !(1 << 2);
        board.rows[38] = 0x3FF & !0b110;
        board.rows[37] = 0x3FF & !(1 << 2);
        board.rows[35] = 0b1100;
        assert_eq!(tslot(&board).map(|s| s.3), Some(3));
        for level in [1, 11] {
            let root = Root { board, current: Piece::T, hold: None, can_hold: true, queue: &[Piece::I, Piece::O], b2b: false, combo: -1 };
            let p = suggest(&root, &Rules::monochrome_rush(level), &Config::default()).unwrap();
            assert_eq!((p.piece, p.spin), (Piece::T, Spin::Full), "level {level}");
            assert_eq!(board.clone().lock(p.piece, p.x, p.y, p.rot), 3, "level {level}");
        }
    }
}
