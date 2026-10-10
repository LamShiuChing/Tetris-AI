import { Application, Container } from 'pixi.js';
import { audio } from './audio';
import { LEVELS, Planner } from './bot';
import { loadEngine } from './engine';
import { Fx } from './fx';
import { Controls, DEFAULT_HANDLING } from './input';
import { Match, type MatchConfig } from './match';
import { makeTextures } from './view';

/** Everything is laid out on a 1280x720 stage, scaled to fit the window. */
const DESIGN = { w: 1280, h: 720 };
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const settings = {
  handling: { ...DEFAULT_HANDLING },
  sfx: 0.7,
  music: 0.35,
  ...JSON.parse(localStorage.getItem('stackfall') ?? '{}'),
};

async function main() {
  await document.fonts.load('800 32px "Baloo 2"').catch(() => {});
  const [engine, app] = await Promise.all([
    loadEngine(),
    (async () => {
      const app = new Application();
      await app.init({ backgroundAlpha: 0, resizeTo: window, antialias: true, resolution: devicePixelRatio, autoDensity: true });
      return app;
    })(),
  ]);
  $('stage').appendChild(app.canvas);

  const root = new Container();
  app.stage.addChild(root);
  const fit = () => {
    const k = Math.min(innerWidth / DESIGN.w, innerHeight / DESIGN.h);
    root.scale.set(k);
    root.position.set((innerWidth - DESIGN.w * k) / 2, (innerHeight - DESIGN.h * k) / 2);
  };
  addEventListener('resize', fit);
  fit();

  const tex = makeTextures(app.renderer);
  const fx = new Fx(tex.dot);
  const planner = new Planner();
  const controls = new Controls();
  controls.handling = settings.handling;
  audio.setVolumes(settings.sfx, settings.music);

  let match: Match | null = null;
  let last: MatchConfig | null = null;

  const start = (cfg: MatchConfig) => {
    audio.unlock();
    audio.setVolumes(settings.sfx, settings.music);
    match?.destroy({ children: true });
    last = cfg;
    match = new Match(engine, tex, fx, planner, controls, cfg);
    root.addChild(match);
    $('menu').classList.add('hidden');
    $('result').classList.add('hidden');
    match.onEnd = (winner) => {
      const human = cfg.sides.findIndex((s) => s.kind === 'human');
      $('resultText').textContent = human < 0 ? (winner === 0 ? 'LEFT BOT WINS' : 'RIGHT BOT WINS') : winner === human ? 'YOU WIN!' : 'YOU LOSE';
      $('result').classList.remove('hidden');
    };
  };

  const toMenu = () => {
    audio.stopMusic();
    match?.destroy({ children: true });
    match = null;
    $('result').classList.add('hidden');
    $('menu').classList.remove('hidden');
  };

  app.ticker.add((ticker) => {
    const dt = Math.min(ticker.deltaMS, 50);
    match?.update(dt);
    fx.update(dt * (last?.speed ?? 1));
  });

  // ---- menu wiring ----
  const bindRange = (id: string, show: (v: number) => string, onChange: (v: number) => void, initial: number) => {
    const input = $<HTMLInputElement>(id);
    input.value = String(initial);
    const update = () => {
      const v = Number(input.value);
      $(`${id}Name`).textContent = show(v);
      onChange(v);
      localStorage.setItem('stackfall', JSON.stringify(settings));
    };
    input.addEventListener('input', update);
    update();
  };
  const levelName = (v: number) => LEVELS[v - 1].name;
  bindRange('vsLevel', levelName, () => {}, Number(localStorage.getItem('stackfall.vs') ?? 4));
  bindRange('wLeft', levelName, () => {}, 8);
  bindRange('wRight', levelName, () => {}, 10);
  bindRange('speed', (v) => `${v}x`, () => {}, 1);
  bindRange('das', (v) => `${v} ms`, (v) => (settings.handling.das = v), settings.handling.das);
  bindRange('arr', (v) => (v === 0 ? 'instant' : `${v} ms`), (v) => (settings.handling.arr = v), settings.handling.arr);
  bindRange('sdf', (v) => (v === 0 ? 'instant' : `${v} ms`), (v) => (settings.handling.sdf = v), settings.handling.sdf);
  bindRange('sfx', (v) => `${Math.round(v * 100)}%`, (v) => ((settings.sfx = v), audio.setVolumes(settings.sfx, settings.music)), settings.sfx);
  bindRange('music', (v) => `${Math.round(v * 100)}%`, (v) => ((settings.music = v), audio.setVolumes(settings.sfx, settings.music)), settings.music);

  const num = (id: string) => Number($<HTMLInputElement>(id).value);
  $('vs').onclick = () => {
    localStorage.setItem('stackfall.vs', String(num('vsLevel')));
    start({ sides: [{ kind: 'human' }, { kind: 'bot', level: num('vsLevel') }], showPlan: false, speed: 1 });
  };
  $('watch').onclick = () =>
    start({
      sides: [
        { kind: 'bot', level: num('wLeft') },
        { kind: 'bot', level: num('wRight') },
      ],
      showPlan: $<HTMLInputElement>('plan').checked,
      speed: num('speed'),
    });
  $('again').onclick = () => last && start(last);
  $('toMenu').onclick = toMenu;
  document.querySelectorAll('button').forEach((b) => b.addEventListener('pointerdown', () => (audio.unlock(), audio.click())));
  addEventListener('keydown', (e) => {
    if (e.code === 'Escape' && match) toMenu();
  });

  $('loading').classList.add('hidden');
}

main();
