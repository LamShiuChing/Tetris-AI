import { Application, Container } from 'pixi.js';
import { audio } from './audio';
import { LEVELS, Planner } from './bot';
import { loadEngine } from './engine';
import { Fx, motion, setUiScale } from './fx';
import { Controls, DEFAULT_HANDLING } from './input';
import { Match, clockText, type MatchConfig, type MatchResult, type Mode, type Replay } from './match';
import { Menus, PadKeys } from './ui';
import { makeTextures } from './view';

/** Everything in a match is laid out on a 1280x720 stage, scaled to fit the window. */
const DESIGN = { w: 1280, h: 720 };
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

/** One line per bot, in level order (12 = Chainlord). */
const BLURBS = [
  'Brand new. Slow, never holds, makes lots of mistakes.',
  'Learning the basics. Still slow and messy.',
  'Clears lines and starts using hold.',
  'Steady. Builds wells for the odd quad.',
  'Reads incoming garbage and starts T-spinning.',
  'Quick, plans ahead, loves a T-spin double.',
  'Fast and clean. Few mistakes.',
  'Knows a few named openers.',
  'Near-flawless. Counts the bag.',
  'Full strength, judged by a neural net.',
  "Titan's power, plays to show off: varied tricks and named setups.",
  "Titan's power, keeps back-to-back chains alive.",
];

const settings = loadSettings();

/** Saved settings merged over defaults; corrupt or blocked storage just means defaults. */
function loadSettings() {
  const defaults = {
    handling: { ...DEFAULT_HANDLING },
    sfx: 0.7,
    music: 0.35,
    reduceMotion: matchMedia('(prefers-reduced-motion: reduce)').matches,
  };
  try {
    const saved = JSON.parse(localStorage.getItem('stackfall') ?? '{}');
    return {
      handling: { ...defaults.handling, ...(saved?.handling ?? {}) },
      sfx: typeof saved?.sfx === 'number' ? saved.sfx : defaults.sfx,
      music: typeof saved?.music === 'number' ? saved.music : defaults.music,
      reduceMotion: typeof saved?.reduceMotion === 'boolean' ? saved.reduceMotion : defaults.reduceMotion,
    };
  } catch {
    return defaults;
  }
}

function save() {
  try {
    localStorage.setItem('stackfall', JSON.stringify(settings));
  } catch {
    // storage unavailable: settings last for this visit only
  }
}

/** Personal bests per solo mode: best score, and best time among completed runs. */
type Best = { score: number; time?: number };
function loadBests(): Record<string, Best> {
  try {
    return JSON.parse(localStorage.getItem('stackfall.pb') ?? '{}') ?? {};
  } catch {
    return {};
  }
}

