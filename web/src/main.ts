import { Application, Container } from 'pixi.js';
import { audio } from './audio';
import { LEVELS, Planner } from './bot';
import { loadEngine } from './engine';
import { Fx } from './fx';
import { Controls, DEFAULT_HANDLING } from './input';
import { Match, type MatchConfig, type Mode, type Replay } from './match';
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
    match.onEnd = (r) => {
      $('resultText').textContent = r.title;
      $('resultDetail').textContent = r.detail;
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
  bindRange('wLeft', levelName, () => {}, 10);
  bindRange('wRight', levelName, () => {}, 11);
  bindRange('soloWho', (v) => (v === 0 ? 'You' : `Bot · ${levelName(v)}`), () => {}, 0);
  bindRange('speed', (v) => `${v}x`, () => {}, 1);
  bindRange('das', (v) => `${v} ms`, (v) => (settings.handling.das = v), settings.handling.das);
  bindRange('arr', (v) => (v === 0 ? 'instant' : `${v} ms`), (v) => (settings.handling.arr = v), settings.handling.arr);
  bindRange('sdf', (v) => (v === 0 ? 'instant' : `${v} ms`), (v) => (settings.handling.sdf = v), settings.handling.sdf);
  bindRange('sfx', (v) => `${Math.round(v * 100)}%`, (v) => ((settings.sfx = v), audio.setVolumes(settings.sfx, settings.music)), settings.sfx);
  bindRange('music', (v) => `${Math.round(v * 100)}%`, (v) => ((settings.music = v), audio.setVolumes(settings.sfx, settings.music)), settings.music);

  const num = (id: string) => Number($<HTMLInputElement>(id).value);
  $('vs').onclick = () => {
    localStorage.setItem('stackfall.vs', String(num('vsLevel')));
    start({ mode: 'versus', sides: [{ kind: 'human' }, { kind: 'bot', level: num('vsLevel') }], showPlan: false, speed: 1 });
  };
  $('solo').onclick = () => {
    const who = num('soloWho');
    start({
      mode: $<HTMLSelectElement>('soloMode').value as Mode,
      sides: [who === 0 ? { kind: 'human' } : { kind: 'bot', level: who }],
      showPlan: who !== 0 && $<HTMLInputElement>('plan').checked,
      speed: who === 0 ? 1 : num('speed'),
    });
  };
  $('watch').onclick = () =>
    start({
      mode: 'versus',
      sides: [
        { kind: 'bot', level: num('wLeft') },
        { kind: 'bot', level: num('wRight') },
      ],
      showPlan: $<HTMLInputElement>('plan').checked,
      speed: num('speed'),
    });
  // Recorded matches (e.g. Titan vs Cold Clear 2, written by examples/vs_coldclear.rs).
  fetch('/replays/index.json')
    .then((r) => (r.ok ? r.json() : []))
    .then((list: { file: string; names: [string, string]; winner: string | null; pieces: number }[]) => {
      if (!list.length) return;
      const box = $('replayList');
      box.textContent = '';
      for (const r of list) {
        const b = document.createElement('button');
        b.textContent = `${r.names[0]} vs ${r.names[1]} — ${r.winner ?? 'draw'} wins (${r.pieces} pieces)`;
        b.onclick = async () => {
          const replay: Replay = await (await fetch(`/replays/${r.file}`)).json();
          start({ mode: 'replay', sides: [{ kind: 'replay' }, { kind: 'replay' }], showPlan: false, speed: num('speed'), replay });
        };
        box.appendChild(b);
      }
    })
    .catch(() => {});
  $('again').onclick = () => last && start(last);
  $('toMenu').onclick = toMenu;
  document.querySelectorAll('button').forEach((b) => b.addEventListener('pointerdown', () => (audio.unlock(), audio.click())));
  addEventListener('keydown', (e) => {
    if (e.code === 'Escape' && match) toMenu();
  });

  $('loading').classList.add('hidden');
}

main();
