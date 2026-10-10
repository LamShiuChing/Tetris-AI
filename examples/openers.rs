//! Do top bots build known openers? Level 10 versus bot with and without opener steering,
//! first 7 bags (49 pieces) of many games. `cargo run --release --example openers [weights...]`

use tetris_ai::bot::{Config, Objective, Root, suggest};
use tetris_ai::game::{Game, Rules, attack};
use tetris_ai::movegen::Spin;
use tetris_ai::setups;

const GAMES: u64 = 128;
const PIECES: usize = 49;

#[derive(Default)]
struct Stats {
    built: u32,
    stages: u32,
    think_ms: f64,
    tspins: u32,
    pcs: u32,
    sent: u32,
}

fn run(cfg: &Config, seed: u64) -> Stats {
    let mut game = Game::new(Rules::versus(), seed);
    let (mut b2b, mut combo) = (false, -1);
    let mut s = Stats::default();
    let mut built = false;
    let mut was_complete = false;
    for _ in 0..PIECES {
        let queue: Vec<_> = game.queue.iter().copied().collect();
        let root = Root { board: game.board, current: game.current, hold: game.hold, can_hold: true, queue: &queue, b2b: game.b2b, combo: game.combo };
        let t = std::time::Instant::now();
        let Some(p) = suggest(&root, &game.rules, cfg) else { break };
        s.think_ms += t.elapsed().as_secs_f64() * 1000.0;
        let info = game.play(&p);
        let complete = setups::progress(&game.board, 0) == 1.0;
        built |= complete;
        s.stages += (complete && !was_complete) as u32;
        was_complete = complete;
        s.sent += attack(info.lines, info.spin, &mut b2b, &mut combo, info.perfect_clear);
        s.tspins += (info.spin == Spin::Full && info.lines > 0) as u32;
        s.pcs += info.perfect_clear as u32;
    }
    s.built = built as u32;
    s
}

fn main() {
    let base = Config::difficulty(10, Objective::Attack);
    let weights: Vec<f32> = std::env::args().skip(1).map(|a| a.parse().unwrap()).collect();
    let weights = if weights.is_empty() { vec![0.0, base.weights.setup] } else { weights };
    for w in weights {
        let cfg = Config { weights: tetris_ai::bot::Weights { setup: w, ..base.weights.clone() }, ..base.clone() };
        let name = format!("opener weight {w}");
        // SEQ=n: play n games one at a time, for think times without CPU contention.
        let all: Vec<Stats> = if let Ok(n) = std::env::var("SEQ") {
            (0..n.parse().unwrap()).map(|seed| run(&cfg, seed)).collect()
        } else {
            std::thread::scope(|sc| {
                let cfg = &cfg;
                let hs: Vec<_> = (0..GAMES).map(|seed| sc.spawn(move || run(cfg, seed))).collect();
                hs.into_iter().map(|h| h.join().unwrap()).collect()
            })
        };
        let sum = |f: fn(&Stats) -> u32| all.iter().map(f).sum::<u32>();
        let games = all.len() as f64;
        println!(
            "{name:22} reached a stage in {:>3}/{} games  stages {:.2}/game  T-spins {:.2}/game  PCs {}  attack {:.2}/game  think {:.1} ms/piece (first {PIECES} pieces)",
            sum(|s| s.built),
            all.len(),
            sum(|s| s.stages) as f64 / games,
            sum(|s| s.tspins) as f64 / games,
            sum(|s| s.pcs),
            sum(|s| s.sent) as f64 / games,
            all.iter().map(|s| s.think_ms).sum::<f64>() / (games * PIECES as f64),
        );
    }
}
