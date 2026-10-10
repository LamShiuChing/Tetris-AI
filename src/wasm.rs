//! Raw C-ABI exports for the browser (no wasm-bindgen). JS writes 40 row bitmasks
//! (bit x = filled, row 39 = floor) into `board_ptr()` and the visible preview into
//! `queue_ptr()`, calls `suggest`, then reads `out_ptr()`.

use crate::board::{Board, HEIGHT};
use crate::bot::{self, Config, Root};
use crate::game::Rules;
use crate::piece::Piece;

static mut BOARD: [u16; HEIGHT as usize] = [0; HEIGHT as usize];
static mut QUEUE: [u8; 16] = [0; 16];
static mut OUT: [u8; 256] = [0; 256];

#[unsafe(no_mangle)]
pub extern "C" fn board_ptr() -> *mut u16 {
    &raw mut BOARD as *mut u16
}

#[unsafe(no_mangle)]
pub extern "C" fn queue_ptr() -> *mut u8 {
    &raw mut QUEUE as *mut u8
}

#[unsafe(no_mangle)]
pub extern "C" fn out_ptr() -> *const u8 {
    &raw const OUT as *const u8
}

/// Pieces are indices into `Piece::ALL` (I J L O S T Z); `hold` is 7 when empty.
/// Writes `[piece, x, y, rot, spin, n_inputs, inputs...]` (x/y as i8) to OUT and returns its length, 0 if no placement exists.
#[unsafe(no_mangle)]
pub extern "C" fn suggest(current: u32, hold: u32, can_hold: u32, queue_len: u32, b2b: u32, combo: i32, level: u32) -> u32 {
    let board = Board { rows: unsafe { *(&raw const BOARD) } };
    let queue: Vec<Piece> = unsafe { &*(&raw const QUEUE) }[..queue_len as usize].iter().map(|&i| Piece::ALL[i as usize]).collect();
    let root = Root {
        board,
        current: Piece::ALL[current as usize],
        hold: Piece::ALL.get(hold as usize).copied(),
        can_hold: can_hold != 0,
        queue: &queue,
        b2b: b2b != 0,
        combo,
    };
    let Some(p) = bot::suggest(&root, &Rules::monochrome_rush(level), &Config::default()) else {
        return 0;
    };
    let out = unsafe { &mut *(&raw mut OUT) };
    let head = [p.piece as u8, p.x as u8, p.y as u8, p.rot, p.spin as u8, p.inputs.len() as u8];
    let len = head.len() + p.inputs.len();
    out[..head.len()].copy_from_slice(&head);
    for (o, &i) in out[head.len()..len].iter_mut().zip(&p.inputs) {
        *o = i as u8;
    }
    len as u32
}
