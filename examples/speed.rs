//! Single-threaded thinking speed: level-10 versus bots (beam 16 as in self-play, and 64 as in
//! play) on fixed seeds. `cargo run --release --example speed`; WIDTHS=64,128,256 to pick widths.
use std::time::Instant;

use tetris_ai::bot::{Config, Objective};
use tetris_ai::versus::play_match;

fn main() {
    let widths: Vec<usize> = std::env::var("WIDTHS").map_or(vec![16, 64], |v| v.split(',').map(|w| w.parse().unwrap()).collect());
    for width in widths {
        let mut cfg = Config { width, ..Config::difficulty(10, Objective::Attack) };
        // NO_SETUPS=1 / NO_SPECULATE=1: measure what each feature costs.
        if std::env::var("NO_SETUPS").is_ok() {
            cfg.weights.setup = 0.0;
        }
        if std::env::var("NO_SPECULATE").is_ok() {
            cfg.speculate = false;
        }
        if std::env::var("NO_NET").is_ok() {
            cfg.use_net = false;
        }
        // NET_WEIGHT=w: level 10 with the value net blended in.
        if let Ok(w) = std::env::var("NET_WEIGHT") {
            cfg.use_net = true;
            cfg.net_weight = w.parse().unwrap();
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
