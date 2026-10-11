//! Beam search over the visible queue (with hold). Nodes are scored by MonochromeRush
//! points earned along the way plus a hand-weighted board evaluation at the leaf.

use std::time::{Duration, Instant};

use crate::board::{Board, HEIGHT, WIDTH};
use crate::game::{Game, Rules, attack, points};
use crate::live::GARBAGE_PER_LOCK;
use crate::movegen::{Placement, Spin, find_placements, find_placements_fast};
use crate::piece::Piece;
use crate::net;
use crate::setups;

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
    /// Reward for building a known opener shape (`setups`), banked as progress is made.
    pub setup: f32,
    /// Per incoming garbage line still uncancelled at the end of a line of play.
    pub incoming: f32,
    /// Charged when a plain single/double/triple ends an active back-to-back chain (a policy
    /// of keeping B2B unless it's really worth breaking).
    pub b2b_break: f32,
}

impl Default for Weights {
    fn default() -> Self {
        Weights { holes: -5.41, bumpiness: -0.76, height: 0.19, danger: -2.42, well: 1.03, tsd: 6.81, tst: 3.45, b2b: 2.82, setup: 0.0, incoming: 0.0, b2b_break: 0.0 }
    }
}

impl Weights {
    /// For Stackfall versus: hand-picked. Not the solo defaults, which learned to *reward* height
    /// (MonochromeRush multiplies score by level) and lose versus games. A width-16 tune
    /// (tune_vs, 25 generations: holes -3.76, bumpiness -4.78, height -5.81, danger -2.53,
    /// well 0.04, tsd 3.53, tst 4.46, b2b 0.74, setup 5.38, incoming -0.90) beat these 65.6% +/- 4.8%
    /// at width 16 but only 46.9% +/- 6.2% at level 10's width 64, so it isn't used.
    pub fn versus() -> Self {
        Weights { holes: -4.0, bumpiness: -0.4, height: -0.05, danger: -1.0, well: 0.7, tsd: 4.0, tst: 6.0, b2b: 1.0, setup: 4.0, incoming: -1.0, b2b_break: 0.0 }
    }

    pub fn to_vec(&self) -> Vec<f32> {
        vec![self.holes, self.bumpiness, self.height, self.danger, self.well, self.tsd, self.tst, self.b2b, self.setup, self.incoming, self.b2b_break]
    }

    pub fn from_slice(v: &[f32]) -> Self {
        Weights { holes: v[0], bumpiness: v[1], height: v[2], danger: v[3], well: v[4], tsd: v[5], tst: v[6], b2b: v[7], setup: v[8], incoming: v[9], b2b_break: v[10] }
    }
}

/// What a personality plays for beyond winning.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Specialty {
    #[default]
    Normal,
    /// Builds a 4-wide well with tall walls and chains single clears into long combos: `step` per
    /// combo step on top of the attack table, `depth` per row of well depth (capped).
    Ren { step: f32, depth: f32 },
    /// Keeps the stack low, flat and fillable, and goes for perfect clears: `bonus` on top of a
    /// PC's attack, `high` per row of stack above 4, `region` per empty region of the PC area
    /// whose size isn't a multiple of 4 (no piece mix can fill it).
    PerfectClear { bonus: f32, high: f32, region: f32 },
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
    /// Plan around incoming garbage (versus). Beginners don't.
    pub sees_garbage: bool,
    /// Look one piece past the visible queue, averaging over what the 7-bag can still deal.
    pub speculate: bool,
    /// Add the learned value net's win prediction (`net`), times `net_weight`, to the leaf score,
    /// when a trained net is embedded.
    pub use_net: bool,
    pub net_weight: f32,
    /// Trick style: among first moves whose line scores within this much of the best, play the one
    /// with the flashiest, least-recently-used tricks (0 = always the best line). Measured to change
    /// little on its own: flashy lines get pruned from the beam before the final pick.
    pub trick_margin: f32,
    /// Trick style inside the search: each trick a line performs earns this times its flair
    /// (halved if done recently, nothing for a recently used setup), so flashy lines survive pruning.
    pub trick_bonus: f32,
    /// Setups this bot completed or fired recently (ids from `setups::best_stage`), and trick kinds
    /// (`trick` bits) it did recently: the trick style prefers something else, for variety.
    pub recent_stages: Vec<u32>,
    pub recent_tricks: u32,
    pub specialty: Specialty,
    /// Keep only the best of beam nodes that reached the same state by different move orders,
    /// so the beam holds `width` distinct futures. Off: it measured 42.2% +/- 6.2% vs not merging
    /// (level 10, 64 matches), i.e. no gain.
    pub dedupe: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { width: 64, depth: 6, weights: Weights::default(), objective: Objective::Points, noise: 0.0, salt: 0, sees_garbage: true, speculate: false, use_net: false, net_weight: 20.0, dedupe: false, trick_margin: 0.0, trick_bonus: 0.0, recent_stages: vec![], recent_tricks: 0, specialty: Specialty::Normal }
    }
}

