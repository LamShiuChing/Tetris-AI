"""Train the Stackfall value net on self-play data and export it for the Rust engine.

Usage: python train/train.py data/selfplay.bin [more.bin ...] --out nets/value.bin

Input rows are little-endian f32: INPUTS features then the target (discounted return), as written
by `cargo run --release --example selfplay`. The net (INPUTS -> 64 -> 32 -> 1, ReLU) and the export
layout must match src/net.rs: scale, w1 [64 x INPUTS], b1, w2 [32 x 64], b2, w3 [32], b3.
"""

import argparse

import numpy as np
import torch
from torch import nn

INPUTS = 10 + 10 + 6 * 10 + 6
H1, H2 = 64, 32


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("data", nargs="+")
    ap.add_argument("--out", default="nets/value.bin")
    ap.add_argument("--epochs", type=int, default=30)
    ap.add_argument("--batch", type=int, default=4096)
    args = ap.parse_args()

    rows = np.concatenate([np.fromfile(p, dtype="<f4").reshape(-1, INPUTS + 1) for p in args.data])
    x, y = rows[:, :INPUTS], rows[:, INPUTS]
    scale = float(np.abs(y).std()) or 1.0
    print(f"{len(rows)} positions, target mean {y.mean():.2f} std {y.std():.2f}, scaling by {scale:.2f}")

    rng = np.random.default_rng(0)
    idx = rng.permutation(len(rows))
    split = int(len(rows) * 0.9)
    dev = "cuda" if torch.cuda.is_available() else "cpu"
    xt = torch.tensor(x[idx[:split]], device=dev)
    yt = torch.tensor(y[idx[:split]] / scale, device=dev)
    xv = torch.tensor(x[idx[split:]], device=dev)
    yv = torch.tensor(y[idx[split:]] / scale, device=dev)

    torch.manual_seed(0)
    net = nn.Sequential(nn.Linear(INPUTS, H1), nn.ReLU(), nn.Linear(H1, H2), nn.ReLU(), nn.Linear(H2, 1)).to(dev)
    opt = torch.optim.AdamW(net.parameters(), lr=2e-3, weight_decay=1e-4)
    sched = torch.optim.lr_scheduler.CosineAnnealingLR(opt, args.epochs)
    loss_fn = nn.MSELoss()
    baseline = float(((yv - yt.mean()) ** 2).mean())
    print(f"device {dev}; predicting the mean gives val MSE {baseline:.4f}")

    for epoch in range(args.epochs):
        net.train()
        perm = torch.randperm(len(xt), device=dev)
        for i in range(0, len(xt), args.batch):
            b = perm[i : i + args.batch]
            opt.zero_grad()
            loss = loss_fn(net(xt[b]).squeeze(1), yt[b])
            loss.backward()
            opt.step()
        sched.step()
        net.eval()
        with torch.no_grad():
            val = float(loss_fn(net(xv).squeeze(1), yv))
        if epoch % 5 == 4 or epoch == args.epochs - 1:
            print(f"epoch {epoch + 1:>3}: val MSE {val:.4f} (explains {100 * (1 - val / baseline):.1f}% of variance)")

    l1, _, l2, _, l3 = net
    parts = [
        np.array([scale], dtype="<f4"),
        l1.weight.detach().cpu().numpy(), l1.bias.detach().cpu().numpy(),
        l2.weight.detach().cpu().numpy(), l2.bias.detach().cpu().numpy(),
        l3.weight.detach().cpu().numpy().reshape(-1), l3.bias.detach().cpu().numpy(),
    ]
    np.concatenate([p.astype("<f4").reshape(-1) for p in parts]).tofile(args.out)
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
