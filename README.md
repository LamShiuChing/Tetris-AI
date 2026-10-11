# Stackfall + Tetris AI

A block-stacking duel game for the browser, and the bot that plays it. Everything here is a
free, non-commercial fan project — not affiliated with or endorsed by any game publisher.

- **Stackfall** (`web/`): Puyo-Puyo-style juicy 1v1 against bots, plus Marathon and Master
  (20G) solo modes, bot-vs-bot watching, and replays. Pixi.js + Web Audio, all sound synthesized.
- **The bot** (`src/`, Rust): a beam search over every reachable placement (full SRS kicks,
  T-spins, hold, garbage, bag speculation), scored by a hand-tuned evaluation plus, for the top
  level, a small neural network trained by self-play. The same Rust engine is the game's
  rules in the browser (WebAssembly), so the bot and the game can never disagree on physics.

## Run it

```sh
# browser game
cd web
npm install
npm run wasm        # builds the Rust engine to web/public/engine.wasm (needs Rust + wasm32 target)
npm run dev         # then open the URL Vite prints

# engine tests
cargo test --release --lib
```

Rust targets used: native (tests, tools) and `wasm32-unknown-unknown` (browser). Native builds
use `target-cpu=native` and fat LTO (see `.cargo/config.toml`, `Cargo.toml`).

## How the bot works

```
 new piece ─▶ list every legal landing (BFS over moves, rotations, SRS kicks)
          ─▶ look ahead through hold + 5 previews (+1 bag-speculated piece at the top levels),
             keeping the best 64 lines at each step (beam search)
          ─▶ score each line: attack/points earned on the way
                             + board judgement (holes, height, wells, T-slots, B2B, setups,
                               incoming garbage, ...) [+ value net for Titan]
          ─▶ play the first move of the best line, think again next piece
```

| File | What |
|---|---|
| `src/piece.rs`, `board.rs` | pieces, SRS kick tables, 10x40 bitboard |
| `src/movegen.rs` | every reachable placement + the key presses to get there (zero gravity and 20G) |
| `src/game.rs` | rules config, 7-bag, scoring (`points`) and versus attack table (`attack`) |
| `src/live.rs` | the real-time game: gravity, lock delay, hold, garbage with cancelling |
| `src/bot.rs` | beam search, evaluation, difficulty levels, personalities, trick style |
| `src/setups.rs`, `setups_data.rs` | 1,939 opener/continuation stages from the Hard Drop wiki, each verified to be a reachable T-spin in this engine |
| `src/net.rs` | the value net (86 inputs → 64 → 32 → 1) and its features |
| `src/versus.rs` | headless bot-vs-bot matches (simultaneous turns, garbage), used for every comparison |
| `src/wasm.rs` | raw C-ABI exports for the browser |

### Bots in the game

| Bot | What it is |
|---|---|
| Levels 1–9 (Pebble … Legend) | search width/depth, mistakes, speed and skills (wells, T-spins, hold, garbage reading, openers) unlock with level |
| **Titan** (10) | full search + bag speculation + opener steering + value net |
| **Trickster** | Titan's strength, showman style: rewards flashy, varied tricks and named setups inside the search |
| **Chainlord** | Titan's strength, keeps back-to-back chains alive when it reasonably can |
| **Renegade** | Titan's strength, combo specialist: builds a 4-wide well and chains single clears |
| **Perfectionist** | Titan's strength, perfect-clear specialist: keeps the stack low and fillable |
| **Virtuoso** | Titan's strength, clears only with tricks (T-spins, minis, perfect clears, S/Z/J/L/I all-spins), never the same trick twice in a row; in versus it may clear plainly once garbage pushes its stack past 10 rows |

## Measured results

All bot-vs-bot numbers are win rates from seat-swapped matches on the headless versus engine
(± one standard error). Anything not clearly above 50% was not shipped as "better".