/// Titan's strength, showman style: varied tricks and named setups when they're nearly free.
pub const TRICKSTER: u32 = 11;
/// Titan's strength, keeps the back-to-back chain alive whenever it reasonably can.
pub const CHAINLORD: u32 = 12;
/// Titan's strength, combo specialist (`Specialty::Ren`).
pub const RENEGADE: u32 = 13;
/// Titan's strength, perfect-clear specialist (`Specialty::PerfectClear`).
pub const PERFECTIONIST: u32 = 14;

/// Trick bits recorded along a line of play.
pub mod trick {
    pub const TSS: u32 = 1;
    pub const TSD: u32 = 2;
    pub const TST: u32 = 4;
    pub const MINI: u32 = 8;
    pub const QUAD: u32 = 16;
    pub const PC: u32 = 32;
    pub const B2B: u32 = 64;
    pub const COMBO: u32 = 128;
    pub const SETUP_DONE: u32 = 256;
    pub const SETUP_FIRED: u32 = 512;
    /// How flashy each trick is, for the trick style.
    pub const FLAIR: [(u32, f32); 10] = [
        (TST, 6.0), (PC, 6.0), (SETUP_FIRED, 6.0), (SETUP_DONE, 4.0), (TSD, 3.0),
        (QUAD, 2.0), (TSS, 2.0), (MINI, 1.0), (B2B, 1.0), (COMBO, 1.0),
    ];
}

