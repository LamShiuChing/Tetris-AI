//! Bot-vs-bot win rate on the headless versus engine (real garbage, equal speed). Each seed is
//! played twice with seats swapped. `cargo run --release --example vs [seeds] [variant]`.

use tetris_ai::bot::{Config, Objective, Weights};
use tetris_ai::versus::play_match;

const MAX_PIECES: u32 = 400;

fn configs(variant: &str) -> [(&'static str, Config); 2] {
    let base = Config::difficulty(10, Objective::Attack);
    match variant {
        "garbage" => [("garbage-aware", base.clone()), ("garbage-blind", Config { sees_garbage: false, ..base })],
        "speculate" => [("speculating", base.clone()), ("not speculating", Config { speculate: false, ..base })],
        "setups" => [("setup steering", base.clone()), ("no setups", Config { weights: Weights { setup: 0.0, ..base.weights.clone() }, ..base })],
        "tuned" => [("tuned versus weights", base.clone()), ("hand-picked", Config { weights: hand(), ..base })],
        _ => panic!("variant: garbage | speculate | setups | tuned"),
    }
}

/// The hand-picked versus weights before tuning, for comparison.
fn hand() -> Weights {
    Weights { holes: -4.0, bumpiness: -0.4, height: -0.05, danger: -1.0, well: 0.7, tsd: 4.0, tst: 6.0, b2b: 1.0, setup: 4.0, incoming: -1.0 }
}

fn main() {
    let seeds: u64 = std::env::args().nth(1).map_or(32, |s| s.parse().unwrap());
    let variant = std::env::args().nth(2).unwrap_or_else(|| "garbage".into());
    let [(a_name, a), (b_name, b)] = configs(&variant);
    // (result, A's seat) per match; results are re-oriented so index 0 is always A.
    let results: Vec<_> = std::thread::scope(|sc| {
        let (a, b) = (&a, &b);
        let hs: Vec<_> = (0..seeds * 2)
            .map(|m| {
                sc.spawn(move || {
                    let seed = 50_000 + m / 2;
                    if m % 2 == 0 {
                        play_match([a, b], seed, MAX_PIECES)
                    } else {
                        let mut r = play_match([b, a], seed, MAX_PIECES);
                        r.winner = r.winner.map(|w| 1 - w);
                        r.sent.swap(0, 1);
                        r
                    }
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let matches = seeds * 2;
    let score: f64 = results.iter().map(|r| match r.winner { Some(0) => 1.0, None => 0.5, _ => 0.0 }).sum();
    let n = matches as f64;
    let p = score / n;
    let stderr = (p * (1.0 - p) / n).sqrt();
    let pieces = results.iter().map(|r| r.pieces as f64).sum::<f64>() / n;
    let sent = |i: usize| results.iter().map(|r| r.sent[i] as f64).sum::<f64>() / n;
    println!("{a_name} vs {b_name}: {:.1}% ± {:.1}% over {matches} matches  (avg {pieces:.0} pieces, sent {:.1} vs {:.1})", p * 100.0, stderr * 100.0, sent(0), sent(1));
}
