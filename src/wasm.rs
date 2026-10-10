//! Raw C-ABI exports for the browser (no wasm-bindgen).
//!
//! Bot: JS writes 40 row bitmasks (bit x = filled, row 39 = floor) into `board_ptr()` and the
//! visible preview into `queue_ptr()`, calls `suggest` (MonochromeRush) or `plan` (Stackfall),
//! then reads `out_ptr()`.
//!
//! Live games (Stackfall): `live_new` per slot, then `live_input` / `live_tick` / `live_garbage`,
//! reading state from `live_snapshot` and effects from `live_events` + `events_ptr()`.

use crate::board::{Board, HEIGHT};
use crate::bot::{self, Config, Objective, Root};
use crate::game::{Gravity, Rules};
use crate::live::{Event, Live};
use crate::movegen::Placement;
use crate::piece::Piece;

static mut BOARD: [u16; HEIGHT as usize] = [0; HEIGHT as usize];
static mut QUEUE: [u8; 16] = [0; 16];
static mut OUT: [u8; 256] = [0; 256];
static mut GAMES: Vec<Live> = Vec::new();
static mut SNAP: [i32; 64] = [0; 64];
static mut COLORS: [u8; 400] = [0; 400];
static mut CELLS: [i8; 8] = [0; 8];
const EVENT_INTS: usize = 8;
const MAX_EVENTS: usize = 64;
static mut EVENTS: [i32; EVENT_INTS * MAX_EVENTS] = [0; EVENT_INTS * MAX_EVENTS];

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

#[unsafe(no_mangle)]
pub extern "C" fn events_ptr() -> *const i32 {
    &raw const EVENTS as *const i32
}

/// MonochromeRush bot. Pieces are indices into `Piece::ALL` (I J L O S T Z); `hold` is 7 when empty.
/// Writes `[piece, x, y, rot, spin, n_inputs, inputs...]` (x/y as i8) to OUT and returns its length, 0 if no placement exists.
#[unsafe(no_mangle)]
pub extern "C" fn suggest(current: u32, hold: u32, can_hold: u32, queue_len: u32, b2b: u32, combo: i32, level: u32) -> u32 {
    let queue = read_queue(queue_len);
    let root = read_root(current, hold, can_hold, &queue, b2b, combo);
    write_out(bot::suggest(&root, &Rules::monochrome_rush(level), &Config::default()))
}

/// Stackfall bot at `difficulty` 1..=10; `salt` varies its mistakes so two bots don't mirror.
/// `flags`: bit 0 = plan for 20G (every action falls to the floor), bit 1 = solo score objective
/// instead of versus attack. Same inputs and output as `suggest`.
#[unsafe(no_mangle)]
pub extern "C" fn plan(current: u32, hold: u32, can_hold: u32, queue_len: u32, b2b: u32, combo: i32, difficulty: u32, salt: u32, flags: u32) -> u32 {
    let queue = read_queue(queue_len);
    let root = read_root(current, hold, can_hold, &queue, b2b, combo);
    let objective = if flags & 2 != 0 { Objective::Points } else { Objective::Attack };
    let cfg = Config { salt: salt as u64, ..Config::difficulty(difficulty, objective) };
    let gravity = if flags & 1 != 0 { Gravity::TwentyG } else { Gravity::Zero };
    write_out(bot::suggest(&root, &Rules { gravity, ..Rules::versus() }, &cfg))
}

fn read_queue(len: u32) -> Vec<Piece> {
    let queue = unsafe { &*(&raw const QUEUE) };
    queue[..len as usize].iter().map(|&i| Piece::ALL[i as usize]).collect()
}

fn read_root(current: u32, hold: u32, can_hold: u32, queue: &[Piece], b2b: u32, combo: i32) -> Root<'_> {
    Root {
        board: Board { rows: unsafe { *(&raw const BOARD) } },
        current: Piece::ALL[current as usize],
        hold: Piece::ALL.get(hold as usize).copied(),
        can_hold: can_hold != 0,
        queue,
        b2b: b2b != 0,
        combo,
    }
}

