// A 1v1 match: two engine players (human or bot), garbage routed between them as flying orbs,
// gravity that speeds up over time, countdown and K.O.
import { Container, Text } from 'pixi.js';
import { audio } from './audio';
import { Bot, LEVELS, type Planner } from './bot';
import { Player, pieceCells, type Exports, type GameEvent, type Snapshot } from './engine';
import { FONT, type Fx } from './fx';
import type { Controls } from './input';
import { BoardView, CELL, COLORS, GARBAGE, H, W, type Textures } from './view';

export type Side = { kind: 'human' } | { kind: 'bot'; level: number };

export interface MatchConfig {
  sides: [Side, Side];
  /** Draw each bot's intended placement (watch mode). */
  showPlan: boolean;
  /** Time scale, for watching bots faster or slower. */
  speed: number;
}

const BOARD_X = [200, 800];
const BOARD_Y = 96;
const LOCK_DELAY = 500;
const COUNTDOWN = 3000;
const SPIN_NAMES = ['', 'SINGLE', 'DOUBLE', 'TRIPLE'];
const CLEAR_NAMES = ['', 'SINGLE', 'DOUBLE', 'TRIPLE', 'QUAD'];

export class Match extends Container {
  onEnd: (winner: number) => void = () => {};
  private players: Player[];
  private views: BoardView[];
  private bots: (Bot | null)[];
  private snaps: Snapshot[];
  private top = new Container();
  private banner: Text;
  private clock: Text;
  private state: 'countdown' | 'playing' | 'over' = 'countdown';
  private t = 0;
  private heartbeat = 0;

  constructor(
    private engine: Exports,
    tex: Textures,
    private fx: Fx,
    planner: Planner,
    private controls: Controls,
    private cfg: MatchConfig,
  ) {
    super();
    const seed = (Math.random() * 2 ** 31) | 0;
    this.players = [0, 1].map((slot) => {
      const p = new Player(engine, slot);
      p.start(seed, 1000, LOCK_DELAY); // same seed: both players get the same pieces
      return p;
    });
    this.views = cfg.sides.map((side, i) => {
      const name = side.kind === 'human' ? 'YOU' : `${LEVELS[side.level - 1].name.toUpperCase()} · LV ${side.level}`;
      const v = new BoardView(engine, tex, name, i === 0 ? 0x5fd3ff : 0xff5fa2);
      v.position.set(BOARD_X[i], BOARD_Y);
      this.addChild(v);
      return v;
    });
    this.bots = cfg.sides.map((side, i) => (side.kind === 'bot' ? new Bot(this.players[i], planner, side.level) : null));
    this.snaps = this.players.map((p) => p.snapshot());

    const vs = new Text({ text: 'VS', style: { fontFamily: FONT, fontWeight: '800', fontSize: 44, fill: 0xffffff, stroke: { color: 0x1a1036, width: 8 } } });
    vs.anchor.set(0.5);
    vs.position.set(640, BOARD_Y + H / 2);
    this.clock = new Text({ text: '0:00', style: { fontFamily: FONT, fontWeight: '800', fontSize: 26, fill: 0xcfc6ff, stroke: { color: 0x1a1036, width: 6 } } });
    this.clock.anchor.set(0.5);
    this.clock.position.set(640, 40);
    this.banner = new Text({ text: '', style: { fontFamily: FONT, fontWeight: '800', fontSize: 120, fill: 0xffffff, stroke: { color: 0x1a1036, width: 16 }, dropShadow: { color: 0x000000, alpha: 0.5, distance: 8, blur: 4, angle: Math.PI / 2 } } });
    this.banner.anchor.set(0.5);
    this.banner.position.set(640, 360);
    this.addChild(vs, this.clock, this.top, this.banner);
    controls.reset();
    this.views.forEach((v, i) => v.render(this.snaps[i], null, 0));
  }

