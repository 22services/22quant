"""Combine the surviving edges: trend (BTC + multi-asset) and funding carry.

Reads the daily return streams written by scripts 05 and 06, aligns them on
the common window (2020→2026-04 where carry exists), and reports the
risk-weighted blend. Diversification across *mechanisms* is the only
reliable way to lift Sharpe without adding leverage.
"""
from __future__ import annotations

import numpy as np
import pandas as pd

from _common import save_results, md_table, banner, DAILY
from quant22.backtest.metrics import sharpe_ratio, max_drawdown, cagr


def main() -> None:
    banner("10 — Ensemble of surviving edges")
    tb = pd.read_parquet("results/_trend_btc_returns.parquet")
    tp = pd.read_parquet("results/_trend_portfolio_returns.parquet")["trend_portfolio"]
    cb = pd.read_parquet("results/_carry_btc_daily_returns.parquet")["carry_btc"]
    df = pd.concat([tb, tp, cb], axis=1).dropna()
    df = df.loc["2020-01-01":]
    print(f"Common window {df.index[0].date()} → {df.index[-1].date()} ({len(df)} days)")
    print("Correlations:\n", df.corr().round(2))
    vols = df.std() * np.sqrt(DAILY)
    # carry is ~1% vol: scale each sleeve to 20% vol, then blend
    scaled = df / vols * 0.20
    # Sleeves are expressed per unit of *capital*: trend sleeves at their native vol-targeted size,
    # carry at 1x notional (fully collateralised spot leg) or 2x (half the capital on margin).
    blends = {
        "BTC buy&hold": df["bh_btc"],
        "trend BTC": df["trend_btc"],
        "trend portfolio (9 coins)": df["trend_portfolio"],
        "carry BTC (1x notional, delta-neutral)": df["carry_btc"],
        "50/50 trend BTC + trend portfolio": 0.5 * df["trend_btc"] + 0.5 * df["trend_portfolio"],
        "trend BTC + carry 1x (capital split 50/50)": 0.5 * df["trend_btc"] + 0.5 * df["carry_btc"],
        "trend BTC (full) + carry 1x on the idle cash (~65% of capital sits in cash)": df["trend_btc"] + 0.65 * df["carry_btc"],
        "trend portfolio + carry 2x on idle cash": df["trend_portfolio"] + 0.65 * 2 * df["carry_btc"],
    }
    rows = [{"sleeve": k, "sharpe": sharpe_ratio(v, DAILY), "cagr": cagr(v, DAILY), "ann_vol": v.std() * np.sqrt(DAILY),
             "mdd": max_drawdown(v), "corr_btc": float(np.corrcoef(v, df["bh_btc"])[0, 1])} for k, v in blends.items()]
    tab = pd.DataFrame(rows)
    print(md_table(tab))
    save_results("10_ensemble", {"table": rows, "corr": df.corr().to_dict(), "window": [str(df.index[0].date()), str(df.index[-1].date())]},
                 "# Ensemble\n\n" + md_table(tab) + "\n\n" + md_table(df.corr().round(2).reset_index()) + "\n")


if __name__ == "__main__":
    main()
