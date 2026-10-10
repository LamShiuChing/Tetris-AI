//! Headless versus: two bots on the real-time engine (garbage, cancelling, attack table), placing
//! one piece each per turn (equal speed). Attacks from a turn land after both have moved, so
//! neither side gets a first-mover edge. For tuning and benchmarking on win rate.

use crate::bot::{Config, Root, suggest};
use crate::game::Rules;
use crate::live::{Event, Live};
use crate::movegen::Input;

#[derive(Debug, Clone, Copy)]
pub struct MatchResult {
    /// None = draw (both survived `max_pieces` and sent the same).
    pub winner: Option<usize>,
    pub pieces: u32,
    pub sent: [u32; 2],
}

/// What one player's turn produced, for self-play data (`play_match_observed`).
pub struct Turn<'a> {
    pub player: usize,
    /// The player's game right after the placement (and any garbage that rose).
    pub game: &'a Live,
    /// What the placement's clear was worth, before cancelling.
    pub attack: u32,
    /// Garbage rows that rose onto this player's board this turn.
    pub risen: u32,
}

/// Plays until one side tops out, or `max_pieces` each (then whoever sent more wins).
pub fn play_match(cfgs: [&Config; 2], seed: u64, max_pieces: u32) -> MatchResult {
    play_match_observed(cfgs, seed, max_pieces, &mut |_| {})
}

/// `play_match`, calling `observe` after every placement.
pub fn play_match_observed(cfgs: [&Config; 2], seed: u64, max_pieces: u32, observe: &mut dyn FnMut(Turn)) -> MatchResult {
    // No gravity: bots place every piece with explicit inputs and a hard drop.
    let mut games = [0u64, 1].map(|slot| Live::new(seed, seed ^ (slot + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15), 1e9, 500.0));
    let rules = Rules::versus();
    for piece in 0..max_pieces {
        let mut attacks = [0; 2];
        for i in 0..2 {
            let g = &mut games[i];
            let Some(active) = g.active else { continue };
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
            match suggest(&root, &rules, cfgs[i]) {
                Some(p) => {
                    if p.piece != active.piece {
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
                    debug_assert_eq!(g.active.map(|a| (a.x, a.rot)), Some((p.x, p.rot)), "bot plan and engine disagree");
                    g.hard_drop();
                }
                None => g.hard_drop(), // every placement tops out
            }
            let (mut sent_now, mut raw, mut risen) = (0, 0, 0);
            for e in g.events.drain(..) {
                match e {
                    Event::Lock { attack, sent, .. } => (raw, sent_now) = (raw + attack, sent_now + sent),
                    Event::GarbageIn { lines } => risen += lines,
                    _ => {}
                }
            }
            attacks[i] = sent_now;
            observe(Turn { player: i, game: g, attack: raw, risen });
        }
        match (games[0].over, games[1].over) {
            (false, false) => {}
            (a, b) => return MatchResult { winner: if a == b { None } else { Some(a as usize) }, pieces: piece + 1, sent: sent(&games) },
        }
        games[1].receive(attacks[0]);
        games[0].receive(attacks[1]);
    }
    let s = sent(&games);
    let winner = if s[0] == s[1] { None } else { Some((s[1] > s[0]) as usize) };
    MatchResult { winner, pieces: max_pieces, sent: s }
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
