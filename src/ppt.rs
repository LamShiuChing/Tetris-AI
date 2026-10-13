//! Drop-in replacement for Zetris's `MisaMino.dll`: the same C exports, Titan's brain.
//! Zetris (github.com/ZetrisAI/Zetris) reads Puyo Puyo Tetris (2) memory and presses a virtual
//! gamepad; copy the built `tetris_ai.dll` over `MisaMino.dll` in the Zetris folder. Our own fork of
//! Zetris (`ppt2/`) also calls `titan_set_level` and `titan_plan`.
//!
//! Zetris coordinates: a piece is its SRS 4x4 box, X = box left + 1, Y = H + 3 - box top row, rows
//! counting up from the floor, H = rows in the field string. Our pivot (y-down, floor row 39) sits
//! at box (1, 1) for every piece, so X = px and Y = H + py - 37. MisaMino's own x/y are X-1, Y-3,
//! and its rotation index runs the other way (MisaMinoNET swaps 1 and 3).

use std::ffi::{CStr, c_char};
use std::sync::Mutex;

use crate::board::Board;
use crate::bot::{Config, Objective, Plan, Root, clear_kind, suggest_with_plan, trick};
use crate::game::{Rules, attack};
use crate::movegen::{Input, Placement, Spin, find_placements, find_placements_from};
use crate::piece::Piece;
use crate::setups::setup_name;

// MisaMinoNET `Instruction` values.
const NULL: u8 = 0;
const DROP: u8 = 9;
const HOLD: u8 = 10;

struct State {
    board: Board,
    h: i8,
    current: Piece,
    hold: Option<Piece>,
    queue: Vec<Piece>,
    combo: i32,
    b2b: u32,
    incoming: u32,
    hold_allowed: bool,
    level: u32,
    last_trick: u32,
    /// What the last decision's line is going for, for display.
    plan: String,
}

static STATE: Mutex<State> = Mutex::new(State {
    board: Board { rows: [0; 40] },
    h: 21,
    current: Piece::T,
    hold: None,
    queue: Vec::new(),
    combo: 0,
    b2b: 0,
    incoming: 0,
    hold_allowed: true,
    level: 10,
    last_trick: 0,
    plan: String::new(),
});

/// MisaMino's piece letters are mirrored (Zetris sends game S as 'Z', J as 'L').
fn piece(c: u8) -> Option<Piece> {
    Some(match c {
        b'Z' => Piece::S,
        b'S' => Piece::Z,
        b'L' => Piece::J,
        b'J' => Piece::L,
        b'T' => Piece::T,
        b'O' => Piece::O,
        b'I' => Piece::I,
        _ => return None,
    })
}

/// MisaMino's gem number, which MisaMinoNET maps back to the game's piece.
fn gem(p: Piece) -> u8 {
    match p {
        Piece::I => 1,
        Piece::T => 2,
        Piece::J => 3,
        Piece::L => 4,
        Piece::S => 5,
        Piece::Z => 6,
        Piece::O => 7,
    }
}

fn swap_rot(r: i32) -> u8 {
    [0, 3, 2, 1][(r & 3) as usize]
}

fn text<'a>(s: *const c_char) -> &'a [u8] {
    if s.is_null() { &[] } else { unsafe { CStr::from_ptr(s) }.to_bytes() }
}

/// Rows top first, ';'-separated; cells ','-separated, mirrored (first = rightmost), '2' = filled.
fn field(s: &[u8]) -> (Board, i8) {
    let rows: Vec<&[u8]> = s.split(|&c| c == b';').collect();
    let h = rows.len() as i8;
    let mut board = Board::default();
    for (k, row) in rows.iter().enumerate() {
        let y = 40 - h as i32 + k as i32;
        if !(0..40).contains(&y) {
            continue;
        }
        for (c, cell) in row.split(|&c| c == b',').take(10).enumerate() {
            if matches!(cell, [b'2'] | [b'3']) {
                board.rows[y as usize] |= 1 << (9 - c);
            }
        }
    }
    (board, h)
}

