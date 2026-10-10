//! Tunes `Weights` with the noisy cross-entropy method (Szita & Lőrincz 2006): sample a population
//! from a diagonal Gaussian, refit it to the top `ELITE`, add decaying noise so it doesn't collapse early.
//! Fitness = mean MonochromeRush score over seeds shared within a generation (common random numbers).
//! `cargo run --release --example tune [generations]`, then paste the printed weights into `Weights::default()`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use tetris_ai::bot::{Config, Weights, play_game};

const POP: usize = 32;
const ELITE: usize = 8;
const SEEDS: u64 = 6;
/// Narrower beam than play (64) so a generation takes under a minute; weights transfer, the bench checks it.
const TUNE_WIDTH: usize = 16;

fn main() {
    let gens: usize = std::env::args().nth(1).map_or(15, |s| s.parse().unwrap());
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut mean = Weights::default().to_vec();
    let mut sd: Vec<f32> = mean.iter().map(|m| (m.abs() * 0.5).max(0.5)).collect();

    for g in 0..gens {
        // Candidate 0 is the current mean, so its fitness tracks progress on the same seeds.
        let cands: Vec<Vec<f32>> = (0..POP)
            .map(|i| mean.iter().zip(&sd).map(|(m, s)| if i == 0 { *m } else { m + s * rng.gauss() }).collect())
            .collect();
        let fitness = evaluate(&cands, g as u64 * 1000);

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
            "gen {g:>2}: mean-candidate {:>9.0}  best {:>9.0}  elite avg {:>9.0}  -> {:?}",
            fitness[0],
            fitness[order[0]],
            elite.iter().map(|&i| fitness[i]).sum::<f64>() / ELITE as f64,
            Weights::from_slice(&mean),
        );
    }
    println!("\ntuned: {:?}", Weights::from_slice(&mean));
}

/// Mean score per candidate over `SEEDS` games, all (candidate, seed) pairs spread over every core.
fn evaluate(cands: &[Vec<f32>], seed0: u64) -> Vec<f64> {
    let jobs: Vec<(usize, u64)> = (0..cands.len()).flat_map(|c| (0..SEEDS).map(move |s| (c, seed0 + s))).collect();
    let next = AtomicUsize::new(0);
    let totals = Mutex::new(vec![0.0; cands.len()]);
    let threads = std::thread::available_parallelism().map_or(8, |n| n.get());
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| {
                while let Some(&(c, seed)) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let cfg = Config { width: TUNE_WIDTH, weights: Weights::from_slice(&cands[c]), ..Config::default() };
                    let score = play_game(&cfg, seed).score;
                    totals.lock().unwrap()[c] += score / SEEDS as f64;
                }
            });
        }
    });
    totals.into_inner().unwrap()
}

/// xorshift64 + Box-Muller; enough randomness for sampling weights.
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
