"""Myth test #1 — Fair Value Gaps (ICT) vs a random walk.

Replicates the experiment described in the interview: measure how often price
"comes back into" a Fair Value Gap on real data, then run the *same detector*
on random-walk surrogates. If the random walk fills gaps as often (or more),
the FVG has no predictive content — it is a property of any wiggly line.

Surrogates:
  (1) Gaussian GBM with matched drift/vol and synthetic highs/lows;
  (2) block-shuffled *real* bars (keeps fat tails, vol clustering and the real
      intrabar geometry; destroys only the serial order beyond the block).
"""
from __future__ import annotations

import numpy as np
import pandas as pd

from _common import load_hourly_btc, save_results, md_table, banner
from quant22.features.levels import fvg_events

HORIZONS = {"1h": [12, 24, 72], "4h": [12, 24, 72], "1D": [5, 10, 20]}
N_SURR = 100


def shuffled_bars(df: pd.DataFrame, block: int, rng: np.random.Generator) -> pd.DataFrame:
    """Block-shuffle real bars: keep each bar's (close-to-close log return, high/close, low/close)."""
    c = df["close"].values
    r = np.diff(np.log(c))
    up = (df["high"].values / df["close"].values)[1:]
    dn = (df["low"].values / df["close"].values)[1:]
    n = len(r)
    nb = int(np.ceil(n / block))
    order = rng.permutation(nb)
    idx = np.concatenate([np.arange(b * block, min((b + 1) * block, n)) for b in order])
    r2, up2, dn2 = r[idx], up[idx], dn[idx]
    close = c[0] * np.exp(np.cumsum(r2))
    # open of bar t = close of t-1; high/low keep the real ratio to close; ensure high>=max(open,close)
    open_ = np.r_[c[0], close[:-1]]
    high = np.maximum(close * up2, np.maximum(open_, close))
    low = np.minimum(close * dn2, np.minimum(open_, close))
    return pd.DataFrame({"open": open_, "high": high, "low": low, "close": close})


def gbm_bars(df: pd.DataFrame, rng: np.random.Generator) -> pd.DataFrame:
    c = df["close"].values
    r = np.diff(np.log(c))
    sim = rng.normal(r.mean(), r.std(ddof=1), size=r.size)
    close = c[0] * np.exp(np.cumsum(sim))
    open_ = np.r_[c[0], close[:-1]]
    # intrabar excursions sampled from the real (high/close, low/close) distribution
    up = rng.choice((df["high"].values / df["close"].values)[1:], size=r.size)
    dn = rng.choice((df["low"].values / df["close"].values)[1:], size=r.size)
    high = np.maximum(close * up, np.maximum(open_, close))
    low = np.minimum(close * dn, np.minimum(open_, close))
    return pd.DataFrame({"open": open_, "high": high, "low": low, "close": close})


def main() -> None:
    banner("01 — Fair Value Gap fill rate: real BTC vs random-walk surrogates")
    h = load_hourly_btc("2017-01-01")
    frames = {"1h": h, "4h": h.resample("4h").agg({"open": "first", "high": "max", "low": "min", "close": "last"}).dropna(),
              "1D": h.resample("1D").agg({"open": "first", "high": "max", "low": "min", "close": "last"}).dropna()}
    rng = np.random.default_rng(0)
    rows = []
    for tf, df in frames.items():
        for hz in HORIZONS[tf]:
            real = fvg_events(df["high"].values, df["low"].values, df["close"].values, hz)
            g_fill, s_fill, g_full, s_full = [], [], [], []
            for _ in range(N_SURR):
                g = gbm_bars(df, rng)
                s = shuffled_bars(df, 24 if tf == "1h" else 6, rng)
                fg = fvg_events(g["high"].values, g["low"].values, g["close"].values, hz)
                fs = fvg_events(s["high"].values, s["low"].values, s["close"].values, hz)
                g_fill.append(fg.fill_rate); s_fill.append(fs.fill_rate)
                g_full.append(fg.full_fill_rate); s_full.append(fs.full_fill_rate)
            rows.append({"timeframe": tf, "horizon_bars": hz, "real_n_fvg": real.n_events, "real_fill": real.fill_rate,
                         "gbm_fill": np.mean(g_fill), "gbm_fill_lo": np.percentile(g_fill, 2.5), "gbm_fill_hi": np.percentile(g_fill, 97.5),
                         "shuffled_fill": np.mean(s_fill), "shuffled_lo": np.percentile(s_fill, 2.5), "shuffled_hi": np.percentile(s_fill, 97.5),
                         "real_full_fill": real.full_fill_rate, "gbm_full_fill": np.mean(g_full), "shuffled_full_fill": np.mean(s_full),
                         "real_median_bars_to_fill": real.median_bars_to_fill})
            print(f"{tf:>3} horizon={hz:>3}: real fill {real.fill_rate:.3f} (n={real.n_events}) | GBM {np.mean(g_fill):.3f} "
                  f"[{np.percentile(g_fill, 2.5):.3f},{np.percentile(g_fill, 97.5):.3f}] | shuffled-real {np.mean(s_fill):.3f}")
    table = pd.DataFrame(rows)
    # Does the direction matter? After a *fill* of a bullish FVG, is the forward return positive?
    df = frames["1h"]
    hi, lo, cl = df["high"].values, df["low"].values, df["close"].values
    fwd = []
    for t in range(2, len(cl) - 48):
        if lo[t] > hi[t - 2]:   # bullish FVG
            gap_hi = lo[t]
            touch = np.where(lo[t + 1:t + 25] <= gap_hi)[0]
            if touch.size:
                k = t + 1 + touch[0]
                fwd.append(cl[k + 24] / cl[k] - 1)   # 24h after the fill
        elif hi[t] < lo[t - 2]:
            gap_lo = hi[t]
            touch = np.where(hi[t + 1:t + 25] >= gap_lo)[0]
            if touch.size:
                k = t + 1 + touch[0]
                fwd.append(-(cl[k + 24] / cl[k] - 1))
    fwd = np.array(fwd)
    tstat = fwd.mean() / (fwd.std(ddof=1) / np.sqrt(fwd.size))
    uncond = np.abs(np.log(cl[24:]) - np.log(cl[:-24])).mean()
    print(f"\nDirectional content after a 1h FVG fill (24h fwd, in FVG direction): mean {fwd.mean()*1e4:.1f} bp, "
          f"t={tstat:.2f}, n={fwd.size}; typical 24h |move| {uncond*1e4:.0f} bp; round-trip taker cost ~15 bp")
    md = "# FVG fill rate — real vs random walk\n\n" + md_table(table) + \
         f"\n\nDirectional edge after fill (1h, 24h fwd): {fwd.mean()*1e4:.1f} bp, t={tstat:.2f}, n={fwd.size}\n"
    save_results("01_fvg_vs_random_walk", {"table": rows, "post_fill_mean_bp": fwd.mean() * 1e4, "post_fill_t": tstat,
                                            "n_fills": int(fwd.size)}, md)


if __name__ == "__main__":
    main()
