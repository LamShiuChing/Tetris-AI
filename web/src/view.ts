// One player's board: glossy blocks, ghost, bot-plan outline, hold/next, garbage meter, and
// spring-based bump/shake. Coordinates: board origin (0, 0) is the top-left of the visible 10x20.
import { Container, Graphics, Sprite, Text, type Renderer, type Texture } from 'pixi.js';
import { pieceCells, type Exports, type Snapshot } from './engine';
import { FONT } from './fx';

export const CELL = 28;
export const W = 10 * CELL;
export const H = 20 * CELL;
/** Board rows drawn: 2 hidden spawn rows above the visible 20. */
const FIRST_ROW = 18;

/** Stackfall's own palette, indexed like the engine's I J L O S T Z (deliberately not the guideline colors). */
export const COLORS = [0xffd93d, 0x9b6bff, 0xff4d6a, 0x4fc3ff, 0x7be36b, 0xff5fa2, 0xff8a3d];
export const GARBAGE = 0x6b7393;

export interface Textures {
  /** Index 1..7 pieces, 8 garbage (matches engine color codes). */
  blocks: Texture[];
  outline: Texture;
  dot: Texture;
}

const shade = (c: number, k: number) => {
  const f = (v: number) => Math.max(0, Math.min(255, Math.round(v * k)));
  return (f(c >> 16) << 16) | (f((c >> 8) & 255) << 8) | f(c & 255);
};

export function makeTextures(renderer: Renderer): Textures {
  const block = (color: number) => {
    const g = new Graphics()
      .roundRect(0, 0, CELL, CELL, 6)
      .fill(shade(color, 0.55))
      .roundRect(1.5, 1.5, CELL - 3, CELL - 3, 5)
      .fill(color)
      .roundRect(3, CELL * 0.6, CELL - 6, CELL * 0.32, 4)
      .fill({ color: shade(color, 0.75), alpha: 0.7 })
      .roundRect(3, 3, CELL - 6, CELL * 0.4, 4)
      .fill({ color: 0xffffff, alpha: 0.32 })
      .circle(CELL * 0.27, CELL * 0.27, 2.4)
      .fill({ color: 0xffffff, alpha: 0.9 });
    return renderer.generateTexture(g);
  };
  const outline = renderer.generateTexture(
    new Graphics().roundRect(1.5, 1.5, CELL - 3, CELL - 3, 6).fill({ color: 0xffffff, alpha: 0.12 }).stroke({ width: 2.5, color: 0xffffff }),
  );
  const dot = renderer.generateTexture(
    new Graphics().circle(16, 16, 16).fill({ color: 0xffffff, alpha: 0.25 }).circle(16, 16, 9).fill({ color: 0xffffff, alpha: 0.6 }).circle(16, 16, 4).fill(0xffffff),
  );
  return { blocks: [outline, ...COLORS.map(block), block(GARBAGE)], outline, dot };
}

function label(text: string, size: number, color = 0xffffff) {
  const t = new Text({
    text,
    style: { fontFamily: FONT, fontWeight: '800', fontSize: size, fill: color, stroke: { color: 0x1a1036, width: 5, join: 'round' }, letterSpacing: 2 },
  });
  return t;
}

export class BoardView extends Container {
  /** Shaken/bumped together with the board; effects tied to the board go in `fx`. */
  readonly inner = new Container();
  readonly fx = new Container();
  private cells: Sprite[] = [];
  private ghost: Sprite[] = [];
  private active: Sprite[] = [];
  private target: Sprite[] = [];
  private holdBox = new Container();
  private nextBox = new Container();
  private meter = new Graphics();
  private danger = new Graphics();
  private stats: Text;
  private bumpY = 0;
  private bumpV = 0;
  private shakeAmp = 0;
  private time = 0;
  isDanger = false;

  constructor(
    private w: Exports,
    private tex: Textures,
    name: string,
    private accent: number,
  ) {
    super();
    this.addChild(this.inner);
    const frame = new Graphics();
    for (let i = 4; i >= 1; i--) frame.roundRect(-4 - i * 3, -4 - i * 3, W + 8 + i * 6, H + 8 + i * 6, 14).fill({ color: accent, alpha: 0.05 });
    frame.roundRect(-4, -4, W + 8, H + 8, 10).fill({ color: 0x0d0a24, alpha: 0.88 }).stroke({ width: 3, color: accent, alpha: 0.9 });
    for (let x = 1; x < 10; x++) frame.rect(x * CELL, 0, 1, H).fill({ color: 0xffffff, alpha: 0.04 });
    for (let y = 1; y < 20; y++) frame.rect(0, y * CELL, W, 1).fill({ color: 0xffffff, alpha: 0.04 });
    this.inner.addChild(frame);

    const grid = new Container();
    for (let r = FIRST_ROW; r < 40; r++) {
      for (let c = 0; c < 10; c++) {
        const s = new Sprite(tex.blocks[1]);
        s.position.set(c * CELL, (r - 20) * CELL);
        s.visible = false;
        grid.addChild(s);
        this.cells.push(s);
      }
    }
    const make = (list: Sprite[], parent: Container) => {
      for (let i = 0; i < 4; i++) {
        const s = new Sprite(tex.blocks[1]);
        parent.addChild(s);
        list.push(s);
      }
    };
    this.inner.addChild(grid);
    make(this.ghost, this.inner);
    make(this.target, this.inner);
    make(this.active, this.inner);
    this.ghost.forEach((s) => (s.alpha = 0.22));
    this.target.forEach((s) => (s.texture = tex.outline));
    this.danger.rect(0, 0, W, H).fill(0xff2244);
    this.danger.alpha = 0;
    this.danger.blendMode = 'add';
    this.inner.addChild(this.danger, this.fx);

    this.meter.position.set(-18, 0);
    this.inner.addChild(this.meter);

    const box = (title: string, x: number, h: number, c: Container) => {
      const g = new Graphics().roundRect(0, 0, 100, h, 12).fill({ color: 0x0d0a24, alpha: 0.8 }).stroke({ width: 2, color: accent, alpha: 0.6 });
      const t = label(title, 16, accent);
      t.anchor.set(0.5, 1);
      t.position.set(50, -4);
      c.position.set(x, 0);
      c.addChild(g, t);
      this.inner.addChild(c);
    };
    box('HOLD', -132, 80, this.holdBox);
    box('NEXT', W + 16, 330, this.nextBox);

    const title = label(name, 22, 0xffffff);
    title.anchor.set(0.5, 0);
    title.position.set(W / 2, H + 12);
    this.stats = label('', 15, 0xcfc6ff);
    this.stats.anchor.set(0.5, 0);
    this.stats.position.set(W / 2, H + 40);
    this.inner.addChild(title, this.stats);
  }

