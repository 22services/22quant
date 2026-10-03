"""Level- and pattern-based event studies with built-in placebo grids.

The structural trap of every "levels" system (round numbers, Fibonacci, pivots,
PO3/Goldbach, FVG zones): the more lines you draw, the more lines price
"respects". So every measurement here is reported *against a placebo of equal
density*: the same grid shifted by a random offset, or the same number of
random ratios. The edge is the difference, never the raw hit rate.

All detectors are vectorised; only the non-overlapping event selection loops,
and only over candidate events.
"""
from __future__ import annotations

from dataclasses import dataclass

import numpy as np
import pandas as pd

GOLDBACH_PCTS = np.array([0, 3, 11, 17, 29, 41, 47, 50, 53, 59, 71, 83, 89, 97, 100]) / 100.0
FIB_RATIOS = np.array([0.236, 0.382, 0.5, 0.618, 0.786])
PO3 = [3**k for k in range(1, 10)]


# ----------------------------------------------------------------------------- FVG
@dataclass
class FVGStats:
    n_events: int
    fill_rate: float            # price re-entered the gap within horizon
    full_fill_rate: float       # price traversed the entire gap within horizon
    median_bars_to_fill: float


def fvg_events(high: np.ndarray, low: np.ndarray, close: np.ndarray, horizon: int, min_gap_frac: float = 0.0) -> FVGStats:
    """ICT-style Fair Value Gap: bullish when low[t] > high[t-2] (3-candle imbalance),
    bearish when high[t] < low[t-2]. Then: does price *return into the gap* (fill)
    within ``horizon`` bars, and does it traverse it fully?"""
    n = len(close)
    t = np.arange(2, n - 1)
    bull = low[t] > high[t - 2]
    bear = high[t] < low[t - 2]
    gap_lo = np.where(bull, high[t - 2], np.where(bear, high[t], np.nan))
    gap_hi = np.where(bull, low[t], np.where(bear, low[t - 2], np.nan))
    ok = (bull | bear) & ((gap_hi - gap_lo) / close[t] >= min_gap_frac)
    ev = t[ok]
    is_bull = bull[ok]
    glo, ghi = gap_lo[ok], gap_hi[ok]
    fills, full, bars = np.zeros(ev.size, bool), np.zeros(ev.size, bool), np.full(ev.size, np.nan)
    for i, (e, b, lo_, hi_) in enumerate(zip(ev, is_bull, glo, ghi)):
        end = min(n, e + 1 + horizon)
        if b:
            touched = np.flatnonzero(low[e + 1:end] <= hi_)
            trav = (low[e + 1:end] <= lo_).any()
        else:
            touched = np.flatnonzero(high[e + 1:end] >= lo_)
            trav = (high[e + 1:end] >= hi_).any()
        fills[i] = touched.size > 0
        full[i] = trav
        if touched.size:
            bars[i] = touched[0] + 1
    if ev.size == 0:
        return FVGStats(0, np.nan, np.nan, np.nan)
    return FVGStats(int(ev.size), float(fills.mean()), float(full.mean()), float(np.nanmedian(bars)))


# ----------------------------------------------------------------------------- generic touch machinery
@dataclass
class LevelReaction:
    n_touches: int
    bounce_rate: float          # P(price is back on the approach side after k bars)
    mean_rejection: float       # mean return *away* from the level over k bars (positive = barrier)
    t_stat: float


def _reaction_from_levels(close: np.ndarray, level: np.ndarray, tol: float, k: int,
                          extra_mask: np.ndarray | None = None) -> LevelReaction:
    """Shared engine: ``level[t]`` is the level nearest to close[t]. A touch at t needs
    |close_t − level_t| ≤ tol·close_t, bar t−1 further than tol from level_t, and the
    optional ``extra_mask[t]``. Events are kept non-overlapping (≥ k bars apart)."""
    n = len(close)
    dist = np.abs(close - level) / close
    near = dist <= tol
    prev_far = np.r_[False, np.abs(close[:-1] - level[1:]) / close[:-1] > tol]
    cand = near & prev_far
    if extra_mask is not None:
        cand &= extra_mask
    cand[: 1] = False
    cand[n - k:] = False
    idx = np.flatnonzero(cand)
    keep, last = [], -10**9
    for t in idx:
        if t - last >= k:
            keep.append(t)
            last = t
    if not keep:
        return LevelReaction(0, np.nan, np.nan, np.nan)
    t = np.array(keep)
    approach = np.sign(level[t] - close[t - 1])
    fwd = close[t + k] / close[t] - 1.0
    r = -approach * fwd
    tstat = r.mean() / (r.std(ddof=1) / np.sqrt(r.size)) if r.size > 1 and r.std() > 0 else np.nan
    return LevelReaction(int(r.size), float((r > 0).mean()), float(r.mean()), float(tstat))