async function main() {
  const [engine, , app] = await Promise.all([
    loadEngine(),
    document.fonts.load('800 32px "Baloo 2"').catch(() => {}),
    (async () => {
      const app = new Application();
      await app.init({ backgroundAlpha: 0, resizeTo: window, antialias: true, resolution: Math.min(devicePixelRatio, 2), autoDensity: true });
      return app;
    })(),
  ]);
  $('stage').appendChild(app.canvas);

  const root = new Container();
  app.stage.addChild(root);
  const fit = () => {
    const k = Math.min(innerWidth / DESIGN.w, innerHeight / DESIGN.h);
    setUiScale(k);
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
  const applyMotion = () => {
    motion.reduced = settings.reduceMotion;
    document.body.classList.toggle('reduce-motion', settings.reduceMotion);
  };
  applyMotion();

  const menus = new Menus(() => audio.move());
  const pad = new PadKeys();
  const bests = loadBests();
  let match: Match | null = null;
  let last: MatchConfig | null = null;
  let picked: HTMLElement | null = null;

  const overlay = (id: 'pause' | 'result', on: boolean) => $(id).classList.toggle('hidden', !on);
  const isOpen = (id: string) => !$(id).classList.contains('hidden');
  const hasHuman = () => !!last?.sides.some((s) => s.kind === 'human');

  const start = (cfg: MatchConfig) => {
    audio.unlock();
    audio.setVolumes(settings.sfx, settings.music);
    fx.clear();
    match?.destroy({ children: true });
    last = cfg;
    match = new Match(engine, tex, fx, planner, controls, cfg);
    controls.active = hasHuman();
    root.addChild(match);
    if (menus.visible) picked = document.activeElement as HTMLElement | null;
    menus.hide();
    overlay('pause', false);
    overlay('result', false);
    (document.activeElement as HTMLElement | null)?.blur();
    match.onEnd = (r) => {
      match?.hideBanner();
      showResult(r);
    };
  };

  const toMenu = () => {
    audio.stopMusic();
    fx.clear();
    match?.destroy({ children: true });
    match = null;
    controls.active = false;
    overlay('pause', false);
    overlay('result', false);
    renderBests();
    menus.show();
    menus.prefer(picked); // back on the card / button that started the match
  };

  const pause = (on: boolean) => {
    if (!match || isOpen('result')) return;
    match.setPaused(on);
    overlay('pause', on);
    if (on) $('resume').focus();
  };

  // Auto-pause a human's match when the window loses focus.
  const autoPause = () => {
    if (match && hasHuman() && !match.paused && !isOpen('result')) pause(true);
  };
  addEventListener('blur', autoPause);
  document.addEventListener('visibilitychange', () => document.hidden && autoPause());

  // ---- results ----
  const showResult = (r: MatchResult) => {
    $('resultText').textContent = r.title;
    $('resultDetail').textContent = r.detail;
    const solo = r.stats.length === 1;
    const rows: [string, (s: MatchResult['stats'][number]) => string][] = solo
      ? [
          ['Score', (s) => s.score.toLocaleString()],
          ['Lines', (s) => String(s.lines)],
          ['Time', () => clockText(r.seconds * 1000)],
          ['Pieces / sec', (s) => s.pps.toFixed(2)],
          ['Max combo', (s) => String(Math.max(0, s.maxCombo))],
          ['T-spins', (s) => String(s.tspins)],
          ['Quads', (s) => String(s.quads)],
          ['Perfect clears', (s) => String(s.pcs)],
        ]
      : [
          ['Lines sent', (s) => String(s.sent)],
          ['Attack / min', (s) => s.apm.toFixed(1)],
          ['Pieces / sec', (s) => s.pps.toFixed(2)],
          ['Lines cleared', (s) => String(s.lines)],
          ['Max combo', (s) => String(Math.max(0, s.maxCombo))],
          ['T-spins', (s) => String(s.tspins)],
          ['Quads', (s) => String(s.quads)],
          ['Perfect clears', (s) => String(s.pcs)],
        ];
    const box = $('resultStats');
    box.style.setProperty('--cols', String(r.stats.length));
    box.textContent = '';
    const row = (cells: string[], cls = '') => {
      const d = document.createElement('div');
      d.className = `r ${cls}`;
      for (const c of cells) d.appendChild(Object.assign(document.createElement('span'), { textContent: c }));
      box.appendChild(d);
    };
    if (!solo) row(['', ...r.stats.map((s) => s.name)], 'head');
    for (const [label, f] of rows) row([label, ...r.stats.map(f)]);
    // Solo personal bests (humans only).
    if (solo && r.human === 0) {
      const s = r.stats[0];
      const pb = bests[r.mode] ?? { score: 0 };
      const notes: string[] = [];
      if (s.score > pb.score) {
        pb.score = s.score;
        notes.push('New best score!');
      }
      const completed = r.winner === 0;
      if (completed && (pb.time === undefined || r.seconds < pb.time)) {
        pb.time = r.seconds;
        notes.push('New best time!');
      }
      bests[r.mode] = pb;
      try {
        localStorage.setItem('stackfall.pb', JSON.stringify(bests));
      } catch {
        // storage unavailable
      }
      if (notes.length) box.appendChild(Object.assign(document.createElement('div'), { className: 'best', textContent: notes.join(' ') }));
    }
    $('again').textContent = last?.replay ? 'WATCH AGAIN' : 'REMATCH';
    overlay('result', true);
    $('again').focus();
  };

  // ---- the frame loop ----
  app.ticker.add((ticker) => {
    const dt = Math.min(ticker.deltaMS, 50);
    match?.update(dt);
    if (!match?.paused) fx.update(dt * (last?.speed ?? 1));
    for (const key of pad.poll(dt)) handleKey(key, true);
  });

  // ---- keyboard / gamepad routing: overlays first, then the match, then the menus ----
  const handleKey = (key: string, fromPad = false): boolean => {
    if (isOpen('result')) {
      if (key === 'Enter' || key === 'KeyR') {
        if (fromPad || key === 'KeyR' || document.activeElement === $('again')) {
          if (last) start(last);
        } else (document.activeElement as HTMLElement | null)?.click();
        return true;
      }
      if (key === 'Escape') return toMenu(), true;
      return menus.navigate(key, $('result'));
    }
    if (isOpen('pause')) {
      if (key === 'Escape' || key === 'Pause') return pause(false), true;
      if (key === 'Enter' && fromPad) return (document.activeElement as HTMLElement | null)?.click(), true;
      return menus.navigate(key, $('pause'));
    }
    if (match) {
      if (key === 'Escape' || key === 'Pause') return pause(true), true;
      return false; // gameplay keys go to the match controls
    }
    if (!menus.visible) return false;
    if (key === 'Escape') return menus.back(), true;
    if (key === 'Enter' && fromPad) return (document.activeElement as HTMLElement | null)?.click(), true;
    return menus.navigate(key, document.querySelector<HTMLElement>('#ui .screen:not(.hidden)')!);
  };
  addEventListener('keydown', (e) => {
    const key = e.key.startsWith('Arrow') || e.key === 'Enter' || e.key === 'Escape' ? e.key : e.code;
    if (handleKey(key)) e.preventDefault();
  });

  // ---- versus: bot character cards ----
  const cards = $('vsCards');
  const lastVs = Number(localStorage.getItem('stackfall.vs') ?? 4);
  LEVELS.forEach((lv, i) => {
    const level = i + 1;
    const b = document.createElement('button');
    b.className = `bot-card${level > 10 ? ' special' : ''}`;
    b.style.setProperty('--c', ['#7be36b', '#7be36b', '#7be36b', '#5fd3ff', '#5fd3ff', '#5fd3ff', '#ffd93d', '#ffd93d', '#ff9a3d', '#ff5fa2', '#9b6bff', '#5fd3ff'][i]);
    b.innerHTML = `<small class="lvl"></small><b></b><span></span><div class="meter"><i></i></div>`;
    b.querySelector('.lvl')!.textContent = level > 10 ? 'SPECIAL' : `LEVEL ${level}`;
    b.querySelector('b')!.textContent = lv.name;
    b.querySelector('span')!.textContent = `${BLURBS[i]} · ${lv.pps} pieces/s`;
    (b.querySelector('.meter i') as HTMLElement).style.width = `${Math.min(level, 10) * 10}%`;
    b.onclick = () => {
      localStorage.setItem('stackfall.vs', String(level));
      start({ mode: 'versus', sides: [{ kind: 'human' }, { kind: 'bot', level }], showPlan: false, speed: 1 });
    };
    cards.appendChild(b);
  });
  document.querySelector<HTMLElement>('[data-go="scr-vs"]')!.addEventListener('click', () => menus.prefer(cards.children[lastVs - 1] as HTMLElement));

  // ---- solo ----
  document.querySelectorAll<HTMLElement>('[data-solo]').forEach((b) => {
    b.onclick = () => start({ mode: b.dataset.solo as Mode, sides: [{ kind: 'human' }], showPlan: false, speed: 1 });
  });
  const renderBests = () => {
    for (const mode of ['marathon', 'master']) {
      const pb = bests[mode];
      $(`pb-${mode}`).textContent = pb ? `Best: ${pb.score.toLocaleString()} pts${pb.time !== undefined ? ` · ${clockText(pb.time * 1000)}` : ''}` : 'No best yet';
    }
  };
  renderBests();

  // ---- spectate ----
  for (const id of ['wLeft', 'wRight', 'soloBot']) {
    const sel = $<HTMLSelectElement>(id);
    LEVELS.forEach((lv, i) => sel.add(new Option(`${lv.name}${i < 10 ? ` · Lv ${i + 1}` : ''}`, String(i + 1))));
  }
  $<HTMLSelectElement>('wLeft').value = '10';
  $<HTMLSelectElement>('wRight').value = '11';
  $<HTMLSelectElement>('soloBot').value = '10';
  const speed = () => Number($<HTMLInputElement>('speed').value);
  const showPlan = () => $<HTMLInputElement>('plan').checked;
  $('watch').onclick = () =>
    start({
      mode: 'versus',
      sides: [
        { kind: 'bot', level: Number($<HTMLSelectElement>('wLeft').value) },
        { kind: 'bot', level: Number($<HTMLSelectElement>('wRight').value) },
      ],
      showPlan: showPlan(),
      speed: speed(),
    });
  $('watchSolo').onclick = () =>
    start({
      mode: $<HTMLSelectElement>('soloBotMode').value as Mode,
      sides: [{ kind: 'bot', level: Number($<HTMLSelectElement>('soloBot').value) }],
      showPlan: showPlan(),
      speed: speed(),
    });
  // Recorded matches (examples/vs_coldclear.rs), numbered so identical-looking ones stay distinct.
  fetch(`${import.meta.env.BASE_URL}replays/index.json`)
    .then((r) => (r.ok ? r.json() : []))
    .then((list: { file: string; names: [string, string]; winner: string | null; pieces: number }[]) => {
      if (!Array.isArray(list) || !list.length) return;
      const box = $('replayList');
      box.textContent = '';
      list.forEach((r, n) => {
        const b = document.createElement('button');
        b.textContent = `#${n + 1} · ${r.names[0]} vs ${r.names[1]} — ${r.winner ?? 'draw'} (${r.pieces} pieces)`;
        b.onclick = async () => {
          try {
            const res = await fetch(`${import.meta.env.BASE_URL}replays/${r.file}`);
            const replay = (await res.json()) as Replay;
            if (!res.ok || !Array.isArray(replay.turns) || replay.names?.length !== 2) throw new Error('bad replay');
            start({ mode: 'replay', sides: [{ kind: 'replay' }, { kind: 'replay' }], showPlan: false, speed: speed(), replay });
          } catch {
            b.textContent += ' · could not load';
          }
        };
        box.appendChild(b);
      });
    })
    .catch(() => {});

  // ---- settings ----
  const bindRange = (id: string, show: (v: number) => string, onChange: (v: number) => void, initial: number) => {
    const input = $<HTMLInputElement>(id);
    input.value = String(initial);
    const update = () => {
      const v = Number(input.value);
      $(`${id}Name`).textContent = show(v);
      onChange(v);
      save();
    };
    input.addEventListener('input', update);
    update();
  };
  bindRange('speed', (v) => `${v}x`, () => {}, 1);
  bindRange('das', (v) => `${v} ms`, (v) => (settings.handling.das = v), settings.handling.das);
  bindRange('arr', (v) => (v === 0 ? 'instant' : `${v} ms`), (v) => (settings.handling.arr = v), settings.handling.arr);
  bindRange('sdf', (v) => (v === 0 ? 'instant' : `${v} ms`), (v) => (settings.handling.sdf = v), settings.handling.sdf);
  bindRange('sfx', (v) => `${Math.round(v * 100)}%`, (v) => ((settings.sfx = v), audio.setVolumes(settings.sfx, settings.music)), settings.sfx);
  bindRange('music', (v) => `${Math.round(v * 100)}%`, (v) => ((settings.music = v), audio.setVolumes(settings.sfx, settings.music)), settings.music);
  const rm = $<HTMLInputElement>('reduceMotion');
  rm.checked = settings.reduceMotion;
  rm.onchange = () => {
    settings.reduceMotion = rm.checked;
    applyMotion();
    save();
  };
  $('fullscreen').onclick = () => {
    if (document.fullscreenElement) void document.exitFullscreen();
    else void document.documentElement.requestFullscreen?.().catch(() => {});
  };

  // ---- overlay buttons ----
  $('resume').onclick = () => pause(false);
  $('restart').onclick = () => last && start(last);
  $('quit').onclick = toMenu;
  $('again').onclick = () => last && start(last);
  $('toMenu').onclick = toMenu;
  document.addEventListener('pointerdown', (e) => {
    audio.unlock();
    if ((e.target as HTMLElement).closest('button')) audio.click();
  });

  $('loading').classList.add('hidden');
  menus.show();
}

main().catch((err) => {
  console.error(err);
  const box = document.getElementById('loading');
  if (box) box.innerHTML = '<h2>Stackfall couldn\'t start.</h2><p class="tag">Try reloading the page. If it keeps happening, your browser may not support WebAssembly or WebGL.</p>';
});
