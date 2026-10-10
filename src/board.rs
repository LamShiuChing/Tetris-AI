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
        for (cx, cy) in piece.cells(x, y, rot) {
            if cy >= 0 {
                self.rows[cy as usize] |= 1 << cx;
            }
        }
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
