// Thin wrapper over the Rust engine's raw WebAssembly exports. Layouts are documented in src/wasm.rs.

export const PIECES = 'IJLOSTZ';

export interface Exports {
  memory: WebAssembly.Memory;
  live_new(slot: number, seed: number, gravityMs: number, lockDelayMs: number): void;
  live_input(slot: number, action: number): number;
  live_tick(slot: number, dtMs: number): void;
  live_garbage(slot: number, lines: number): void;
  live_set_gravity(slot: number, gravityMs: number): void;
  live_set_lock_delay(slot: number, ms: number): void;
  live_set_level(slot: number, level: number): void;
  live_snapshot(slot: number): number;
  live_colors(slot: number): number;
  live_events(slot: number): number;
  events_ptr(): number;
  piece_cells(piece: number, rot: number): number;
  board_ptr(): number;
  queue_ptr(): number;
  out_ptr(): number;
  plan(current: number, hold: number, canHold: number, queueLen: number, b2b: number, combo: number, difficulty: number, salt: number, flags: number): number;
}

export async function loadEngine(): Promise<Exports> {
  const bytes = await (await fetch('/engine.wasm')).arrayBuffer();
  const { instance } = await WebAssembly.instantiate(bytes);
  return instance.exports as unknown as Exports;
}

/** Engine input codes for `live_input`. */
export const Action = { Left: 0, Right: 1, Cw: 2, Ccw: 3, Soft: 4, Hard: 5, Hold: 6 } as const;
export type Action = (typeof Action)[keyof typeof Action];

export interface Snapshot {
  rows: Int32Array;
  /** 40 x 10, 0 empty, 1..7 piece index + 1, 8 garbage. */
  colors: Uint8Array;
  piece: number;
  x: number;
  y: number;
  rot: number;
  ghostY: number;
  hold: number;
  canHold: boolean;
  queue: number[];
  b2b: boolean;
  combo: number;
  pending: number;
  over: boolean;
  lockProgress: number;
  lines: number;
  pieces: number;
  sent: number;
  score: number;
  level: number;
}

export type GameEvent =
  | { kind: 'move' }
  | { kind: 'rotate' }
  | { kind: 'soft' }
  | { kind: 'hold' }
  | { kind: 'hard'; rows: number }
  | {
      kind: 'lock';
      /** Where the piece locked. */
      piece: number;
      x: number;
      y: number;
      rot: number;
      lines: number;
      spin: number;
      combo: number;
      b2b: boolean;
      pc: boolean;
      sent: number;
      /** Rows (pre-clear y) that were full. */
      cleared: number[];
    }
  | { kind: 'garbage'; lines: number }
  | { kind: 'topout' };

/** Cell offsets [dx, dy] of a piece at a rotation, straight from the engine's shape table. */
export function pieceCells(w: Exports, piece: number, rot: number): [number, number][] {
  const c = new Int8Array(w.memory.buffer, w.piece_cells(piece, rot), 8);
  return [0, 1, 2, 3].map((i) => [c[2 * i], c[2 * i + 1]]);
}

export class Player {
  constructor(private w: Exports, readonly slot: number) {}

  start(seed: number, gravityMs: number, lockDelayMs: number) {
    this.w.live_new(this.slot, seed, gravityMs, lockDelayMs);
  }

  input(a: Action): boolean {
    return this.w.live_input(this.slot, a) !== 0;
  }

  tick(dtMs: number) {
    this.w.live_tick(this.slot, dtMs);
  }

  garbage(lines: number) {
    this.w.live_garbage(this.slot, lines);
  }

  /** Mode physics: ms per row (0 = 20G), lock delay, and the level that multiplies score. */
  setPhysics(gravityMs: number, lockDelayMs: number, level: number) {
    this.w.live_set_gravity(this.slot, gravityMs);
    this.w.live_set_lock_delay(this.slot, lockDelayMs);
    this.w.live_set_level(this.slot, level);
  }

  snapshot(): Snapshot {
    const s = new Int32Array(this.w.memory.buffer, this.w.live_snapshot(this.slot), 62);
    const colors = new Uint8Array(this.w.memory.buffer, this.w.live_colors(this.slot), 400).slice();
    return {
      rows: s.slice(0, 40),
      colors,
      piece: s[40],
      x: s[41],
      y: s[42],
      rot: s[43],
      ghostY: s[44],
      hold: s[45],
      canHold: s[46] !== 0,
      queue: Array.from(s.slice(47, 52)),
      b2b: s[52] !== 0,
      combo: s[53],
      pending: s[54],
      over: s[55] !== 0,
      lockProgress: s[56] / 1000,
      lines: s[57],
      pieces: s[58],
      sent: s[59],
      score: s[60],
      level: s[61],
    };
  }

  events(): GameEvent[] {
    const n = this.w.live_events(this.slot);
    const e = new Int32Array(this.w.memory.buffer, this.w.events_ptr(), n * 8);
    const out: GameEvent[] = [];
    for (let i = 0; i < n; i++) {
      const [kind, a, b, c, d, f, lo, hi] = e.subarray(i * 8, i * 8 + 8);
      switch (kind) {
        case 1: out.push({ kind: 'move' }); break;
        case 2: out.push({ kind: 'rotate' }); break;
        case 3: out.push({ kind: 'soft' }); break;
        case 4: out.push({ kind: 'hold' }); break;
        case 5: out.push({ kind: 'hard', rows: a }); break;
        case 6: {
          const cleared: number[] = [];
          for (let y = 0; y < 40; y++) {
            if ((y < 32 ? (lo >>> y) & 1 : (hi >>> (y - 32)) & 1) === 1) cleared.push(y);
          }
          out.push({
            kind: 'lock',
            piece: (d >> 2) & 7,
            rot: (d >> 5) & 3,
            x: ((d >> 7) & 15) - 2,
            y: ((d >> 12) & 63) - 4,
            lines: a,
            spin: b,
            combo: c,
            b2b: (d & 1) !== 0,
            pc: (d & 2) !== 0,
            sent: f,
            cleared,
          });
          break;
        }
        case 7: out.push({ kind: 'garbage', lines: a }); break;
        case 8: out.push({ kind: 'topout' }); break;
      }
    }
    return out;
  }
}
