// A match: versus (two players, garbage as flying orbs, gravity rising with time) or a solo
// score mode (Marathon / Master) for one player. Each side is a human or a bot.
import { Container, Text } from 'pixi.js';
import { audio } from './audio';
import { Bot, LEVELS, type Physics, type Planner } from './bot';
import { Action, Player, pieceCells, setupName, type Exports, type GameEvent, type Snapshot } from './engine';
import { FONT, textResolution, type Fx } from './fx';
import type { Controls } from './input';
import { BoardView, CELL, COLORS, GARBAGE, H, W, type Textures } from './view';

export type Side = { kind: 'human' } | { kind: 'bot'; level: number } | { kind: 'replay' };
export type Mode = 'versus' | 'marathon' | 'master' | 'replay';

/** A recorded match (examples/vs_coldclear.rs): seed + each turn's moves for both sides. */
export interface Replay {
  seed: number;
  names: [string, string];
  winner: string | null;
  /** Per turn, per side: hold first?, and inputs (L/R move, C/W rotate, D drop to floor). */
  turns: ({ h: boolean; i: string } | null)[][];
}

export interface MatchConfig {
  mode: Mode;
  /** Two sides for versus, one for solo modes. */
  sides: Side[];
  /** Draw each bot's intended placement (watch mode). */
  showPlan: boolean;
  /** Time scale, for watching bots faster or slower. */
  speed: number;
  replay?: Replay;
}

interface ModeRules {
  title: string;
  /** Lines to finish; 0 = play until someone tops out. */
  goal: number;
  gravityMs(level: number, ms: number): number;
  lockDelayMs(level: number): number;
}

export const MODES: Record<Mode, ModeRules> = {
  // Gravity ramps from 1 s/row toward ~25 ms/row over a few minutes, forcing a winner.
  versus: { title: 'VERSUS', goal: 0, gravityMs: (_, ms) => Math.max(25, 1000 * 0.9 ** (ms / 10000)), lockDelayMs: () => 500 },
  // Classic climb: every 10 lines is a level, each level falls ~20% faster.
  marathon: { title: 'MARATHON', goal: 150, gravityMs: (lv) => Math.max(16, 1000 * 0.8 ** (lv - 1)), lockDelayMs: () => 500 },
  // 20G from the first piece; lock delay tightens from 500 ms to 200 ms by level 30.
  master: { title: 'MASTER', goal: 300, gravityMs: () => 0, lockDelayMs: (lv) => Math.max(200, 500 - (lv - 1) * 10.5) },
  // Recorded matches were played without gravity: pieces move only by the recorded inputs.
  replay: { title: 'REPLAY', goal: 0, gravityMs: () => 1e9, lockDelayMs: () => 500 },
};

/** Replay pacing: ms between a recorded input and the next. */
const REPLAY_STEP_MS = 45;

const BOARD_Y = 96;
const COUNTDOWN = 3000;
const SPIN_NAMES = ['', 'SINGLE', 'DOUBLE', 'TRIPLE'];
const CLEAR_NAMES = ['', 'SINGLE', 'DOUBLE', 'TRIPLE', 'QUAD'];

/** End-of-match numbers for one side, for the results screen. */
export interface SideStats {
  name: string;
  pieces: number;
  pps: number;
  apm: number;
  sent: number;
  lines: number;
  maxCombo: number;
  tspins: number;
  quads: number;
  pcs: number;
  score: number;
}

export interface MatchResult {
  winner: number;
  title: string;
  detail: string;
  mode: Mode;
  /** Index of the human side, or -1. */
  human: number;
  seconds: number;
  stats: SideStats[];
}

