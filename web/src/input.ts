// Keyboard + gamepad with DAS/ARR auto-repeat, feeding one Player.
import { Action, type Player } from './engine';

export interface Handling {
  /** Delay before auto-repeat starts, ms. */
  das: number;
  /** Auto-repeat interval, ms; 0 = instant to the wall. */
  arr: number;
  /** Soft drop interval per row, ms; 0 = instant to the floor. */
  sdf: number;
}

export const DEFAULT_HANDLING: Handling = { das: 130, arr: 30, sdf: 25 };

type Key = 'left' | 'right' | 'soft' | 'hard' | 'cw' | 'ccw' | 'hold';

const KEYS: Record<string, Key> = {
  ArrowLeft: 'left',
  ArrowRight: 'right',
  ArrowDown: 'soft',
  Space: 'hard',
  ArrowUp: 'cw',
  KeyX: 'cw',
  KeyZ: 'ccw',
  ControlLeft: 'ccw',
  KeyC: 'hold',
  ShiftLeft: 'hold',
};

// W3C "standard" gamepad mapping: 0-3 face buttons, 4/5 shoulders, 12-15 d-pad up/down/left/right.
const PAD: [number, Key][] = [
  [14, 'left'],
  [15, 'right'],
  [13, 'soft'],
  [12, 'hard'],
  [0, 'cw'],
  [3, 'cw'],
  [1, 'ccw'],
  [2, 'ccw'],
  [4, 'hold'],
  [5, 'hold'],
];

export class Controls {
  handling: Handling = { ...DEFAULT_HANDLING };
  /** Set while a match is on screen; menus keep their normal keyboard behaviour otherwise. */
  active = false;
  private keyboard = new Set<Key>();
  private pad = new Set<Key>();
  private pressed: Key[] = [];
  private dasDir: 'left' | 'right' | null = null;
  private dasTimer = 0;
  private arrTimer = 0;
  private softTimer = 0;

  constructor() {
    addEventListener('keydown', (e) => {
      const k = KEYS[e.code];
      if (!k || !this.active) return;
      e.preventDefault();
      if (!e.repeat && !this.keyboard.has(k)) this.press(k);
      this.keyboard.add(k);
    });
    addEventListener('keyup', (e) => {
      const k = KEYS[e.code];
      if (k) this.keyboard.delete(k);
    });
    addEventListener('blur', () => this.keyboard.clear());
  }

  /** Clears held keys, e.g. between rounds. */
  reset() {
    this.pressed = [];
    this.dasDir = null;
  }

  update(dtMs: number, player: Player, onAction: (k: Key, ok: boolean) => void) {
    this.pollPad();
    for (const k of this.pressed) {
      if (k === 'left' || k === 'right') {
        this.dasDir = k;
        this.dasTimer = 0;
        this.arrTimer = 0;
        onAction(k, player.input(k === 'left' ? Action.Left : Action.Right));
      } else if (k === 'soft') {
        this.softTimer = this.handling.sdf;
      } else {
        const a = { hard: Action.Hard, cw: Action.Cw, ccw: Action.Ccw, hold: Action.Hold }[k];
        onAction(k, player.input(a));
      }
    }
    this.pressed = [];

    // Auto-repeat: the most recently pressed held direction wins.
    if (this.dasDir && !this.held(this.dasDir)) {
      const other = this.dasDir === 'left' ? 'right' : 'left';
      this.dasDir = this.held(other) ? other : null;
      this.dasTimer = this.arrTimer = 0;
    }
    if (this.dasDir) {
      const a = this.dasDir === 'left' ? Action.Left : Action.Right;
      this.dasTimer += dtMs;
      if (this.dasTimer >= this.handling.das) {
        if (this.handling.arr === 0) {
          while (player.input(a)) {}
        } else {
          this.arrTimer += dtMs;
          while (this.arrTimer >= this.handling.arr) {
            this.arrTimer -= this.handling.arr;
            if (!player.input(a)) break;
          }
        }
      }
    }

    if (this.held('soft')) {
      if (this.handling.sdf === 0) {
        while (player.input(Action.Soft)) {}
      } else {
        this.softTimer += dtMs;
        while (this.softTimer >= this.handling.sdf) {
          this.softTimer -= this.handling.sdf;
          if (!player.input(Action.Soft)) break;
        }
      }
    }
  }

  private held(k: Key) {
    return this.keyboard.has(k) || this.pad.has(k);
  }

  private press(k: Key) {
    this.pressed.push(k);
  }

  private pollPad() {
    const pad = navigator.getGamepads?.().find((p) => p);
    if (!pad) return;
    const now = new Set<Key>();
    for (const [i, k] of PAD) if (pad.buttons[i]?.pressed) now.add(k);
    for (const k of now) if (!this.pad.has(k) && !this.keyboard.has(k)) this.press(k);
    this.pad = now;
  }
}
