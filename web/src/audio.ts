// Every sound is synthesized with Web Audio: no audio files, no licensing. The music is an
// original procedural loop (never the traditional "Korobeiniki" theme).

type Wave = OscillatorType;

/** Major-scale semitone offsets, used to pitch combos up the scale like an arcade "ren" ladder. */
const SCALE = [0, 2, 4, 5, 7, 9, 11, 12, 14, 16, 17, 19, 21, 23, 24];

const midi = (n: number) => 440 * 2 ** ((n - 69) / 12);

class Audio {
  sfxVolume = 0.7;
  musicVolume = 0.35;
  private ctx: AudioContext | null = null;
  private sfx!: GainNode;
  private music!: GainNode;
  private noiseBuf!: AudioBuffer;
  private musicTimer = 0;
  private nextBeat = 0;
  private beat = 0;

  /** Browsers only allow audio after a user gesture; call from a click/keydown. */
  unlock() {
    if (this.ctx) {
      void this.ctx.resume();
      return;
    }
    this.ctx = new AudioContext();
    const comp = this.ctx.createDynamicsCompressor();
    comp.connect(this.ctx.destination);
    this.sfx = this.ctx.createGain();
    this.music = this.ctx.createGain();
    this.sfx.connect(comp);
    this.music.connect(comp);
    this.setVolumes(this.sfxVolume, this.musicVolume);
    this.noiseBuf = this.ctx.createBuffer(1, this.ctx.sampleRate, this.ctx.sampleRate);
    const d = this.noiseBuf.getChannelData(0);
    for (let i = 0; i < d.length; i++) d[i] = Math.random() * 2 - 1;
  }

  setVolumes(sfx: number, music: number) {
    this.sfxVolume = sfx;
    this.musicVolume = music;
    if (!this.ctx) return;
    this.sfx.gain.value = sfx;
    this.music.gain.value = music;
  }

  // ---- building blocks ----

  private tone(freq: number, dur: number, o: { type?: Wave; vol?: number; slide?: number; delay?: number; pan?: number; out?: GainNode } = {}) {
    const ctx = this.ctx;
    if (!ctx) return;
    const t = ctx.currentTime + (o.delay ?? 0);
    const osc = ctx.createOscillator();
    const g = ctx.createGain();
    osc.type = o.type ?? 'square';
    osc.frequency.setValueAtTime(freq, t);
    if (o.slide) osc.frequency.exponentialRampToValueAtTime(Math.max(20, freq * o.slide), t + dur);
    g.gain.setValueAtTime(0, t);
    g.gain.linearRampToValueAtTime(o.vol ?? 0.2, t + 0.005);
    g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
    osc.connect(g).connect(this.panner(o.pan ?? 0, o.out ?? this.sfx));
    osc.start(t);
    osc.stop(t + dur + 0.02);
  }

  private noise(dur: number, o: { vol?: number; freq?: number; to?: number; type?: BiquadFilterType; delay?: number; pan?: number; out?: GainNode } = {}) {
    const ctx = this.ctx;
    if (!ctx) return;
    const t = ctx.currentTime + (o.delay ?? 0);
    const src = ctx.createBufferSource();
    src.buffer = this.noiseBuf;
    const f = ctx.createBiquadFilter();
    f.type = o.type ?? 'lowpass';
    f.frequency.setValueAtTime(o.freq ?? 1000, t);
    if (o.to) f.frequency.exponentialRampToValueAtTime(o.to, t + dur);
    const g = ctx.createGain();
    g.gain.setValueAtTime(o.vol ?? 0.3, t);
    g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
    src.connect(f).connect(g).connect(this.panner(o.pan ?? 0, o.out ?? this.sfx));
    src.start(t, Math.random() * 0.5);
    src.stop(t + dur + 0.02);
  }

  private panner(pan: number, out: GainNode): AudioNode {
    if (!pan) return out;
    const p = this.ctx!.createStereoPanner();
    p.pan.value = pan;
    p.connect(out);
    return p;
  }

  // ---- game sounds (pan: -1 left player, +1 right player) ----

  move(pan = 0) {
    this.tone(1400, 0.025, { vol: 0.05, pan });
  }

  rotate(pan = 0) {
    this.tone(660, 0.05, { type: 'triangle', vol: 0.12, slide: 1.4, pan });
  }

  soft(pan = 0) {
    this.tone(300, 0.02, { type: 'triangle', vol: 0.04, pan });
  }

  hold(pan = 0) {
    this.tone(500, 0.09, { type: 'sine', vol: 0.15, slide: 1.6, pan });
  }

  hardDrop(rows: number, pan = 0) {
    this.noise(0.12, { vol: 0.25 + Math.min(rows, 20) * 0.01, freq: 900, to: 120, pan });
    this.tone(140, 0.12, { type: 'sine', vol: 0.3, slide: 0.5, pan });
  }

  lock(pan = 0) {
    this.tone(220, 0.04, { type: 'sine', vol: 0.12, pan });
  }

