//! How do the hand eval and the value net rate stacks of growing height (rows full except a
//! one-wide well on the right)? `cargo run --release --example netprobe`
use tetris_ai::board::Board;
use tetris_ai::bot::{Weights, hand_eval};
use tetris_ai::net::{features, value_net};

fn main() {
    let w = Weights::versus();
    let net = value_net().expect("no net embedded");
    for h in [0usize, 2, 4, 6, 8, 10, 12, 14, 16] {
        let mut b = Board::default();
        for y in 40 - h..40 {
            b.rows[y] = 0x3FF & !(1 << 9);
        }
        let hand = hand_eval(&b, false, 0, &w);
        let corr = net.eval(&features(&b, false, -1, 0, false, false));
        println!("height {h:>2}: hand {hand:>8.1}  net correction {corr:>8.1}  total {:>8.1}", hand + corr);
    }
}
