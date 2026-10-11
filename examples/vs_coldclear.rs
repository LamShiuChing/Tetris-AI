//! Titan vs Cold Clear 2 (https://github.com/MinusKelvin/cold-clear-2, built in external/), with
//! Cold Clear talking the Tetris Bot Protocol over stdin/stdout. Our engine referees: same rules,
//! garbage and simultaneous turns as every other versus test. Each match is saved as a replay
//! the web app can play back.
//! `cargo run --release --example vs_coldclear -- [matches] [cold clear ms per move] [titan level]`
//! Env: NET_WEIGHT (Titan's value-net blend), WIDTH (Titan's beam width), SEED0 (first seed, default 77000), REPLAY_DIR
//! (default web/public/replays — point evaluation runs elsewhere so they don't replace those).
//! Each move also records, for the replay's analysis overlay: the cells it lands on (`c`), think
//! time (`ms`), positions searched (`n`), the value net's read of the board after it (`v`, -1..1),
//! and for Titan the line it planned (`l`: piece and cells per placement).

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tetris_ai::bot::{Config, Objective, Root, suggest_with_plan, tslot_kind};
use tetris_ai::game::Rules;
use tetris_ai::live::Live;
use tetris_ai::movegen::{Input, Placement, Spin, find_placements};
use tetris_ai::net::{features, value_net};
use tetris_ai::piece::Piece;
use tetris_ai::versus::{Brain, Turn, play_brains};

const CC_EXE: &str = "external/cold-clear-2/target/release/cold-clear-2.exe";
const LETTERS: &[u8; 7] = b"IJLOSTZ";

struct ColdClear {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    think: Duration,
    /// Suggestions none of which matched a legal placement in our engine (should stay 0).
    unmatched: u32,
    /// Search stats of the last move, for the replay.
    info: Rc<RefCell<Value>>,
}

/// Titan (`BotBrain`), keeping what it planned and searched for the replay.
struct Titan {
    cfg: Config,
    info: Rc<RefCell<Value>>,
}

impl Brain for Titan {
    fn choose(&mut self, g: &Live, recent_tricks: u32, recent_stages: &[u32]) -> Option<(Placement, Option<u32>)> {
        let active = g.active?;
        let queue: Vec<_> = g.queue.iter().copied().collect();
        let root = Root { board: g.board, current: active.piece, hold: g.hold, can_hold: g.can_hold, queue: &queue, b2b: g.b2b, combo: g.combo, pending: g.pending_lines(), bag: g.bag_mask() };
        let cfg = Config { recent_tricks, recent_stages: recent_stages.to_vec(), ..self.cfg.clone() };
        let t = Instant::now();
        let (p, plan) = suggest_with_plan(&root, &Rules::versus(), &cfg)?;
        let line: Vec<Value> = plan.line.iter().map(|&(piece, x, y, rot)| json!({ "p": letter(piece), "c": cells(piece, x, y, rot) })).collect();
        *self.info.borrow_mut() = json!({ "ms": (t.elapsed().as_secs_f64() * 1000.0).round(), "n": plan.nodes, "l": line });
        Some((p, plan.stage))
    }
}

fn cells(piece: Piece, x: i8, y: i8, rot: u8) -> Value {
    json!(piece.cells(x, y, rot).map(|(x, y)| [x, y]))
}

impl ColdClear {
    fn spawn(think: Duration) -> Self {
        let mut child = Command::new(CC_EXE)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("build Cold Clear 2 in external/cold-clear-2 first");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut cc = ColdClear { child, stdin, stdout, think, unmatched: 0, info: Rc::default() };
        assert_eq!(cc.recv()["type"], "info");
        cc.send(json!({ "type": "rules" }));
        assert_eq!(cc.recv()["type"], "ready");
        cc
    }

    fn send(&mut self, v: Value) {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn recv(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("bad message from Cold Clear: {line:?} ({e})"))
    }
}

impl Drop for ColdClear {
    fn drop(&mut self) {
        self.send(json!({ "type": "quit" }));
        let _ = self.child.wait();
    }
}

fn letter(p: Piece) -> String {
    (LETTERS[p as usize] as char).to_string()
}

