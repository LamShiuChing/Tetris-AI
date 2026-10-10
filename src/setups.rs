//! Opener and continuation stages from the Hard Drop wiki, used to steer the top bot levels
//! through known setups. Rows are top to bottom, ending at the floor: piece letters are cells
//! this stage fills, `X` is stack that must already be there (left by an earlier stage), `P`
//! marks the T-spin cells to keep open, `.` stays empty. Every stage is also used mirrored.

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
    /// Per row (top first), as board bitmasks: cells that must already be filled...
    ctx: Vec<u16>,
    /// ...and cells this stage fills.
    fill: Vec<u16>,
    cells: u32,
    /// Each piece the stage places and its cells.
    pieces: Vec<(Piece, Vec<u16>)>,
}

struct Index {
    shapes: Vec<Shape>,
    /// For each of the 1024 possible board bottom rows, the shapes that allow it.
    by_bottom: Vec<Vec<usize>>,
}

fn mask(row: &str, mirror: bool, keep: impl Fn(u8) -> bool) -> u16 {
    row.bytes().enumerate().filter(|&(_, c)| keep(c)).fold(0, |m, (x, _)| m | 1 << if mirror { 9 - x } else { x })
}

fn index() -> &'static Index {
    static CACHE: OnceLock<Index> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut shapes = vec![];
        for (_, rows) in SHAPES.iter().chain(WIKI_SHAPES) {
            for mirror in [false, true] {
                let ctx: Vec<u16> = rows.iter().map(|r| mask(r, mirror, |c| c == b'X')).collect();
                let fill: Vec<u16> = rows.iter().map(|r| mask(r, mirror, |c| b"IJLOSTZ".contains(&c))).collect();
                let pieces = Piece::ALL
                    .iter()
                    .map(|&p| (p, rows.iter().map(|r| mask(r, mirror, |c| c == b"IJLOSTZ"[p as usize])).collect::<Vec<u16>>()))
                    .filter(|(_, cells)| cells.iter().any(|&r| r != 0))
                    .collect();
                let cells = fill.iter().map(|r| r.count_ones()).sum();
                shapes.push(Shape { ctx, fill, cells, pieces });
            }
        }
        // A bottom row fits a shape if it has the shape's context and nothing outside context + fill.
        let by_bottom = (0..1u16 << 10)
            .map(|row| {
                (0..shapes.len())
                    .filter(|&i| {
                        let (c, f) = (*shapes[i].ctx.last().unwrap(), *shapes[i].fill.last().unwrap());
                        row & c == c && row & !(c | f) == 0
                    })
                    .collect()
            })
            .collect();
        Index { shapes, by_bottom }
    })
}

/// Bit `piece as u8` set for each piece type available to finish a shape.
pub fn piece_mask(pieces: impl IntoIterator<Item = Piece>) -> u8 {
    pieces.into_iter().fold(0, |m, p| m | 1 << p as u8)
}

/// How far the board is into any known stage, 0..=1. A stage counts only if the board has its
/// context, nothing else inside its rows, and every piece it still needs is in `available`
/// (a `piece_mask`), so the bot never chases a stage it can't finish with the pieces it sees.
/// Above the stage, the bot may keep stacking (it has to while it waits for the T), but only in
/// columns the stage's top row covers: the open columns are the T's way in.
pub fn progress(b: &Board, available: u8) -> f32 {
    let idx = index();
    let height = HEIGHT as usize - b.rows.iter().position(|&r| r != 0).unwrap_or(HEIGHT as usize);
    if height == 0 {
        return 0.0;
    }
    // above[k] = columns with any cell in rows 0..k, so "anything above row k" is one lookup.
    let mut above = [0u16; HEIGHT as usize + 1];
    for k in 0..HEIGHT as usize {
        above[k + 1] = above[k] | b.rows[k];
    }
    let bottom = b.rows[HEIGHT as usize - 1];
    idx.by_bottom[bottom as usize].iter().filter_map(|&i| stage_progress(&idx.shapes[i], b, &above, available)).fold(0.0, f32::max)
}

fn stage_progress(s: &Shape, b: &Board, above: &[u16], available: u8) -> Option<f32> {
    let top = HEIGHT as usize - s.fill.len();
    if above[top] & !(s.ctx[0] | s.fill[0]) & 0x3FF != 0 {
        return None;
    }
    let rows = &b.rows[top..];
    let mut have = 0;
    // Bottom-up: the context (most of what tells stages apart) sits low, so mismatches show early.
    for ((&r, &c), &f) in rows.iter().zip(&s.ctx).zip(&s.fill).rev() {
        if r & c != c || r & !(c | f) != 0 {
            return None;
        }
        have += (r & f).count_ones();
    }
    let missing = s.pieces.iter().filter(|(_, cells)| cells.iter().zip(rows).any(|(&c, &r)| r & c != c));
    let mut needed = missing.map(|(p, _)| 1u8 << *p as u8);
    needed.all(|bit| available & bit != 0).then(|| have as f32 / s.cells as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Rules;
    use crate::movegen::{Spin, find_placements};

    fn board(rows: &[&str]) -> Board {
        let mut b = Board::default();
        for (i, r) in rows.iter().enumerate() {
            b.rows[HEIGHT as usize - rows.len() + i] = mask(r, false, |c| c != b'.' && c != b'P');
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
        // Waiting for the T: stacking above covered columns keeps TKI complete...
        assert_eq!(progress(&board(&["...OO.....", "...OO.....", "L..ZZ.S...", "L...ZZSSOO", "LL.IIIISOO"]), 0), 1.0);
        // ...but covering the columns over the T-slot doesn't.
        assert_eq!(progress(&board(&[".OO.......", ".OO.......", "L..ZZ.S...", "L...ZZSSOO", "LL.IIIISOO"]), 0), 0.0);
    }

    #[test]
    fn continuation_needs_its_context() {
        // A stage with X context: the board must already hold the X cells.
        let (_, rows) = WIKI_SHAPES.iter().find(|(_, r)| r.iter().any(|row| row.contains('X'))).unwrap();
        let ctx_only: Vec<String> = rows.iter().map(|r| r.chars().map(|c| if c == 'X' { 'X' } else { '.' }).collect()).collect();
        let ctx_only: Vec<&str> = ctx_only.iter().map(|s| s.as_str()).collect();
        assert_eq!(progress(&board(rows), 0), 1.0, "context + stage pieces is complete");
        let all = piece_mask(Piece::ALL);
        assert!(progress(&board(&ctx_only), all) < 1.0, "context alone isn't complete");
    }

    /// Every spin stage must really be a T-spin in this engine: from spawn, the move search finds
    /// a T placement on exactly the `P` cells that counts as a full T-spin and clears lines.
    #[test]
    fn wiki_shapes_are_reachable_tspins() {
        let mut bad = vec![];
        for (name, rows) in SHAPES.iter().chain(WIKI_SHAPES) {
            if !rows.iter().any(|r| r.contains('P')) {
                continue; // perfect-clear shapes
            }
            let b = board(rows);
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
