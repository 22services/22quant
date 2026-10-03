"""Candidate edge #2 — perpetual funding carry (Binance BTC/ETH 8h, 2020→2026-04).

  1. Funding statistics and regime table (the carry is a *regime* income).
  2. Delta-neutral carry backtest with realistic costs (two legs!) and the
     opportunity cost of cash; taker vs maker.
  3. Does funding predict reversals? (quintile → forward return)
  4. Funding as a *cost* on a directional trend strategy on the perp.
"""
from __future__ import annotations

import numpy as np
import pandas as pd

from _common import load_funding, load_binance_daily, save_results, md_table, banner, DAILY
from quant22.strategies.carry import CarryParams, carry_backtest
from quant22.strategies.trend import TrendParams, trend_positions
from quant22.backtest.engine import CostModel, run_backtest


def main() -> None:
    banner("06 — Funding statistics")
    stats, regimes, carry_rows, quint_rows = [], [], [], []
    for a in ["btc", "eth"]:
        f = load_funding(a)
        bps = f * 1e4
        stats.append({"asset": a, "mean_bp_8h": bps.mean(), "median_bp_8h": bps.median(), "std_bp_8h": bps.std(),
                      "annualised": f.mean() * 3 * 365, "share_negative": float((f < 0).mean()), "skew": float(bps.skew()),
                      "kurtosis": float(bps.kurt() + 3), "share_at_default_1bp": float((bps.round(2) == 1.0).mean())})
        yr = f.groupby(f.index.year).agg(["mean", "count"])
        for y, row in yr.iterrows():
            regimes.append({"asset": a, "year": int(y), "ann_funding": row["mean"] * 3 * 365, "n": int(row["count"])})
        for lab, p in [("taker", CarryParams()), ("maker", CarryParams(fee_bps_per_leg_side=2.0)),
                       ("taker, entry 2bp", CarryParams(entry_bps=2.0, exit_bps=0.5))]:
            res = carry_backtest(f, p)
            s = res.summary
            carry_rows.append({"asset": a, "costs": lab, "net_cagr": s.cagr, "ann_vol": s.ann_vol, "sharpe_(inflated,flat 65%)": s.sharpe,
                               "mdd": s.max_drawdown, "n_trades": res.n_trades, "time_in_market": res.time_in_market,
                               "gross_funding_ann": res.gross_funding_ann})
            # by year net return
            by_year = res.returns.groupby(res.returns.index.year).sum()
            carry_rows[-1].update({f"net_{y}": v for y, v in by_year.items()})
        # funding quintile -> forward perp return
        perp = load_binance_daily(a, "perp")["close"]
        fd = f.resample("1D").sum().reindex(perp.index).dropna()
        q = pd.qcut(fd.rank(method="first"), 5, labels=[1, 2, 3, 4, 5])
        for h in [1, 3, 7]:
            fwd = perp.shift(-h) / perp - 1
            g = fwd.reindex(fd.index).groupby(q).mean()
            quint_rows.append({"asset": a, "horizon_d": h, **{f"Q{k}": v for k, v in g.items()}})
    stats, regimes, carry_tab, quint = pd.DataFrame(stats), pd.DataFrame(regimes), pd.DataFrame(carry_rows), pd.DataFrame(quint_rows)
    print(md_table(stats)); print(md_table(regimes.pivot(index="year", columns="asset", values="ann_funding").reset_index()))
    print(md_table(carry_tab[[c for c in carry_tab.columns if not c.startswith("net_2")]]))
    print(md_table(quint))

    banner("06 — Funding as a cost for a directional trend strategy on the BTC perp (2020→2026-04)")
    perp = load_binance_daily("btc", "perp")
    close = perp["close"]
    f_daily = load_funding("btc").resample("1D").sum().reindex(close.index).fillna(0.0)
    tgt = trend_positions(close, TrendParams(), DAILY)
    no_f = run_backtest(close, tgt, CostModel(), DAILY, with_trades=False)
    with_f = run_backtest(close, tgt, CostModel(funding=f_daily), DAILY, with_trades=False)
    long_share = float((no_f.positions > 0).mean())
    print(f"Trend on perp: Sharpe {no_f.summary.sharpe:.2f} ignoring funding → {with_f.summary.sharpe:.2f} paying/receiving funding; "
          f"CAGR {no_f.summary.cagr:.1%} → {with_f.summary.cagr:.1%}; long {long_share:.0%} of the time; "
          f"funding P&L {(-(with_f.costs - no_f.costs)).sum():+.2%} total")
    # Exploratory (3 variants, so treat as hypothesis, not result): skip longs when funding is extreme.
    from quant22.strategies.carry import funding_overlay
    overlay_rows = []
    for thr in (6.0, 10.0, 15.0):     # bp per day ≈ 22%, 37%, 55% annualised
        o = run_backtest(close, funding_overlay(tgt, f_daily, thr), CostModel(funding=f_daily), DAILY, with_trades=False)
        overlay_rows.append({"max_funding_bp_per_day": thr, "sharpe": o.summary.sharpe, "cagr": o.summary.cagr, "mdd": o.summary.max_drawdown})
        print(f"  funding overlay ≤{thr:.0f}bp/day: Sharpe {o.summary.sharpe:.2f}, CAGR {o.summary.cagr:.1%}, MDD {o.summary.max_drawdown:.1%}")
    spot = load_binance_daily("btc", "spot")["close"]
    tgt_s = trend_positions(spot, TrendParams(), DAILY)
    # longs on spot (no funding), shorts on the perp (receive funding when positive)
    f_short_only = f_daily.where(tgt_s.shift(1).reindex(f_daily.index).fillna(0) < 0, 0.0)
    hyb = run_backtest(spot, tgt_s, CostModel(funding=f_short_only), DAILY, with_trades=False)
    print(f"  longs via spot, shorts via perp: Sharpe {hyb.summary.sharpe:.2f}, CAGR {hyb.summary.cagr:.1%}, MDD {hyb.summary.max_drawdown:.1%}")
    md = ("# Funding carry\n\n## Stats\n\n" + md_table(stats) + "\n\n## Regimes\n\n" +
          md_table(regimes.pivot(index="year", columns="asset", values="ann_funding").reset_index()) +
          "\n\n## Carry backtests\n\n" + md_table(carry_tab) + "\n\n## Funding quintile → forward perp return\n\n" + md_table(quint) +
          f"\n\n## Trend on perp with/without funding\n\nSharpe {no_f.summary.sharpe:.2f} → {with_f.summary.sharpe:.2f}\n")
    save_results("06_carry_funding", {"stats": stats.to_dict(orient="records"), "regimes": regimes.to_dict(orient="records"),
                                      "carry": carry_tab.to_dict(orient="records"), "quintiles": quint.to_dict(orient="records"),
                                      "trend_on_perp": {"sharpe_no_funding": no_f.summary.sharpe, "sharpe_with_funding": with_f.summary.sharpe,
                                                        "cagr_no_funding": no_f.summary.cagr, "cagr_with_funding": with_f.summary.cagr,
                                                        "funding_overlay_exploratory": overlay_rows,
                                                        "longs_spot_shorts_perp": hyb.summary.to_dict()}}, md)
    carry_btc = carry_backtest(load_funding("btc"), CarryParams()).returns.resample("1D").sum()
    carry_btc.rename("carry_btc").to_frame().to_parquet("results/_carry_btc_daily_returns.parquet")


if __name__ == "__main__":
    main()