export class Match extends Container {
  onEnd: (r: MatchResult) => void = () => {};
  private endTimer = 0;
  private rules: ModeRules;
  private players: Player[];
  private views: BoardView[];
  private bots: (Bot | null)[];
  private snaps: Snapshot[];
  private physics: Physics[];
  private top = new Container();
  private banner: Text;
  private clock: Text;
  private state: 'countdown' | 'playing' | 'over' = 'countdown';
  private t = 0;
  private heartbeat = 0;
  /** Replay playback: next turn, each side's pending actions, and attack to deliver at turn end. */
  private replayTurn = 0;
  private replayActions: (() => void)[][] = [[], []];
  private replayWait = 0;
  private turnAttack = [0, 0];
  /** Per board, the callouts for the latest clear; a new clear hides them so they never pile up. */
  private callouts: Text[][] = [[], []];
  /** Per side: biggest combo, T-spin clears, quads and perfect clears so far. */
  private extra = [0, 1].map(() => ({ maxCombo: 0, tspins: 0, quads: 0, pcs: 0 }));
  private names: string[] = [];
  /** Solo side panel: mode, score, level, lines, time. */
  private hud: Text[] = [];
  paused = false;

  constructor(
    private engine: Exports,
    tex: Textures,
    private fx: Fx,
    planner: Planner,
    private controls: Controls,
    private cfg: MatchConfig,
  ) {
    super();
    this.rules = MODES[cfg.mode];
    const solo = cfg.sides.length === 1;
    const seed = cfg.replay?.seed ?? (Math.random() * 2 ** 31) | 0;
    this.players = cfg.sides.map((_, slot) => {
      const p = new Player(engine, slot);
      p.start(seed, this.rules.gravityMs(1, 0), this.rules.lockDelayMs(1)); // same seed: same pieces for everyone
      return p;
    });
    const xs = solo ? [500] : [200, 800];
    this.views = cfg.sides.map((side, i) => {
      const name =
        side.kind === 'human'
          ? 'YOU'
          : side.kind === 'replay'
            ? cfg.replay!.names[i].toUpperCase()
            : side.level > 10
              ? LEVELS[side.level - 1].name.toUpperCase() // named personality
              : `${LEVELS[side.level - 1].name.toUpperCase()} · LV ${side.level}`;
      this.names[i] = name;
      const v = new BoardView(engine, tex, name, i === 0 ? 0x5fd3ff : 0xff5fa2);
      v.position.set(xs[i], BOARD_Y);
      this.addChild(v);
      return v;
    });
    this.bots = cfg.sides.map((side, i) => (side.kind === 'bot' ? new Bot(this.players[i], planner, side.level) : null));
    this.snaps = this.players.map((p) => p.snapshot());
    this.physics = this.players.map(() => ({ gravityMs: this.rules.gravityMs(1, 0), lockDelayMs: this.rules.lockDelayMs(1), solo }));

    const style = { fontFamily: FONT, fontWeight: '800' as const, fill: 0xffffff, stroke: { color: 0x1a1036, width: 8 } };
    const text = (t: string, size: number, fill: number, x: number, y: number, ax = 0.5) => {
      const o = new Text({ text: t, resolution: textResolution(), style: { ...style, fontSize: size, fill, stroke: { color: 0x1a1036, width: Math.max(4, size / 5) } } });
      o.anchor.set(ax, 0.5);
      o.position.set(x, y);
      this.addChild(o);
      return o;
    };
    if (solo) {
      // Side panel right of the NEXT box, clear of the spawning piece.
      const x = 930;
      text(this.rules.title, 34, 0xffd93d, x, BOARD_Y + 14, 0);
      text('SCORE', 16, 0xcfc6ff, x, BOARD_Y + 70, 0);
      text('LEVEL', 16, 0xcfc6ff, x, BOARD_Y + 150, 0);
      text('LINES', 16, 0xcfc6ff, x, BOARD_Y + 220, 0);
      text('TIME', 16, 0xcfc6ff, x, BOARD_Y + 290, 0);
      this.hud = [102, 182, 252, 322].map((dy) => text('', 30, 0xffffff, x, BOARD_Y + dy, 0));
      this.clock = this.hud[3];
    } else {
      text('VS', 44, 0xffffff, 640, BOARD_Y + H / 2);
      this.clock = text('0:00', 26, 0xcfc6ff, 640, 40);
    }
    this.banner = new Text({ text: '', resolution: textResolution(), style: { ...style, fontSize: 120, stroke: { color: 0x1a1036, width: 16 }, dropShadow: { color: 0x000000, alpha: 0.5, distance: 8, blur: 4, angle: Math.PI / 2 } } });
    this.banner.anchor.set(0.5);
    this.banner.position.set(640, 360);
    this.addChild(this.top, this.banner);
    controls.reset();
    this.views.forEach((v, i) => v.render(this.snaps[i], null, this.statsText(i)));
  }

