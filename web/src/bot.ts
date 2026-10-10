// Drives a Player with the bot: asks the worker for a plan when a new piece appears, then
// presses the planned inputs at a human-like pace for its difficulty.
import { Action, type Player, type Snapshot } from './engine';
import type { PlanRequest, PlanResponse } from './bot.worker';

/** Pace per difficulty. Beginners are slow, press keys slowly, and never use hold. */
export const LEVELS = [
  { name: 'Pebble', pps: 0.35, inputMs: 220, hold: false },
  { name: 'Sprout', pps: 0.5, inputMs: 180, hold: false },
  { name: 'Rookie', pps: 0.7, inputMs: 140, hold: true },
  { name: 'Hopper', pps: 0.9, inputMs: 110, hold: true },
  { name: 'Ace', pps: 1.1, inputMs: 85, hold: true },
  { name: 'Hotshot', pps: 1.4, inputMs: 65, hold: true },
  { name: 'Veteran', pps: 1.8, inputMs: 45, hold: true },
  { name: 'Master', pps: 2.3, inputMs: 32, hold: true },
  { name: 'Legend', pps: 3.0, inputMs: 22, hold: true },
  { name: 'Titan', pps: 4.5, inputMs: 12, hold: true },
];

/** One worker shared by every bot; requests are matched to replies by id. */
export class Planner {
  private worker = new Worker(new URL('./bot.worker.ts', import.meta.url), { type: 'module' });
  private waiting = new Map<number, (out: number[]) => void>();
  private nextId = 0;

  constructor() {
    this.worker.onmessage = (e: MessageEvent<PlanResponse>) => {
      this.waiting.get(e.data.id)?.(e.data.out);
      this.waiting.delete(e.data.id);
    };
  }

  plan(req: Omit<PlanRequest, 'id'>): Promise<number[]> {
    const id = this.nextId++;
    return new Promise((resolve) => {
      this.waiting.set(id, resolve);
      this.worker.postMessage({ ...req, id });
    });
  }
}

/** Sonic drop: soft drop all the way, without locking. */
const SONIC = -1;

/** The mode's current physics, which decide how the bot plans and paces itself. */
export interface Physics {
  gravityMs: number;
  lockDelayMs: number;
  solo: boolean;
}

/** Below this many ms per row, plan as 20G and sonic-drop after every move so the plan holds. */
const FAST_GRAVITY_MS = 150;

export class Bot {
  /** Where the bot intends to put the current piece, for the "show plan" overlay. */
  target: { piece: number; x: number; y: number; rot: number } | null = null;
  private plannedFor = -1;
  private thinking = false;
  private actions: number[] = [];
  private wait = 0;
  private gap = 0;
  private salt = (Math.random() * 2 ** 31) | 0;

  constructor(
    private player: Player,
    private planner: Planner,
    public level: number,
  ) {}

  update(dtMs: number, snap: Snapshot, physics: Physics) {
    if (snap.over || snap.piece < 0) return;
    // Changes on every new piece and after a hold.
    const key = snap.pieces * 2 + (snap.canHold ? 0 : 1);
    if (key !== this.plannedFor) {
      if (!this.thinking) this.request(snap, key, physics);
      return;
    }
    if (this.thinking) return;
    this.wait -= dtMs;
    while (this.wait <= 0 && this.actions.length > 0) {
      const a = this.actions.shift()!;
      if (a === SONIC) {
        while (this.player.input(Action.Soft)) {}
        continue;
      }
      this.player.input(a as Action);
      if (a === Action.Hold || a === Action.Hard) return;
      this.wait += this.gap;
    }
  }

  private async request(snap: Snapshot, key: number, physics: Physics) {
    this.thinking = true;
    this.plannedFor = key;
    this.target = null;
    const twentyG = physics.gravityMs < FAST_GRAVITY_MS;
    const out = await this.planner.plan({
      rows: snap.rows,
      current: snap.piece,
      hold: snap.hold,
      canHold: snap.canHold && LEVELS[this.level - 1].hold,
      queue: snap.queue,
      b2b: snap.b2b,
      combo: snap.combo,
      difficulty: this.level,
      salt: this.salt,
      twentyG,
      solo: physics.solo,
    });
    this.thinking = false;
    if (key !== this.plannedFor) return;
    if (out.length === 0) {
      this.actions = [Action.Hard];
      return;
    }
    const [piece, x, y, rot] = out;
    if (piece !== snap.piece) {
      this.actions = [Action.Hold]; // the next update replans for the swapped-in piece
      return;
    }
    this.target = { piece, x, y, rot };
    const inputs = out.slice(6).map((i) => (i === 4 ? SONIC : i));
    const { pps, inputMs } = LEVELS[this.level - 1];
    if (twentyG) {
      // The piece is already on the stack and lock delay is running: no thinking pause, and
      // keep every gap well inside the lock delay. Sonic drops make fast gravity act like 20G.
      this.actions = [SONIC, ...inputs.flatMap((i) => [i, SONIC]), Action.Hard];
      this.gap = Math.min(inputMs, physics.lockDelayMs * 0.4);
      this.wait = 0;
    } else {
      // Spread the piece over 1/pps seconds: think first, then move.
      this.actions = [...inputs, Action.Hard];
      this.gap = inputMs;
      this.wait = Math.max(0, 1000 / pps - inputs.length * inputMs);
    }
  }
}
