// Menu navigation: one screen visible at a time with a back stack. Arrow keys / D-pad move focus
// to the nearest control in that direction (so grids behave like a console menu), Enter / A
// selects, Esc / B goes back. Range sliders and selects change with Left/Right.

const FOCUSABLE = 'button:not([disabled]), select, input';

export class Menus {
  private stack: string[] = ['scr-title'];
  private ui = document.getElementById('ui')!;

  constructor(private onClick: () => void) {
    document.querySelectorAll<HTMLElement>('[data-go]').forEach((b) => b.addEventListener('click', () => this.go(b.dataset.go!)));
    document.querySelectorAll<HTMLElement>('[data-back]').forEach((b) => b.addEventListener('click', () => this.back()));
  }

  get visible() {
    return !this.ui.classList.contains('hidden');
  }

  /** Open a screen on top of the current one. */
  go(id: string) {
    this.screen(this.top).classList.add('hidden');
    this.stack.push(id);
    this.reveal();
  }

  /** Back one screen (never past the title). */
  back() {
    if (this.stack.length <= 1) return;
    this.screen(this.top).classList.add('hidden');
    this.stack.pop();
    this.reveal();
  }

  /** Hide all menus (a match is starting). */
  hide() {
    this.ui.classList.add('hidden');
  }

  /** Show the menus again on the screen the player left from. */
  show() {
    this.ui.classList.remove('hidden');
    this.reveal();
  }

  /** Focus the element that should be selected when a screen opens (if it's on that screen). */
  prefer(el: HTMLElement | null) {
    if (el && this.screen(this.top).contains(el)) el.focus();
  }

  /** Arrow / D-pad navigation inside `root` (a screen or an overlay). Returns true if handled. */
  navigate(key: string, root: HTMLElement): boolean {
    const active = document.activeElement as HTMLElement | null;
    if (active instanceof HTMLInputElement && active.type === 'range' && (key === 'ArrowLeft' || key === 'ArrowRight')) {
      return false; // let the slider move
    }
    if (active instanceof HTMLSelectElement && (key === 'ArrowLeft' || key === 'ArrowRight')) {
      const n = active.options.length;
      active.selectedIndex = (active.selectedIndex + (key === 'ArrowLeft' ? n - 1 : 1)) % n;
      active.dispatchEvent(new Event('change'));
      return true;
    }
    const dir = { ArrowUp: [0, -1], ArrowDown: [0, 1], ArrowLeft: [-1, 0], ArrowRight: [1, 0] }[key];
    if (!dir) return false;
    const items = [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((e) => e.offsetParent !== null);
    if (!items.length) return true;
    if (!active || !items.includes(active)) {
      items[0].focus();
      return true;
    }
    const from = center(active);
    let best: HTMLElement | null = null;
    let bestScore = Infinity;
    for (const el of items) {
      if (el === active) continue;
      const c = center(el);
      const along = (c.x - from.x) * dir[0] + (c.y - from.y) * dir[1];
      if (along <= 4) continue; // not in that direction
      const across = Math.abs((c.x - from.x) * dir[1]) + Math.abs((c.y - from.y) * dir[0]);
      const score = along + across * 2;
      if (score < bestScore) {
        bestScore = score;
        best = el;
      }
    }
    best?.focus();
    if (best) this.onClick();
    return true;
  }

  private get top() {
    return this.stack[this.stack.length - 1];
  }

  private screen(id: string) {
    return document.getElementById(id)!;
  }

  private reveal() {
    const s = this.screen(this.top);
    s.classList.remove('hidden');
    s.querySelector<HTMLElement>(FOCUSABLE)?.focus();
  }
}

function center(el: HTMLElement) {
  const r = el.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
}

/** Gamepad presses for menus, as key names, with auto-repeat on held directions. */
export class PadKeys {
  private held = new Map<number, number>();

  /** Call once per frame; returns keys pressed this frame. */
  poll(dtMs: number): string[] {
    const pad = navigator.getGamepads?.().find((p) => p);
    if (!pad) return [];
    const map: [number, string, boolean][] = [
      [12, 'ArrowUp', true],
      [13, 'ArrowDown', true],
      [14, 'ArrowLeft', true],
      [15, 'ArrowRight', true],
      [0, 'Enter', false],
      [1, 'Escape', false],
      [9, 'Pause', false],
    ];
    const out: string[] = [];
    for (const [i, key, repeats] of map) {
      if (!pad.buttons[i]?.pressed) {
        this.held.delete(i);
        continue;
      }
      const t = this.held.get(i);
      if (t === undefined) {
        out.push(key);
        this.held.set(i, -280); // first repeat after 280 ms
      } else if (repeats) {
        const next = t + dtMs;
        if (next >= 110) {
          out.push(key);
          this.held.set(i, 0);
        } else {
          this.held.set(i, next);
        }
      }
    }
    return out;
  }
}