| Change | Result |
|---|---|
| Weight tuning (cross-entropy), MonochromeRush scoring | +47% score on 32 held-out games |
| Garbage awareness vs blind | 70.8% ± 6.6% |
| Bag speculation vs none | 58.3% ± 7.1% |
| Versus weight tuning at beam 16 | 65.6% ± 4.8% at beam 16, but 46.9% ± 6.2% at Titan's beam 64 → not used |
| Duplicate-state merging / beam 128 | 42.2% ± 6.2% / 54.7% ± 6.2% → off / not used |
| B2B-break penalty as default | 40.6% ± 6.1% → only in Chainlord's personality |
| **Value net (Titan) vs hand evaluation** | **73.2% ± 2.8%** over 256 matches on unseen seeds (round-5 net; the round-3 net it replaced: 62.9% ± 3.0% on the same seeds) |
| Titan vs Cold Clear 2 (100 ms/move, our rules) | round-3 net: 9–11 over two 10-match series (6–4, then 3–7). Round-5 net: 3–7. Cold Clear sends far more in long matches |
| Titan's beam width vs Cold Clear 2 (32 matches each, same seeds) | 64: 15–17 · **256 (~140 ms): 23–9** (shipped) · 512 (~250 ms): 25–7 · 512 vs Cold Clear at 200 ms: 22–10 |
| Personalities vs Titan (16 matches, beam 64) | Trickster 62.5%, 1.1 named setups fired a match (Titan ~0.6) · Chainlord 50.0%, longest B2B chain 5.1 (Titan ~3.5–4) · Renegade 50.0%, longest combo 9.2 (Titan ~6) · Perfectionist 56.2%, 2.2 perfect clears a match (Titan ~0.7) |

Think time for Titan: 81 → 27 ms/piece single-threaded after the movegen/eval optimizations
(same moves), then 1.6–1.9x faster again (inline setup prefilter, sparse value-net eval, cached
speculation placements). At the shipped beam 256: ~110 ms natively, ~190 ms in the browser.

## Tools (`examples/`)

| Command | Purpose |
|---|---|
| `cargo run --release --example vs -- <seeds> <variant> [width]` | bot-vs-bot win rate (`net`, `trickster`, `chainlord`, `garbage`, `speculate`, ...) |
| `cargo run --release --example selfplay -- <matches> <out.bin> [net]` | self-play data for the value net |
| `python train/train.py data/*.bin --out nets/value.bin` | train the value net on the GPU (PyTorch) |
| `bash train/loop.sh <round>` | one self-play improvement round: play, retrain, test |
| `cargo run --release --example vs_coldclear -- [matches] [ms] [level]` | Titan vs Cold Clear 2 over the Tetris Bot Protocol; writes replays to `web/public/replays/` |
| `cargo run --release --example tune_vs` / `tune` | cross-entropy weight tuning (versus / MonochromeRush) |
| `cargo run --release --example solo -- [level] [games] [pieces]` | a personality's solo play: each clear's kind, rule breaks, peak height |
| `cargo run --release --example speed` | single-threaded think time (`WIDTHS=64,256`) |
| `python web/render_video.py <replay> <out.mp4> [--wide [max s]]` | frame-exact MP4 of a replay: 9:16, or 16:9 with live search analysis (see the script for setup) |
| `bench`, `openers`, `spins`, `netprobe`, `netcheck` | other measurements and checks |

Cold Clear 2 isn't included: clone and build it into `external/cold-clear-2`
(https://github.com/MinusKelvin/cold-clear-2, MIT/Apache-2.0) to run `vs_coldclear`.

## Credits

- Setup shapes (opener and T-spin stage boards) come from the Hard Drop wiki, https://harddrop.com/wiki/ ;
  each kept shape is re-verified in this engine. The raw research catalogs stay out of the repo.
- Cold Clear 2 by MinusKelvin, used only as an external opponent for benchmarking.
- The first harness for the bot was MonochromeRush (a separate game).