/// TBP's piece center relative to our pivot (y down). SRS centers are our pivots except for I and O.
fn center_offset(piece: Piece, rot: u8) -> (i8, i8) {
    match piece {
        Piece::I => [(0, 0), (1, 0), (1, 1), (0, 1)][rot as usize],
        Piece::O => [(0, 0), (0, -1), (1, -1), (1, 0)][rot as usize],
        _ => (0, 0),
    }
}

impl Brain for ColdClear {
    fn choose(&mut self, g: &Live, _: u32, _: &[u32]) -> Option<(Placement, Option<u32>)> {
        let active = g.active?;
        // Fresh position every move (garbage can change the board between moves); Titan also
        // searches from scratch each move.
        let board: Vec<Vec<Value>> = (0..40)
            .map(|up| {
                g.colors[39 - up]
                    .iter()
                    .map(|&c| match c {
                        0 => Value::Null,
                        8 => json!("G"),
                        p => json!(letter(Piece::ALL[p as usize - 1])),
                    })
                    .collect()
            })
            .collect();
        let queue: Vec<String> = std::iter::once(active.piece).chain(g.queue.iter().copied()).map(letter).collect();
        let bag: Vec<String> = Piece::ALL.into_iter().filter(|&p| g.bag_mask() & 1 << p as u8 != 0).map(letter).collect();
        self.send(json!({ "type": "stop" }));
        self.send(json!({
            "type": "start",
            "board": board,
            "queue": queue,
            "hold": g.hold.map(letter),
            "combo": (g.combo + 1).max(0),
            "back_to_back": g.b2b,
            "randomizer": { "type": "seven_bag", "bag_state": bag },
        }));
        std::thread::sleep(self.think);
        self.send(json!({ "type": "suggest" }));
        let msg = loop {
            let m = self.recv();
            if m["type"] == "suggestion" {
                break m;
            }
        };
        *self.info.borrow_mut() = json!({ "ms": self.think.as_millis(), "n": msg["move_info"]["nodes"] });
        let allowed = [Some(active.piece), if g.can_hold { Some(g.hold.unwrap_or(g.queue[0])) } else { None }];
        for mv in msg["moves"].as_array().into_iter().flatten() {
            if let Some(p) = to_placement(g, mv, &allowed) {
                return Some((p, None));
            }
        }
        self.unmatched += 1;
        let top = (0..40).find(|&y| g.board.rows[y] != 0).map_or(0, |y| 40 - y);
        eprintln!("unmatched: active {:?} hold {:?} stack {top} rows, moves {}", active.piece, g.hold, msg["moves"]);
        None
    }
}

/// A TBP move as one of our engine's placements: same piece and same four cells (preferring the
/// same spin), so a mismatch in coordinate conventions can't silently play something else.
fn to_placement(g: &Live, mv: &Value, allowed: &[Option<Piece>]) -> Option<Placement> {
    let loc = &mv["location"];
    let c = loc["type"].as_str()?.bytes().next()?;
    let piece = Piece::ALL[LETTERS.iter().position(|&l| l == c)?];
    if !allowed.contains(&Some(piece)) {
        return None;
    }
    let rot = ["north", "east", "south", "west"].iter().position(|&o| loc["orientation"] == o)? as u8;
    let (dx, dy) = center_offset(piece, rot);
    let (x, y) = (loc["x"].as_i64()? as i8 - dx, 39 - loc["y"].as_i64()? as i8 - dy);
    let mut want = piece.cells(x, y, rot);
    want.sort();
    let spin = match mv["spin"].as_str() {
        Some("full") => Spin::Full,
        Some("mini") => Spin::Mini,
        _ => Spin::None,
    };
    let mut candidates: Vec<Placement> = find_placements(&g.board, piece, &Rules::versus())
        .into_iter()
        .filter(|p| {
            let mut cells = p.piece.cells(p.x, p.y, p.rot);
            cells.sort();
            cells == want
        })
        .collect();
    candidates.sort_by_key(|p| p.spin != spin);
    candidates.into_iter().next()
}