def grid_touch_reaction(close: np.ndarray, step_fn, offset: float, tol: float, k: int) -> LevelReaction:
    """Periodic grid ``level = (floor(price/step) + offset) × step``; ``step_fn(price)`` allows
    price-dependent spacing. ``offset`` in [0,1) shifts the grid (placebo)."""
    steps = np.array([step_fn(c) for c in close], dtype=float)
    base = np.floor(close / steps) * steps
    cands = np.stack([base + (offset - 1) * steps, base + offset * steps, base + (offset + 1) * steps], axis=1)
    j = np.argmin(np.abs(cands - close[:, None]), axis=1)
    level = cands[np.arange(len(close)), j]
    return _reaction_from_levels(close, level, tol, k)


def round_number_step(step: float):
    return lambda price: step


def po3_step(po3: int):
    return lambda price: float(po3)


def goldbach_levels_reaction(close: np.ndarray, po3: int, pcts: np.ndarray, tol: float, k: int) -> LevelReaction:
    """Touches of internal Goldbach levels (``pcts`` of the dealing range of size ``po3``)."""
    dr_low = np.floor(close / po3) * po3
    lv = dr_low[:, None] + pcts[None, :] * po3
    j = np.argmin(np.abs(lv - close[:, None]), axis=1)
    level = lv[np.arange(len(close)), j]
    return _reaction_from_levels(close, level, tol, k)


# ----------------------------------------------------------------------------- Fibonacci retracements
def fib_retracement_reaction(close: np.ndarray, high: np.ndarray, low: np.ndarray, swing_window: int,
                             ratios: np.ndarray, tol: float, k: int) -> LevelReaction:
    """Swing = max/min over the previous ``swing_window`` bars. If the high came after
    the low the impulse is up and we watch pullbacks into ``low + ratio·range``
    (bounce = price higher k bars later); symmetric for down impulses."""
    n = len(close)
    hs = pd.Series(high)
    ls = pd.Series(low)
    sh = hs.rolling(swing_window).max().shift(1).values
    sl = ls.rolling(swing_window).min().shift(1).values
    # position (bars ago) of the swing extreme, to know which came later
    ih = hs.rolling(swing_window).apply(np.argmax, raw=True).shift(1).values
    il = ls.rolling(swing_window).apply(np.argmin, raw=True).shift(1).values
    valid = ~np.isnan(sh) & ~np.isnan(sl) & (sh > sl)
    up = ih > il
    rng = sh - sl
    levels = np.where(up[:, None], sl[:, None] + ratios[None, :] * rng[:, None], sh[:, None] - ratios[None, :] * rng[:, None])
    j = np.nanargmin(np.abs(np.where(valid[:, None], levels, np.inf) - close[:, None]), axis=1)
    level = levels[np.arange(n), j]
    level = np.where(valid, level, np.inf)
    pullback = np.where(up, close < sh, close > sl) & valid
    res = _reaction_from_levels(close, np.where(np.isfinite(level), level, 1e18), tol, k, extra_mask=pullback)
    if res.n_touches == 0:
        return res
    # _reaction_from_levels measured rejection relative to the approach direction; for
    # retracements we want "did the impulse resume": recompute on the kept events.
    dist = np.abs(close - level) / close
    near = dist <= tol
    prev_far = np.r_[False, np.abs(close[:-1] - level[1:]) / close[:-1] > tol]
    cand = near & prev_far & pullback
    cand[:1] = False
    cand[n - k:] = False
    idx = np.flatnonzero(cand)
    keep, last = [], -10**9
    for t in idx:
        if t - last >= k:
            keep.append(t)
            last = t
    t = np.array(keep)
    fwd = close[t + k] / close[t] - 1.0
    r = np.where(up[t], fwd, -fwd)
    tstat = r.mean() / (r.std(ddof=1) / np.sqrt(r.size)) if r.size > 1 and r.std() > 0 else np.nan
    return LevelReaction(int(r.size), float((r > 0).mean()), float(r.mean()), float(tstat))
