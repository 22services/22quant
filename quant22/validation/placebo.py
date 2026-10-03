"""Null models. An edge is only the *difference* between a rule and its placebo.

* :func:`stationary_bootstrap` – Politis & Romano (1994) resampling that keeps
  short-range dependence (volatility clustering) intact.
* :func:`bootstrap_sharpe_pvalue` – p-value of an observed Sharpe under the null
  of zero mean, preserving the return distribution's shape.
* :func:`block_permutation_pvalue` – destroys the alignment between a signal
  and future returns (block-shuffles the signal) while keeping both marginals.
* :func:`gbm_surrogates` / :func:`bootstrap_surrogate_prices` – random-walk price
  paths with matched volatility, for testing "does price revisit X" claims.
* :func:`placebo_level_offsets` – random offsets for level-based tests (round
  numbers, Fibonacci, PO3 / Goldbach) that preserve line density.
"""
from __future__ import annotations

from typing import Callable

import numpy as np
import pandas as pd


def stationary_bootstrap(x: np.ndarray, n_samples: int, mean_block: int, rng: np.random.Generator) -> np.ndarray:
    """Return an array (n_samples, len(x)) of stationary-bootstrap resamples."""
    n = len(x)
    p = 1.0 / max(mean_block, 1)
    out = np.empty((n_samples, n))
    for s in range(n_samples):
        idx = np.empty(n, dtype=int)
        idx[0] = rng.integers(n)
        new_block = rng.random(n) < p
        starts = rng.integers(0, n, size=n)
        for t in range(1, n):
            idx[t] = starts[t] if new_block[t] else (idx[t - 1] + 1) % n
        out[s] = x[idx]
    return out


def bootstrap_sharpe_pvalue(returns: pd.Series | np.ndarray, n_boot: int = 2000, mean_block: int = 20,
                            seed: int = 0) -> tuple[float, float, np.ndarray]:
    """One-sided p-value that the true mean return is <= 0.

    Returns (p_value, observed_sharpe_per_bar, bootstrap_sharpes).
    """
    r = np.asarray(returns, dtype=float)
    r = r[~np.isnan(r)]
    rng = np.random.default_rng(seed)
    obs = r.mean() / r.std(ddof=1) if r.std(ddof=1) > 0 else 0.0
    centred = r - r.mean()
    samples = stationary_bootstrap(centred, n_boot, mean_block, rng)
    sd = samples.std(axis=1, ddof=1)
    with np.errstate(divide="ignore", invalid="ignore"):
        bs = np.where(sd > 0, samples.mean(axis=1) / sd, 0.0)
    p = float((np.sum(bs >= obs) + 1) / (n_boot + 1))
    return p, float(obs), bs


def block_permutation_pvalue(signal: pd.Series, forward_returns: pd.Series, stat: Callable[[np.ndarray, np.ndarray], float],
                             n_perm: int = 1000, block: int = 24, seed: int = 0) -> tuple[float, float, np.ndarray]:
    """Permute the *signal* in blocks relative to the returns and recompute ``stat``.

    Keeps the autocorrelation structure of both series; destroys only the
    signal→return alignment, which is exactly the thing a forecaster claims.
    """
    s = np.asarray(signal, dtype=float)
    r = np.asarray(forward_returns, dtype=float)
    ok = ~(np.isnan(s) | np.isnan(r))
    s, r = s[ok], r[ok]
    rng = np.random.default_rng(seed)
    obs = stat(s, r)
    n = len(s)
    nb = int(np.ceil(n / block))
    perms = np.empty(n_perm)
    for i in range(n_perm):
        order = rng.permutation(nb)
        idx = np.concatenate([np.arange(b * block, min((b + 1) * block, n)) for b in order])
        shift = rng.integers(n)
        perms[i] = stat(np.roll(s[idx], shift), r)
    p = float((np.sum(perms >= obs) + 1) / (n_perm + 1))
    return p, float(obs), perms


def gbm_surrogates(log_returns: np.ndarray, n_paths: int, seed: int = 0, start_price: float = 100.0) -> np.ndarray:
    """Gaussian random walks with the same per-bar mean and volatility.

    Shape (n_paths, len(log_returns)+1) of price levels.
    """
    rng = np.random.default_rng(seed)
    lr = np.asarray(log_returns, dtype=float)
    lr = lr[~np.isnan(lr)]
    sims = rng.normal(lr.mean(), lr.std(ddof=1), size=(n_paths, lr.size))
    return np.hstack([np.full((n_paths, 1), start_price), start_price * np.exp(np.cumsum(sims, axis=1))])


def bootstrap_surrogate_prices(log_returns: np.ndarray, n_paths: int, mean_block: int = 24, seed: int = 0,
                               start_price: float = 100.0) -> np.ndarray:
    """Random-walk paths built from *resampled real* returns: same fat tails and
    volatility clustering as the data, but no predictable structure across blocks."""
    rng = np.random.default_rng(seed)
    lr = np.asarray(log_returns, dtype=float)
    lr = lr[~np.isnan(lr)]
    samples = stationary_bootstrap(lr - lr.mean(), n_paths, mean_block, rng) + lr.mean()
    return np.hstack([np.full((n_paths, 1), start_price), start_price * np.exp(np.cumsum(samples, axis=1))])


def ohlc_from_path(path: np.ndarray, intrabar_vol: float, rng: np.random.Generator) -> pd.DataFrame:
    """Build plausible high/low around a close path: high = max(o,c)·(1+|ε|), low = min(o,c)·(1−|ε|)."""
    close = path[1:]
    open_ = path[:-1]
    e1 = np.abs(rng.normal(0, intrabar_vol, size=close.size))
    e2 = np.abs(rng.normal(0, intrabar_vol, size=close.size))
    high = np.maximum(open_, close) * (1 + e1)
    low = np.minimum(open_, close) * (1 - e2)
    return pd.DataFrame({"open": open_, "high": high, "low": low, "close": close})


def placebo_level_offsets(n: int, rng: np.random.Generator) -> np.ndarray:
    """Uniform random offsets in [0,1) used to shift a periodic level grid."""
    return rng.random(n)
