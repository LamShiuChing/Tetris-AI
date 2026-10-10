//! Real-time game for the Stackfall web app: gravity, lock delay, hold, garbage. It shares the
//! board, pieces, kicks and T-spin rule with the bot's simulator, so the two can't disagree on physics.

use std::collections::VecDeque;

use crate::board::{Board, HEIGHT};
use crate::game::{Bag, Rules, attack};
use crate::movegen::{Spin, t_spin};
use crate::piece::Piece;

/// Moves/rotations on the ground that refresh lock delay, per piece (reset by reaching a new lowest row).
const MAX_RESETS: u32 = 15;
/// Garbage rows that can enter on a single lock.
const GARBAGE_PER_LOCK: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Active {
    pub piece: Piece,
    pub x: i8,
    pub y: i8,
    pub rot: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Move,
    Rotate,
    SoftDrop,
    Hold,
    HardDrop { rows: u32 },
    /// `cleared` has bit y set for each row that was full before clearing.
    Lock { at: Active, lines: u32, spin: Spin, combo: i32, b2b: bool, perfect_clear: bool, sent: u32, cleared: u64 },
    GarbageIn { lines: u32 },
    TopOut,
}

pub struct Live {
    pub board: Board,
    /// Per cell, for rendering: 0 empty, 1..=7 piece (`Piece as u8 + 1`), 8 garbage.
    pub colors: [[u8; 10]; HEIGHT as usize],
    pub active: Option<Active>,
    pub hold: Option<Piece>,
    pub can_hold: bool,
    pub queue: VecDeque<Piece>,
    pub b2b: bool,
    pub combo: i32,
    /// Incoming garbage not yet on the board: (lines, hole column).
    pub pending: VecDeque<(u32, i8)>,
    pub over: bool,
    pub lines: u32,
    pub pieces: u32,
    pub sent: u32,
    /// Milliseconds per row of gravity; 0 means 20G.
    pub gravity_ms: f32,
    pub lock_delay_ms: f32,
    pub events: Vec<Event>,
    bag: Bag,
    rng: u64,
    fall: f32,
    lock: f32,
    resets: u32,
    lowest: i8,
    rotated: bool,
}

impl Live {
    /// `seed` picks the pieces (share it so both players get the same ones); `garbage_seed` picks hole columns.
    pub fn new(seed: u64, garbage_seed: u64, gravity_ms: f32, lock_delay_ms: f32) -> Self {
        let mut bag = Bag::new(seed);
        let queue = (0..Rules::versus().preview).map(|_| bag.next()).collect();
        let mut live = Live {
            board: Board::default(),
            colors: [[0; 10]; HEIGHT as usize],
            active: None,
            hold: None,
            can_hold: true,
            queue,
            b2b: false,
            combo: -1,
            pending: VecDeque::new(),
            over: false,
            lines: 0,
            pieces: 0,
            sent: 0,
            gravity_ms,
            lock_delay_ms,
            events: vec![],
            bag,
            rng: garbage_seed ^ 0xD1B5_4A32_D192_ED03 | 1,
            fall: 0.0,
            lock: 0.0,
            resets: 0,
            lowest: 0,
            rotated: false,
        };
        live.spawn_next();
        live
    }

    pub fn shift(&mut self, dx: i8) -> bool {
        let Some(a) = self.active else { return false };
        let moved = Active { x: a.x + dx, ..a };
        if !self.fits(moved) {
            return false;
        }
        self.rotated = false;
        self.moved(moved);
        self.events.push(Event::Move);
        true
    }

    pub fn rotate(&mut self, cw: bool) -> bool {
        let Some(a) = self.active else { return false };
        if a.piece == Piece::O {
            return false;
        }
        let rot = (a.rot + if cw { 1 } else { 3 }) % 4;
        let kicked = a.piece.kicks(a.rot, cw).iter().map(|&(dx, dy)| Active { x: a.x + dx, y: a.y + dy, rot, ..a });
        let Some(next) = kicked.into_iter().find(|&n| self.fits(n)) else { return false };
        self.rotated = true;
        self.moved(next);
        self.events.push(Event::Rotate);
        true
    }

