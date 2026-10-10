//! Do top bots build known openers? Level 10 versus bot with and without opener steering,
//! first 5 bags (35 pieces) of many games. `cargo run --release --example openers [weights...]`

use tetris_ai::bot::{Config, Objective, Root, suggest};
use tetris_ai::game::{Game, Rules, attack};
use tetris_ai::movegen::Spin;
use tetris_ai::setups;

const GAMES: u64 = 128;
const PIECES: usize = 35;

#[derive(Default)]
struct Stats {
    built: u32,
    tspins: u32,
    pcs: u32,
    sent: u32,
}

fn run(cfg: &Config, seed: u64) -> Stats {
    let mut game = Game::new(Rules::versus(), seed);
    let (mut b2b, mut combo) = (false, -1);
    let mut s = Stats::default();
    let mut built = false;
    for _ in 0..PIECES {
        let queue: Vec<_> = game.queue.iter().copied().collect();
        let root = Root { board: game.board, current: game.current, hold: game.hold, can_hold: true, queue: &queue, b2b: game.b2b, combo: game.combo };
        let Some(p) = suggest(&root, &game.rules, cfg) else { break };
        let info = game.play(&p);
        built |= setups::progress(&game.board, 0) == 1.0;
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
        let all: Vec<Stats> = std::thread::scope(|sc| {
            let cfg = &cfg;
            let hs: Vec<_> = (0..GAMES).map(|seed| sc.spawn(move || run(cfg, seed))).collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let sum = |f: fn(&Stats) -> u32| all.iter().map(f).sum::<u32>();
        println!(
            "{name:22} built an opener in {:>2}/{GAMES} games  T-spins {:.2}/game  PCs {}  attack {:.2}/game (first {PIECES} pieces)",
            sum(|s| s.built),
            sum(|s| s.tspins) as f64 / GAMES as f64,
            sum(|s| s.pcs),
            sum(|s| s.sent) as f64 / GAMES as f64,
        );
    }
}