  /** Board cell (col, row in engine coords) to this view's local coords (cell center). */
  cellCenter(col: number, row: number) {
    return { x: col * CELL + CELL / 2, y: (row - 20) * CELL + CELL / 2 };
  }

  kick(v: number) {
    this.bumpV += v;
  }

  shake(amp: number) {
    this.shakeAmp = Math.max(this.shakeAmp, amp);
  }

  update(dtMs: number) {
    this.time += dtMs;
    const dt = Math.min(dtMs, 50) / 1000;
    // Damped spring back to rest.
    this.bumpV += (-320 * this.bumpY - 18 * this.bumpV) * dt;
    this.bumpY += this.bumpV * dt;
    this.shakeAmp *= Math.exp(-dt * 9);
    this.inner.position.set((Math.random() - 0.5) * this.shakeAmp, this.bumpY + (Math.random() - 0.5) * this.shakeAmp);
    this.danger.alpha = this.isDanger ? 0.08 + 0.06 * Math.sin(this.time / 120) : 0;
  }

  render(s: Snapshot, target: { piece: number; x: number; y: number; rot: number } | null, stats: string) {
    for (let r = FIRST_ROW; r < 40; r++) {
      for (let c = 0; c < 10; c++) {
        const sprite = this.cells[(r - FIRST_ROW) * 10 + c];
        const color = s.colors[r * 10 + c];
        sprite.visible = color !== 0;
        if (color) sprite.texture = this.tex.blocks[color];
      }
    }
    this.isDanger = s.rows.slice(20, 25).some((row) => row !== 0);

    this.place(this.active, s.piece, s.x, s.y, s.rot, (sp) => {
      sp.texture = this.tex.blocks[s.piece + 1];
      sp.alpha = 1 - 0.4 * s.lockProgress;
    });
    this.place(this.ghost, s.piece, s.x, s.ghostY, s.rot, (sp) => (sp.texture = this.tex.blocks[s.piece + 1]));
    this.place(this.target, target ? target.piece : -1, target?.x ?? 0, target?.y ?? 0, target?.rot ?? 0, (sp) => (sp.tint = COLORS[target!.piece]));

    this.drawMini(this.holdBox, s.hold < 0 ? [] : [s.hold], 50, 46, 0.75);
    this.holdBox.alpha = s.canHold ? 1 : 0.45;
    this.drawMini(this.nextBox, s.queue, 50, 52, 0.7, 62);

    const pending = Math.min(s.pending, 20);
    const color = s.pending >= 8 ? 0xff3355 : s.pending >= 4 ? 0xff9a3d : 0xffd93d;
    this.meter.clear().roundRect(0, 0, 10, H, 5).fill({ color: 0x000000, alpha: 0.5 });
    if (pending > 0) {
      const glow = s.pending >= 8 ? 0.75 + 0.25 * Math.sin(this.time / 80) : 1;
      this.meter.roundRect(0, H - pending * CELL, 10, pending * CELL, 5).fill({ color, alpha: glow });
    }
    this.stats.text = stats;
  }

  private place(sprites: Sprite[], piece: number, x: number, y: number, rot: number, style: (s: Sprite) => void) {
    if (piece < 0) {
      sprites.forEach((s) => (s.visible = false));
      return;
    }
    pieceCells(this.w, piece, rot).forEach(([dx, dy], i) => {
      const s = sprites[i];
      s.visible = y + dy >= FIRST_ROW;
      s.position.set((x + dx) * CELL, (y + dy - 20) * CELL);
      style(s);
    });
  }

  /** Small centered pieces in a hold/next box; rebuilt only when the pieces change. */
  private drawMini(box: Container, pieces: number[], cx: number, cy: number, scale: number, step = 0) {
    const key = pieces.join(',');
    if (box.label === key) return;
    box.label = key;
    box.children.slice(2).forEach((c) => c.destroy());
    pieces.forEach((p, i) => {
      const cells = pieceCells(this.w, p, 0);
      const xs = cells.map((c) => c[0]);
      const ys = cells.map((c) => c[1]);
      const k = i === 0 ? scale : scale * 0.85;
      const midX = ((Math.min(...xs) + Math.max(...xs) + 1) / 2) * CELL * k;
      const midY = ((Math.min(...ys) + Math.max(...ys) + 1) / 2) * CELL * k;
      for (const [dx, dy] of cells) {
        const s = new Sprite(this.tex.blocks[p + 1]);
        s.scale.set(k);
        s.position.set(cx - midX + dx * CELL * k, cy + i * step - midY + dy * CELL * k);
        box.addChild(s);
      }
    });
  }
}
