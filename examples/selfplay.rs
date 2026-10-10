//! Self-play data for the value net. Plays versus matches between copies of one bot (with noise
//! and per-side salts for variety) and writes, for every placement, the board features and the
//! discounted return that followed: sum of gamma^k * (2*attack - 2*garbage risen) over the
//! player's later pieces, with -100 for topping out. Rows of little-endian f32: INPUTS features,
//! then the target. `cargo run --release --example selfplay -- <matches> <out.bin> [net]`

use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tetris_ai::bot::{Config, Objective};
use tetris_ai::net::{INPUTS, features};
use tetris_ai::versus::{Turn, play_match_observed};

const GAMMA: f32 = 0.95;
const DEATH: f32 = -100.0;
const MAX_PIECES: u32 = 300;

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
    println!("{} positions from {matches} matches -> {out}", rows.len() / (INPUTS + 1));
}

fn play_one(seed: u64, use_net: bool, out: &mut Vec<f32>) {
    let base = Config { width: 16, noise: 0.5, use_net, ..Config::difficulty(10, Objective::Attack) };
    let cfgs = [Config { salt: seed * 2, ..base.clone() }, Config { salt: seed * 2 + 1, ..base }];
    // Per player: (features, reward of the *next* turns accumulate into it later).
    let mut traj: [Vec<([f32; INPUTS], f32)>; 2] = [vec![], vec![]];
    let mut died = [false; 2];
    play_match_observed([&cfgs[0], &cfgs[1]], 7_000_000 + seed, MAX_PIECES, &mut |t: Turn| {
        let g = t.game;
        let reward = 2.0 * t.attack as f32 - 2.0 * t.risen as f32;
        let slot = tetris_ai::bot::tslot_kind(&g.board);
        let x = features(&g.board, g.b2b, g.combo, g.pending_lines(), slot == Some(2), slot == Some(3));
        traj[t.player].push((x, reward));
        died[t.player] |= g.over;
    });
    for p in 0..2 {
        // State t's target: rewards of turns t+1.. (turn t's own reward is already in the search's
        // banked reward), discounted, plus the death penalty after the last turn if this side lost.
        let mut g = if died[p] { DEATH } else { 0.0 };
        for (x, r_next) in traj[p].iter().rev().map(|(x, _)| x).zip(
            std::iter::once(0.0).chain(traj[p].iter().rev().map(|&(_, r)| r)),
        ) {
            g = r_next + GAMMA * g;
            out.extend_from_slice(x);
            out.push(g);
        }
    }
}
