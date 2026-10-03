"""Does account size let you run the system on regulated CME contracts?

The trend sleeve wants continuous exposures (e.g. 0.43× equity). Exchange
contracts come in lumps: CME Micro Bitcoin = 0.1 BTC (≈ $8.5k at $85k),
Micro Ether = 0.1 ETH, Bitcoin Friday futures are 0.02 BTC (weekly expiry, so rolls every week), and a
perp DEX like Hyperliquid accepts ~$10 orders. Rounding to whole contracts adds
tracking error and, at small sizes, turns a smooth vol-targeted position into
an on/off bet. We measure the damage for several account sizes.
"""
from __future__ import annotations

import numpy as np
import pandas as pd

from _common import btc_daily_ohlc, save_results, md_table, banner, DAILY
from quant22.backtest.engine import CostModel, run_backtest
from quant22.strategies.trend import TrendParams, trend_positions


def rounded_exposure(target: pd.Series, prices: pd.Series, equity: float, contract_btc: float) -> pd.Series:
    """Whole-contract exposure for a constant-equity account (no compounding, to isolate rounding)."""
    contracts = np.round(target * equity / (contract_btc * prices))
    return contracts * contract_btc * prices / equity


def main() -> None:
    banner("12 — Contract granularity: continuous vs whole contracts")
    d = btc_daily_ohlc("2018-01-01")
    close = d["close"]
    tgt = trend_positions(close, TrendParams(), DAILY)
    base = run_backtest(close, tgt, CostModel(), DAILY, with_trades=False)
    rows = [{"venue_contract": "continuous (perp DEX, ~$10 min)", "equity_usd": np.nan, "sharpe": base.summary.sharpe,
             "cagr": base.summary.cagr, "mdd": base.summary.max_drawdown, "tracking_error_ann": 0.0, "share_days_flat_by_rounding": 0.0}]
    for label, size in [("CME Micro BTC (0.1 BTC)", 0.1), ("CME Bitcoin Friday (0.02 BTC, weekly expiry)", 0.02)]:
        for eq in (5_000, 10_000, 25_000, 50_000, 100_000):
            ex = rounded_exposure(tgt, close, eq, size)
            r = run_backtest(close, ex, CostModel(fee_bps=2.0, slippage_bps=2.0), DAILY, with_trades=False)
            te = float((r.returns - base.returns).std() * np.sqrt(DAILY))
            flat = float(((ex == 0) & (tgt.abs() > 0.05)).mean())
            rows.append({"venue_contract": label, "equity_usd": eq, "sharpe": r.summary.sharpe, "cagr": r.summary.cagr,
                         "mdd": r.summary.max_drawdown, "tracking_error_ann": te, "share_days_flat_by_rounding": flat})
            print(f"{label:<44} equity ${eq:>7,}: Sharpe {r.summary.sharpe:.2f} (cont. {base.summary.sharpe:.2f}), "
                  f"tracking error {te:.1%}, wanted exposure but held 0 contracts on {flat:.0%} of days")
    tab = pd.DataFrame(rows)
    save_results("12_contract_granularity", {"table": rows}, "# Contract granularity\n\n" + md_table(tab) + "\n")


if __name__ == "__main__":
    main()
