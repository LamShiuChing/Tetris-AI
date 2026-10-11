"""Render a replay to a vertical 1080x1920 60 fps MP4, frame-exact, with the game's own sounds.

The page runs on a virtual clock (rAF, timers, performance.now advance exactly 1/60 s per frame), so
the capture never drops or stretches frames however slow screenshots are. Web Audio goes to an
OfflineAudioContext on the same clock and is rendered to WAV at the end.

Usage (from web/, with `npx vite preview --port 5191` running after `npm run build`):
  python render_video.py titan-vs-coldclear-5.json ../video/titan-vs-coldclear.mp4
  python render_video.py <replay> <out.mp4> --wide [max seconds]   # 1920x1080 analysis layout
"""

import base64
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from playwright.sync_api import sync_playwright

FPS = 60
TAIL_S = 3.5  # keep the results card up this long
MAX_S = 60

CLOCK = """
(() => {
  let now = 0, id = 1, raf = [];
  const t0 = Date.now(), timers = new Map();
  performance.now = () => now;
  Date.now = () => t0 + now;
  window.requestAnimationFrame = (cb) => (raf.push([id, cb]), id++);
  window.cancelAnimationFrame = (h) => { raf = raf.filter((r) => r[0] !== h); };
  window.setTimeout = (cb, ms = 0, ...a) => (timers.set(id, { at: now + Math.max(0, ms), cb, a, every: 0 }), id++);
  window.setInterval = (cb, ms = 0, ...a) => (timers.set(id, { at: now + Math.max(1, ms), cb, a, every: Math.max(1, ms) }), id++);
  window.clearTimeout = window.clearInterval = (h) => timers.delete(h);
  window.__now = () => now;
  window.__step = (ms) => {
    const end = now + ms;
    for (;;) {
      let next = null;
      for (const e of timers) if (e[1].at <= end && (!next || e[1].at < next[1].at)) next = e;
      if (!next) break;
      const [k, t] = next;
      now = t.at;
      if (t.every) t.at += t.every; else timers.delete(k);
      if (typeof t.cb === 'function') t.cb(...t.a);
    }
    now = end;
    const cbs = raf;
    raf = [];
    for (const [, cb] of cbs) cb(now);
  };
  const SR = 48000;
  window.AudioContext = function () {
    const c = new OfflineAudioContext(2, SR * (MAX_S + 15), SR);
    Object.defineProperty(c, 'currentTime', { get: () => now / 1000 });
    return (window.__audio = c);
  };
  window.__renderAudio = async (fromMs, toMs) => {
    if (!window.__audio) return null;
    const buf = await window.__audio.startRendering();
    const a = Math.floor((fromMs / 1000) * SR), b = Math.min(buf.length, Math.floor((toMs / 1000) * SR));
    const n = b - a, out = new DataView(new ArrayBuffer(44 + n * 4));
    const str = (o, s) => [...s].forEach((ch, i) => out.setUint8(o + i, ch.charCodeAt(0)));
    str(0, 'RIFF'); out.setUint32(4, 36 + n * 4, true); str(8, 'WAVEfmt ');
    out.setUint32(16, 16, true); out.setUint16(20, 1, true); out.setUint16(22, 2, true);
    out.setUint32(24, SR, true); out.setUint32(28, SR * 4, true); out.setUint16(32, 4, true); out.setUint16(34, 16, true);
    str(36, 'data'); out.setUint32(40, n * 4, true);
    const ch = [buf.getChannelData(0), buf.getChannelData(1)];
    for (let i = 0; i < n; i++) for (let c = 0; c < 2; c++)
      out.setInt16(44 + i * 4 + c * 2, Math.max(-1, Math.min(1, ch[c][a + i])) * 32767, true);
    let s = '';
    const bytes = new Uint8Array(out.buffer);
    for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(s);
  };
})();
"""


def main():
    replay, out = sys.argv[1], Path(sys.argv[2])
    wide = "--wide" in sys.argv
    max_s = int(sys.argv[sys.argv.index("--wide") + 1]) if wide and len(sys.argv) > sys.argv.index("--wide") + 1 else MAX_S
    url = "http://localhost:5191/"
    tmp = Path(tempfile.mkdtemp(prefix="stackfall-frames-"))
    out.parent.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as pw:
        browser = pw.chromium.launch(args=["--autoplay-policy=no-user-gesture-required"])
        size = {"width": 1920, "height": 1080} if wide else {"width": 1080, "height": 1920}
        page = browser.new_page(viewport=size, device_scale_factor=1)
        page.add_init_script(CLOCK.replace("MAX_S", str(max_s)))
        page.goto(f"{url}?video={replay}{'&wide' if wide else ''}")
        # Loading is real async (wasm, fonts, replay fetch): tick until the title card is up.
        while page.evaluate("document.getElementById('videoTitle').classList.contains('hidden')"):
            page.evaluate("__step(1000 / 60)")
            page.wait_for_timeout(20)
        start = page.evaluate("__now()")
        frame, tail = 0, None
        while frame < max_s * FPS and (tail is None or frame < tail):
            page.screenshot(path=str(tmp / f"{frame:05d}.png"))
            page.evaluate("__step(1000 / 60)")
            frame += 1
            if tail is None and page.evaluate("!document.getElementById('result').classList.contains('hidden')"):
                tail = frame + int(TAIL_S * FPS)
            if frame % 300 == 0:
                print(f"{frame / FPS:.0f}s", flush=True)
        wav = page.evaluate(f"__renderAudio({start}, {start + frame * 1000 / FPS})")
        browser.close()
    audio = []
    if wav:
        (tmp / "audio.wav").write_bytes(base64.b64decode(wav))
        audio = ["-i", str(tmp / "audio.wav"), "-c:a", "aac", "-b:a", "192k", "-shortest"]
    subprocess.run(
        ["ffmpeg", "-y", "-loglevel", "error", "-framerate", str(FPS), "-i", str(tmp / "%05d.png"), *audio,
         "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18", "-movflags", "+faststart", str(out)],
        check=True,
    )
    shutil.rmtree(tmp)
    print(f"{frame} frames ({frame / FPS:.1f}s){' + audio' if wav else ', silent'} -> {out}")


if __name__ == "__main__":
    main()
