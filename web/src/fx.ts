// Juice: particles, bouncy text popups, row flashes, shockwave rings and attack orbs.
import { Container, Graphics, Sprite, Text, type Texture } from 'pixi.js';

interface Tween {
  t: number;
  dur: number;
  delay: number;
  step: (p: number) => void;
  done?: () => void;
}

interface Particle {
  s: Sprite;
  vx: number;
  vy: number;
  life: number;
  max: number;
  gravity: number;
}

const easeOutBack = (p: number) => 1 + 2.7 * (p - 1) ** 3 + 1.7 * (p - 1) ** 2;
const easeOutCubic = (p: number) => 1 - (1 - p) ** 3;

export const FONT = '"Baloo 2", "Arial Rounded MT Bold", system-ui, sans-serif';

export class Fx {
  private tweens: Tween[] = [];
  private particles: Particle[] = [];

  constructor(private dot: Texture) {}

  update(dtMs: number) {
    for (const tw of this.tweens) {
      if (tw.delay > 0) {
        tw.delay -= dtMs;
        continue;
      }
      tw.t += dtMs;
      tw.step(Math.min(tw.t / tw.dur, 1));
    }
    const finished = this.tweens.filter((tw) => tw.delay <= 0 && tw.t >= tw.dur);
    this.tweens = this.tweens.filter((tw) => !(tw.delay <= 0 && tw.t >= tw.dur));
    finished.forEach((tw) => tw.done?.());

    const dt = dtMs / 1000;
    for (const p of this.particles) {
      p.life -= dtMs;
      p.vy += p.gravity * dt;
      p.s.x += p.vx * dt;
      p.s.y += p.vy * dt;
      const k = Math.max(p.life / p.max, 0);
      p.s.alpha = k;
      p.s.scale.set(p.s.scale.x * (0.985 + 0.015 * k));
    }
    for (const p of this.particles) if (p.life <= 0) p.s.destroy();
    this.particles = this.particles.filter((p) => p.life > 0);
  }

  tween(dur: number, step: (p: number) => void, done?: () => void, delay = 0) {
    this.tweens.push({ t: 0, dur, delay, step, done });
  }

  burst(layer: Container, x: number, y: number, color: number, n: number, power = 260, o: { up?: number; size?: number; life?: number } = {}) {
    for (let i = 0; i < n; i++) {
      const s = new Sprite(this.dot);
      s.anchor.set(0.5);
      s.tint = color;
      s.blendMode = 'add';
      s.position.set(x, y);
      s.scale.set((o.size ?? 0.35) * (0.6 + Math.random() * 0.8));
      layer.addChild(s);
      const a = Math.random() * Math.PI * 2;
      const v = power * (0.3 + Math.random() * 0.7);
      const life = (o.life ?? 650) * (0.6 + Math.random() * 0.6);
      this.particles.push({ s, vx: Math.cos(a) * v, vy: Math.sin(a) * v - (o.up ?? 120), life, max: life, gravity: 700 });
    }
  }

  /** Big bouncy label: pops in with overshoot, hangs, then floats up and fades. */
  popup(layer: Container, text: string, x: number, y: number, o: { size?: number; color?: number; delay?: number; hold?: number } = {}): Text {
    const t = new Text({
      text,
      style: {
        fontFamily: FONT,
        fontWeight: '800',
        fontSize: o.size ?? 30,
        fill: o.color ?? 0xffffff,
        stroke: { color: 0x1a1036, width: Math.round((o.size ?? 30) / 4), join: 'round' },
        dropShadow: { color: 0x000000, alpha: 0.45, distance: 4, blur: 2, angle: Math.PI / 2 },
        letterSpacing: 1,
      },
    });
    t.anchor.set(0.5);
    t.position.set(x, y);
    t.scale.set(0);
    layer.addChild(t);
    const hold = o.hold ?? 700;
    this.tween(220, (p) => t.scale.set(easeOutBack(p)), undefined, o.delay ?? 0);
    this.tween(
      450,
      (p) => {
        t.y = y - 30 * easeOutCubic(p);
        t.alpha = 1 - p;
      },
      () => t.destroy(),
      (o.delay ?? 0) + 220 + hold,
    );
    return t;
  }

  flashRect(layer: Container, x: number, y: number, w: number, h: number, color = 0xffffff, dur = 260) {
    const g = new Graphics().rect(-w / 2, -h / 2, w, h).fill(color);
    g.position.set(x + w / 2, y + h / 2);
    g.blendMode = 'add';
    layer.addChild(g);
    this.tween(
      dur,
      (p) => {
        g.alpha = 1 - p;
        g.scale.set(1 + 0.08 * p, 1 - 0.7 * p);
      },
      () => g.destroy(),
    );
  }

  ring(layer: Container, x: number, y: number, color: number, radius = 160) {
    const g = new Graphics().circle(0, 0, 50).stroke({ width: 10, color });
    g.position.set(x, y);
    g.blendMode = 'add';
    layer.addChild(g);
    this.tween(
      500,
      (p) => {
        g.scale.set((0.2 + easeOutCubic(p) * radius) / 50);
        g.alpha = 1 - p;
      },
      () => g.destroy(),
    );
  }

  /** Vertical streak behind a hard-dropped cell. */
  streak(layer: Container, x: number, top: number, bottom: number, w: number, color: number) {
    if (bottom <= top) return;
    const g = new Graphics().rect(0, 0, w, bottom - top).fill({ color, alpha: 0.5 });
    g.position.set(x, top);
    g.blendMode = 'add';
    layer.addChild(g);
    this.tween(
      220,
      (p) => {
        g.alpha = 1 - p;
        g.scale.x = 1 - 0.6 * p;
        g.x = x + (w * 0.6 * p) / 2;
      },
      () => g.destroy(),
    );
  }

  /** Glowing orb flying along an arc from `from` to `to` (global coords in `layer`), leaving sparks. */
  orb(layer: Container, from: { x: number; y: number }, to: { x: number; y: number }, color: number, lines: number, onArrive: () => void) {
    const s = new Sprite(this.dot);
    s.anchor.set(0.5);
    s.tint = color;
    s.blendMode = 'add';
    s.scale.set(0.8 + Math.min(lines, 10) * 0.12);
    layer.addChild(s);
    const mid = { x: (from.x + to.x) / 2, y: Math.min(from.y, to.y) - 160 };
    this.tween(
      520,
      (p) => {
        const e = easeOutCubic(p);
        const u = 1 - e;
        s.x = u * u * from.x + 2 * u * e * mid.x + e * e * to.x;
        s.y = u * u * from.y + 2 * u * e * mid.y + e * e * to.y;
        if (Math.random() < 0.8) this.burst(layer, s.x, s.y, color, 1, 40, { up: 0, size: 0.25, life: 300 });
      },
      () => {
        s.destroy();
        this.burst(layer, to.x, to.y, color, 18, 220);
        onArrive();
      },
    );
  }
}
