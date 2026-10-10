//! Self-play data for the value net. Plays versus matches between copies of one bot (with noise
//! and per-side salts for variety) and writes, for every placement, the board features, the
//! discounted return that followed (sum of gamma^k * (2*attack - 2*garbage risen) over the
//! player's later pieces, with -100 for topping out), the hand eval's score for that board, and
//! the match outcome seen from that board: +1 won / -1 lost / 0 draw, times WIN_GAMMA^(pieces to the
//! end). Rows of little-endian f32: INPUTS features, return, hand eval, outcome.
//! `cargo run --release --example selfplay -- <matches> <out.bin> [net]`: with `net`, one seat
//! (alternating) uses the value net at blend NET_WEIGHT (env, default 60) against the hand-eval bot,
//! so the boards the net wrongly likes show up in the data, labelled with how they really ended.

use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tetris_ai::bot::{Config, Objective, hand_eval};
use tetris_ai::net::{INPUTS, features};
use tetris_ai::versus::{Turn, play_match_observed};

const GAMMA: f32 = 0.95;
const DEATH: f32 = -100.0;
const MAX_PIECES: u32 = 300;
/// How fast the outcome label fades with distance from the end of the match.
const WIN_GAMMA: f32 = 0.98;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let matches: u64 = args.get(1).map_or(1000, |s| s.parse().unwrap());
    let out = args.get(2).cloned().unwrap_or_else(|| "data/selfplay.bin".into());
    let use_net = args.get(3).is_some_and(|a| a == "net");
    let rows = Mutex::new(Vec::<f32>::new());
    let next = AtomicU64::new(0);
    let threads = std::thread::available_parallelism().map_or(8, |n| n.get());
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| {
                while let m = next.fetch_add(1, Ordering::Relaxed)
                    && m < matches
                {
                    let mut local = Vec::new();
                    play_one(m, use_net, &mut local);
                    rows.lock().unwrap().extend(local);
                }
            });
        }
    });
    let rows = rows.into_inner().unwrap();
    let mut f = std::io::BufWriter::new(std::fs::File::create(&out).unwrap());
    for v in &rows {
        f.write_all(&v.to_le_bytes()).unwrap();
    }
    println!("{} positions from {matches} matches -> {out}", rows.len() / (INPUTS + 3));
}

fn play_one(seed: u64, use_net: bool, out: &mut Vec<f32>) {
    let base = Config { width: 16, noise: 0.5, ..Config::difficulty(10, Objective::Attack) };
    let mut cfgs = [Config { salt: seed * 2, ..base.clone() }, Config { salt: seed * 2 + 1, ..base }];
    if use_net {
        let weight = std::env::var("NET_WEIGHT").map_or(60.0, |v| v.parse().unwrap());
        let seat = (seed % 2) as usize;
        cfgs[seat].use_net = true;
        cfgs[seat].net_weight = weight;
    }
    // Per player: (features, reward of the *next* turns accumulate into it later).
    // Per player and turn: (features, hand eval, reward of that turn).
    let mut traj: [Vec<([f32; INPUTS], f32, f32)>; 2] = [vec![], vec![]];
    let mut died = [false; 2];
    let result = play_match_observed([&cfgs[0], &cfgs[1]], 7_000_000 + seed, MAX_PIECES, &mut |t: Turn| {
        let g = t.game;
        let reward = 2.0 * t.attack as f32 - 2.0 * t.risen as f32;
        let slot = tetris_ai::bot::tslot_kind(&g.board);
        let x = features(&g.board, g.b2b, g.combo, g.pending_lines(), slot == Some(2), slot == Some(3));
        let hand = hand_eval(&g.board, g.b2b, g.pending_lines(), &cfgs[t.player].weights);
        traj[t.player].push((x, hand, reward));
        died[t.player] |= g.over;
    });
    for p in 0..2 {
        let outcome = match result.winner {
            Some(w) if w == p => 1.0,
            Some(_) => -1.0,
            None => 0.0,
        };
        let n = traj[p].len();
        // State t's target: rewards of turns t+1.. (turn t's own reward is already in the search's
        // banked reward), discounted, plus the death penalty after the last turn if this side lost.
        let mut g = if died[p] { DEATH } else { 0.0 };
        let mut rows = vec![];
        for (k, ((x, hand, _), r_next)) in traj[p].iter().rev().zip(std::iter::once(0.0).chain(traj[p].iter().rev().map(|t| t.2))).enumerate() {
            g = r_next + GAMMA * g;
            rows.push((x, g, *hand, outcome * WIN_GAMMA.powi(k as i32)));
        }
        debug_assert_eq!(rows.len(), n);
        for (x, g, hand, z) in rows {
            out.extend_from_slice(x);
            out.extend_from_slice(&[g, hand, z]);
        }
    }
}