fn write_out(p: Option<Placement>) -> u32 {
    let Some(p) = p else { return 0 };
    let out = unsafe { &mut *(&raw mut OUT) };
    let head = [p.piece as u8, p.x as u8, p.y as u8, p.rot, p.spin as u8, p.inputs.len() as u8];
    let len = head.len() + p.inputs.len();
    out[..head.len()].copy_from_slice(&head);
    for (o, &i) in out[head.len()..len].iter_mut().zip(&p.inputs) {
        *o = i as u8;
    }
    len as u32
}

fn game(slot: u32) -> &'static mut Live {
    let games = unsafe { &mut *(&raw mut GAMES) };
    &mut games[slot as usize]
}

/// Starts a fresh game in `slot` (0, 1, ...). `gravity_ms` is ms per row, 0 for 20G.
#[unsafe(no_mangle)]
pub extern "C" fn live_new(slot: u32, seed: u32, gravity_ms: f32, lock_delay_ms: f32) {
    let games = unsafe { &mut *(&raw mut GAMES) };
    // Same pieces for every slot; garbage holes differ per slot.
    let g = Live::new(seed as u64, seed as u64 ^ (slot as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15), gravity_ms, lock_delay_ms);
    if (slot as usize) < games.len() {
        games[slot as usize] = g;
    } else {
        games.push(g);
    }
}

