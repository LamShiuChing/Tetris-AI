//! Prints the embedded value net's output for the first rows of a self-play file, to compare
//! against the training side. `cargo run --release --example netcheck -- data/x.bin`
use tetris_ai::net::{INPUTS, value_net};

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let bytes = std::fs::read(path).unwrap();
    let v: Vec<f32> = bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let net = value_net().expect("no net embedded");
    for row in v.chunks_exact(INPUTS + 3).take(5) {
        let x: [f32; INPUTS] = row[..INPUTS].try_into().unwrap();
        println!("rust {:.4}  outcome {:.4}", net.eval(&x), row[INPUTS + 2]);
    }
}
