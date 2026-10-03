"""Candidate edge #3 — short-horizon mean reversion on hourly BTC (2017→2026).

Known to exist in crypto (negative 1–4h return autocorrelation; liquidity
provision after taker-driven moves). The question is not "is it there" but
"who gets to keep it after costs": a taker pays ~15bp round trip; a maker ~3bp
plus adverse selection (not modelled — fills are assumed, which flatters the
maker line).
"""
from __future__ import annotations

import numpy as np
import pandas as pd

from _common import load_hourly_btc, save_results, md_table, banner, HOURLY, period_table
from quant22.backtest.engine import CostModel, run_backtest
from quant22.backtest.metrics import sharpe_ratio
from quant22.strategies.reversal import ReversalParams, reversal_positions
from quant22.validation.sharpe_tests import deflated_sharpe_ratio, probabilistic_sharpe_ratio
from quant22.backtest.metrics import skew_kurt
from quant22.validation.cscv import pbo_cscv


def main() -> None:
    banner("07 — Hourly mean reversion: gross edge vs costs")
    h = load_hourly_btc("2017-01-01")
    close = h["close"]
    r = np.log(close).diff()
    ac = [float(r.autocorr(lag)) for lag in (1, 2, 4, 8, 24)]
    print("log-return autocorrelation lags 1,2,4,8,24h:", [f"{a:+.3f}" for a in ac])
    grid = [ReversalParams(move_window=mw, z_entry=z, hold_bars=hb) for mw in (1, 4) for z in (2.0, 2.5, 3.0) for hb in (3, 6, 12)]
    rows, rets_gross = [], []
    for p in grid:
        tgt = reversal_positions(close, p)
        g = run_backtest(close, tgt, CostModel.zero(), HOURLY, with_trades=False)
        m = run_backtest(close, tgt, CostModel.maker(), HOURLY, with_trades=False)
        t = run_backtest(close, tgt, CostModel(), HOURLY, with_trades=False)
        rets_gross.append(g.returns.values)
        n_ev = int((tgt.diff().abs() > 0).sum() / 2)
        rows.append({"move_window": p.move_window, "z_entry": p.z_entry, "hold_bars": p.hold_bars, "n_trades": n_ev,
                     "sharpe_gross": g.summary.sharpe, "sharpe_maker": m.summary.sharpe, "sharpe_taker": t.summary.sharpe,
                     "cagr_gross": g.summary.cagr, "cagr_taker": t.summary.cagr, "avg_gross_bp_per_trade": g.returns.sum() / max(n_ev, 1) * 1e4})
    tab = pd.DataFrame(rows)
    print(md_table(tab))
    R = np.column_stack(rets_gross)
    pp = tab["sharpe_gross"].values / np.sqrt(HOURLY)
    b = int(np.argmax(pp))
    sk, ku = skew_kurt(R[:, b])
    dsr, bench = deflated_sharpe_ratio(pp[b], R.shape[0], pp, sk, ku)
    pbo = pbo_cscv(R, n_blocks=10)
    print(f"Gross grid: best Sharpe {tab['sharpe_gross'].max():.2f}, DSR {dsr:.3f}, PBO {pbo.pbo:.2f} | "
          f"share of variants positive after taker costs: {(tab['sharpe_taker'] > 0).mean():.0%}")
    best = grid[b]
    tgt = reversal_positions(close, best)
    g = run_backtest(close, tgt, CostModel.zero(), HOURLY, with_trades=False)
    per = period_table(g.returns, HOURLY, label="gross")
    print(md_table(per))
    save_results("07_reversal_hourly", {"autocorr": dict(zip(["1", "2", "4", "8", "24"], ac)), "grid": rows, "dsr_best_gross": dsr,
                                        "pbo_gross": pbo.pbo, "share_positive_after_taker": float((tab["sharpe_taker"] > 0).mean()),
                                        "best_gross_periods": per.to_dict(orient="records")},
                 "# Hourly reversal\n\n" + md_table(tab) + f"\n\nDSR best gross {dsr:.3f}, PBO {pbo.pbo:.2f}\n\n" + md_table(per) + "\n")


if __name__ == "__main__":
    main()
