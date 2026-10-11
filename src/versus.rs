//! Headless versus: two bots on the real-time engine (garbage, cancelling, attack table), placing
//! one piece each per turn (equal speed). Attacks from a turn land after both have moved, so
//! neither side gets a first-mover edge. For tuning and benchmarking on win rate.

use crate::bot::{Config, Root, clear_kind, suggest_with_plan, trick};
use crate::game::Rules;
use crate::live::{Event, Live};
use crate::movegen::{Input, Placement, Spin};

#[derive(Debug, Clone)]
pub struct MatchResult {
    /// None = draw (both survived `max_pieces` and sent the same).
    pub winner: Option<usize>,
    pub pieces: u32,
    pub sent: [u32; 2],
    pub style: [Style; 2],
    /// Per side, setup stages (`setups::setup_name` ids) whose T-spin it fired (repeats included),
    /// and the distinct stages it steered toward at some point.
    pub fires: [Vec<u32>; 2],
    pub attempts: [Vec<u32>; 2],
}

/// What each side actually did, for comparing personalities.
#[derive(Debug, Clone, Copy, Default)]
pub struct Style {
    /// Distinct trick kinds performed (`trick` bits OR'd).
    pub tricks: u32,
    /// Distinct named setups whose T-spin it fired.
    pub setups: u32,
    pub b2b_clears: u32,
    pub b2b_breaks: u32,
    /// How many of the rarer, flashier clears it made.
    pub tsd: u32,
    pub tst: u32,
    pub pc: u32,
    /// T-spins that fired a named setup (repeats included).
    pub setup_fires: u32,
    /// Longest combo (1 = two clears in a row, as the game counts it).
    pub max_combo: u32,
    /// Longest run of back-to-back-eligible clears (quads, T-spins) with no plain clear between.
    pub max_b2b: u32,
    /// Clears that weren't a trick, or repeated the previous trick's kind (what `tricks_only` avoids);
    /// `repeats` counts the second kind alone.
    pub fouls: u32,
    pub repeats: u32,
    chain: u32,
}

/// Per-side memory the trick style uses to avoid repeating itself.
#[derive(Default, Clone)]
struct Memory {
    recent_tricks: std::collections::VecDeque<u32>,
    recent_stages: std::collections::VecDeque<u32>,
    fired: Vec<u32>,
    fires: Vec<u32>,
    attempts: Vec<u32>,
    last_trick: u32,
}

/// Anything that can choose moves in a versus match: our bot, or an external one (Cold Clear over
/// the Tetris Bot Protocol, see examples/vs_coldclear.rs).
pub trait Brain {
    /// A placement for the active piece or its hold swap (with the inputs that reach it), and the
    /// named setup it is working on, if any. None = no legal move (the game hard drops).
    /// `last_trick`: kind of this side's last clearing trick (`trick` bit, 0 = none).
    fn choose(&mut self, g: &Live, recent_tricks: u32, recent_stages: &[u32], last_trick: u32) -> Option<(Placement, Option<u32>)>;
}

/// Our bot as a `Brain`.
pub struct BotBrain(pub Config);

impl Brain for BotBrain {
    fn choose(&mut self, g: &Live, recent_tricks: u32, recent_stages: &[u32], last_trick: u32) -> Option<(Placement, Option<u32>)> {
        let active = g.active?;
        let queue: Vec<_> = g.queue.iter().copied().collect();
        let root = Root {
            board: g.board,
            current: active.piece,
            hold: g.hold,
            can_hold: g.can_hold,
            queue: &queue,
            b2b: g.b2b,
            combo: g.combo,
            pending: g.pending_lines(),
            bag: g.bag_mask(),
        };
        let mut cfg = self.0.clone();
        cfg.recent_tricks = recent_tricks;
        cfg.recent_stages = recent_stages.to_vec();
        cfg.last_trick = last_trick;
        suggest_with_plan(&root, &Rules::versus(), &cfg).map(|(p, plan)| (p, plan.stage))
    }
}