impl Config {
    /// Versus bot strength, 1 (beginner) ..= 10 (full search, no mistakes). Skills unlock with
    /// level: low levels don't plan quad wells or T-spins at all and misjudge a lot.
    /// `objective` picks the weights: `Attack` (versus) or `Points` (solo score modes).
    /// Levels above 10 are named personalities with Titan's strength: `TRICKSTER`, `CHAINLORD`.
    pub fn difficulty(level: u32, objective: Objective) -> Self {
        let i = level.clamp(1, 10) as usize - 1;
        let spin_skill = [0.0, 0.0, 0.0, 0.2, 0.4, 0.6, 0.8, 1.0, 1.0, 1.0][i];
        let well_skill = [0.0, 0.0, 0.3, 0.6, 0.8, 1.0, 1.0, 1.0, 1.0, 1.0][i];
        // Named openers are expert knowledge: only the top three levels know them.
        let setup_skill = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.6, 1.0, 1.0][i];
        let base = match objective {
            Objective::Attack => Weights::versus(),
            Objective::Points => Weights::default(),
        };
        let cfg = Config {
            // Level 10 at 256 beat Cold Clear 2 (100 ms/move) 23-9; at 64 it went 15-17 (32 matches each).
            width: [1, 1, 1, 2, 4, 8, 16, 24, 48, 256][i],
            depth: [1, 1, 1, 2, 2, 3, 4, 5, 6, 6][i],
            noise: [14.0, 10.0, 7.0, 4.5, 3.0, 1.8, 1.0, 0.4, 0.15, 0.0][i],
            objective,
            weights: Weights {
                tsd: base.tsd * spin_skill,
                tst: base.tst * spin_skill,
                b2b: base.b2b * spin_skill,
                b2b_break: base.b2b_break * spin_skill,
                well: base.well * well_skill,
                setup: base.setup * setup_skill,
                ..base
            },
            salt: 0,
            sees_garbage: level >= 5,
            speculate: level >= 9,
            // Verified for level 10 only: 73.2% +/- 2.8% vs the hand eval over 256 fresh matches
            // (net from 5 self-play rounds, nets/value.bin = nets/gen6w.bin).
            use_net: level >= 10 && objective == Objective::Attack,
            net_weight: 20.0,
            dedupe: false,
            trick_margin: 0.0,
            trick_bonus: 0.0,
            recent_stages: vec![],
            recent_tricks: 0,
            specialty: Specialty::Normal,
        };
        // Named personalities at Titan's strength.
        match level {
            // Style settings: the strongest that kept roughly even with Titan (vs, 16 matches at beam 64;
            // stronger ones lost more without more style).
            // Bonus 10 / setups x3: 62.5% vs Titan, 1.1 named setups fired a match (Titan ~0.6).
            TRICKSTER => Config { trick_bonus: 10.0, weights: Weights { setup: cfg.weights.setup * 3.0, ..cfg.weights }, ..cfg },
            // B2B x4, -5 per break: 50.0% vs Titan, longest B2B chain 5.1 (Titan ~3.5-4), breaks 8 a match (Titan ~20).
            CHAINLORD => Config { weights: Weights { b2b: cfg.weights.b2b * 4.0, b2b_break: -5.0, ..cfg.weights }, ..cfg },
            // Combo play: T-spin shapes and setups only get in the way of a clean well. 50.0% vs Titan,
            // longest combo 9.2 a match (Titan ~6).
            RENEGADE => Config { specialty: Specialty::Ren { step: 2.0, depth: 5.0 }, weights: Weights { tsd: 0.0, tst: 0.0, setup: 0.0, well: 0.0, bumpiness: -0.25, ..cfg.weights }, ..cfg },
            // Perfect clears: no T-spin shapes, wells or setups; height costs more. 56.2% vs Titan,
            // 2.2 perfect clears a match (Titan ~0.7).
            PERFECTIONIST => Config { specialty: Specialty::PerfectClear { bonus: 30.0, high: 3.0, region: 4.0 }, weights: Weights { tsd: 0.0, tst: 0.0, setup: 0.0, well: 0.0, b2b: 0.0, height: -0.3, ..cfg.weights }, ..cfg },
            _ => cfg,
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
    /// Incoming garbage lines not yet on the board (versus).
    pub pending: u32,
    /// Piece types still to come from the current 7-bag after the visible queue (bit = `Piece as u8`),
    /// which any player can count. 0 = unknown: no speculation.
    pub bag: u8,
}

#[derive(Clone, Copy)]
struct Node {
    board: Board,
    hold: Option<Piece>,
    /// Index into [current, queue..] of the next piece to play.
    next: usize,
    b2b: bool,
    combo: i32,
    /// Incoming garbage still pending along this line.
    pending: u32,
    reward: f32,
    value: f32,
    /// Opener-shape progress (`setups::progress`), tracked when setups or the trick style are on.
    progress: f32,
    /// The setup that progress belongs to (`NO_STAGE` if none), and tricks done along this line.
    stage: u32,
    tricks: u32,
    /// Index of the root placement this line started with.
    first: usize,
    /// The placement that made this node (piece, x, y, rot) and its parent's index in the layer
    /// before, to read back the chosen line.
    mv: (Piece, i8, i8, u8),
    parent: u32,
}

/// The line the bot chose: the setup it's working on (if any) and the tricks it plans.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub stage: Option<u32>,
    pub tricks: u32,
    /// The planned placements (piece, x, y, rot), first = the move played, while they still sit
    /// where they'd lock on today's board: up to and including the first that clears lines or
    /// lets garbage rise.
    pub line: Vec<(Piece, i8, i8, u8)>,
    /// Positions scored for this move.
    pub nodes: u32,
}

/// `history[k]` is the layer depth-k children point into (`history[0]` = the root).
fn plan_of(n: &Node, history: &[Vec<Node>], nodes: u32) -> Plan {
    let mut path = vec![*n];
    for layer in history.iter().rev() {
        path.push(layer[path.last().unwrap().parent as usize]);
    }
    path.reverse();
    let mut line = vec![];
    for w in path.windows(2) {
        let (piece, x, y, rot) = w[1].mv;
        line.push(w[1].mv);
        let mut b = w[0].board;
        if b.lock(piece, x, y, rot) > 0 || b != w[1].board {
            break;
        }
    }
    Plan { stage: (n.stage != NO_STAGE && n.progress > 0.0).then_some(n.stage), tricks: n.tricks, line, nodes }
}

pub fn suggest(root: &Root, rules: &Rules, cfg: &Config) -> Option<Placement> {
    suggest_with_plan(root, rules, cfg).map(|(p, _)| p)
}