  update(realDtMs: number) {
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
      }
      return;
    }
    if (this.state === 'over') return;

    // Gravity ramps from 1 s/row toward ~25 ms/row over a few minutes, forcing the match to end.
    const gravity = Math.max(25, 1000 * 0.9 ** (this.t / 10000));
    const seconds = Math.floor(this.t / 1000);
    this.clock.text = `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;

    for (let i = 0; i < 2; i++) {
      const p = this.players[i];
      p.setGravity(gravity);
      const bot = this.bots[i];
      if (bot) bot.update(dt, this.snaps[i]);
      else this.controls.update(realDtMs, p, (k, ok) => ok && this.inputSound(k, i));
      p.tick(dt);
    }
    for (let i = 0; i < 2; i++) {
      const before = this.snaps[i];
      this.snaps[i] = this.players[i].snapshot();
      for (const e of this.players[i].events()) this.onEvent(i, e, before);
      const pps = this.snaps[i].pieces / Math.max(this.t / 1000, 1);
      this.views[i].render(this.snaps[i], this.cfg.showPlan ? (this.bots[i]?.target ?? null) : null, pps);
    }

    const humanIdx = this.cfg.sides.findIndex((s) => s.kind === 'human');
    if (humanIdx >= 0 && this.views[humanIdx].isDanger) {
      this.heartbeat -= realDtMs;
      if (this.heartbeat <= 0) {
        audio.danger();
        this.heartbeat = 800;
      }
    }

    const dead = this.snaps.map((s) => s.over);
    if (dead.some(Boolean)) this.finish(dead[0] && dead[1] ? -1 : dead[0] ? 1 : 0);
  }

  private pan(i: number) {
    return i === 0 ? -0.5 : 0.5;
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
        this.fx.popup(v.fx, e.spin === 2 ? 'T-SPIN' : 'MINI T-SPIN', W / 2, H * 0.35, { size: 30, color });
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

    const center = cells.reduce((a, c) => ({ x: a.x + c.x / 4, y: a.y + c.y / 4 }), { x: 0, y: 0 });
    const big = e.lines >= 4 || e.spin === 2 || e.pc;
    const name = e.spin === 2 ? `T-SPIN ${SPIN_NAMES[e.lines]}` : e.spin === 1 ? `MINI T-SPIN ${SPIN_NAMES[e.lines]}` : CLEAR_NAMES[e.lines];
    if (e.lines >= 2 || e.spin) this.fx.popup(v.fx, name, W / 2, H * 0.35, { size: big ? 40 : 30, color: e.spin ? 0xff8ce0 : e.lines >= 4 ? 0xffe066 : 0xffffff });
    if (e.b2b) this.fx.popup(v.fx, 'BACK-TO-BACK', W / 2, H * 0.35 - 44, { size: 20, color: 0x9ff7ff, delay: 80 });
    if (e.combo >= 1) this.fx.popup(v.fx, `${e.combo} REN`, W / 2, H * 0.35 + 46, { size: 24 + Math.min(e.combo, 10) * 2, color: 0xffb347, delay: 60 });
    if (big) {
      this.fx.ring(v.fx, center.x, center.y, e.spin ? 0xff8ce0 : 0xffe066);
      this.fx.burst(v.fx, center.x, center.y, color, 30, 420);
      v.shake(e.pc ? 14 : 8);
    }
    if (e.pc) {
      this.fx.popup(v.fx, 'PERFECT CLEAR', W / 2, H * 0.55, { size: 46, color: 0x7dffb2, hold: 1200 });
      for (let k = 0; k < 6; k++) this.fx.burst(v.fx, Math.random() * W, Math.random() * H, COLORS[k], 20, 380);
    }

    if (e.sent > 0) {
      const j = 1 - i;
      audio.send(e.sent, pan);
      const from = this.toLocal(v.inner.toGlobal(center));
      const target = this.views[j];
      const to = this.toLocal(target.inner.toGlobal({ x: -13, y: H - 20 }));
      this.fx.popup(v.fx, `+${e.sent}`, center.x, center.y - 30, { size: 26, color: 0xff6b8b, hold: 300 });
      this.fx.orb(this.top, from, to, i === 0 ? 0x5fd3ff : 0xff5fa2, e.sent, () => {
        if (this.state !== 'playing') return;
        this.players[j].garbage(e.sent);
        target.shake(3);
      });
    }
  }

  private finish(winner: number) {
    this.state = 'over';
    audio.stopMusic();
    const humanIdx = this.cfg.sides.findIndex((s) => s.kind === 'human');
    const text = winner < 0 ? 'DRAW' : humanIdx < 0 ? (winner === 0 ? 'LEFT WINS' : 'RIGHT WINS') : winner === humanIdx ? 'YOU WIN!' : 'K.O.';
    this.banner.text = text;
    this.banner.alpha = 1;
    this.banner.scale.set(0.2);
    this.fx.tween(500, (p) => this.banner.scale.set(0.2 + 0.8 * (1 + 2.7 * (p - 1) ** 3 + 1.7 * (p - 1) ** 2)));
    if (humanIdx >= 0 && winner !== humanIdx) audio.lose();
    else audio.win();
    if (winner >= 0) {
      const loser = this.views[1 - winner];
      loser.shake(18);
      this.fx.tween(900, (p) => (loser.alpha = 1 - 0.6 * p));
      const w = this.views[winner];
      for (let k = 0; k < 5; k++) this.fx.burst(w.fx, Math.random() * W, Math.random() * H * 0.6, COLORS[k], 25, 400);
    }
    setTimeout(() => this.onEnd(winner), 1800);
  }
}
