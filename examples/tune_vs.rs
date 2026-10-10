//! Tunes versus `Weights` by playing real matches (garbage, cancelling) against the current
//! champion, with the noisy cross-entropy method (as in `tune.rs`). Fitness per match is the
//! lines-sent margin in -1..1, plus 1 for a K.O. win (minus 1 for a K.O. loss): equal-speed
//! bots rarely top out, so the margin keeps every match informative.
//! Each seed is played from both seats. `cargo run --release --example tune_vs [generations]`, then paste the result into `Weights::versus()`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use tetris_ai::bot::{Config, Objective, Weights};
use tetris_ai::versus::play_match;

const POP: usize = 32;
const ELITE: usize = 8;
const MATCHES: u64 = 4;
const MAX_PIECES: u32 = 250;
/// Narrower than the real level-10 beam (64) to keep generations short; the vs example checks transfer.
const TUNE_WIDTH: usize = 16;

fn config(weights: Weights) -> Config {
    Config { width: TUNE_WIDTH, weights, ..Config::difficulty(10, Objective::Attack) }
}

fn main() {
    let gens: usize = std::env::args().nth(1).map_or(20, |s| s.parse().unwrap());
    let champion = config(Weights::versus());
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut mean = Weights::versus().to_vec();
    let mut sd: Vec<f32> = mean.iter().map(|m| (m.abs() * 0.5).max(0.5)).collect();

    for g in 0..gens {
        let cands: Vec<Vec<f32>> = (0..POP)
            .map(|i| mean.iter().zip(&sd).map(|(m, s)| if i == 0 { *m } else { m + s * rng.gauss() }).collect())
            .collect();
        let fitness = evaluate(&cands, &champion, 1000 * g as u64);
        let mut order: Vec<usize> = (0..POP).collect();
        order.sort_by(|&a, &b| fitness[b].total_cmp(&fitness[a]));
        let elite = &order[..ELITE];
        let noise = 0.3 * (1.0 - g as f32 / gens as f32);
        for k in 0..mean.len() {
            let m = elite.iter().map(|&i| cands[i][k]).sum::<f32>() / ELITE as f32;
            let var = elite.iter().map(|&i| (cands[i][k] - m).powi(2)).sum::<f32>() / ELITE as f32;
            mean[k] = m;
            sd[k] = var.sqrt() + noise;
        }
        println!(
            "gen {g:>2}: mean-candidate {:+.3}  best {:+.3}  elite avg {:+.3}  -> {:?}",
            fitness[0],
            fitness[order[0]],
            elite.iter().map(|&i| fitness[i]).sum::<f64>() / ELITE as f64,
            Weights::from_slice(&mean),
        );
    }
    println!("\ntuned: {:?}", Weights::from_slice(&mean));
}

/// Mean fitness per candidate over `MATCHES` matches against the champion, all on every core.
fn evaluate(cands: &[Vec<f32>], champion: &Config, seed0: u64) -> Vec<f64> {
    let jobs: Vec<(usize, u64)> = (0..cands.len()).flat_map(|c| (0..MATCHES).map(move |m| (c, seed0 + m))).collect();
    let next = AtomicUsize::new(0);
    let totals = Mutex::new(vec![0.0; cands.len()]);
    let threads = std::thread::available_parallelism().map_or(8, |n| n.get());
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| {
                while let Some(&(c, seed)) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let cand = config(Weights::from_slice(&cands[c]));
                    // Even seeds: candidate in seat 0; odd: seat 1 (re-oriented so index 0 is the candidate).
                    let r = if seed % 2 == 0 {
                        play_match([&cand, champion], seed / 2, MAX_PIECES)
                    } else {
                        let mut r = play_match([champion, &cand], seed / 2, MAX_PIECES);
                        r.winner = r.winner.map(|w| 1 - w);
                        r.sent.swap(0, 1);
                        r
                    };
                    let margin = (r.sent[0] as f64 - r.sent[1] as f64) / (r.sent[0] + r.sent[1] + 1) as f64;
                    let ko = match r.winner {
                        _ if r.pieces >= MAX_PIECES => 0.0, // went the distance: margin decides
                        Some(0) => 1.0,
                        Some(_) => -1.0,
                        None => 0.0, // double K.O.
                    };
                    totals.lock().unwrap()[c] += (margin + ko) / MATCHES as f64;
                }
            });
        }
    });
    totals.into_inner().unwrap()
}

/// xorshift64 + Box-Muller.
struct Rng(u64);

impl Rng {
    fn uniform(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32 + 0.5) / (1u64 << 24) as f32
    }

    fn gauss(&mut self) -> f32 {
        let (u, v) = (self.uniform(), self.uniform());
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
}