fn rules(h: i8, hold: bool) -> Rules {
    Rules { spawn: (4, 40 - h), hold, ..Rules::versus() }
}

fn moves(first: u8, inputs: &[Input]) -> String {
    let codes = inputs.iter().map(|i| match i {
        Input::Left => 1,
        Input::Right => 2,
        Input::SoftDrop => 6,
        Input::Ccw => 7,
        Input::Cw => 8,
    });
    std::iter::once(first).chain(codes).chain([DROP]).map(|c| c.to_string()).collect::<Vec<_>>().join(",")
}

/// The inputs for Zetris, and whether it must replay them exactly (a tuck or spin) rather than just
/// rotate, shift and hard drop. Our search reaches every spot by soft dropping to rest, and its
/// fewest-inputs path may tuck along the floor even for a plain spot: plain spots get a plain route.
fn route(board: &Board, rules: &Rules, p: &Placement) -> (Vec<Input>, bool) {
    match plain_route(board, rules, p) {
        Some(plain) => (plain, false),
        None => (exact(p), true),
    }
}

/// The search's path, minus the final soft drop to rest: Zetris hard drops after the inputs anyway.
fn exact(p: &Placement) -> Vec<Input> {
    let mut inputs = p.inputs.clone();
    while inputs.last() == Some(&Input::SoftDrop) {
        inputs.pop();
    }
    inputs
}

/// Rotate at spawn, shift, hard drop, if that lands exactly on `p`.
fn plain_route(board: &Board, rules: &Rules, p: &Placement) -> Option<Vec<Input>> {
    let (mut x, y) = rules.spawn;
    // ponytail: checks only the final rotation at spawn, not the one passed through for a 180.
    if p.spin != Spin::None || p.all_spin || board.collides(p.piece, x, y, p.rot) {
        return None;
    }
    let mut inputs = match p.rot {
        1 => vec![Input::Cw],
        2 => vec![Input::Cw, Input::Cw],
        3 => vec![Input::Ccw],
        _ => vec![],
    };
    while x != p.x {
        x += (p.x - x).signum();
        if board.collides(p.piece, x, y, p.rot) {
            return None;
        }
        inputs.push(if p.x < rules.spawn.0 { Input::Left } else { Input::Right });
    }
    let mut floor = y;
    while !board.collides(p.piece, x, floor + 1, p.rot) {
        floor += 1;
    }
    (floor == p.y).then_some(inputs)
}

fn write(out: *mut c_char, len: i32, s: &str) {
    if out.is_null() || len <= 0 {
        return;
    }
    let n = s.len().min(len as usize - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(s.as_ptr(), out as *mut u8, n);
        *out.add(n) = 0;
    }
}