  /** Freeze the match (pause menu, window lost focus). */
  setPaused(paused: boolean) {
    if (this.paused === paused || this.state === 'over') return;
    this.paused = paused;
    if (paused) audio.stopMusic();
    else if (this.state === 'playing') audio.startMusic();
    this.controls.reset();
  }

  /** The results screen covers the canvas; drop the big banner so it doesn't show through. */
  hideBanner() {
    this.banner.visible = false;
  }

  update(realDtMs: number) {
    if (this.paused) return;
    const dt = realDtMs * this.cfg.speed;
    this.views.forEach((v) => v.update(dt));
    this.t += dt;

    if (this.state === 'countdown') {
      const n = Math.ceil((COUNTDOWN - this.t) / 1000);
      const text = n > 0 ? String(n) : 'GO!';
      if (this.banner.text !== text) {
        this.banner.text = text;
        this.banner.scale.set(1.6);
        audio.countdown(n <= 0);
      }
      this.banner.scale.set(1 + (this.banner.scale.x - 1) * Math.exp(-dt / 60));
      if (this.t >= COUNTDOWN) {
        this.state = 'playing';
        this.t = 0;
        this.fx.tween(400, (p) => (this.banner.alpha = 1 - p), () => {
          this.banner.text = '';
          this.banner.alpha = 1;
        });
        audio.startMusic();
        this.controls.reset(); // drop keys pressed during the countdown
      }
      return;
    }
    if (this.state === 'over') return;

    this.clock.text = clockText(this.t);
    if (this.hud.length) {
      const s = this.snaps[0];
      this.hud[0].text = s.score.toLocaleString();
      this.hud[1].text = String(this.level(0));
      this.hud[2].text = `${Math.min(s.lines, this.rules.goal)} / ${this.rules.goal}`;
    }

    for (let i = 0; i < this.players.length; i++) {
      const p = this.players[i];
      const level = this.level(i);
      const phys = this.physics[i];
      phys.gravityMs = this.rules.gravityMs(level, this.t);
      phys.lockDelayMs = this.rules.lockDelayMs(level);
      p.setPhysics(phys.gravityMs, phys.lockDelayMs, level);
      const bot = this.bots[i];
      if (bot) bot.update(dt, this.snaps[i], phys);
      else if (!this.cfg.replay) this.controls.update(realDtMs, p, (k, ok) => ok && this.inputSound(k, i));
      p.tick(dt);
    }
    if (this.cfg.replay) this.playReplay(dt);
    for (let i = 0; i < this.players.length; i++) {
      const before = this.snaps[i];
      this.snaps[i] = this.players[i].snapshot();
      for (const e of this.players[i].events()) this.onEvent(i, e, before);
      if (this.level(i) > before.level && this.rules.goal) {
        this.fx.popup(this.views[i].fx, `LEVEL ${this.level(i)}`, W / 2, H * 0.2, { size: 34, color: 0x7dffb2 });
        audio.levelUp();
      }
      this.views[i].render(this.snaps[i], this.cfg.showPlan ? (this.bots[i]?.target ?? null) : null, this.statsText(i));
      const goal = this.bots[i]?.goal ?? null;
      this.views[i].setGoal(goal === null ? '' : `→ ${setupName(this.engine, goal)}`);
    }

    const humanIdx = this.cfg.sides.findIndex((s) => s.kind === 'human');
    if (humanIdx >= 0 && this.views[humanIdx].isDanger) {
      this.heartbeat -= realDtMs;
      if (this.heartbeat <= 0) {
        audio.danger();
        this.heartbeat = 800;
      }
    }

    if (this.cfg.sides.length === 1) {
      const s = this.snaps[0];
      if (s.over) this.finish(-1);
      else if (s.lines >= this.rules.goal) this.finish(0);
    } else {
      const dead = this.snaps.map((s) => s.over);
      if (dead.some(Boolean)) this.finish(dead[0] && dead[1] ? -1 : dead[0] ? 1 : 0);
    }
  }

