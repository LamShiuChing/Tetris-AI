//! Single-threaded thinking speed: level-10 versus bots (beam 16 as in self-play, and 64 as in
//! play) on fixed seeds. `cargo run --release --example speed`
use std::time::Instant;

use tetris_ai::bot::{Config, Objective};
use tetris_ai::versus::play_match;

fn main() {
    for width in [16, 64] {
        let mut cfg = Config { width, ..Config::difficulty(10, Objective::Attack) };
        // NO_SETUPS=1 / NO_SPECULATE=1: measure what each feature costs.
        if std::env::var("NO_SETUPS").is_ok() {
            cfg.weights.setup = 0.0;
        }
        if std::env::var("NO_SPECULATE").is_ok() {
            cfg.speculate = false;
        }
        let t = Instant::now();
        let mut pieces = 0;
        for seed in 0..3 {
            pieces += 2 * play_match([&cfg, &cfg], 900 + seed, 120).pieces;
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!("beam {width:>2}: {:.2} ms per piece ({pieces} pieces)", ms / pieces as f64);
    }
}
