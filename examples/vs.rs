//! Bot-vs-bot win rate on the headless versus engine (real garbage, equal speed). Each seed is
//! played twice with seats swapped. `cargo run --release --example vs [seeds] [variant] [beam width]`.

use tetris_ai::bot::{CHAINLORD, Config, Objective, PERFECTIONIST, RENEGADE, Specialty, TRICKSTER, Weights};
use tetris_ai::versus::play_match;

const MAX_PIECES: u32 = 400;

fn configs(variant: &str, width: usize) -> [(&'static str, Config); 2] {
    let base = Config { width, ..Config::difficulty(10, Objective::Attack) };
    match variant {
        "garbage" => [("garbage-aware", base.clone()), ("garbage-blind", Config { sees_garbage: false, ..base })],
        "speculate" => [("speculating", base.clone()), ("not speculating", Config { speculate: false, ..base })],
        "setups" => [("setup steering", base.clone()), ("no setups", Config { weights: Weights { setup: 0.0, ..base.weights.clone() }, ..base })],
        "net" => [("value net", Config { use_net: true, net_weight: net_weight(), ..base.clone() }), ("hand-weighted eval", Config { use_net: false, ..base })],
        "trickster" => [("Trickster", tuned(Config::difficulty(TRICKSTER, Objective::Attack))), ("Titan", base)],
        "chainlord" => [("Chainlord", tuned(Config::difficulty(CHAINLORD, Objective::Attack))), ("Titan", base)],
        "renegade" => [("Renegade", tuned(Config { width, ..Config::difficulty(RENEGADE, Objective::Attack) })), ("Titan", base)],
        "perfectionist" => [("Perfectionist", tuned(Config { width, ..Config::difficulty(PERFECTIONIST, Objective::Attack) })), ("Titan", base)],
        "dedupe" => [("merging duplicate states", base.clone()), ("no merging", Config { dedupe: false, ..base })],
        "width128" => [("beam 128", Config { width: 128, ..base.clone() }), ("beam 64", base)],
        "b2b" => [("B2B policy", base.clone()), ("no B2B policy", Config { weights: Weights { b2b_break: 0.0, ..base.weights.clone() }, ..base })],
        "tuned" => [("tuned versus weights", base.clone()), ("hand-picked", Config { weights: hand(), ..base })],
        _ => panic!("variant: trickster | chainlord | renegade | perfectionist | garbage | speculate | setups | dedupe | width128 | b2b | net | tuned"),
    }
}

/// Env overrides for sweeping a personality: TRICK_MARGIN, TRICK_BONUS, B2B_BREAK, B2B_HOLD (multiplies
/// Titan's b2b), SETUP (multiplies Titan's setup), NET (0 = value net off), REN_STEP, REN_DEPTH,
/// PC_BONUS, PC_HIGH, PC_REGION.
fn tuned(mut c: Config) -> Config {
    let env = |k: &str| std::env::var(k).ok().map(|v| v.parse::<f32>().unwrap());
    let titan = Config::difficulty(10, Objective::Attack).weights;
    if let Some(m) = env("SETUP") {
        c.weights.setup = titan.setup * m;
    }
    if env("NET") == Some(0.0) {
        c.use_net = false;
    }
    match &mut c.specialty {
        Specialty::Ren { step, depth } => {
            *step = env("REN_STEP").unwrap_or(*step);
            *depth = env("REN_DEPTH").unwrap_or(*depth);
        }
        Specialty::PerfectClear { bonus, high, region } => {
            *bonus = env("PC_BONUS").unwrap_or(*bonus);
            *high = env("PC_HIGH").unwrap_or(*high);
            *region = env("PC_REGION").unwrap_or(*region);
        }
        Specialty::Normal => {}
    }
    if let Some(m) = env("TRICK_MARGIN") {
        c.trick_margin = m;
    }
    if let Some(b) = env("TRICK_BONUS") {
        c.trick_bonus = b;
    }
    if let Some(b) = env("B2B_BREAK") {
        c.weights.b2b_break = b;
    }
    if let Some(h) = env("B2B_HOLD") {
        c.weights.b2b = titan.b2b * h;
    }
    c
}

/// `SEED0` env var, default 50000: change it for confirmation runs on seeds never used to pick a candidate.
fn seed0() -> u64 {
    std::env::var("SEED0").map_or(50_000, |v| v.parse().unwrap())
}

/// `NET_WEIGHT` env var, default 20.
fn net_weight() -> f32 {
    std::env::var("NET_WEIGHT").map_or(20.0, |v| v.parse().unwrap())
}

/// The hand-picked versus weights before tuning, for comparison.
fn hand() -> Weights {
    Weights { holes: -4.0, bumpiness: -0.4, height: -0.05, danger: -1.0, well: 0.7, tsd: 4.0, tst: 6.0, b2b: 1.0, setup: 4.0, incoming: -1.0, b2b_break: 0.0 }
}

fn main() {
    let seeds: u64 = std::env::args().nth(1).map_or(32, |s| s.parse().unwrap());
    let variant = std::env::args().nth(2).unwrap_or_else(|| "garbage".into());
    let width: usize = std::env::args().nth(3).map_or(64, |s| s.parse().unwrap());
    let [(a_name, a), (b_name, b)] = configs(&variant, width);
    // (result, A's seat) per match; results are re-oriented so index 0 is always A.
    let results: Vec<_> = std::thread::scope(|sc| {
        let (a, b) = (&a, &b);
        let hs: Vec<_> = (0..seeds * 2)
            .map(|m| {
                sc.spawn(move || {
                    let seed = seed0() + m / 2;
                    if m % 2 == 0 {
                        play_match([a, b], seed, MAX_PIECES)
                    } else {
                        let mut r = play_match([b, a], seed, MAX_PIECES);
                        r.winner = r.winner.map(|w| 1 - w);
                        r.sent.swap(0, 1);
                        r.style.swap(0, 1);
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
    for (i, name) in [a_name, b_name].into_iter().enumerate() {
        let avg = |f: fn(&tetris_ai::versus::Style) -> u32| results.iter().map(|r| f(&r.style[i]) as f64).sum::<f64>() / n;
        println!(
            "  {name:>10}: per match TSD {:.1}  TST {:.2}  PC {:.2}  setup fires {:.2} ({:.2} distinct)  B2B clears {:.1} / breaks {:.1}  longest B2B {:.1}  max combo {:.1}  trick kinds {:.1}",
            avg(|s| s.tsd),
            avg(|s| s.tst),
            avg(|s| s.pc),
            avg(|s| s.setup_fires),
            avg(|s| s.setups),
            avg(|s| s.b2b_clears),
            avg(|s| s.b2b_breaks),
            avg(|s| s.max_b2b),
            avg(|s| s.max_combo),
            avg(|s| s.tricks.count_ones()),
        );
    }
}
