//! Plays full MonochromeRush games in parallel on held-out seeds and compares configs.
//! `cargo run --release --example bench [games]`
//! Think times are measured under full parallel load, so they read higher than a lone game would.

use tetris_ai::bot::{Config, GameStats, Weights, play_game};

/// Held out from the tuner, which uses seeds below this.
const SEED0: u64 = 1_000_000;

fn main() {
    let games: u64 = std::env::args().nth(1).map_or(32, |s| s.parse().unwrap());
    let hand = Weights { holes: -4.0, bumpiness: -0.4, height: -0.05, danger: -1.0, well: 0.7, tsd: 4.0, tst: 6.0, b2b: 1.0 };
    let configs = [
        ("hand-picked weights", Config { weights: hand, ..Config::default() }),
        ("default weights", Config::default()),
    ];
    for (name, cfg) in configs {
        let stats: Vec<GameStats> = std::thread::scope(|sc| {
            let cfg = &cfg;
            let handles: Vec<_> = (0..games).map(|i| sc.spawn(move || play_game(cfg, SEED0 + i))).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let n = games as f64;
        let scores: Vec<f64> = stats.iter().map(|s| s.score).collect();
        let mean = scores.iter().sum::<f64>() / n;
        let stderr = (scores.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt() / n.sqrt();
        let avg = |f: fn(&GameStats) -> u32| stats.iter().map(|s| f(s) as f64).sum::<f64>() / n;
        let pieces: u32 = stats.iter().map(|s| s.pieces).sum();
        let think: f64 = stats.iter().map(|s| s.think.as_secs_f64()).sum();
        println!(
            "{name:20} score {mean:>9.0} ± {stderr:>6.0}  died {}/{games}  quads {:>4.1}  TSD {:>4.1}  TST {:>4.1}  think {:>5.1} ms/piece",
            stats.iter().filter(|s| s.lines < 300).count(),
            avg(|s| s.quads),
            avg(|s| s.tsds),
            avg(|s| s.tsts),
            think * 1000.0 / pieces as f64,
        );
    }
}
