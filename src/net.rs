//! Learned board evaluation: a small MLP over board features, trained on the GPU from self-play
//! (train/train.py) and embedded from nets/value.bin. It predicts the match outcome from a board
//! (+1 = this side wins, -1 = loses), and the search adds it, times `Config::net_weight`, to the
//! hand-weighted eval.

use std::sync::OnceLock;

use crate::board::{Board, HEIGHT, WIDTH};

/// Rows of the surface window: the top of the stack, where slots and overhangs are.
const WINDOW: usize = 6;
pub const INPUTS: usize = 10 + 10 + WINDOW * 10 + 6;
const H1: usize = 64;
const H2: usize = 32;

pub fn features(b: &Board, b2b: bool, combo: i32, pending: u32, tsd: bool, tst: bool) -> [f32; INPUTS] {
    let mut f = [0.0f32; INPUTS];
    let top = b.rows.iter().position(|&r| r != 0).unwrap_or(HEIGHT as usize);
    // One pass top-down: heights from each column's first filled cell, holes = empty cells under it.
    let (mut covered, mut holes) = (0u16, [0u32; WIDTH as usize]);
    for (y, &row) in b.rows.iter().enumerate().skip(top) {
        let mut new = row & !covered;
        while new != 0 {
            f[new.trailing_zeros() as usize] = (HEIGHT as usize - y) as f32 / 20.0;
            new &= new - 1;
        }
        covered |= row;
        let mut gaps = covered & !row;
        while gaps != 0 {
            holes[gaps.trailing_zeros() as usize] += 1;
            gaps &= gaps - 1;
        }
    }
    for x in 0..WIDTH as usize {
        f[10 + x] = holes[x] as f32 / 5.0;
    }
    for k in 0..WINDOW {
        let row = b.rows.get(top + k).copied().unwrap_or(0x3FF);
        for x in 0..10 {
            f[20 + k * 10 + x] = (row >> x & 1) as f32;
        }
    }
    let m = 20 + WINDOW * 10;
    f[m] = b2b as u8 as f32;
    f[m + 1] = combo.max(0) as f32 / 8.0;
    f[m + 2] = pending as f32 / 8.0;
    f[m + 3] = (HEIGHT as usize - top) as f32 / 20.0;
    f[m + 4] = tsd as u8 as f32;
    f[m + 5] = tst as u8 as f32;
    f
}

pub struct Net {
    w1: Vec<f32>,
    b1: Vec<f32>,
    w2: Vec<f32>,
    b2: Vec<f32>,
    w3: Vec<f32>,
    b3: f32,
    /// Targets were divided by this for training.
    scale: f32,
}

impl Net {
    /// Little-endian f32s: scale, w1 [H1 x INPUTS], b1, w2 [H2 x H1], b2, w3 [H2], b3.
    fn from_bytes(bytes: &[u8]) -> Net {
        let v: Vec<f32> = bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        assert_eq!(v.len(), 1 + H1 * INPUTS + H1 + H2 * H1 + H2 + H2 + 1, "value net size doesn't match INPUTS/H1/H2");
        let mut it = v.into_iter();
        let mut take = |n: usize| (&mut it).take(n).collect::<Vec<f32>>();
        let scale = take(1)[0];
        Net { w1: take(H1 * INPUTS), b1: take(H1), w2: take(H2 * H1), b2: take(H2), w3: take(H2), b3: take(1)[0], scale }
    }

    pub fn eval(&self, x: &[f32; INPUTS]) -> f32 {
        let mut h1 = [0.0f32; H1];
        for (j, h) in h1.iter_mut().enumerate() {
            *h = (self.b1[j] + dot(&self.w1[j * INPUTS..(j + 1) * INPUTS], x)).max(0.0);
        }
        let mut out = self.b3;
        for j in 0..H2 {
            out += self.w3[j] * (self.b2[j] + dot(&self.w2[j * H1..(j + 1) * H1], &h1)).max(0.0);
        }
        out * self.scale
    }
}

/// Dot product with 8 independent running sums, so the compiler can use 8-wide vector math (a
/// plain `.sum()` must add in order, which blocks that).
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let tail: f32 = ca.remainder().iter().zip(cb.remainder()).map(|(x, y)| x * y).sum();
    for (x, y) in ca.zip(cb) {
        for k in 0..8 {
            acc[k] += x[k] * y[k];
        }
    }
    acc.iter().sum::<f32>() + tail
}

/// The embedded value net, or None until one has been trained (nets/value.bin is empty).
pub fn value_net() -> Option<&'static Net> {
    static NET: OnceLock<Option<Net>> = OnceLock::new();
    NET.get_or_init(|| {
        let bytes = include_bytes!("../nets/value.bin");
        (!bytes.is_empty()).then(|| Net::from_bytes(bytes))
    })
    .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// When a net is embedded it loads and gives finite values that depend on the board.
    #[test]
    fn embedded_net_evaluates() {
        let Some(net) = value_net() else { return };
        let mut tall = Board::default();
        for y in 25..40 {
            tall.rows[y] = 0x3FF & !(1 << (y % 10));
        }
        let (empty, high) = (net.eval(&features(&Board::default(), false, -1, 0, false, false)), net.eval(&features(&tall, false, -1, 6, false, false)));
        // The fast feature pass must give the same heights/holes as a cell-by-cell count.
        let f = features(&tall, false, -1, 6, false, false);
        for x in 0..10 {
            let col_top = (0..40).find(|&y| tall.rows[y] >> x & 1 != 0).unwrap_or(40);
            assert_eq!(f[x], (40 - col_top) as f32 / 20.0);
            assert_eq!(f[10 + x], (col_top..40).filter(|&y| tall.rows[y] >> x & 1 == 0).count() as f32 / 5.0);
        }
        assert!(empty.is_finite() && high.is_finite());
        assert_ne!(empty, high);
    }
}