/// What one player's turn produced, for self-play data and replays.
pub struct Turn<'a> {
    pub player: usize,
    /// The player's game right after the placement (and any garbage that rose).
    pub game: &'a Live,
    /// What the placement's clear was worth, before cancelling.
    pub attack: u32,
    /// What was left to send after cancelling (delivered to the opponent after both moved).
    pub sent: u32,
    /// Garbage rows that rose onto this player's board this turn.
    pub risen: u32,
    /// The move played (None if there was none), and whether it held first.
    pub placement: Option<&'a Placement>,
    pub held: bool,
}

/// Plays until one side tops out, or `max_pieces` each (then whoever sent more wins).
pub fn play_match(cfgs: [&Config; 2], seed: u64, max_pieces: u32) -> MatchResult {
    play_match_observed(cfgs, seed, max_pieces, &mut |_| {})
}

/// `play_match`, calling `observe` after every placement.
pub fn play_match_observed(cfgs: [&Config; 2], seed: u64, max_pieces: u32, observe: &mut dyn FnMut(Turn)) -> MatchResult {
    let mut a = BotBrain(cfgs[0].clone());
    let mut b = BotBrain(cfgs[1].clone());
    play_brains([&mut a, &mut b], seed, max_pieces, observe)
}

/// The match loop for any two brains.
pub fn play_brains(brains: [&mut dyn Brain; 2], seed: u64, max_pieces: u32, observe: &mut dyn FnMut(Turn)) -> MatchResult {
    // No gravity: bots place every piece with explicit inputs and a hard drop. Seeds match the
    // browser's `live_new`, so a recorded match replays identically there.
    let mut games = [0u64, 1].map(|slot| Live::new(seed, seed ^ (slot + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15), 1e9, 500.0));
    let mut memory = [Memory::default(), Memory::default()];
    let mut style = [Style::default(); 2];
    for piece in 0..max_pieces {
        let mut attacks = [0; 2];
        for i in 0..2 {
            let g = &mut games[i];
            let Some(active) = g.active else { continue };
            let recent_tricks = memory[i].recent_tricks.iter().fold(0, |m, &t| m | t);
            let recent_stages: Vec<u32> = memory[i].recent_stages.iter().copied().collect();
            let b2b_before = g.b2b;
            let chosen = brains[i].choose(g, recent_tricks, &recent_stages, memory[i].last_trick);
            let planned_stage = chosen.as_ref().and_then(|c| c.1);
            if let Some(s) = planned_stage.filter(|s| !memory[i].attempts.contains(s)) {
                memory[i].attempts.push(s);
            }
            let held = chosen.as_ref().is_some_and(|(p, _)| p.piece != active.piece);
            match &chosen {
                Some((p, _)) => {
                    if held {
                        g.hold();
                    }
                    for input in &p.inputs {
                        match input {
                            Input::Left => _ = g.shift(-1),
                            Input::Right => _ = g.shift(1),
                            Input::Cw => _ = g.rotate(true),
                            Input::Ccw => _ = g.rotate(false),
                            Input::SoftDrop => while g.soft_drop() {},
                        }
                    }
                    debug_assert_eq!(g.active.map(|a| (a.x, a.rot)), Some((p.x, p.rot)), "plan and engine disagree");
                    g.hard_drop();
                }
                None => g.hard_drop(), // every placement tops out
            }
            let (mut sent_now, mut raw, mut risen) = (0, 0, 0);
            for e in g.events.drain(..) {
                match e {
                    Event::Lock { at, attack, sent, lines, spin, all_spin, combo, perfect_clear, .. } => {
                        (raw, sent_now) = (raw + attack, sent_now + sent);
                        style[i].max_combo = style[i].max_combo.max(combo.max(0) as u32);
                        if lines > 0 {
                            let kind = clear_kind(lines, spin, perfect_clear, all_spin.then_some(at.piece));
                            style[i].fouls += (kind == 0 || kind == memory[i].last_trick) as u32;
                            style[i].repeats += (kind != 0 && kind == memory[i].last_trick) as u32;
                            if kind != 0 {
                                memory[i].last_trick = kind;
                            }
                            style[i].chain = if g.b2b { style[i].chain + 1 } else { 0 };
                            style[i].max_b2b = style[i].max_b2b.max(style[i].chain);
                        }
                        let done = performed(lines, spin, combo, perfect_clear, b2b_before && g.b2b) | if all_spin && lines > 0 { trick::ALLSPIN } else { 0 };
                        style[i].tricks |= done;
                        style[i].tsd += (done & trick::TSD != 0) as u32;
                        style[i].tst += (done & trick::TST != 0) as u32;
                        style[i].pc += (done & trick::PC != 0) as u32;
                        if lines > 0 && b2b_before {
                            if g.b2b { style[i].b2b_clears += 1 } else { style[i].b2b_breaks += 1 }
                        }
                        let m = &mut memory[i];
                        m.recent_tricks.push_back(done);
                        if m.recent_tricks.len() > 12 {
                            m.recent_tricks.pop_front();
                        }
                        if let Some(stage) = planned_stage.filter(|_| spin == Spin::Full && lines > 0) {
                            style[i].setup_fires += 1;
                            m.fires.push(stage);
                            if !m.fired.contains(&stage) {
                                m.fired.push(stage);
                            }
                            m.recent_stages.push_back(stage);
                            if m.recent_stages.len() > 3 {
                                m.recent_stages.pop_front();
                            }
                        }
                    }
                    Event::GarbageIn { lines } => risen += lines,
                    _ => {}
                }
            }
            attacks[i] = sent_now;
            observe(Turn { player: i, game: g, attack: raw, sent: sent_now, risen, placement: chosen.as_ref().map(|c| &c.0), held });
        }
        match (games[0].over, games[1].over) {
            (false, false) => {}
            (a, b) => {
                let (fires, attempts) = setup_log(&memory);
                return MatchResult { winner: if a == b { None } else { Some(a as usize) }, pieces: piece + 1, sent: sent(&games), style: finish(style, &memory), fires, attempts };
            }
        }
        games[1].receive(attacks[0]);
        games[0].receive(attacks[1]);
    }
    let s = sent(&games);
    let winner = if s[0] == s[1] { None } else { Some((s[1] > s[0]) as usize) };
    let (fires, attempts) = setup_log(&memory);
    MatchResult { winner, pieces: max_pieces, sent: s, style: finish(style, &memory), fires, attempts }
}