    /// One row down; doesn't refresh lock delay.
    pub fn soft_drop(&mut self) -> bool {
        let Some(a) = self.active else { return false };
        if self.grounded(a) {
            return false;
        }
        self.fell(Active { y: a.y + 1, ..a });
        self.events.push(Event::SoftDrop);
        true
    }

    pub fn hard_drop(&mut self) {
        let Some(mut a) = self.active else { return };
        let start = a.y;
        while !self.grounded(a) {
            a.y += 1;
        }
        if a.y != start {
            self.rotated = false;
        }
        self.active = Some(a);
        self.events.push(Event::HardDrop { rows: (a.y - start) as u32 });
        self.lock_piece();
    }

    pub fn hold(&mut self) -> bool {
        let Some(a) = self.active else { return false };
        if !self.can_hold {
            return false;
        }
        self.can_hold = false;
        self.events.push(Event::Hold);
        match self.hold.replace(a.piece) {
            Some(held) => self.enter(held),
            None => self.spawn_next(),
        }
        true
    }

    /// Advances gravity and lock delay by `dt` milliseconds.
    pub fn tick(&mut self, dt: f32) {
        let Some(a) = self.active else { return };
        if self.grounded(a) {
            self.lock += dt;
            if self.lock >= self.lock_delay_ms {
                self.lock_piece();
            }
        } else if self.gravity_ms <= 0.0 {
            self.settle();
        } else {
            self.fall += dt;
            while self.fall >= self.gravity_ms {
                self.fall -= self.gravity_ms;
                let Some(a) = self.active.filter(|&a| !self.grounded(a)) else { break };
                self.fell(Active { y: a.y + 1, ..a });
            }
        }
    }

    /// Queues incoming garbage; it rises on this player's next lock that doesn't clear.
    pub fn receive(&mut self, lines: u32) {
        if lines > 0 {
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 7;
            self.rng ^= self.rng << 17;
            self.pending.push_back((lines, (self.rng % 10) as i8));
        }
    }

    pub fn pending_lines(&self) -> u32 {
        self.pending.iter().map(|g| g.0).sum()
    }

    /// Where a hard drop would land.
    pub fn ghost_y(&self) -> Option<i8> {
        let mut a = self.active?;
        while !self.grounded(a) {
            a.y += 1;
        }
        Some(a.y)
    }

    /// Lock delay used so far, 0..1.
    pub fn lock_progress(&self) -> f32 {
        (self.lock / self.lock_delay_ms).min(1.0)
    }

    fn fits(&self, a: Active) -> bool {
        !self.board.collides(a.piece, a.x, a.y, a.rot)
    }

    fn grounded(&self, a: Active) -> bool {
        self.board.collides(a.piece, a.x, a.y + 1, a.rot)
    }

    /// After a successful shift/rotate: refresh lock delay while resets remain, then apply 20G.
    fn moved(&mut self, a: Active) {
        self.active = Some(a);
        self.note_lowest(a);
        if self.grounded(a) && self.resets < MAX_RESETS {
            self.lock = 0.0;
            self.resets += 1;
        }
        if self.gravity_ms <= 0.0 {
            self.settle();
        }
    }

    fn fell(&mut self, a: Active) {
        self.active = Some(a);
        self.rotated = false;
        self.lock = 0.0;
        self.note_lowest(a);
    }

    fn note_lowest(&mut self, a: Active) {
        if a.y > self.lowest {
            self.lowest = a.y;
            self.resets = 0;
        }
    }

    fn settle(&mut self) {
        while let Some(a) = self.active.filter(|&a| !self.grounded(a)) {
            self.fell(Active { y: a.y + 1, ..a });
        }
    }

    fn spawn_next(&mut self) {
        let piece = self.queue.pop_front().unwrap();
        self.queue.push_back(self.bag.next());
        self.enter(piece);
    }

    fn enter(&mut self, piece: Piece) {
        let (x, y) = Rules::versus().spawn;
        let a = Active { piece, x, y, rot: 0 };
        self.fall = 0.0;
        self.lock = 0.0;
        self.resets = 0;
        self.lowest = y;
        self.rotated = false;
        if !self.fits(a) {
            self.active = None;
            self.over = true;
            self.events.push(Event::TopOut);
            return;
        }
        self.active = Some(a);
        if self.gravity_ms <= 0.0 {
            self.settle();
        }
    }

