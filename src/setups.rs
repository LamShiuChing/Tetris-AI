//! Opener shapes from the Hard Drop wiki (catalog in research/openers.md), used to steer the
//! top bot levels into known setups. Rows are top to bottom, ending at the floor; piece letters
//! are cells to fill, `P` marks the T-spin cells to leave open, `.` stays empty.
//! Each shape is also used mirrored.

use std::sync::OnceLock;

use crate::board::{Board, HEIGHT};
use crate::piece::Piece;
use crate::setups_data::WIKI_SHAPES;

const SHAPES: &[(&str, &[&str])] = &[
    // https://harddrop.com/wiki/TKI_3_Opening (regular base): TSD on the left.
    ("TKI 3", &["L..ZZ.S...", "LPPPZZSSOO", "LLPIIIISOO"]),
    // Same page, castle top: J stacked for the follow-up TST.
    ("TKI 3 castle top", &[".........J", ".........J", "L..ZZ.S.JJ", "LPPPZZSSOO", "LLPIIIISOO"]),
    // https://harddrop.com/wiki/Albatross_Special: air TSD, holes below kept for the TST.
    ("Albatross Special", &["...ZZ....I", "SPPPZZJJJI", "SSPOOLLLJI", ".S.OOL...I"]),
    // https://harddrop.com/wiki/Perfect_Clear_Opener: first bag, with and without the I.
    ("Perfect Clear Opener", &["LLLI....SS", "LOOI...SST", "JOOI..ZZTT", "JJJI...ZZT"]),
    ("Perfect Clear Opener (no I)", &["LLL.....SS", "LOO....SST", "JOO...ZZTT", "JJJ....ZZT"]),
];

struct Shape {
    /// Filled cells per row, top row first, as board bitmasks (bit x = column x).
    fill: Vec<u16>,
    cells: u32,
    /// Each piece in the shape and its cells (same row layout as `fill`).
    pieces: Vec<(Piece, Vec<u16>)>,
}

fn shapes() -> &'static [Shape] {
    static CACHE: OnceLock<Vec<Shape>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut out = vec![];
        for (_, rows) in SHAPES.iter().chain(WIKI_SHAPES) {
            for mirror in [false, true] {
                let fill: Vec<u16> = rows
                    .iter()
                    .map(|r| {
                        r.bytes().enumerate().filter(|&(_, c)| c.is_ascii_uppercase() && c != b'P').fold(0, |m, (x, _)| {
                            m | 1 << if mirror { 9 - x } else { x }
                        })
                    })
                    .collect();
                let cells = fill.iter().map(|r| r.count_ones()).sum();
                let pieces = Piece::ALL
                    .iter()
                    .map(|&p| {
                        let letter = b"IJLOSTZ"[p as usize];
                        let rows = rows
                            .iter()
                            .map(|r| r.bytes().enumerate().filter(|&(_, c)| c == letter).fold(0, |m, (x, _)| m | 1 << if mirror { 9 - x } else { x }))
                            .collect::<Vec<u16>>();
                        (p, rows)
                    })
                    .filter(|(_, rows)| rows.iter().any(|&r| r != 0))
                    .collect();
                out.push(Shape { fill, cells, pieces });
            }
        }
        out
    })
}

/// Bit `piece as u8` set for each piece type available to finish a shape.
pub fn piece_mask(pieces: impl IntoIterator<Item = Piece>) -> u8 {
    pieces.into_iter().fold(0, |m, p| m | 1 << p as u8)
}

/// How far the board is into any known opener shape, 0..=1. A shape counts only if the board has
/// no cell outside it (or above it) and every piece it still needs is in `available`
/// (a `piece_mask`), so the bot never chases a shape it can't finish with the pieces it sees.
pub fn progress(b: &Board, available: u8) -> f32 {
    let mut best = 0.0f32;
    for s in shapes() {
        let top = HEIGHT as usize - s.fill.len();
        if b.rows[..top].iter().any(|&r| r != 0) {
            continue;
        }
        let mut have = 0;
        let fits = s.fill.iter().zip(&b.rows[top..]).all(|(&f, &r)| {
            have += r.count_ones();
            r & !f == 0
        });
        let rows = &b.rows[top..];
        let missing = s.pieces.iter().filter(|(_, cells)| cells.iter().zip(rows).any(|(&c, &r)| r & c != c));
        if fits && missing.map(|(p, _)| 1u8 << *p as u8).all(|bit| available & bit != 0) {
            best = best.max(have as f32 / s.cells as f32);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Rules;
    use crate::movegen::{Spin, find_placements};

    fn board(rows: &[&str]) -> Board {
        let mut b = Board::default();
        for (i, r) in rows.iter().enumerate() {
            b.rows[HEIGHT as usize - rows.len() + i] =
                r.bytes().enumerate().filter(|&(_, c)| c != b'.').fold(0, |m, (x, _)| m | 1 << x);
        }
        b
    }

    #[test]
    fn progress_tracks_shapes_and_mirrors() {
        let all = piece_mask(Piece::ALL);
        assert_eq!(progress(&Board::default(), all), 0.0);
        // TKI 3 complete, and its mirror: nothing more needed.
        assert_eq!(progress(&board(&["L..ZZ.S...", "L...ZZSSOO", "LL.IIIISOO"]), 0), 1.0);
        assert_eq!(progress(&board(&["...S.ZZ..L", "OOSSZZ...L", "OOSIIII.LL"]), 0), 1.0);
        // Just the I placed: partial progress, but only if the rest of TKI's pieces are coming.
        let i_only = board(&["...IIII..."]);
        let p = progress(&i_only, all);
        assert!(p > 0.0 && p < 0.25, "{p}");
        assert_eq!(progress(&i_only, piece_mask([Piece::L, Piece::Z, Piece::S])), 0.0, "no O in sight");
        // A cell higher than every shape reaches.
        assert_eq!(progress(&board(&["....X.....", "..........", "..........", "..........", "..........", ".........."]), all), 0.0);
    }

    /// Every spin shape must really be a T-spin in this engine: from spawn, the move search finds
    /// a T placement on exactly the `P` cells that counts as a full T-spin and clears lines.
    #[test]
    fn wiki_shapes_are_reachable_tspins() {
        let mut bad = vec![];
        for (name, rows) in SHAPES.iter().chain(WIKI_SHAPES) {
            if !rows.iter().any(|r| r.contains('P')) {
                continue; // perfect-clear shapes
            }
            let b = board(&rows.iter().map(|r| r.replace('P', ".")).collect::<Vec<_>>().iter().map(|s| s.as_str()).collect::<Vec<_>>());
            let mut want: Vec<(i8, i8)> = vec![];
            for (i, r) in rows.iter().enumerate() {
                for (x, c) in r.bytes().enumerate() {
                    if c == b'P' {
                        want.push((x as i8, (HEIGHT as usize - rows.len() + i) as i8));
                    }
                }
            }
            want.sort();
            let ok = find_placements(&b, Piece::T, &Rules::versus()).iter().any(|p| {
                let mut cells = p.piece.cells(p.x, p.y, p.rot);
                cells.sort();
                p.spin == Spin::Full && cells.to_vec() == want && b.clone().lock(p.piece, p.x, p.y, p.rot) > 0
            });
            if !ok {
                bad.push(*name);
            }
        }
        assert!(bad.is_empty(), "not a reachable T-spin here: {bad:?}");
    }
}