/// `suggest`, plus what the chosen line is going for.
pub fn suggest_with_plan(root: &Root, rules: &Rules, cfg: &Config) -> Option<(Placement, Plan)> {
    let seq: Vec<Piece> = std::iter::once(root.current).chain(root.queue.iter().copied()).collect();
    let mut first_moves = vec![];
    let mut layer = vec![Node {
        board: root.board,
        hold: root.hold,
        next: 0,
        b2b: root.b2b,
        combo: root.combo,
        pending: if cfg.sees_garbage { root.pending } else { 0 },
        reward: 0.0,
        value: 0.0,
        progress: 0.0,
        stage: NO_STAGE,
        tricks: 0,
        first: 0,
        mv: (root.current, 0, 0, 0),
        parent: 0,
    }];
    if tracks_setups(cfg) {
        let (p, stage) = setups::best_stage(&root.board, available(&seq, 0, root.hold));
        (layer[0].progress, layer[0].stage) = (p, stage.unwrap_or(NO_STAGE));
    }
    let mut best = None;
    let (mut history, mut nodes) = (vec![], 0);

    for depth in 0..cfg.depth {
        let mut children = vec![];
        for (i, node) in layer.iter().enumerate() {
            for (piece, hold, next) in options(node, &seq, depth > 0 || root.can_hold) {
                let placements = if depth == 0 { find_placements(&node.board, piece, rules) } else { find_placements_fast(&node.board, piece, rules) };
                for p in placements {
                    let Some(mut child) = play(node, &p, hold, next, &seq, rules, cfg) else { continue };
                    child.parent = i as u32;
                    if depth == 0 {
                        first_moves.push(p);
                        child.first = first_moves.len() - 1;
                    }
                    children.push(child);
                }
            }
        }
        if children.is_empty() {
            break;
        }
        nodes += children.len() as u32;
        // ponytail: full sort per layer; select_nth_unstable if profiling says so.
        children.sort_unstable_by(|a, b| b.value.total_cmp(&a.value));
        if cfg.dedupe {
            let mut seen = std::collections::HashSet::with_capacity(cfg.width * 2);
            children.retain(|c| seen.len() < cfg.width && seen.insert(state_key(c)));
        }
        children.truncate(cfg.width);
        best = Some(children[0].first);
        history.push(std::mem::replace(&mut layer, children));
    }

    // One piece past the visible queue: unknown, but it is one of the pieces left in the bag, so
    // score each line by its average over those (playing that piece or the held one).
    if cfg.speculate && root.bag != 0 && !first_moves.is_empty() {
        // Only the best few lines: the pick essentially always comes from them, and speculating
        // the whole beam was ~40% of all thinking time.
        let mut spec: Vec<Node> = layer.iter().take(SPECULATE_TOP).copied().collect();
        for node in spec.iter_mut().filter(|n| n.next >= seq.len()) {
            let possible: Vec<Piece> = Piece::ALL.into_iter().filter(|&p| root.bag & 1 << p as u8 != 0).collect();
            let mut total = 0.0;
            // Placements per piece type on this board: the held piece's are needed for every guess.
            let mut moves: [Option<Vec<Placement>>; 7] = Default::default();
            for &piece in &possible {
                let mut choices = vec![(piece, node.hold)];
                if let Some(h) = node.hold.filter(|&h| h != piece) {
                    choices.push((h, Some(piece)));
                }
                let mut best_here = DEATH;
                for (play_piece, hold) in choices {
                    for p in moves[play_piece as usize].get_or_insert_with(|| find_placements_fast(&node.board, play_piece, rules)).iter() {
                        nodes += 1;
                        if let Some(c) = play(node, p, hold, node.next + 1, &seq, rules, cfg) {
                            best_here = best_here.max(c.value);
                        }
                    }
                }
                total += best_here;
            }
            node.value = total / possible.len() as f32;
        }
        let chosen = choose(&spec, cfg);
        return chosen.map(|n| (first_moves.swap_remove(n.first), plan_of(&n, &history, nodes)));
    }
    best?; // no legal placement at all
    choose(&layer, cfg).map(|n| (first_moves.swap_remove(n.first), plan_of(&n, &history, nodes)))
}

/// How many of the final beam's best lines get the extra speculated piece.
const SPECULATE_TOP: usize = 16;

const NO_STAGE: u32 = u32::MAX;

fn tracks_setups(cfg: &Config) -> bool {
    cfg.weights.setup != 0.0 || cfg.trick_margin > 0.0 || cfg.trick_bonus > 0.0
}

