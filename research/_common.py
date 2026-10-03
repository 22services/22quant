"""Shared helpers for the research scripts."""
from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
import pandas as pd

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from quant22.data.loaders import load_hourly_btc, load_daily, load_daily_closes, load_funding, load_binance_daily  # noqa: E402
from quant22.backtest.metrics import sharpe_ratio, max_drawdown, cagr  # noqa: E402
from quant22.utils import save_results, md_table, banner  # noqa: E402

DAILY = 365.25
HOURLY = 24 * 365.25
PERIODS = ["2014-01-01", "2017-01-01", "2019-01-01", "2021-01-01", "2023-01-01", "2025-01-01", "2027-01-01"]


def btc_daily_ohlc(start: str = "2014-01-01") -> pd.DataFrame:
    h = load_hourly_btc(start)
    d = h.resample("1D").agg({"open": "first", "high": "max", "low": "min", "close": "last", "volume_usd": "sum"})
    return d.dropna(subset=["close"])


def period_table(returns: pd.Series, bars_per_year: float, bounds: list[str] = PERIODS, label: str = "strategy",
                 benchmark: pd.Series | None = None) -> pd.DataFrame:
    rows = []
    for a, b in zip(bounds[:-1], bounds[1:]):
        r = returns.loc[a:b]
        if len(r) < 30:
            continue
        row = {"period": f"{a[:4]}-{int(b[:4]) - 1}", "bars": len(r), f"{label}_sharpe": sharpe_ratio(r, bars_per_year),
               f"{label}_cagr": cagr(r, bars_per_year), f"{label}_mdd": max_drawdown(r)}
        if benchmark is not None:
            bm = benchmark.loc[a:b]
            row.update({"bh_sharpe": sharpe_ratio(bm, bars_per_year), "bh_cagr": cagr(bm, bars_per_year), "bh_mdd": max_drawdown(bm)})
        rows.append(row)
    return pd.DataFrame(rows)


def verdict(effect: float, placebo: np.ndarray, higher_is_better: bool = True) -> tuple[str, float]:
    """Compare an observed effect to a placebo distribution; return (label, p-value)."""
    placebo = np.asarray(placebo, dtype=float)
    placebo = placebo[~np.isnan(placebo)]
    if placebo.size == 0 or np.isnan(effect):
        return "NOT PROVEN (no data)", np.nan
    p = float((np.sum(placebo >= effect) + 1) / (placebo.size + 1)) if higher_is_better else \
        float((np.sum(placebo <= effect) + 1) / (placebo.size + 1))
    if p < 0.01:
        return "DISTINGUISHABLE from placebo (p<0.01)", p
    if p < 0.05:
        return "weakly distinguishable (p<0.05)", p
    return "INDISTINGUISHABLE from placebo", p
