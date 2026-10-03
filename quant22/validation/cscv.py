"""Probability of Backtest Overfitting via Combinatorially Symmetric Cross-Validation.

Reference: Bailey, Borwein, López de Prado & Zhu (2017), "The Probability of
Backtest Overfitting", Journal of Computational Finance 20(4).

Given a T×N matrix of per-bar returns for N strategy variants (the parameter
grid you searched), split time into S blocks; for every way to pick S/2 blocks
as in-sample, select the best IS variant and record its *rank* out-of-sample.
If selection carried information, the OOS rank should be high; PBO is the
fraction of splits in which the IS-best variant is below the OOS median.
"""
from __future__ import annotations

from dataclasses import dataclass
from itertools import combinations

import numpy as np


def _sharpe_cols(m: np.ndarray) -> np.ndarray:
    mu = m.mean(axis=0)
    sd = m.std(axis=0, ddof=1)
    with np.errstate(divide="ignore", invalid="ignore"):
        sr = np.where(sd > 0, mu / sd, 0.0)
    return sr


@dataclass
class PBOResult:
    pbo: float                     # probability of backtest overfitting
    n_splits: int
    logits: np.ndarray             # λ per split; λ<0 means IS-best was below OOS median
    is_sharpe_best: np.ndarray     # IS Sharpe of the selected variant per split
    oos_sharpe_best: np.ndarray    # OOS Sharpe of that same variant
    degradation_slope: float       # OLS slope of OOS vs IS Sharpe across splits (1 = no degradation)
    prob_oos_loss: float           # fraction of splits where the IS-best variant lost money OOS

    def to_dict(self) -> dict:
        return {"pbo": self.pbo, "n_splits": self.n_splits, "degradation_slope": self.degradation_slope,
                "prob_oos_loss": self.prob_oos_loss, "median_is_sharpe": float(np.median(self.is_sharpe_best)),
                "median_oos_sharpe": float(np.median(self.oos_sharpe_best))}


def pbo_cscv(returns_matrix: np.ndarray, n_blocks: int = 10, max_splits: int = 2000, seed: int = 0) -> PBOResult:
    m = np.asarray(returns_matrix, dtype=float)
    m = m[~np.isnan(m).any(axis=1)]
    T, N = m.shape
    if N < 2:
        raise ValueError("need at least two strategy variants")
    if n_blocks % 2:
        n_blocks += 1
    blocks = np.array_split(np.arange(T), n_blocks)
    combos = list(combinations(range(n_blocks), n_blocks // 2))
    rng = np.random.default_rng(seed)
    if len(combos) > max_splits:
        idx = rng.choice(len(combos), size=max_splits, replace=False)
        combos = [combos[i] for i in idx]
    logits, is_b, oos_b = [], [], []
    all_blocks = set(range(n_blocks))
    for c in combos:
        is_idx = np.concatenate([blocks[i] for i in c])
        oos_idx = np.concatenate([blocks[i] for i in sorted(all_blocks - set(c))])
        sr_is = _sharpe_cols(m[is_idx])
        sr_oos = _sharpe_cols(m[oos_idx])
        best = int(np.argmax(sr_is))
        # rank of best IS variant within OOS performance (1 = worst, N = best)
        rank = 1 + np.sum(sr_oos < sr_oos[best]) + 0.5 * (np.sum(sr_oos == sr_oos[best]) - 1)
        w = rank / (N + 1.0)
        logits.append(np.log(w / (1.0 - w)))
        is_b.append(sr_is[best])
        oos_b.append(sr_oos[best])
    logits = np.array(logits)
    is_b, oos_b = np.array(is_b), np.array(oos_b)
    slope = float(np.polyfit(is_b, oos_b, 1)[0]) if np.std(is_b) > 0 else np.nan
    return PBOResult(float(np.mean(logits <= 0)), len(combos), logits, is_b, oos_b, slope, float(np.mean(oos_b < 0)))
