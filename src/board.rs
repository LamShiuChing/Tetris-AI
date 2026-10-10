use crate::piece::Piece;

pub const WIDTH: i8 = 10;
pub const HEIGHT: i8 = 40;
const FULL_ROW: u16 = (1 << WIDTH) - 1;

/// 10x40, y-down like MonochromeRush: rows 0..20 hidden, 20..40 visible, row 39 is the floor.
/// Each row is a bitmask, bit x set = filled.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Board {
    pub rows: [u16; HEIGHT as usize],
}

impl Default for Board {
    fn default() -> Self {
        Board { rows: [0; HEIGHT as usize] }
    }
}

impl Board {
    /// Above the top (y < 0) is open sky, as in `Game.collide`.
    pub fn occupied(&self, x: i8, y: i8) -> bool {
        x < 0 || x >= WIDTH || y >= HEIGHT || (y >= 0 && self.rows[y as usize] >> x & 1 != 0)
    }

    pub fn collides(&self, piece: Piece, x: i8, y: i8, rot: u8) -> bool {
        piece.cells(x, y, rot).iter().any(|&(cx, cy)| self.occupied(cx, cy))
    }

    /// Writes the piece and clears full rows, returning lines cleared.
    /// Cells above the top are discarded, as in `Game.lock`.
    pub fn lock(&mut self, piece: Piece, x: i8, y: i8, rot: u8) -> u32 {
        self.place(piece, x, y, rot);
        self.clear_full()
    }

    pub fn place(&mut self, piece: Piece, x: i8, y: i8, rot: u8) {
        for (cx, cy) in piece.cells(x, y, rot) {
            if cy >= 0 {
                self.rows[cy as usize] |= 1 << cx;
            }
        }
    }

    /// Bit y set = row y is full.
    pub fn full_rows(&self) -> u64 {
        (0..HEIGHT as usize).filter(|&y| self.rows[y] == FULL_ROW).fold(0, |m, y| m | 1 << y)
    }

    /// Pushes `n` garbage rows (full except `hole`) in from the bottom. Returns false if
    /// filled cells were pushed off the top.
    pub fn push_garbage(&mut self, n: usize, hole: i8) -> bool {
        let n = n.min(HEIGHT as usize);
        let fits = self.rows[..n].iter().all(|&r| r == 0);
        self.rows.copy_within(n.., 0);
        self.rows[HEIGHT as usize - n..].fill(FULL_ROW & !(1 << hole));
        fits
    }

    /// Removes full rows, returning how many.
    pub fn clear_full(&mut self) -> u32 {
        let mut dst = HEIGHT as usize;
        for src in (0..HEIGHT as usize).rev() {
            if self.rows[src] != FULL_ROW {
                dst -= 1;
                self.rows[dst] = self.rows[src];
            }
        }
        self.rows[..dst].fill(0);
        dst as u32
    }

    pub fn is_empty(&self) -> bool {
        self.rows.iter().all(|&r| r == 0)
    }
}
