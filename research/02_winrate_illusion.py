"""Myth test #2 — the win-rate illusion and MFE-tuned take-profits.

Part A (synthetic): a strategy with an 85% win rate and negative expectancy
versus one with a 38% win rate and positive expectancy. After 50 trades the
first *looks* better far more often than it is.

Part B (real): take the trend-following trades on BTC daily bars, measure each
trade's MFE, then cap profits at a fraction of the median MFE. Win rate rises
mechanically; expectancy and total return do not — exactly the interview's
point about MFE-tuned TPs, with the second half it left out.
"""
from __future__ import annotations

import numpy as np
import pandas as pd

from _common import btc_daily_ohlc, save_results, md_table, banner, DAILY
from quant22.backtest.engine import CostModel, run_backtest
from quant22.strategies.trend import TrendParams, trend_positions


def part_a() -> dict:
    banner("02A — synthetic: high win rate ≠ positive expectancy")
    rng = np.random.default_rng(0)
    specs = {"A: 85% WR, +0.3R/-2.5R": (0.85, 0.3, -2.5), "B: 38% WR, +2.5R/-1R": (0.38, 2.5, -1.0)}
    out = {}
    for name, (p, w, l) in specs.items():
        n_paths, n_tr = 5000, 500
        wins = rng.random((n_paths, n_tr)) < p
        r = np.where(wins, w, l)
        cum = np.cumsum(r, axis=1)
        exp_ = p * w + (1 - p) * l
        out[name] = {"expectancy_R": exp_, "p_positive_after_50": float((cum[:, 49] > 0).mean()),
                     "p_positive_after_500": float((cum[:, -1] > 0).mean()),
                     "median_R_after_500": float(np.median(cum[:, -1])),
                     "p_ever_below_-20R": float((cum.min(axis=1) < -20).mean())}
        print(f"{name}: expectancy {exp_:+.3f}R | P(+ after 50 trades) {out[name]['p_positive_after_50']:.2f} | "
              f"P(+ after 500) {out[name]['p_positive_after_500']:.2f} | median after 500: {out[name]['median_R_after_500']:+.0f}R")
    return out


def part_b() -> tuple[pd.DataFrame, dict]:
    banner("02B — real BTC trend trades: TP as a fraction of median MFE")
    d = btc_daily_ohlc("2014-01-01")
    close = d["close"]
    params = TrendParams()
    tgt = trend_positions(close, params, DAILY)
    res = run_backtest(close, tgt, CostModel(), DAILY, high=d["high"], low=d["low"])
    trades = res.trades
    mfe_w = np.array([t.mfe for t in trades if t.pnl > 0])
    med_mfe = float(np.median(mfe_w))
    print(f"{len(trades)} trades, median MFE of winners {med_mfe:.1%}, win rate {res.trade_summary.win_rate:.1%}, "
          f"expectancy {res.trade_summary.expectancy*100:.2f}% per trade")
    hi, lo, cl = d["high"].values, d["low"].values, close.values
    cost = 2 * CostModel().per_side
    rows = []
    for frac in [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, None]:
        pnls = []
        for t in trades:
            entry = t.entry_price
            if frac is None:
                pnls.append(t.side * (t.exit_price / entry - 1) * t.size - cost * t.size)
                continue
            tp = frac * med_mfe
            hit = False
            for k in range(t.entry_idx, t.exit_idx + 1):
                if (t.side > 0 and hi[k] >= entry * (1 + tp)) or (t.side < 0 and lo[k] <= entry * (1 - tp)):
                    hit = True
                    break
            pnl = tp * t.size if hit else t.side * (t.exit_price / entry - 1) * t.size
            pnls.append(pnl - cost * t.size)
        pnls = np.array(pnls)
        rows.append({"tp_frac_of_median_MFE": "none (signal exit)" if frac is None else frac, "win_rate": float((pnls > 0).mean()),
                     "expectancy_pct": float(pnls.mean() * 100), "total_pct_sum": float(pnls.sum() * 100),
                     "profit_factor": float(pnls[pnls > 0].sum() / -pnls[pnls <= 0].sum()) if (pnls <= 0).any() else np.inf,
                     "n": len(pnls)})
    table = pd.DataFrame(rows)
    print(md_table(table))
    return table, {"n_trades": len(trades), "median_mfe_winners": med_mfe}


def main() -> None:
    a = part_a()
    table, info = part_b()
    md = "# Win-rate illusion\n\n## Synthetic\n\n" + md_table(pd.DataFrame(a).T.reset_index().rename(columns={"index": "strategy"})) + \
         "\n\n## Real BTC trend trades — TP at fraction of median MFE\n\n" + md_table(table) + "\n"
    save_results("02_winrate_illusion", {"synthetic": a, "tp_table": table.to_dict(orient="records"), **info}, md)


if __name__ == "__main__":
    main()
