"""Time-series momentum / trend following with volatility targeting.

Economic rationale (the "why should this exist"): slow diffusion of information,
herding and under-reaction, risk-management flows (stop-outs, deleveraging) and
in crypto specifically, the reflexivity of leverage: liquidation cascades extend
moves (Moskowitz, Ooi & Pedersen 2012; Hurst, Ooi & Pedersen 2017; Liu & Tsyvinski
2021 for crypto). Costs are low because turnover is low (signals change slowly).
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Sequence

import numpy as np
import pandas as pd

from ..features.signals import donchian_breakout, ewma_trend, tsmom_sign, vol_scaled_position


@dataclass
class TrendParams:
    lookbacks: Sequence[int] = (20, 60, 120)   # in bars; for daily data ≈ 1, 3, 6 months
    ewma_pairs: Sequence[tuple[int, int]] = ((8, 24), (16, 48), (32, 96))
    use_breakout: bool = True
    breakout_lookback: int = 55
    target_vol: float = 0.30                   # annualised, per asset (crypto vol is ~60-80%)
    vol_lookback: int = 30
    max_leverage: float = 1.5                  # gap risk: BTC fell 39% close-to-close on 2020-03-12; 2x would have been -78%
    long_only: bool = False
    weights: dict = field(default_factory=lambda: {"tsmom": 1.0, "ewma": 1.0, "breakout": 1.0})

    def to_dict(self) -> dict:
        return {"lookbacks": list(self.lookbacks), "ewma_pairs": [list(p) for p in self.ewma_pairs],
                "use_breakout": self.use_breakout, "breakout_lookback": self.breakout_lookback,
                "target_vol": self.target_vol, "vol_lookback": self.vol_lookback, "max_leverage": self.max_leverage,
                "long_only": self.long_only}


def trend_signal(prices: pd.Series, params: TrendParams) -> pd.Series:
    """Composite trend score in [-1, 1] (before vol scaling)."""
    parts = []
    w = params.weights
    if params.lookbacks and w.get("tsmom", 0):
        ts = pd.concat([tsmom_sign(prices, lb) for lb in params.lookbacks], axis=1).mean(axis=1)
        parts.append(w["tsmom"] * ts)
    if params.ewma_pairs and w.get("ewma", 0):
        ew = pd.concat([ewma_trend(prices, s, l) for s, l in params.ewma_pairs], axis=1).mean(axis=1)
        parts.append(w["ewma"] * ew.clip(-1, 1))
    if params.use_breakout and w.get("breakout", 0):
        parts.append(w["breakout"] * donchian_breakout(prices, params.breakout_lookback))
    total_w = sum(v for k, v in w.items() if (k != "breakout" or params.use_breakout))
    sig = pd.concat(parts, axis=1).sum(axis=1) / max(total_w, 1e-9)
    if params.long_only:
        sig = sig.clip(lower=0.0)
    return sig.fillna(0.0)


def trend_positions(prices: pd.Series, params: TrendParams, bars_per_year: float) -> pd.Series:
    sig = trend_signal(prices, params)
    return vol_scaled_position(sig, prices, params.target_vol, params.vol_lookback, bars_per_year, params.max_leverage)


def simple_tsmom_positions(prices: pd.Series, lookback: int, target_vol: float, vol_lookback: int,
                           bars_per_year: float, max_leverage: float = 1.5) -> pd.Series:
    """Single-lookback TSMOM; used for parameter-grid robustness scans (and PBO)."""
    sig = tsmom_sign(prices, lookback)
    return vol_scaled_position(sig, prices, target_vol, vol_lookback, bars_per_year, max_leverage)


def default_grid(bars_per_year: float) -> list[dict]:
    if bars_per_year > 1000:  # hourly
        lbs = [24, 48, 96, 168, 336, 504, 720, 1440]
        vls = [48, 168, 336]
    else:
        lbs = [5, 10, 20, 40, 60, 90, 120, 180, 250]
        vls = [20, 40, 60]
    return [{"lookback": lb, "vol_lookback": vl} for lb in lbs for vl in vls]
