// Runs the bot's search off the main thread so planning never stalls a frame.
import { loadEngine } from './engine';

export interface PlanRequest {
  id: number;
  rows: Int32Array;
  current: number;
  hold: number;
  canHold: boolean;
  queue: number[];
  b2b: boolean;
  combo: number;
  difficulty: number;
  /** Varies the bot's mistakes so two bots at one level don't mirror each other. */
  salt: number;
  /** Plan as if every action falls to the floor (20G, or fast gravity the bot sonic-drops through). */
  twentyG: boolean;
  /** Solo score modes optimize points; versus optimizes attack. */
  solo: boolean;
  /** Incoming garbage lines. */
  pending: number;
  /** Pieces left in the current bag after the queue. */
  bag: number;
}

/** [piece, x, y, rot, spin, n, inputs...] or empty when every placement tops out. */
export interface PlanResponse {
  id: number;
  out: number[];
}

const engine = loadEngine();
/** The worker global, typed via the DOM lib's Worker (its postMessage takes no target origin). */
const ctx = self as unknown as Worker;

ctx.onmessage = async (e: MessageEvent<PlanRequest>) => {
  const r = e.data;
  try {
    ctx.postMessage({ id: r.id, out: plan(await engine, r) } satisfies PlanResponse);
  } catch {
    ctx.postMessage({ id: r.id, out: [] } satisfies PlanResponse); // the bot hard-drops instead of hanging
  }
};

function plan(w: Awaited<typeof engine>, r: PlanRequest): number[] {
  new Uint16Array(w.memory.buffer, w.board_ptr(), 40).set(r.rows);
  new Uint8Array(w.memory.buffer, w.queue_ptr(), r.queue.length).set(r.queue);
  const len = w.plan(r.current, r.hold < 0 ? 7 : r.hold, r.canHold ? 1 : 0, r.queue.length, r.b2b ? 1 : 0, r.combo, r.difficulty, r.salt, (r.twentyG ? 1 : 0) | (r.solo ? 2 : 0), r.pending, r.bag);
  return Array.from(new Int8Array(w.memory.buffer, w.out_ptr(), len));
}