  private level(i: number) {
    return this.rules.goal ? 1 + Math.floor(this.snaps[i].lines / 10) : 1;
  }

  private statsText(i: number) {
    const s = this.snaps[i];
    const pps = s.pieces / Math.max(this.t / 1000, 1);
    if (!this.rules.goal) return `SENT ${s.sent}  ·  LINES ${s.lines}  ·  ${pps.toFixed(2)} PPS`;
    return `${pps.toFixed(2)} PIECES / SEC`;
  }

  private pan(i: number) {
    return this.players.length === 1 ? 0 : i === 0 ? -0.5 : 0.5;
  }

  private inputSound(k: string, i: number) {
    if (k === 'left' || k === 'right') audio.move(this.pan(i));
    else if (k === 'cw' || k === 'ccw') audio.rotate(this.pan(i));
  }

  private onEvent(i: number, e: GameEvent, before: Snapshot) {
    const v = this.views[i];
    const pan = this.pan(i);
    const human = this.cfg.sides[i].kind === 'human';
    switch (e.kind) {
      case 'rotate':
        if (!human) audio.rotate(pan);
        break;
      case 'hold':
        audio.hold(pan);
        break;
      case 'hard': {
        audio.hardDrop(e.rows, pan);
        v.kick(Math.min(60 + e.rows * 14, 320));
        if (before.piece >= 0) {
          for (const [dx, dy] of pieceCells(this.engine, before.piece, before.rot)) {
            const top = (before.y + dy - 20) * CELL;
            const bottom = (before.ghostY + dy - 20) * CELL;
            this.fx.streak(v.fx, (before.x + dx) * CELL, top, bottom, CELL, COLORS[before.piece]);
          }
        }
        break;
      }
      case 'lock':
        this.onLock(i, e, before);
        break;
      case 'garbage':
        audio.receive(e.lines, pan);
        v.kick(-120 - e.lines * 20);
        v.shake(4 + e.lines * 1.5);
        break;
    }
  }