    fn lock_piece(&mut self) {
        let Some(a) = self.active.take() else { return };
        let spin = if a.piece == Piece::T && self.rotated { t_spin(&self.board, a.x, a.y, a.rot) } else { Spin::None };
        self.board.place(a.piece, a.x, a.y, a.rot);
        for (x, y) in a.piece.cells(a.x, a.y, a.rot) {
            if y >= 0 {
                self.colors[y as usize][x as usize] = a.piece as u8 + 1;
            }
        }
        let cleared = self.board.full_rows();
        let lines = self.board.clear_full();
        let mut dst = HEIGHT as usize;
        for src in (0..HEIGHT as usize).rev() {
            if cleared >> src & 1 == 0 {
                dst -= 1;
                self.colors[dst] = self.colors[src];
            }
        }
        self.colors[..dst].fill([0; 10]);
        let perfect_clear = lines > 0 && self.board.is_empty();
        let b2b_before = self.b2b;
        let mut out = attack(lines, spin, &mut self.b2b, &mut self.combo, perfect_clear);

        // Outgoing attack cancels incoming garbage first.
        while out > 0 {
            let Some(front) = self.pending.front_mut() else { break };
            let cancel = out.min(front.0);
            front.0 -= cancel;
            out -= cancel;
            if front.0 == 0 {
                self.pending.pop_front();
            }
        }
        self.lines += lines;
        self.pieces += 1;
        self.sent += out;
        let b2b = b2b_before && self.b2b && lines > 0;
        self.events.push(Event::Lock { at: a, lines, spin, combo: self.combo, b2b, perfect_clear, sent: out, cleared });

        if lines == 0 {
            let mut room = GARBAGE_PER_LOCK;
            let mut risen = 0;
            while let Some(front) = self.pending.front_mut().filter(|_| room > 0) {
                let n = front.0.min(room);
                if !self.board.push_garbage(n as usize, front.1) {
                    self.over = true;
                }
                let rows = n as usize;
                self.colors.copy_within(rows.., 0);
                let mut row = [8; 10];
                row[front.1 as usize] = 0;
                self.colors[HEIGHT as usize - rows..].fill(row);
                front.0 -= n;
                room -= n;
                risen += n;
                if front.0 == 0 {
                    self.pending.pop_front();
                }
            }
            if risen > 0 {
                self.events.push(Event::GarbageIn { lines: risen });
            }
            if self.over {
                self.events.push(Event::TopOut);
                return;
            }
        }
        self.can_hold = true;
        self.spawn_next();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gravity_and_lock_delay() {
        let mut g = Live::new(1, 1, 100.0, 500.0);
        let y0 = g.active.unwrap().y;
        g.tick(250.0);
        assert_eq!(g.active.unwrap().y, y0 + 2);
        g.hard_drop();
        assert_eq!(g.pieces, 1);
        // Next piece falls row by row, then sits for the full lock delay before locking.
        let rows = (g.ghost_y().unwrap() - g.active.unwrap().y) as f32;
        let mut t = 0.0;
        while g.pieces == 1 {
            g.tick(10.0);
            t += 10.0;
        }
        assert!((t - (rows * 100.0 + 500.0)).abs() <= 20.0, "locked after {t} ms for {rows} rows");
    }

    #[test]
    fn attack_cancels_then_garbage_rises() {
        let mut g = Live::new(2, 2, 1000.0, 500.0);
        g.receive(3);
        assert_eq!(g.pending_lines(), 3);
        g.hard_drop(); // no clear: garbage rises
        assert_eq!(g.pending_lines(), 0);
        assert!(g.events.contains(&Event::GarbageIn { lines: 3 }));
        assert_eq!(g.board.rows[37..].iter().filter(|&&r| r.count_ones() >= 9).count(), 3);
        for (y, row) in g.colors.iter().enumerate() {
            for (x, &c) in row.iter().enumerate() {
                assert_eq!(c != 0, g.board.occupied(x as i8, y as i8), "color/board mismatch at ({x}, {y})");
            }
        }
    }

    #[test]
    fn hold_once_per_piece() {
        let mut g = Live::new(3, 3, 1000.0, 500.0);
        let first = g.active.unwrap().piece;
        assert!(g.hold());
        assert!(!g.hold());
        assert_eq!(g.hold, Some(first));
        g.hard_drop();
        assert!(g.hold());
    }
}
