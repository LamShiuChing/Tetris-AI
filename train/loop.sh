#!/usr/bin/env bash
# One self-play improvement round: the newest net (blend NET_WEIGHT) plays the formula bot,
# a new net trains on every round so far, and it's tested at blends 20 and 60.
# Usage: bash train/loop.sh <round number>   (needs nets/value.bin = previous round's net)
set -euo pipefail
r=$1
export PATH="$PATH:$HOME/.cargo/bin" NET_WEIGHT=60
cargo build --release --example selfplay --example vs 2>&1 | grep -E "^error" && exit 1
./target/release/examples/selfplay 2000 "data/gen${r}w.bin" net
python train/train.py data/gen*w.bin --out "nets/gen$((r + 1))w.bin" --epochs 30 | tail -1
cp "nets/gen$((r + 1))w.bin" nets/value.bin
cargo build --release --example vs 2>&1 | grep -E "^error" && exit 1
for k in 20 60; do echo "round $r net, weight $k: $(NET_WEIGHT=$k ./target/release/examples/vs 32 net)"; done