/// 0 left, 1 right, 2 cw, 3 ccw, 4 soft drop (one row), 5 hard drop, 6 hold. Returns 1 if it did anything.
#[unsafe(no_mangle)]
pub extern "C" fn live_input(slot: u32, action: u32) -> u32 {
    let g = game(slot);
    (match action {
        0 => g.shift(-1),
        1 => g.shift(1),
        2 => g.rotate(true),
        3 => g.rotate(false),
        4 => g.soft_drop(),
        5 => {
            g.hard_drop();
            true
        }
        6 => g.hold(),
        _ => false,
    }) as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn live_tick(slot: u32, dt_ms: f32) {
    game(slot).tick(dt_ms);
}

#[unsafe(no_mangle)]
pub extern "C" fn live_garbage(slot: u32, lines: u32) {
    game(slot).receive(lines);
}

#[unsafe(no_mangle)]
pub extern "C" fn live_set_gravity(slot: u32, gravity_ms: f32) {
    game(slot).gravity_ms = gravity_ms;
}

#[unsafe(no_mangle)]
pub extern "C" fn live_set_lock_delay(slot: u32, ms: f32) {
    game(slot).lock_delay_ms = ms;
}

/// Solo modes: the level multiplies line-clear points.
#[unsafe(no_mangle)]
pub extern "C" fn live_set_level(slot: u32, level: u32) {
    game(slot).level = level;
}

/// i32 layout: [0..40) rows, 40 piece (-1 none), 41 x, 42 y, 43 rot, 44 ghost y, 45 hold (-1 none),
/// 46 can hold, 47..52 queue, 52 b2b, 53 combo, 54 pending garbage, 55 over, 56 lock progress (0..1000),
/// 57 lines, 58 pieces, 59 lines sent, 60 score, 61 level.
#[unsafe(no_mangle)]
pub extern "C" fn live_snapshot(slot: u32) -> *const i32 {
    let g = game(slot);
    let s = unsafe { &mut *(&raw mut SNAP) };
    for (d, &r) in s[..40].iter_mut().zip(&g.board.rows) {
        *d = r as i32;
    }
    let (piece, x, y, rot) = g.active.map_or((-1, 0, 0, 0), |a| (a.piece as i32, a.x as i32, a.y as i32, a.rot as i32));
    s[40..45].copy_from_slice(&[piece, x, y, rot, g.ghost_y().unwrap_or(0) as i32]);
    s[45] = g.hold.map_or(-1, |p| p as i32);
    s[46] = g.can_hold as i32;
    for (d, p) in s[47..52].iter_mut().zip(&g.queue) {
        *d = *p as i32;
    }
    s[52..60].copy_from_slice(&[
        g.b2b as i32,
        g.combo,
        g.pending_lines() as i32,
        g.over as i32,
        (g.lock_progress() * 1000.0) as i32,
        g.lines as i32,
        g.pieces as i32,
        g.sent as i32,
    ]);
    s[60] = g.score as i32;
    s[61] = g.level as i32;
    s.as_ptr()
}

/// The 4 cells of `piece` (index into I J L O S T Z) at `rot`, as x,y offsets from its pivot.
/// Lets the renderer draw pieces from the engine's own shape table.
#[unsafe(no_mangle)]
pub extern "C" fn piece_cells(piece: u32, rot: u32) -> *const i8 {
    let out = unsafe { &mut *(&raw mut CELLS) };
    for (i, (x, y)) in Piece::ALL[piece as usize].cells(0, 0, rot as u8).into_iter().enumerate() {
        out[2 * i] = x;
        out[2 * i + 1] = y;
    }
    out.as_ptr()
}

/// Cell colors, 40 rows x 10 bytes: 0 empty, 1..=7 piece (index into I J L O S T Z, +1), 8 garbage.
#[unsafe(no_mangle)]
pub extern "C" fn live_colors(slot: u32) -> *const u8 {
    let out = unsafe { &mut *(&raw mut COLORS) };
    for (d, row) in out.chunks_mut(10).zip(&game(slot).colors) {
        d.copy_from_slice(row);
    }
    out.as_ptr()
}

/// Drains events into `events_ptr()`, 8 ints each: [kind, ...]. Kinds: 1 move, 2 rotate, 3 soft drop,
/// 4 hold, 5 hard drop [rows], 6 lock [lines, spin 0/1/2, combo, flags, sent, cleared lo, cleared hi],
/// flags = b2b | pc << 1 | piece << 2 | rot << 5 | (x + 2) << 7 | (y + 4) << 12,
/// 7 garbage in [lines], 8 top out. Returns the count.
#[unsafe(no_mangle)]
pub extern "C" fn live_events(slot: u32) -> u32 {
    let g = game(slot);
    let out = unsafe { &mut *(&raw mut EVENTS) };
    let n = g.events.len().min(MAX_EVENTS);
    for (e, o) in g.events.drain(..).take(n).zip(out.chunks_mut(EVENT_INTS)) {
        let v: [i32; EVENT_INTS] = match e {
            Event::Move => [1, 0, 0, 0, 0, 0, 0, 0],
            Event::Rotate => [2, 0, 0, 0, 0, 0, 0, 0],
            Event::SoftDrop => [3, 0, 0, 0, 0, 0, 0, 0],
            Event::Hold => [4, 0, 0, 0, 0, 0, 0, 0],
            Event::HardDrop { rows } => [5, rows as i32, 0, 0, 0, 0, 0, 0],
            Event::Lock { at, lines, spin, combo, b2b, perfect_clear, sent, cleared } => [
                6,
                lines as i32,
                spin as i32,
                combo,
                b2b as i32
                    | (perfect_clear as i32) << 1
                    | (at.piece as i32) << 2
                    | (at.rot as i32) << 5
                    | (at.x as i32 + 2) << 7
                    | (at.y as i32 + 4) << 12,
                sent as i32,
                cleared as u32 as i32,
                (cleared >> 32) as i32,
            ],
            Event::GarbageIn { lines } => [7, lines as i32, 0, 0, 0, 0, 0, 0],
            Event::TopOut => [8, 0, 0, 0, 0, 0, 0, 0],
        };
        o.copy_from_slice(&v);
    }
    n as u32
}
