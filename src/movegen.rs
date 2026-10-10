use crate::board::Board;
use crate::game::{Gravity, Rules};
use crate::piece::Piece;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    Left,
    Right,
    Cw,
    Ccw,
    /// Drop to the floor without locking.
    SoftDrop,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Spin {
    None,
    Mini,
    Full,
}

/// A reachable lock position and the inputs from spawn that reach it.
/// `spin` assumes the piece locks via lock delay: MonochromeRush's hard drop
/// sets `lastAction = 'drop'`, which voids the T-spin.
#[derive(Clone, Debug)]
pub struct Placement {
    pub piece: Piece,
    pub x: i8,
    pub y: i8,
    pub rot: u8,
    pub spin: Spin,
    pub inputs: Vec<Input>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct State {
    x: i8,
    y: i8,
    rot: u8,
    /// Last action was a rotation that didn't fall afterwards (T-spin eligible).
    rotated: bool,
}

/// BFS over every input sequence from spawn; one result per distinct (cells, spin), each with the
/// inputs that reach it.
pub fn find_placements(board: &Board, piece: Piece, rules: &Rules) -> Vec<Placement> {
    search(board, piece, rules, true)
}

/// `find_placements` without the input sequences (left empty): the search only needs those for
/// the move it actually plays, and building them for every node is most of the cost.
pub fn find_placements_fast(board: &Board, piece: Piece, rules: &Rules) -> Vec<Placement> {
    search(board, piece, rules, false)
}

// Every reachable state fits in a small array: pivot x in -3..13, y in -4..40, 4 rotations,
// rotated or not. Arrays instead of hash maps keep the BFS allocation-free and cache-friendly.
const XS: usize = 16;
const YS: usize = 44;
const STATES: usize = XS * YS * 4 * 2;

fn index(s: State) -> Option<usize> {
    let (x, y) = (s.x as i32 + 3, s.y as i32 + 4);
    ((0..XS as i32).contains(&x) && (0..YS as i32).contains(&y))
        .then(|| (((x as usize * YS + y as usize) * 4 + s.rot as usize) * 2) + s.rotated as usize)
}

fn search(board: &Board, piece: Piece, rules: &Rules, want_inputs: bool) -> Vec<Placement> {
    let (x, y) = rules.spawn;
    if board.collides(piece, x, y, 0) {
        return vec![];
    }
    // ponytail: ignores IRS and the 15 lock-reset cap; BFS paths are short. Add both if 20G paths get cut off.
    let start = settle(board, piece, rules.gravity, State { x, y, rot: 0, rotated: false });
    let Some(start_i) = index(start) else { return vec![] };
    let mut seen = [false; STATES];
    // Only filled in when `want_inputs`: (parent state index, input that led here).
    let mut parent = if want_inputs { vec![(u16::MAX, Input::Left); STATES] } else { vec![] };
    let mut queue: Vec<State> = Vec::with_capacity(256);
    let mut locked: Vec<u64> = Vec::with_capacity(64);
    let mut out = vec![];
    seen[start_i] = true;
    queue.push(start);

    let mut head = 0;
    while let Some(&s) = queue.get(head) {
        head += 1;
        if board.collides(piece, s.x, s.y + 1, s.rot) {
            let spin = spin(board, piece, s);
            let mut cells = piece.cells(s.x, s.y, s.rot);
            cells.sort();
            let key = cells.iter().fold(spin as u64, |k, &(cx, cy)| k << 10 | ((cx as u64) << 6 | (cy as i32 + 4) as u64));
            if !locked.contains(&key) {
                locked.push(key);
                let inputs = if want_inputs { path(&parent, &queue, s) } else { vec![] };
                out.push(Placement { piece, x: s.x, y: s.y, rot: s.rot, spin, inputs });
            }
        }
        let si = index(s).unwrap() as u16;
        for input in [Input::Left, Input::Right, Input::Cw, Input::Ccw, Input::SoftDrop] {
            if let Some(n) = step(board, piece, rules.gravity, s, input)
                && let Some(ni) = index(n)
                && !seen[ni]
            {
                seen[ni] = true;
                if want_inputs {
                    parent[ni] = (si, input);
                }
                queue.push(n);
            }
        }
    }
    out
}

fn step(board: &Board, piece: Piece, gravity: Gravity, s: State, input: Input) -> Option<State> {
    let moved = match input {
        Input::Left | Input::Right => {
            let x = s.x + if input == Input::Left { -1 } else { 1 };
            if board.collides(piece, x, s.y, s.rot) {
                return None;
            }
            State { x, rotated: false, ..s }
        }
        Input::Cw | Input::Ccw => rotate(board, piece, s, input == Input::Cw)?,
        Input::SoftDrop => {
            let d = drop(board, piece, s);
            if d.y == s.y {
                return None;
            }
            d
        }
    };
    Some(settle(board, piece, gravity, moved))
}

fn rotate(board: &Board, piece: Piece, s: State, cw: bool) -> Option<State> {
    if piece == Piece::O {
        return None;
    }
    let rot = (s.rot + if cw { 1 } else { 3 }) % 4;
    piece
        .kicks(s.rot, cw)
        .iter()
        .map(|&(dx, dy)| State { x: s.x + dx, y: s.y + dy, rot, rotated: true })
        .find(|n| !board.collides(piece, n.x, n.y, n.rot))
}

fn drop(board: &Board, piece: Piece, s: State) -> State {
    let mut y = s.y;
    while !board.collides(piece, s.x, y + 1, s.rot) {
        y += 1;
    }
    if y == s.y { s } else { State { y, rotated: false, ..s } }
}

/// 20G pulls the piece to the floor after every action.
fn settle(board: &Board, piece: Piece, gravity: Gravity, s: State) -> State {
    match gravity {
        Gravity::Zero => s,
        Gravity::TwentyG => drop(board, piece, s),
    }
}

fn spin(board: &Board, piece: Piece, s: State) -> Spin {
    if piece != Piece::T || !s.rotated {
        return Spin::None;
    }
    t_spin(board, s.x, s.y, s.rot)
}

/// 3-corner rule for a T at this pose whose last move was a rotation, mirroring MonochromeRush `Game.isTSpin`.
pub(crate) fn t_spin(board: &Board, x: i8, y: i8, rot: u8) -> Spin {
    let c = [(-1, -1), (1, -1), (-1, 1), (1, 1)].map(|(dx, dy)| board.occupied(x + dx, y + dy));
    if c.iter().filter(|&&b| b).count() < 3 {
        return Spin::None;
    }
    let (a, b) = match rot {
        0 => (c[0], c[1]),
        1 => (c[1], c[3]),
        2 => (c[2], c[3]),
        _ => (c[0], c[2]),
    };
    if a && b { Spin::Full } else { Spin::Mini }
}

fn path(parent: &[(u16, Input)], _queue: &[State], s: State) -> Vec<Input> {
    let mut inputs = vec![];
    let mut i = index(s).unwrap();
    while parent[i].0 != u16::MAX {
        inputs.push(parent[i].1);
        i = parent[i].0 as usize;
    }
    inputs.reverse();
    inputs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(gravity: Gravity) -> Rules {
        Rules { gravity, ..Rules::monochrome_rush(1) }
    }

    #[test]
    fn empty_board_counts() {
        for gravity in [Gravity::Zero, Gravity::TwentyG] {
            for (piece, n) in [(Piece::I, 17), (Piece::O, 9), (Piece::T, 34), (Piece::S, 17), (Piece::Z, 17), (Piece::J, 34), (Piece::L, 34)] {
                assert_eq!(find_placements(&Board::default(), piece, &rules(gravity)).len(), n, "{piece:?} {gravity:?}");
            }
        }
    }

    /// The fast variant finds exactly the same placements, just without input sequences, and the
    /// full variant's inputs really lead to each placement.
    #[test]
    fn fast_matches_full() {
        let mut board = Board::default();
        board.rows[39] = 0x3FF & !(1 << 1);
        board.rows[38] = 0x3FF & !0b111;
        board.rows[37] = 1;
        for gravity in [Gravity::Zero, Gravity::TwentyG] {
            for piece in Piece::ALL {
                let full = find_placements(&board, piece, &rules(gravity));
                let fast = find_placements_fast(&board, piece, &rules(gravity));
                let key = |v: &[Placement]| v.iter().map(|p| (p.x, p.y, p.rot, p.spin)).collect::<Vec<_>>();
                assert_eq!(key(&full), key(&fast), "{piece:?} {gravity:?}");
                assert!(fast.iter().all(|p| p.inputs.is_empty()));
            }
        }
    }

    #[test]
    fn twenty_g_cannot_climb_wall() {
        let mut board = Board::default();
        for y in 30..40 {
            board.rows[y] = 1 << 6;
        }
        let right_of_wall = |g| {
            find_placements(&board, Piece::T, &rules(g))
                .iter()
                .any(|p| p.piece.cells(p.x, p.y, p.rot).iter().any(|&(x, _)| x > 6))
        };
        assert!(right_of_wall(Gravity::Zero));
        assert!(!right_of_wall(Gravity::TwentyG));
    }

    #[test]
    fn finds_tsd() {
        let mut board = Board::default();
        board.rows[39] = 0x3FF & !(1 << 1);
        board.rows[38] = 0x3FF & !0b111;
        board.rows[37] = 1;
        for gravity in [Gravity::Zero, Gravity::TwentyG] {
            let p = find_placements(&board, Piece::T, &rules(gravity))
                .into_iter()
                .find(|p| p.spin == Spin::Full)
                .unwrap_or_else(|| panic!("no T-spin under {gravity:?}"));
            assert_eq!((p.x, p.y, p.rot), (1, 38, 2));
            assert_eq!(board.clone().lock(p.piece, p.x, p.y, p.rot), 2);
        }
    }
}
