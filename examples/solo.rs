//! A personality's solo play (no garbage, score objective, as in Watch Solo): what each clear was,
//! how many broke the tricks-only rule, and how tall the stack got.
//! `cargo run --release --example solo -- [level] [games] [pieces]` (default Virtuoso, 8 games, 300 pieces).

use tetris_ai::bot::{Config, Objective, Root, VIRTUOSO, clear_kind, suggest, trick};
use tetris_ai::game::{Game, Rules};

fn main() {
    let arg = |i: usize, d: u32| std::env::args().nth(i).map_or(d, |s| s.parse().unwrap());
    let (level, games, max_pieces) = (arg(1, VIRTUOSO), arg(2, 8), arg(3, 300));
    let results: Vec<_> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..games).map(|g| sc.spawn(move || play(level, 5_000 + g as u64, max_pieces))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let names = [(trick::TSS, "TSS"), (trick::TSD, "TSD"), (trick::TST, "TST"), (trick::MINI, "mini"), (trick::PC, "PC"), (0, "plain")];
    for (g, r) in results.iter().enumerate() {
        let mut kinds: Vec<String> = names.iter().map(|&(k, n)| format!("{n} {}", r.kinds.iter().filter(|&&x| x == k).count())).collect();
        kinds.push(format!("all-spins {}", r.kinds.iter().filter(|&&x| x & trick::ALLSPIN_KIND != 0).count()));
        println!("game {g}: {} pieces{}, peak height {}, repeats {}  | {}", r.pieces, if r.over { " (topped out)" } else { "" }, r.peak, r.repeats, kinds.join("  "));
    }
}

struct Run {
    pieces: u32,
    over: bool,
    peak: usize,
    repeats: u32,
    /// Kind of every clear in order (0 = plain).
    kinds: Vec<u32>,
}

fn play(level: u32, seed: u64, max_pieces: u32) -> Run {
    let mut game = Game::new(Rules::monochrome_rush(1), seed);
    let mut cfg = Config::difficulty(level, Objective::Points);
    let mut r = Run { pieces: 0, over: false, peak: 0, repeats: 0, kinds: vec![] };
    while r.pieces < max_pieces && !game.over {
        let queue: Vec<_> = game.queue.iter().copied().collect();
        let root = Root { board: game.board, current: game.current, hold: game.hold, can_hold: true, queue: &queue, b2b: game.b2b, combo: game.combo, pending: 0, bag: game.bag_mask() };
        let Some(p) = suggest(&root, &game.rules, &cfg) else { break };
        let spun = p.all_spin.then_some(p.piece);
        let info = game.play(&p);
        r.pieces += 1;
        if info.lines > 0 {
            let kind = clear_kind(info.lines, info.spin, info.perfect_clear, spun);
            r.repeats += (kind != 0 && kind == cfg.last_trick) as u32;
            if kind != 0 {
                cfg.last_trick = kind;
            }
            r.kinds.push(kind);
        }
        let top = game.board.rows.iter().position(|&row| row != 0).unwrap_or(40);
        r.peak = r.peak.max(40 - top);
    }
    r.over = game.over;
    r
}
