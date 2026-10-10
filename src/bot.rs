//! Beam search over the visible queue (with hold). Nodes are scored by MonochromeRush
//! points earned along the way plus a hand-weighted board evaluation at the leaf.

use std::time::{Duration, Instant};

use crate::board::{Board, HEIGHT, WIDTH};
use crate::game::{Game, Rules, points};
use crate::movegen::{Placement, Spin, find_placements};
use crate::piece::Piece;

/// Board evaluation weights, in units of 100 game points (a single = 1, a quad = 8).
/// Hand-picked for now; these are what step 4 tunes.
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
        Weights { holes: -4.0, bumpiness: -0.4, height: -0.05, danger: -1.0, well: 0.7, tsd: 4.0, tst: 6.0, b2b: 1.0 }
    }
}

impl Weights {
    pub fn to_vec(&self) -> Vec<f32> {
        vec![self.holes, self.bumpiness, self.height, self.danger, self.well, self.tsd, self.tst, self.b2b]
    }

    pub fn from_slice(v: &[f32]) -> Self {
        Weights { holes: v[0], bumpiness: v[1], height: v[2], danger: v[3], well: v[4], tsd: v[5], tst: v[6], b2b: v[7] }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    /// Nodes kept per depth.
    pub width: usize,
    /// Pieces looked ahead, capped by the visible queue.
    pub depth: usize,
    pub weights: Weights,
}

impl Default for Config {
    fn default() -> Self {
        Config { width: 64, depth: 6, weights: Weights::default() }
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
                    let reward = node.reward + points(lines, p.spin, &mut b2b, &mut combo) / 100.0;
                    let value = reward + eval(&board, b2b, &cfg.weights);
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

/// Anything in the spawn area means the next piece can't appear.
fn tops_out(board: &Board, rules: &Rules) -> bool {
    let (x, y) = rules.spawn;
    let mask = 0b1111 << (x - 1);
    (y - 1..=y).any(|r| board.rows[r as usize] & mask != 0)
}

fn eval(b: &Board, b2b: bool, w: &Weights) -> f32 {
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
    let (tsd, tst) = tslots(b);

    w.holes * holes as f32
        + w.bumpiness * bumpiness as f32
        + w.height * h.iter().sum::<i32>() as f32
        + w.danger * danger as f32
        + w.well * well_depth as f32
        + w.tsd * tsd as u8 as f32
        + w.tst * tst as u8 as f32
        + w.b2b * b2b as u8 as f32
}

/// (TSD slot, TST slot) present. Reachability isn't checked; corners are implied by the rows being full.
/// TSD: nose-down T at (x, y) under an overhang completes rows y and y+1.
/// TST: upright T at (x, y) with its nose in a side notch completes rows y-1..=y+1.
fn tslots(b: &Board) -> (bool, bool) {
    const FULL: u16 = 0x3FF;
    // Only `bits` are missing from row y.
    let only = |y: i8, bits: u16| b.rows[y as usize] | bits == FULL && b.rows[y as usize] & bits == 0;
    let (mut tsd, mut tst) = (false, false);
    for x in 1..WIDTH - 1 {
        let (t, stem) = (0b111u16 << (x - 1), 1u16 << x);
        for y in 1..HEIGHT - 1 {
            tsd |= only(y, t) && only(y + 1, stem) && (b.occupied(x - 1, y - 1) || b.occupied(x + 1, y - 1));
            tst |= only(y - 1, stem) && only(y + 1, stem) && [x - 1, x + 1].iter().any(|&n| only(y, stem | 1 << n));
        }
    }
    (tsd, tst)
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
        assert_eq!(tslots(&board), (true, false));
        let root = Root { board, current: Piece::T, hold: None, can_hold: true, queue: &[Piece::I, Piece::O], b2b: false, combo: -1 };
        let p = suggest(&root, &Rules::monochrome_rush(1), &Config::default()).unwrap();
        assert_eq!((p.piece, p.spin), (Piece::T, Spin::Full));
    }

    /// Needs the SRS triple-twist kick (0, +2) under a two-block overhang.
    #[test]
    fn takes_the_tst() {
        let mut board = Board::default();
        board.rows[39] = 0x3FF & !(1 << 2);
        board.rows[38] = 0x3FF & !0b110;
        board.rows[37] = 0x3FF & !(1 << 2);
        board.rows[35] = 0b1100;
        assert_eq!(tslots(&board), (false, true));
        for level in [1, 11] {
            let root = Root { board, current: Piece::T, hold: None, can_hold: true, queue: &[Piece::I, Piece::O], b2b: false, combo: -1 };
            let p = suggest(&root, &Rules::monochrome_rush(level), &Config::default()).unwrap();
            assert_eq!((p.piece, p.spin), (Piece::T, Spin::Full), "level {level}");
            assert_eq!(board.clone().lock(p.piece, p.x, p.y, p.rot), 3, "level {level}");
        }
    }
}
