//! Versus-bot skill per difficulty: solo games (no garbage), counting spins and attack.
//! `cargo run --release --example spins`

use tetris_ai::bot::{Config, Objective, Root, suggest};
use tetris_ai::game::{Game, Rules, attack};
use tetris_ai::movegen::Spin;

const PIECES: u32 = 300;
const SEEDS: u64 = 4;

fn main() {
    for level in [1, 3, 5, 7, 10] {
        let cfg = Config::difficulty(level, Objective::Attack);
        let (mut tsd, mut tst, mut quads, mut sent, mut deaths) = (0, 0, 0, 0, 0);
        for seed in 0..SEEDS {
            let mut game = Game::new(Rules::versus(), seed);
            let (mut b2b, mut combo) = (false, -1);
            for _ in 0..PIECES {
                let queue: Vec<_> = game.queue.iter().copied().collect();
                let root = Root { board: game.board, current: game.current, hold: game.hold, can_hold: true, queue: &queue, b2b: game.b2b, combo: game.combo, pending: 0, bag: game.bag_mask() };
                let Some(p) = suggest(&root, &game.rules, &cfg) else { break };
                let info = game.play(&p);
                sent += attack(info.lines, info.spin, &mut b2b, &mut combo, info.perfect_clear);
                quads += (info.lines == 4) as u32;
                if info.spin == Spin::Full {
                    tsd += (info.lines == 2) as u32;
                    tst += (info.lines == 3) as u32;
                }
                if game.over {
                    deaths += 1;
                    break;
                }
            }
        }
        let n = SEEDS as f64;
        println!(
            "level {level:>2}: TSD {:>5.1}  TST {:>4.1}  quads {:>5.1}  attack/piece {:.3}  died {deaths}/{SEEDS}",
            tsd as f64 / n,
            tst as f64 / n,
            quads as f64 / n,
            sent as f64 / (n * PIECES as f64),
        );
    }
}
