use std::collections::{HashMap, HashSet, VecDeque};

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

/// BFS over every input sequence from spawn; one result per distinct (cells, spin).
pub fn find_placements(board: &Board, piece: Piece, rules: &Rules) -> Vec<Placement> {
    let (x, y) = rules.spawn;
    if board.collides(piece, x, y, 0) {
        return vec![];
    }
    // ponytail: ignores IRS and the 15 lock-reset cap; BFS paths are short. Add both if 20G paths get cut off.
    let start = settle(board, piece, rules.gravity, State { x, y, rot: 0, rotated: false });
    let mut parent: HashMap<State, (State, Input)> = HashMap::new();
    let mut seen = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    let mut locked = HashSet::new();
    let mut out = vec![];

    while let Some(s) = queue.pop_front() {
        if board.collides(piece, s.x, s.y + 1, s.rot) {
            let spin = spin(board, piece, s);
            let mut cells = piece.cells(s.x, s.y, s.rot);
            cells.sort();
            if locked.insert((cells, spin)) {
                out.push(Placement { piece, x: s.x, y: s.y, rot: s.rot, spin, inputs: path(&parent, s) });
            }
        }
        for input in [Input::Left, Input::Right, Input::Cw, Input::Ccw, Input::SoftDrop] {
            if let Some(n) = step(board, piece, rules.gravity, s, input) {
                if seen.insert(n) {
                    parent.insert(n, (s, input));
                    queue.push_back(n);
                }
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

fn path(parent: &HashMap<State, (State, Input)>, mut s: State) -> Vec<Input> {
    let mut inputs = vec![];
    while let Some(&(p, input)) = parent.get(&s) {
        inputs.push(input);
        s = p;
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