  private onLock(i: number, e: Extract<GameEvent, { kind: 'lock' }>, before: Snapshot) {
    const v = this.views[i];
    const pan = this.pan(i);
    const color = COLORS[e.piece];
    const cells = pieceCells(this.engine, e.piece, e.rot).map(([dx, dy]) => v.cellCenter(e.x + dx, e.y + dy));
    for (const c of cells) this.fx.flashRect(v.fx, c.x - CELL / 2, c.y - CELL / 2, CELL, CELL, 0xffffff, 160);

    if (e.lines === 0) {
      if (e.spin) {
        audio.spinNoClear(pan);
        this.callouts[i].forEach((t) => (t.visible = false));
        this.callouts[i] = [this.fx.popup(v.fx, e.spin === 2 ? 'T-SPIN' : 'MINI T-SPIN', W / 2, H * 0.35, { size: 30, color })];
      } else {
        audio.lock(pan);
      }
      return;
    }

    audio.clear(e.lines, e.combo, e.spin, e.b2b, e.pc, pan);
    for (const y of e.cleared) {
      const rowY = (y - 20) * CELL;
      this.fx.flashRect(v.fx, 0, rowY, W, CELL);
      for (let x = 0; x < 10; x++) {
        const code = before.colors[y * 10 + x];
        const c = code === 8 ? GARBAGE : code ? COLORS[code - 1] : color;
        this.fx.burst(v.fx, x * CELL + CELL / 2, rowY + CELL / 2, c, 3, 220);
      }
    }

    const ex = this.extra[i];
    ex.maxCombo = Math.max(ex.maxCombo, e.combo);
    ex.tspins += e.spin === 2 ? 1 : 0;
    ex.quads += e.lines === 4 ? 1 : 0;
    ex.pcs += e.pc ? 1 : 0;
    const center = cells.reduce((a, c) => ({ x: a.x + c.x / 4, y: a.y + c.y / 4 }), { x: 0, y: 0 });
    const big = e.lines >= 4 || e.spin === 2 || e.pc;
    const name = e.spin === 2 ? `T-SPIN ${SPIN_NAMES[e.lines]}` : e.spin === 1 ? `MINI T-SPIN ${SPIN_NAMES[e.lines]}` : CLEAR_NAMES[e.lines];
    const callouts: Text[] = [];
    if (e.lines >= 2 || e.spin) callouts.push(this.fx.popup(v.fx, name, W / 2, H * 0.35, { size: big ? 40 : 30, color: e.spin ? 0xff8ce0 : e.lines >= 4 ? 0xffe066 : 0xffffff, maxWidth: W - 12 }));
    if (e.b2b) callouts.push(this.fx.popup(v.fx, 'BACK-TO-BACK', W / 2, H * 0.35 - 44, { size: 20, color: 0x9ff7ff, delay: 80 }));
    if (e.combo >= 1) callouts.push(this.fx.popup(v.fx, `${e.combo} REN`, W / 2, H * 0.35 + 46, { size: 24 + Math.min(e.combo, 10) * 2, color: 0xffb347, delay: 60 }));
    if (callouts.length) {
      this.callouts[i].forEach((t) => (t.visible = false)); // their tweens still clean them up
      this.callouts[i] = callouts;
    }
    if (big) {
      this.fx.ring(v.fx, center.x, center.y, e.spin ? 0xff8ce0 : 0xffe066);
      this.fx.burst(v.fx, center.x, center.y, color, 30, 420);
      v.shake(e.pc ? 14 : 8);
    }
    if (e.pc) {
      this.fx.popup(v.fx, 'PERFECT CLEAR', W / 2, H * 0.55, { size: 46, color: 0x7dffb2, hold: 1200, maxWidth: W - 12 });
      for (let k = 0; k < 6; k++) this.fx.burst(v.fx, Math.random() * W, Math.random() * H, COLORS[k], 20, 380);
    }

    if (e.sent > 0 && this.players.length === 2) {
      const j = 1 - i;
      audio.send(e.sent, pan);
      const from = this.toLocal(v.inner.toGlobal(center));
      const target = this.views[j];
      const to = this.toLocal(target.inner.toGlobal({ x: -13, y: H - 20 }));
      this.fx.popup(v.fx, `+${e.sent}`, center.x, center.y - 30, { size: 26, color: 0xff6b8b, hold: 300 });
      // Replays deliver garbage at the end of the turn, exactly as the recorded match did.
      if (this.cfg.replay) this.turnAttack[i] += e.sent;
      this.fx.orb(this.top, from, to, i === 0 ? 0x5fd3ff : 0xff5fa2, e.sent, () => {
        if (this.state !== 'playing') return;
        if (!this.cfg.replay) this.players[j].garbage(e.sent);
        target.shake(3);
      });
    }
  }

  override destroy(options?: Parameters<Container['destroy']>[0]) {
    clearTimeout(this.endTimer);
    super.destroy(options);
  }

