"""Growth-optimal sizing and the arithmetic of ruin.

The question "how do I make money as fast as possible?" has a precise answer:
maximise the expected *logarithmic* growth rate (Kelly 1956, Breiman 1961,
Thorp 2006). Betting more than the Kelly fraction makes you grow *slower* and
eventually go broke with probability one; betting at Kelly accepts drawdowns
most humans (and all prop firms) cannot tolerate; hence fractional Kelly.

Ergodicity-economics framing (Peters & Gell-Mann 2016): the ensemble average
(expected wealth) is irrelevant to an individual; the time-average growth rate
is what you actually experience. Everything below optimises the latter.
"""
from __future__ import annotations

from dataclasses import dataclass

import numpy as np


def kelly_binary(p_win: float, payoff_ratio: float) -> float:
    """Optimal fraction for a bet winning ``payoff_ratio``×stake with prob ``p_win``."""
    q = 1.0 - p_win
    return max(p_win - q / payoff_ratio, 0.0)


def kelly_continuous(mu: float, sigma: float) -> float:
    """f* = μ/σ² for per-period excess return μ and volatility σ (small-return approximation)."""
    return mu / sigma**2 if sigma > 0 else 0.0


def growth_rate(f: float, mu: float, sigma: float) -> float:
    """Expected log-growth per period at leverage ``f``: g(f) = fμ − f²σ²/2."""
    return f * mu - 0.5 * f**2 * sigma**2


def kelly_from_sharpe(sharpe_ann: float, vol_ann: float) -> float:
    """f* expressed with annual inputs: μ = SR·σ ⇒ f* = SR/σ."""
    return sharpe_ann / vol_ann if vol_ann > 0 else 0.0


def optimal_growth_ann(sharpe_ann: float) -> float:
    """g* = SR²/2 per year at full Kelly — the speed limit set by your edge."""
    return 0.5 * sharpe_ann**2


def time_to_multiply(target_multiple: float, sharpe_ann: float, kelly_fraction: float = 1.0) -> float:
    """Expected years to multiply capital by ``target_multiple`` at k×Kelly.

    g(k·f*) = (k − k²/2)·SR²  ⇒  T = ln(M) / g.
    """
    g = (kelly_fraction - 0.5 * kelly_fraction**2) * sharpe_ann**2
    return np.inf if g <= 0 else float(np.log(target_multiple) / g)


def prob_drawdown_to(fraction_of_peak: float, kelly_fraction: float) -> float:
    """P(wealth ever falls to ``fraction_of_peak`` of its current level) when betting
    k×Kelly continuously: x^(2/k − 1). Full Kelly: P(halve) = 0.5. Half Kelly: 0.125.
    (Thorp 2006, §7; exact for the continuous-time lognormal model.)"""
    if kelly_fraction <= 0:
        return 0.0
    if kelly_fraction >= 2:
        return 1.0
    return float(fraction_of_peak ** (2.0 / kelly_fraction - 1.0))


@dataclass
class RuinResult:
    p_ruin: float
    p_target: float
    median_final: float
    p5_final: float
    p95_final: float
    median_max_dd: float


def simulate_fixed_fraction(p_win: float, payoff_ratio: float, risk_fraction: float, n_trades: int,
                            ruin_level: float = 0.5, target_level: float = 2.0, n_sims: int = 20000,
                            seed: int = 0) -> RuinResult:
    """Monte Carlo of a binary-outcome strategy risking ``risk_fraction`` of equity per trade."""
    rng = np.random.default_rng(seed)
    wins = rng.random((n_sims, n_trades)) < p_win
    step = np.where(wins, 1.0 + risk_fraction * payoff_ratio, 1.0 - risk_fraction)
    eq = np.cumprod(step, axis=1)
    peak = np.maximum.accumulate(eq, axis=1)
    dd = (eq / peak - 1.0).min(axis=1)
    ruined = (eq <= ruin_level).any(axis=1)
    hit = (eq >= target_level).any(axis=1)
    final = eq[:, -1]
    return RuinResult(float(ruined.mean()), float(hit.mean()), float(np.median(final)), float(np.percentile(final, 5)),
                      float(np.percentile(final, 95)), float(np.median(dd)))