fn setup_log(memory: &[Memory; 2]) -> ([Vec<u32>; 2], [Vec<u32>; 2]) {
    (memory.clone().map(|m| m.fires.clone()), memory.clone().map(|m| m.attempts.clone()))
}

fn finish(mut style: [Style; 2], memory: &[Memory; 2]) -> [Style; 2] {
    for i in 0..2 {
        style[i].setups = memory[i].fired.len() as u32;
    }
    style
}

/// Trick bits for a lock that actually happened.
fn performed(lines: u32, spin: Spin, combo: i32, perfect_clear: bool, b2b_chain: bool) -> u32 {
    if lines == 0 {
        return 0;
    }
    let kind = match (spin, lines) {
        (Spin::Full, 1) => trick::TSS,
        (Spin::Full, 2) => trick::TSD,
        (Spin::Full, _) => trick::TST,
        (Spin::Mini, _) => trick::MINI,
        (Spin::None, 4) => trick::QUAD,
        _ => 0,
    };
    kind | if perfect_clear { trick::PC } else { 0 } | if b2b_chain { trick::B2B } else { 0 } | if combo >= 3 { trick::COMBO } else { 0 }
}

fn sent(games: &[Live; 2]) -> [u32; 2] {
    [games[0].sent, games[1].sent]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::Objective;

    /// Identical bots: neither seat wins systematically (no first-mover edge).
    #[test]
    fn seats_are_fair() {
        let cfg = Config::difficulty(5, Objective::Attack);
        let mut wins = [0u32; 2];
        for seed in 0..12 {
            if let Some(w) = play_match([&cfg, &cfg], seed, 200).winner {
                wins[w] += 1;
            }
        }
        assert!(wins[0].abs_diff(wins[1]) <= 6, "seat wins {wins:?}");
    }

    /// A strong bot beats a beginner nearly every time; garbage really flows between the boards.
    #[test]
    fn strong_beats_weak() {
        let strong = Config::difficulty(7, Objective::Attack);
        let weak = Config::difficulty(1, Objective::Attack);
        let mut wins = 0;
        for seed in 0..6 {
            let r = play_match([&strong, &weak], seed, 300);
            wins += (r.winner == Some(0)) as u32;
            assert!(r.sent[0] > 0);
        }
        assert!(wins >= 5, "strong won only {wins}/6");
    }
}