/// The best final line, or with the trick style on: among first moves whose best line scores
/// within `trick_margin` of the top, the line with the most flair (recently used setups and trick
/// kinds count less), ties broken by score.
fn choose(nodes: &[Node], cfg: &Config) -> Option<Node> {
    let top = *nodes.iter().max_by(|a, b| a.value.total_cmp(&b.value))?;
    if cfg.trick_margin <= 0.0 {
        return Some(top);
    }
    nodes
        .iter()
        .filter(|n| n.value >= top.value - cfg.trick_margin)
        .max_by(|a, b| flair(a.tricks, a.stage, cfg).total_cmp(&flair(b.tricks, b.stage, cfg)).then(a.value.total_cmp(&b.value)))
        .copied()
}

/// How flashy a set of tricks is: rarer tricks score more; tricks this bot did recently score
/// half, and a setup it fired recently scores nothing (variety).
fn flair(tricks: u32, stage: u32, cfg: &Config) -> f32 {
    trick::FLAIR
        .iter()
        .filter(|&&(bit, _)| tricks & bit != 0)
        .map(|&(bit, f)| {
            let setup = bit & (trick::SETUP_DONE | trick::SETUP_FIRED) != 0;
            if setup && cfg.recent_stages.contains(&stage) {
                0.0
            } else if cfg.recent_tricks & bit != 0 {
                f / 2.0
            } else {
                f
            }
        })
        .sum()
}

/// Value given to a line of play that tops out on a speculated piece.
const DEATH: f32 = -1000.0;

/// The node after locking placement `p` (with `hold` / `next` as given by `options`), scored.
/// None if it tops out.
fn play(node: &Node, p: &Placement, hold: Option<Piece>, next: usize, seq: &[Piece], rules: &Rules, cfg: &Config) -> Option<Node> {
    let mut board = node.board;
    let lines = board.lock(p.piece, p.x, p.y, p.rot);
    if tops_out(&board, rules) {
        return None;
    }
    let (mut b2b, mut combo) = (node.b2b, node.combo);
    let mut pending = node.pending;
    let mut gain = match cfg.objective {
        Objective::Points => points(lines, p.spin, &mut b2b, &mut combo) / 100.0,
        Objective::Attack => {
            let pc = lines > 0 && board.is_empty();
            let sent = attack(lines, p.spin, &mut b2b, &mut combo, pc);
            pending -= sent.min(pending); // attack cancels incoming garbage first
            2.0 * sent as f32
        }
    };
    // Like the game: a lock that clears nothing lets incoming garbage rise. The real hole column
    // is random; assume it lands under the lowest column.
    if lines == 0 && pending > 0 {
        let rise = pending.min(GARBAGE_PER_LOCK);
        pending -= rise;
        if !board.push_garbage(rise as usize, lowest_column(&board)) || tops_out(&board, rules) {
            return None;
        }
    }
    // Bank setup progress as it's made, counting only stages the visible pieces can still
    // finish. Losing progress gives it back, unless this placement is the T-spin that fires the
    // setup. (Filling the slot with another piece that happens to clear a line must not keep it.)
    let (mut progress, mut stage) = (node.progress, node.stage);
    let fired = lines > 0 && p.spin == Spin::Full;
    if tracks_setups(cfg) {
        let (now, id) = setups::best_stage(&board, available(seq, next, hold));
        progress = now;
        if let Some(id) = id {
            stage = id;
        }
        let delta = progress - node.progress;
        if delta > 0.0 || !fired {
            gain += cfg.weights.setup * delta;
        }
    }
    let mut tricks = node.tricks;
    if lines > 0 {
        tricks |= match (p.spin, lines) {
            (Spin::Full, 1) => trick::TSS,
            (Spin::Full, 2) => trick::TSD,
            (Spin::Full, _) => trick::TST,
            (Spin::Mini, _) => trick::MINI,
            (Spin::None, 4) => trick::QUAD,
            _ => 0,
        };
        if board.is_empty() {
            tricks |= trick::PC;
        }
        if node.b2b && b2b {
            tricks |= trick::B2B;
        }
        if combo >= 3 {
            tricks |= trick::COMBO;
        }
    }
    if progress == 1.0 && node.progress < 1.0 {
        tricks |= trick::SETUP_DONE;
    }
    if fired && node.progress == 1.0 {
        tricks |= trick::SETUP_FIRED;
    }
    if cfg.trick_bonus > 0.0 {
        gain += cfg.trick_bonus * flair(tricks & !node.tricks, stage, cfg);
    }
    if node.b2b && !b2b && lines > 0 {
        gain += cfg.weights.b2b_break;
    }
    match cfg.specialty {
        Specialty::Normal => {}
        // Each further step of a running combo is worth more than the attack table says.
        Specialty::Ren { step, .. } if lines > 0 => gain += step * combo.min(12) as f32,
        // Dropping a running combo costs what keeping it would have earned.
        Specialty::Ren { step, .. } => gain -= step * node.combo.clamp(0, 12) as f32,
        Specialty::PerfectClear { bonus, .. } if lines > 0 && board.is_empty() => gain += bonus,
        Specialty::PerfectClear { .. } => {}
    }
    let reward = node.reward + gain;
    let slot = tslot(&board);
    let mut leaf = eval_with_slot(&board, b2b, &cfg.weights, slot) + cfg.weights.incoming * pending as f32;
    leaf += match cfg.specialty {
        Specialty::Normal => 0.0,
        Specialty::Ren { depth, .. } => depth * ren_depth(&board) as f32,
        Specialty::PerfectClear { high, region, .. } => pc_shape(&board, high, region),
    };
    if let Some(n) = net::value_net().filter(|_| cfg.use_net) {
        let lines = slot.map(|s| s.3);
        leaf += cfg.net_weight * n.eval(&net::features(&board, b2b, combo, pending, lines == Some(2), lines == Some(3)));
    }
    let mut value = reward + leaf;
    if cfg.noise > 0.0 {
        value += cfg.noise * jitter(&board, cfg.salt);
    }
    Some(Node { board, hold, next, b2b, combo, pending, reward, value, progress, stage, tricks, first: node.first, mv: (p.piece, p.x, p.y, p.rot), parent: 0 })
}