  /** Lockstep playback: both sides play turn t's recorded inputs, then each receives what the other
   * sent, then turn t+1 — the same order the referee used, so the engine reproduces the match. */
  private playReplay(dt: number) {
    const replay = this.cfg.replay!;
    this.replayWait -= dt;
    while (this.replayWait <= 0 && this.state === 'playing') {
      if (this.replayActions.every((q) => q.length === 0)) {
        if (this.replayTurn > 0) {
          this.players[1].garbage(this.turnAttack[0]);
          this.players[0].garbage(this.turnAttack[1]);
          this.turnAttack = [0, 0];
        }
        const turn = replay.turns[this.replayTurn++];
        if (!turn) {
          // The recorded match ended in a K.O. before this, so getting here means the engine
          // didn't reproduce it (or the match went the distance); fall back to the recorded winner.
          const w = replay.names.findIndex((n) => n === replay.winner);
          this.finish(w);
          return;
        }
        turn.forEach((mv, i) => {
          if (!mv) return;
          const p = this.players[i];
          const q: (() => void)[] = mv.h ? [() => p.input(Action.Hold)] : [];
          for (const c of mv.i) {
            if (c === 'D') q.push(() => {
              while (p.input(Action.Soft)) {}
            });
            else q.push(() => p.input({ L: Action.Left, R: Action.Right, C: Action.Cw, W: Action.Ccw }[c as 'L' | 'R' | 'C' | 'W']));
          }
          q.push(() => p.input(Action.Hard));
          this.replayActions[i] = q;
        });
      }
      for (const q of this.replayActions) q.shift()?.();
      this.replayWait += REPLAY_STEP_MS;
    }
  }

  /** `winner`: side index, or -1 for a draw / solo game over. Solo goal reached = winner 0. */
  private finish(winner: number) {
    this.state = 'over';
    audio.stopMusic();
    const solo = this.players.length === 1;
    const humanIdx = this.cfg.sides.findIndex((s) => s.kind === 'human');
    const time = this.clock.text;
    let title: string;
    let detail = '';
    if (solo) {
      const s = this.snaps[0];
      title = winner === 0 ? 'COMPLETE!' : 'GAME OVER';
      detail = `${this.rules.title} · ${s.score.toLocaleString()} pts · ${Math.min(s.lines, this.rules.goal)} lines · ${time}`;
    } else {
      const names = this.cfg.replay?.names;
      title =
        winner < 0 ? 'DRAW' : names ? `${names[winner].toUpperCase()} WINS` : humanIdx < 0 ? (winner === 0 ? 'LEFT BOT WINS' : 'RIGHT BOT WINS') : winner === humanIdx ? 'YOU WIN!' : 'YOU LOSE';
      detail = time;
    }
    this.banner.text = solo ? title : winner < 0 ? 'DRAW' : humanIdx >= 0 && winner !== humanIdx ? 'K.O.' : 'WIN!';
    this.banner.alpha = 1;
    this.banner.scale.set(0.2);
    this.fx.tween(500, (p) => this.banner.scale.set(0.2 + 0.8 * (1 + 2.7 * (p - 1) ** 3 + 1.7 * (p - 1) ** 2)));
    const happy = solo ? winner === 0 : humanIdx < 0 || winner === humanIdx;
    if (happy) audio.win();
    else audio.lose();
    if (winner >= 0) {
      const w = this.views[winner];
      for (let k = 0; k < 5; k++) this.fx.burst(w.fx, Math.random() * W, Math.random() * H * 0.6, COLORS[k], 25, 400);
      if (!solo) {
        const loser = this.views[1 - winner];
        loser.shake(18);
        this.fx.tween(900, (p) => (loser.alpha = 1 - 0.6 * p));
      }
    } else {
      this.views.forEach((v) => (v.shake(18), this.fx.tween(900, (p) => (v.alpha = 1 - 0.6 * p))));
    }
    const minutes = Math.max(this.t / 60000, 1 / 60);
    const stats: SideStats[] = this.snaps.map((s, i) => ({
      name: this.names[i],
      pieces: s.pieces,
      pps: s.pieces / (minutes * 60),
      apm: s.sent / minutes,
      sent: s.sent,
      lines: s.lines,
      ...this.extra[i],
      score: s.score,
    }));
    const mode = this.cfg.mode;
    this.endTimer = window.setTimeout(() => this.onEnd({ winner, title, detail, mode, human: humanIdx, seconds: this.t / 1000, stats }), 1800);
  }
}

export function clockText(ms: number) {
  const seconds = Math.floor(ms / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}
