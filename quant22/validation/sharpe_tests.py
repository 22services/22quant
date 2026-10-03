"""Probabilistic / Deflated Sharpe Ratio and Minimum Track Record Length.

References
----------
* Bailey, D. H. & López de Prado, M. (2012). "The Sharpe Ratio Efficient Frontier",
  Journal of Risk 15(2). (PSR, MinTRL)
* Bailey, D. H. & López de Prado, M. (2014). "The Deflated Sharpe Ratio: Correcting
  for Selection Bias, Backtest Overfitting and Non-Normality", Journal of Portfolio
  Management 40(5). (DSR, expected maximum Sharpe of N trials)

All Sharpe ratios in this module are **per-period** (not annualised) unless the
function name says otherwise. ``kurt`` is *raw* kurtosis (normal = 3).
"""
from __future__ import annotations

import numpy as np
from scipy.stats import norm

EULER_MASCHERONI = 0.5772156649015329


def probabilistic_sharpe_ratio(sr: float, sr_benchmark: float, n_obs: int, skew: float = 0.0,
                               kurt: float = 3.0) -> float:
    """P[true SR > sr_benchmark] given an observed per-period ``sr`` over ``n_obs`` bars."""
    if n_obs < 2:
        return 0.0
    denom = np.sqrt(max(1.0 - skew * sr + (kurt - 1.0) / 4.0 * sr**2, 1e-12))
    z = (sr - sr_benchmark) * np.sqrt(n_obs - 1.0) / denom
    return float(norm.cdf(z))


def expected_max_sharpe(n_trials: int, var_sr: float, sr_mean: float = 0.0) -> float:
    """Expected maximum Sharpe among ``n_trials`` independent trials whose true
    Sharpe is ``sr_mean`` (default 0) and whose estimated Sharpes have variance
    ``var_sr``. Eq. (3) in Bailey & López de Prado (2014)."""
    if n_trials <= 1:
        return sr_mean
    sd = np.sqrt(max(var_sr, 0.0))
    g = EULER_MASCHERONI
    return float(sr_mean + sd * ((1 - g) * norm.ppf(1 - 1.0 / n_trials) + g * norm.ppf(1 - 1.0 / (n_trials * np.e))))


def deflated_sharpe_ratio(sr: float, n_obs: int, trial_sharpes: np.ndarray, skew: float = 0.0,
                          kurt: float = 3.0) -> tuple[float, float]:
    """DSR of the *selected* (best) strategy given all ``trial_sharpes`` tried.

    Returns (dsr, sr_benchmark). DSR < 0.95 => the best backtest is not
    distinguishable from the best of N noise strategies.
    """
    ts = np.asarray(trial_sharpes, dtype=float)
    ts = ts[~np.isnan(ts)]
    var = float(ts.var(ddof=1)) if ts.size > 1 else 0.0
    sr0 = expected_max_sharpe(ts.size, var)
    return probabilistic_sharpe_ratio(sr, sr0, n_obs, skew, kurt), sr0


def min_track_record_length(sr: float, sr_benchmark: float = 0.0, skew: float = 0.0, kurt: float = 3.0,
                            confidence: float = 0.95) -> float:
    """Number of bars needed for PSR to reach ``confidence`` at the observed SR."""
    if sr <= sr_benchmark:
        return np.inf
    z = norm.ppf(confidence)
    return float(1.0 + (1.0 - skew * sr + (kurt - 1.0) / 4.0 * sr**2) * (z / (sr - sr_benchmark))**2)


def annualized_to_per_period(sr_ann: float, bars_per_year: float) -> float:
    return sr_ann / np.sqrt(bars_per_year)


def per_period_to_annualized(sr: float, bars_per_year: float) -> float:
    return sr * np.sqrt(bars_per_year)