/// Identifies a search state regardless of how it was reached (FNV-1a over everything that
/// affects the future: board, hold, queue position, B2B, combo, pending garbage).
fn state_key(n: &Node) -> u64 {
    let extra = [n.hold.map_or(7, |p| p as u64), n.next as u64, n.b2b as u64, n.combo as u64 & 0xFF, n.pending as u64];
    n.board.rows.iter().map(|&r| r as u64).chain(extra).fold(0xCBF2_9CE4_8422_2325, |h, v| (h ^ v).wrapping_mul(0x100_0000_01B3))
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

/// Pieces still to come in the visible sequence from `next`, plus the held piece.
fn available(seq: &[Piece], next: usize, hold: Option<Piece>) -> u8 {
    setups::piece_mask(seq.get(next..).unwrap_or(&[]).iter().copied().chain(hold))
}

/// Deterministic pseudo-random 0..1 from the board, so noisy play stays reproducible.
fn jitter(board: &Board, salt: u64) -> f32 {
    let h = board.rows.iter().fold(0xCBF2_9CE4_8422_2325u64 ^ salt, |h, &r| (h ^ r as u64).wrapping_mul(0x100_0000_01B3));
    (h >> 40) as f32 / (1u64 << 24) as f32
}

fn lowest_column(b: &Board) -> i8 {
    (0..WIDTH).max_by_key(|&x| (0..HEIGHT).find(|&y| b.occupied(x, y)).unwrap_or(HEIGHT)).unwrap()
}

/// Anything in the spawn area means the next piece can't appear.
fn tops_out(board: &Board, rules: &Rules) -> bool {
    let (x, y) = rules.spawn;
    let mask = 0b1111 << (x - 1);
    (y - 1..=y).any(|r| board.rows[r as usize] & mask != 0)
}

/// The hand-weighted leaf score a versus bot uses, for self-play data (the net learns its error).
pub fn hand_eval(board: &Board, b2b: bool, pending: u32, w: &Weights) -> f32 {
    eval(board, b2b, w) + w.incoming * pending as f32
}

fn eval(board: &Board, b2b: bool, w: &Weights) -> f32 {
    eval_with_slot(board, b2b, w, tslot(board))
}

/// `eval` with the board's T-slot already found (the search also needs it for the value net).
fn eval_with_slot(board: &Board, b2b: bool, w: &Weights, slot: Option<(i8, i8, u8, u32)>) -> f32 {
    // Cutout (as in Cold Clear): if a T-spin slot is ready, score the board as if the T-spin
    // happened. Otherwise the slot's covered cells count as holes and setups get punished.
    let mut cut = *board;
    if let Some((x, y, rot, _)) = slot {
        cut.lock(Piece::T, x, y, rot);
    }
    let b = &cut;

    // One pass top-down: a column's height is set by its first filled cell; every empty cell
    // in a column that already has a filled cell above it is a hole.
    let mut h = [0i32; WIDTH as usize];
    let (mut covered, mut holes) = (0u16, 0usize);
    for (y, &row) in b.rows.iter().enumerate() {
        holes += (covered & !row).count_ones() as usize;
        let mut new = row & !covered;
        while new != 0 {
            h[new.trailing_zeros() as usize] = HEIGHT as i32 - y as i32;
            new &= new - 1;
        }
        covered |= row;
    }
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

/// Ren well depth counted at most this deep.
const REN_MAX_DEPTH: i32 = 10;

fn column_heights(b: &Board) -> [i32; WIDTH as usize] {
    let mut h = [0i32; WIDTH as usize];
    for x in 0..WIDTH as usize {
        h[x] = (0..HEIGHT as usize).find(|&y| b.rows[y] >> x & 1 != 0).map_or(0, |y| HEIGHT as i32 - y as i32);
    }
    h
}

/// How deep the best 4-wide well (columns 0-3, 3-6 or 6-9) sits below the walls around it.
fn ren_depth(b: &Board) -> i32 {
    let h = column_heights(b);
    let depth = [0, 3, 6]
        .into_iter()
        .map(|k| {
            let inner = h[k..k + 4].iter().max().unwrap();
            let wall = (0..WIDTH as usize).filter(|x| !(k..k + 4).contains(x)).map(|x| h[x]).min().unwrap();
            wall - inner
        })
        .max()
        .unwrap();
    depth.clamp(0, REN_MAX_DEPTH)
}

/// How ready the board is for a perfect clear of the bottom 2 or 4 rows: stack above that costs
/// per row; otherwise each run of columns between columns already full to the top of the clear
/// must have a multiple of 4 empty cells (assumes no holes; holes are punished anyway).
fn pc_shape(b: &Board, high: f32, region: f32) -> f32 {
    let h = column_heights(b);
    let top = *h.iter().max().unwrap();
    if top > 4 {
        return -high * (top - 4) as f32;
    }
    let bad = |rows: i32| {
        let (mut bad, mut run) = (0, 0);
        for &c in h.iter().chain(std::iter::once(&rows)) {
            if c >= rows {
                bad += (run % 4 != 0) as i32;
                run = 0;
            } else {
                run += rows - c;
            }
        }
        bad
    };
    let best = if top <= 2 { bad(2).min(bad(4)) } else { bad(4) };
    -region * best as f32
}

/// Lines the best ready T-spin slot would clear (2 = TSD, 3 = TST), for net features.
pub fn tslot_kind(b: &Board) -> Option<u32> {
    tslot(b).map(|s| s.3)
}

/// The best ready T-spin slot as (x, y, rot, lines): a TST if any, else a TSD.
/// Corners are implied by the rows being full; full reachability isn't checked.
/// TSD: nose-down T at (x, y) under an overhang completes rows y and y+1.
/// TST: upright T at (x, y), nose in a side notch, completes rows y-1..=y+1; needs a roof over
/// its column, since the T only gets in through the SRS triple-twist kick.
pub(crate) fn tslot(b: &Board) -> Option<(i8, i8, u8, u32)> {
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
            pending: 0,
            bag: game.bag_mask(),
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

    /// Ren: a 4-wide well between tall walls scores by its depth; perfect clear: a low board
    /// whose empty area splits into pieces of 4 cells is fine, an odd region or a tall stack isn't.
    #[test]
    fn specialty_shapes() {
        let mut ren = Board::default();
        for y in 32..40 {
            ren.rows[y] = 0x3FF & !(0b1111 << 3); // columns 3-6 open, 8 rows of wall
        }
        assert_eq!(ren_depth(&ren), 8);
        assert_eq!(ren_depth(&Board::default()), 0);

        let mut pc = Board::default();
        assert_eq!(pc_shape(&pc, 3.0, 4.0), 0.0);
        pc.rows[39] = 0b1111_1111; // 8 cells: 12 empty in a 2-row clear, 32 in a 4-row one
        assert_eq!(pc_shape(&pc, 3.0, 4.0), 0.0);
        pc.rows[39] = 0b111; // 17 empty in a 2-row clear, 37 in a 4-row one: neither divides by 4
        assert_eq!(pc_shape(&pc, 3.0, 4.0), -4.0);
        let mut tall = Board::default();
        for y in 34..40 {
            tall.rows[y] = 1;
        }
        assert_eq!(pc_shape(&tall, 3.0, 4.0), -6.0);
    }

    /// Trickster prefers a flashier line when it costs little; with the trick style off the bot
    /// takes the best-scoring line. Both still find the T-spin double when it's on offer.
    #[test]
    fn trickster_and_titan_both_take_the_tsd() {
        let mut board = Board::default();
        board.rows[39] = 0x3FF & !(1 << 1);
        board.rows[38] = 0x3FF & !0b111;
        board.rows[37] = 1;
        for level in [10, TRICKSTER, CHAINLORD] {
            let root = Root { board, current: Piece::T, hold: None, can_hold: true, queue: &[Piece::I, Piece::O], b2b: false, combo: -1, pending: 0, bag: 0 };
            let (p, plan) = suggest_with_plan(&root, &Rules::versus(), &Config::difficulty(level, Objective::Attack)).unwrap();
            assert_eq!((p.piece, p.spin), (Piece::T, Spin::Full), "level {level}");
            assert!(plan.tricks & trick::TSD != 0, "level {level}: plan {plan:?}");
            // The line starts with the move played and stops at the clear (the board shifts).
            assert_eq!(plan.line, vec![(p.piece, p.x, p.y, p.rot)]);
        }
        // On an empty board nothing clears for a while, so the line runs several pieces deep.
        let queue = [Piece::I, Piece::O, Piece::S, Piece::Z, Piece::L];
        let root = Root { board: Board::default(), current: Piece::J, hold: None, can_hold: true, queue: &queue, b2b: false, combo: -1, pending: 0, bag: 0 };
        let (p, plan) = suggest_with_plan(&root, &Rules::versus(), &Config::difficulty(10, Objective::Attack)).unwrap();
        assert_eq!(plan.line[0], (p.piece, p.x, p.y, p.rot));
        assert!(plan.line.len() >= 3 && plan.nodes > 1000, "{plan:?}");
    }

    #[test]
    fn takes_the_tsd() {
        let mut board = Board::default();
        board.rows[39] = 0x3FF & !(1 << 1);
        board.rows[38] = 0x3FF & !0b111;
        board.rows[37] = 1;
        assert_eq!(tslot(&board).map(|s| s.3), Some(2));
        let root = Root { board, current: Piece::T, hold: None, can_hold: true, queue: &[Piece::I, Piece::O], b2b: false, combo: -1, pending: 0, bag: 0 };
        let p = suggest(&root, &Rules::monochrome_rush(1), &Config::default()).unwrap();
        assert_eq!((p.piece, p.spin), (Piece::T, Spin::Full));
    }

    /// Versus bots keep their stack clearly lower than the score-tuned weights would (those learned
    /// to reward height, which loses versus games). Relative, so it doesn't break whenever the
    /// search changes a little.
    #[test]
    fn versus_levels_stack_lower_than_score_weights() {
        let avg_height = |cfg: &Config| {
            let (mut sum, mut n) = (0, 0);
            for seed in 0..2 {
                let mut game = Game::new(Rules::versus(), seed);
                for _ in 0..200 {
                    let queue: Vec<_> = game.queue.iter().copied().collect();
                    let root = Root { board: game.board, current: game.current, hold: game.hold, can_hold: true, queue: &queue, b2b: game.b2b, combo: game.combo, pending: 0, bag: game.bag_mask() };
                    game.play(&suggest(&root, &game.rules, cfg).unwrap());
                    assert!(!game.over);
                    sum += 40 - game.board.rows.iter().position(|&r| r != 0).unwrap_or(40);
                    n += 1;
                }
            }
            sum as f32 / n as f32
        };
        for level in [4, 7, 10] {
            let versus = Config::difficulty(level, Objective::Attack);
            let score_weights = Config { weights: Weights { setup: versus.weights.setup, ..Weights::default() }, ..versus.clone() };
            let (v, s) = (avg_height(&versus), avg_height(&score_weights));
            assert!(v < s, "level {level}: versus weights {v:.1} rows vs score weights {s:.1}");
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
            let root = Root { board, current: Piece::T, hold: None, can_hold: true, queue: &[Piece::I, Piece::O], b2b: false, combo: -1, pending: 0, bag: 0 };
            let p = suggest(&root, &Rules::monochrome_rush(level), &Config::default()).unwrap();
            assert_eq!((p.piece, p.spin), (Piece::T, Spin::Full), "level {level}");
            assert_eq!(board.clone().lock(p.piece, p.x, p.y, p.rot), 3, "level {level}");
        }
    }
}