  /** Line clear: a chord that climbs the scale with each combo step. */
  clear(lines: number, combo: number, spin: number, b2b: boolean, pc: boolean, pan = 0) {
    const root = 72 + SCALE[Math.min(Math.max(combo, 0), SCALE.length - 1)];
    const chord = lines >= 4 || spin === 2 ? [0, 4, 7, 12, 16] : lines >= 2 ? [0, 4, 7] : [0, 7];
    chord.forEach((n, i) => this.tone(midi(root + n), 0.35, { type: 'square', vol: 0.07, delay: i * 0.025, pan }));
    this.noise(0.25, { vol: 0.08, freq: 6000, type: 'highpass', pan });
    if (spin === 2) [0, 7, 12, 19, 24].forEach((n, i) => this.tone(midi(84 + n), 0.18, { type: 'sine', vol: 0.12, delay: 0.05 + i * 0.04, pan }));
    if (lines >= 4) this.tone(midi(48), 0.5, { type: 'sawtooth', vol: 0.12, slide: 0.98, pan });
    if (b2b) [0, 12].forEach((n, i) => this.tone(midi(96 + n), 0.3, { type: 'sine', vol: 0.08, delay: 0.12 + i * 0.06, pan }));
    if (pc) [0, 4, 7, 12, 16, 19, 24].forEach((n, i) => this.tone(midi(72 + n), 0.5, { type: 'triangle', vol: 0.12, delay: i * 0.07, pan }));
  }

  spinNoClear(pan = 0) {
    [0, 7, 12].forEach((n, i) => this.tone(midi(79 + n), 0.12, { type: 'sine', vol: 0.1, delay: i * 0.03, pan }));
  }

  send(lines: number, pan = 0) {
    this.noise(0.35, { vol: 0.12 + Math.min(lines, 10) * 0.02, freq: 400, to: 4000, type: 'bandpass', pan });
  }

  receive(lines: number, pan = 0) {
    this.tone(90, 0.25, { type: 'sine', vol: 0.3 + Math.min(lines, 8) * 0.03, slide: 0.6, pan });
    this.noise(0.2, { vol: 0.2, freq: 300, pan });
  }

  levelUp() {
    [0, 4, 7, 12].forEach((n, i) => this.tone(midi(76 + n), 0.15, { type: 'square', vol: 0.08, delay: i * 0.05 }));
  }

  danger() {
    this.tone(70, 0.12, { type: 'sine', vol: 0.25 });
    this.tone(70, 0.1, { type: 'sine', vol: 0.18, delay: 0.16 });
  }

  countdown(go: boolean) {
    this.tone(go ? 880 : 440, go ? 0.4 : 0.15, { type: 'square', vol: 0.12 });
  }

  win() {
    [0, 4, 7, 12, 7, 12, 16, 19].forEach((n, i) => this.tone(midi(72 + n), 0.25, { type: 'square', vol: 0.09, delay: i * 0.1 }));
  }

  lose() {
    [12, 7, 3, 0].forEach((n, i) => this.tone(midi(60 + n), 0.35, { type: 'triangle', vol: 0.14, delay: i * 0.18 }));
  }

  click() {
    this.tone(880, 0.05, { type: 'triangle', vol: 0.1 });
  }

  // ---- music: original I–V–vi–IV loop in C, 132 bpm, scheduled slightly ahead ----

  startMusic() {
    if (!this.ctx || this.musicTimer) return;
    this.nextBeat = this.ctx.currentTime + 0.1;
    this.beat = 0;
    this.musicTimer = window.setInterval(() => this.scheduleMusic(), 25);
  }

  stopMusic() {
    clearInterval(this.musicTimer);
    this.musicTimer = 0;
  }

  private scheduleMusic() {
    const ctx = this.ctx!;
    const eighth = 60 / 132 / 2;
    const chords = [
      [60, 64, 67],
      [55, 59, 62],
      [57, 60, 64],
      [53, 57, 60],
    ];
    const melody = [76, 79, 81, 79, 76, 74, 72, 74, 74, 76, 79, 76, 74, 72, 71, 67, 72, 76, 79, 81, 84, 81, 79, 76, 77, 76, 74, 72, 74, 72, 71, 72];
    // Background tabs throttle timers to ~1/s; skip beats we missed instead of playing them all at once.
    if (this.nextBeat < ctx.currentTime) this.nextBeat = ctx.currentTime + 0.05;
    while (this.nextBeat < ctx.currentTime + 0.12) {
      const delay = this.nextBeat - ctx.currentTime;
      const b = this.beat;
      const chord = chords[Math.floor(b / 8) % 4];
      this.tone(midi(chord[b % 3] + 12), eighth * 0.9, { type: 'square', vol: 0.035, delay, out: this.music });
      if (b % 2 === 0) this.tone(midi(chord[0] - 24), eighth * 1.8, { type: 'triangle', vol: 0.16, delay, out: this.music });
      if (b % 4 === 2) this.noise(0.05, { vol: 0.05, freq: 7000, type: 'highpass', delay, out: this.music });
      if (b % 8 === 0) this.noise(0.12, { vol: 0.12, freq: 150, delay, out: this.music });
      if (b >= 32) this.tone(midi(melody[b % 32]), eighth * 0.95, { type: 'triangle', vol: 0.07, delay, out: this.music });
      this.beat = (b + 1) % 64;
      this.nextBeat += eighth;
    }
  }
}

export const audio = new Audio();