fn inputs_text(p: &Placement) -> String {
    p.inputs
        .iter()
        .map(|i| match i {
            Input::Left => 'L',
            Input::Right => 'R',
            Input::Cw => 'C',
            Input::Ccw => 'W',
            Input::SoftDrop => 'D',
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let matches: u64 = args.get(1).map_or(10, |s| s.parse().unwrap());
    let think = Duration::from_millis(args.get(2).map_or(100, |s| s.parse().unwrap()));
    let level: u32 = args.get(3).map_or(10, |s| s.parse().unwrap());
    let titan_name = if level >= 10 { "Titan".to_string() } else { format!("Level {level}") };
    let mut cc = ColdClear::spawn(think);
    let mut titan_cfg = Config::difficulty(level, Objective::Attack);
    if let Ok(w) = std::env::var("NET_WEIGHT") {
        titan_cfg.net_weight = w.parse().unwrap();
    }
    if let Ok(w) = std::env::var("WIDTH") {
        titan_cfg.width = w.parse().unwrap();
    }
    let seed0: u64 = std::env::var("SEED0").map_or(77_000, |v| v.parse().unwrap());
    let dir = std::env::var("REPLAY_DIR").unwrap_or_else(|_| "web/public/replays".into());
    std::fs::create_dir_all(&dir).unwrap();
    let mut titan = Titan { cfg: titan_cfg, info: Rc::default() };
    let (mut titan_wins, mut cc_wins, mut draws) = (0, 0, 0);
    let mut index = vec![];
    for m in 0..matches {
        let seed = seed0 + m;
        // Alternate seats; record each turn's moves for the replay.
        let titan_seat = (m % 2) as usize;
        let names = if titan_seat == 0 { [titan_name.clone(), "Cold Clear 2".into()] } else { ["Cold Clear 2".into(), titan_name.clone()] };
        let mut turns: Vec<[Value; 2]> = vec![];
        let (titan_info, cc_info) = (titan.info.clone(), cc.info.clone());
        let mut record = |t: Turn| {
            // No placement means no legal move: the referee hard-dropped the piece where it was.
            let mut mv = t.placement.map_or(json!({ "h": false, "i": "" }), |p| json!({ "h": t.held, "i": inputs_text(p), "c": cells(p.piece, p.x, p.y, p.rot) }));
            if t.placement.is_some() {
                let info = if t.player == titan_seat { &titan_info } else { &cc_info };
                for (k, v) in info.borrow().as_object().unwrap() {
                    mv[k] = v.clone();
                }
                let (g, slot) = (t.game, tslot_kind(&t.game.board));
                let v = value_net().unwrap().eval(&features(&g.board, g.b2b, g.combo, g.pending_lines(), slot == Some(2), slot == Some(3)));
                mv["v"] = json!((v as f64 * 100.0).round() / 100.0);
            }
            if t.player == 0 || turns.is_empty() {
                turns.push([Value::Null, Value::Null]);
            }
            turns.last_mut().unwrap()[t.player] = mv;
        };
        let r = if titan_seat == 0 {
            play_brains([&mut titan, &mut cc], seed, 500, &mut record)
        } else {
            play_brains([&mut cc, &mut titan], seed, 500, &mut record)
        };
        let winner = r.winner.map(|w| names[w].clone());
        match r.winner {
            Some(w) if w == titan_seat => titan_wins += 1,
            Some(_) => cc_wins += 1,
            None => draws += 1,
        }
        println!(
            "match {m}: {} vs {} -> {}  ({} pieces, sent {} vs {})",
            names[0],
            names[1],
            winner.as_deref().unwrap_or("draw"),
            r.pieces,
            r.sent[0],
            r.sent[1]
        );
        let file = format!("titan-vs-coldclear-{m}.json");
        let replay = json!({ "seed": seed, "names": names, "winner": winner, "turns": turns });
        std::fs::write(format!("{dir}/{file}"), replay.to_string()).unwrap();
        index.push(json!({ "file": file, "names": names, "winner": winner, "pieces": r.pieces }));
    }
    std::fs::write(format!("{dir}/index.json"), Value::Array(index).to_string()).unwrap();
    println!(
        "\n{titan_name} {titan_wins} - {cc_wins} Cold Clear 2 ({draws} draws) over {matches} matches; Cold Clear thinks {} ms/move; unmatched Cold Clear moves: {}",
        think.as_millis(),
        cc.unmatched
    );
}