/// The placement covering the same cells as the target pose (S/Z/I poses can share cells).
fn path_to(placements: Vec<Placement>, p: Piece, x: i8, y: i8, rot: u8) -> Option<Placement> {
    let mut target = p.cells(x, y, rot);
    target.sort();
    placements.into_iter().find(|c| {
        let mut cells = c.piece.cells(c.x, c.y, c.rot);
        cells.sort();
        cells == target
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn set_abort(_callback: *const ()) {}

/// `param` is MisaMino's 22-int style struct (by reference on x64); Titan ignores it, and the
/// search width, 180 and game flags too (PPT: SRS, no 180).
#[unsafe(no_mangle)]
pub extern "C" fn configure(_param: *const i32, hold_allowed: i32, _spins: i32, _tsd_only: i32, _width: i32, _allow180: i32, _game: i32, _tmini: i32) {
    let mut s = STATE.lock().unwrap();
    s.hold_allowed = hold_allowed != 0;
    s.last_trick = 0;
}

/// Which bot plays: 1-10, or a personality at Titan's strength (`bot::TRICKSTER`..`bot::VIRTUOSO`).
#[unsafe(no_mangle)]
pub extern "C" fn titan_set_level(level: i32) {
    STATE.lock().unwrap().level = level.clamp(1, 15) as u32;
}

/// The last decision's plan, e.g. "TKI 3 Opening: TSD, Back-to-Back".
#[unsafe(no_mangle)]
pub extern "C" fn titan_plan(out: *mut c_char, len: i32) {
    write(out, len, &STATE.lock().unwrap().plan);
}

fn describe(plan: &Plan) -> String {
    const NAMES: [(u32, &str); 9] = [
        (trick::PC, "Perfect Clear"), (trick::TST, "TST"), (trick::TSD, "TSD"), (trick::TSS, "TSS"), (trick::MINI, "T-Spin Mini"),
        (trick::ALLSPIN, "All-Spin"), (trick::QUAD, "Quad"), (trick::B2B, "Back-to-Back"), (trick::COMBO, "Combo"),
    ];
    let tricks = NAMES.iter().filter(|(bit, _)| plan.tricks & bit != 0).map(|(_, n)| *n).collect::<Vec<_>>().join(", ");
    match (plan.stage.map(setup_name), tricks.is_empty()) {
        (Some(setup), true) => setup.to_string(),
        (Some(setup), false) => format!("{setup}: {tricks}"),
        (None, false) => tricks,
        (None, true) => "Stacking".into(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn update_next(queue: *const c_char) {
    STATE.lock().unwrap().queue = text(queue).iter().filter_map(|&c| piece(c)).collect();
}

#[unsafe(no_mangle)]
pub extern "C" fn update_current(p: *const c_char) {
    if let Some(p) = text(p).first().and_then(|&c| piece(c)) {
        STATE.lock().unwrap().current = p;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn update_hold(p: *const c_char) {
    STATE.lock().unwrap().hold = text(p).first().and_then(|&c| piece(c));
}

#[unsafe(no_mangle)]
pub extern "C" fn update_incoming(attack: i32) {
    STATE.lock().unwrap().incoming = attack.max(0) as u32;
}

#[unsafe(no_mangle)]
pub extern "C" fn update_combo(combo: i32) {
    STATE.lock().unwrap().combo = combo;
}

#[unsafe(no_mangle)]
pub extern "C" fn update_b2b(b2b: i32) {
    STATE.lock().unwrap().b2b = b2b.max(0) as u32;
}

#[unsafe(no_mangle)]
pub extern "C" fn update_field(f: *const c_char) {
    let (board, h) = field(text(f));
    let mut s = STATE.lock().unwrap();
    s.board = board;
    s.h = h;
}

#[unsafe(no_mangle)]
pub extern "C" fn update_reset() {
    let mut s = STATE.lock().unwrap();
    s.board = Board::default();
    s.hold = None;
    s.combo = 0;
    s.b2b = 0;
    s.incoming = 0;
    s.last_trick = 0;
}

#[unsafe(no_mangle)]
pub extern "C" fn alive() -> i32 {
    1
}

/// Best move as `inputs|gem|spin|b2b|nodes|depth|attack|x,y`, or `-1`.
#[unsafe(no_mangle)]
pub extern "C" fn action(_second_choice: i32, out: *mut c_char, len: i32) {
    write(out, len, &decide().unwrap_or_else(|| "-1".into()));
}

fn decide() -> Option<String> {
    let (board, h, current, hold, queue, combo, b2b, pending, can_hold, level, last_trick) = {
        let s = STATE.lock().unwrap();
        (s.board, s.h, s.current, s.hold, s.queue.clone(), s.combo, s.b2b, s.incoming, s.hold_allowed, s.level, s.last_trick)
    };
    // Only the 5 previews PPT shows: Zetris also predicts pieces past them from the game's RNG.
    let queue = &queue[..queue.len().min(5)];
    // ponytail: assumes the game's combo counts clears so far (ours is -1 for none); verify against PPT2 memory if combo play looks off.
    let root = Root { board, current, hold, can_hold, queue, b2b: b2b > 0, combo: combo - 1, pending, bag: 0 };
    let cfg = Config { last_trick, ..Config::difficulty(level, Objective::Attack) };
    let rules = rules(h, can_hold);
    let (p, plan) = suggest_with_plan(&root, &rules, &cfg)?;
    let (inputs, tricky) = route(&board, &rules, &p);

    let mut after = board;
    let lines = after.lock(p.piece, p.x, p.y, p.rot);
    let (mut b2b_on, mut combo) = (b2b > 0, combo - 1);
    let sent = attack(lines, p.spin, &mut b2b_on, &mut combo, lines > 0 && after.is_empty());
    let b2b = if lines == 0 { b2b } else if b2b_on { b2b + 1 } else { 0 };
    let kind = clear_kind(lines, p.spin, lines > 0 && after.is_empty(), p.all_spin.then_some(p.piece));
    {
        let mut s = STATE.lock().unwrap();
        if kind != 0 {
            s.last_trick = kind;
        }
        s.plan = describe(&plan);
    }

    let first = if p.piece != current { HOLD } else { NULL };
    Some(format!(
        "{}|{}|{}|{}|{}|{}|{}|{},{}",
        moves(first, &inputs),
        gem(p.piece),
        tricky as u8,
        b2b,
        plan.nodes,
        queue.len() + 1,
        sent,
        p.x - 1,
        h + p.y - 40,
    ))
}

/// Inputs from spawn to MisaMino position (x, y, r) as `inputs|spin`, or `0` if unreachable.
#[unsafe(no_mangle)]
pub extern "C" fn findpath(f: *const c_char, p: *const c_char, x: i32, y: i32, r: i32, hold: i32, out: *mut c_char, len: i32) {
    let (board, h) = field(text(f));
    let rules = rules(h, true);
    let found = text(p).first().and_then(|&c| piece(c)).and_then(|pc| {
        path_to(find_placements(&board, pc, &rules), pc, x as i8 + 1, (y + 40) as i8 - h, swap_rot(r))
    });
    write(out, len, &found.map_or("0".into(), |c| {
        let (inputs, tricky) = route(&board, &rules, &c);
        format!("{}|{}", moves(if hold != 0 { HOLD } else { NULL }, &inputs), tricky as u8)
    }));
}

/// `findpath` for a piece already at (sx, sy, sr) instead of spawn (Zetris recovering a misdrop).
#[unsafe(no_mangle)]
pub extern "C" fn findpathfrom(f: *const c_char, p: *const c_char, sx: i32, sy: i32, sr: i32, x: i32, y: i32, r: i32, out: *mut c_char, len: i32) {
    let (board, h) = field(text(f));
    let found = text(p).first().and_then(|&c| piece(c)).and_then(|pc| {
        let all = find_placements_from(&board, pc, &rules(h, true), (sx as i8 + 1, (sy + 40) as i8 - h, swap_rot(sr)));
        path_to(all, pc, x as i8 + 1, (y + 40) as i8 - h, swap_rot(r))
    });
    write(out, len, &found.map_or("0".into(), |c| format!("{}|1", moves(NULL, &exact(&c)))));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(f: impl FnOnce(*mut c_char, i32)) -> String {
        let mut buf = [0u8; 501];
        f(buf.as_mut_ptr() as *mut c_char, 500);
        CStr::from_bytes_until_nul(&buf).unwrap().to_str().unwrap().into()
    }

    /// Empty 21-row field, as Zetris encodes it.
    fn empty() -> std::ffi::CString {
        std::ffi::CString::new(vec!["0,0,0,0,0,0,0,0,0,0"; 21].join(";")).unwrap()
    }

    /// Targets the real MisaMino.dll accepts (its x/y/r): a flat T upside down at the left wall,
    /// and a vertical I on the floor; a floating T is unreachable, as in MisaMino.
    #[test]
    fn findpath_matches_misamino_coordinates() {
        let (f, t, i) = (empty(), c"T", c"I");
        assert_eq!(call(|o, n| findpath(f.as_ptr(), t.as_ptr(), 0, 19, 2, 0, o, n)), "0,8,8,1,1,1,9|0");
        assert_eq!(call(|o, n| findpath(f.as_ptr(), i.as_ptr(), 3, 18, 3, 1, o, n)), "10,8,9|0");
        assert_eq!(call(|o, n| findpath(f.as_ptr(), t.as_ptr(), 3, 19, 0, 0, o, n)), "0");
    }
}
