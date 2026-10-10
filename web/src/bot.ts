// Drives a Player with the bot: asks the worker for a plan when a new piece appears, then
// presses the planned inputs at a human-like pace for its difficulty.
import { Action, type Player, type Snapshot } from './engine';
import type { PlanRequest, PlanResponse } from './bot.worker';

export const LEVELS = [
  { name: 'Pebble', pps: 0.7, inputMs: 120 },
  { name: 'Sprout', pps: 0.9, inputMs: 100 },
  { name: 'Rookie', pps: 1.1, inputMs: 85 },
  { name: 'Hopper', pps: 1.4, inputMs: 70 },
  { name: 'Ace', pps: 1.7, inputMs: 58 },
  { name: 'Hotshot', pps: 2.1, inputMs: 46 },
  { name: 'Veteran', pps: 2.5, inputMs: 36 },
  { name: 'Master', pps: 3.0, inputMs: 28 },
  { name: 'Legend', pps: 3.6, inputMs: 20 },
  { name: 'Titan', pps: 4.5, inputMs: 12 },
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

export class Bot {
  /** Where the bot intends to put the current piece, for the "show plan" overlay. */
  target: { piece: number; x: number; y: number; rot: number } | null = null;
  private plannedFor = -1;
  private thinking = false;
  private actions: number[] = [];
  private wait = 0;
  private salt = (Math.random() * 2 ** 31) | 0;

  constructor(
    private player: Player,
    private planner: Planner,
    public level: number,
  ) {}

  update(dtMs: number, snap: Snapshot) {
    if (snap.over || snap.piece < 0) return;
    // Changes on every new piece and after a hold.
    const key = snap.pieces * 2 + (snap.canHold ? 0 : 1);
    if (key !== this.plannedFor) {
      if (!this.thinking) this.request(snap, key);
      return;
    }
    if (this.thinking) return;
    this.wait -= dtMs;
    while (this.wait <= 0 && this.actions.length > 0) {
      const a = this.actions.shift()!;
      if (a === SONIC) {
        while (this.player.input(Action.Soft)) {}
      } else {
        this.player.input(a as Action);
      }
      if (a === Action.Hold || a === Action.Hard) return;
      this.wait += LEVELS[this.level - 1].inputMs;
    }
  }

  private async request(snap: Snapshot, key: number) {
    this.thinking = true;
    this.plannedFor = key;
    this.target = null;
    const out = await this.planner.plan({
      rows: snap.rows,
      current: snap.piece,
      hold: snap.hold,
      canHold: snap.canHold,
      queue: snap.queue,
      b2b: snap.b2b,
      combo: snap.combo,
      difficulty: this.level,
      salt: this.salt,
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
    this.actions = [...inputs, Action.Hard];
    // Spread the piece over 1/pps seconds: think first, then move.
    const { pps, inputMs } = LEVELS[this.level - 1];
    this.wait = Math.max(0, 1000 / pps - inputs.length * inputMs);
  }
}
