"""Candidate edge #1 — time-series momentum with volatility targeting.

Protocol (fixed before looking at results):
  1. Parameter grid of single-lookback TSMOM on BTC daily (2014→2026-10), net
     of taker costs. Report every variant, the Deflated Sharpe of the best,
     and the Probability of Backtest Overfitting (CSCV) of the grid.
  2. The pre-specified composite (TrendParams defaults — three TSMOM
     lookbacks, three EWMA pairs, one breakout, vol-targeted) on BTC: full
     sample, sub-periods, bootstrap p-value, cost sensitivity, walk-forward.
  3. The same composite across 9 coins, equal-risk-weighted (diversification
     is the only free lunch).
  4. Hourly TSMOM: does a faster version survive costs?
"""
from __future__ import annotations

import numpy as np
import pandas as pd

from _common import (btc_daily_ohlc, load_daily_closes, load_hourly_btc, save_results, md_table, banner, period_table,
                     DAILY, HOURLY)
from quant22.backtest.engine import CostModel, run_backtest
from quant22.backtest.metrics import sharpe_ratio, skew_kurt, max_drawdown, cagr
from quant22.strategies.trend import TrendParams, trend_positions, simple_tsmom_positions, default_grid
from quant22.validation.sharpe_tests import deflated_sharpe_ratio, probabilistic_sharpe_ratio, min_track_record_length
from quant22.validation.cscv import pbo_cscv
from quant22.validation.placebo import bootstrap_sharpe_pvalue
from quant22.validation.walkforward import walk_forward
from quant22.risk.vol_target import risk_parity_weights


def grid_scan(close: pd.Series, bars_per_year: float, costs: CostModel, label: str) -> tuple[pd.DataFrame, dict]:
    grid = default_grid(bars_per_year)
    rets, rows = [], []
    for g in grid:
        tgt = simple_tsmom_positions(close, g["lookback"], 0.30, g["vol_lookback"], bars_per_year)
        res = run_backtest(close, tgt, costs, bars_per_year, with_trades=False)
        rets.append(res.returns.values)
        rows.append({**g, "sharpe": res.summary.sharpe, "cagr": res.summary.cagr, "mdd": res.summary.max_drawdown,
                     "turnover_py": res.summary.turnover_per_year, "cost_drag_py": res.costs.sum() / res.summary.years})
    tab = pd.DataFrame(rows)
    R = np.column_stack(rets)
    n = R.shape[0]
    per_period = tab["sharpe"].values / np.sqrt(bars_per_year)
    best = int(np.argmax(per_period))
    sk, ku = skew_kurt(R[:, best])
    dsr, bench = deflated_sharpe_ratio(per_period[best], n, per_period, sk, ku)
    psr = probabilistic_sharpe_ratio(per_period[best], 0.0, n, sk, ku)
    pbo = pbo_cscv(R, n_blocks=10)
    share_pos = float((tab["sharpe"] > 0).mean())
    print(f"[{label}] {len(grid)} variants: {share_pos:.0%} positive Sharpe; median {tab['sharpe'].median():.2f}; "
          f"best {tab['sharpe'].max():.2f} (lb={grid[best]['lookback']}) | PSR(best) {psr:.3f} | DSR(best) {dsr:.3f} "
          f"(benchmark SR_ann {bench*np.sqrt(bars_per_year):.2f}) | PBO {pbo.pbo:.2f} | OOS-degradation slope {pbo.degradation_slope:.2f}")
    return tab, {"share_positive": share_pos, "median_sharpe": float(tab["sharpe"].median()), "best_sharpe": float(tab["sharpe"].max()),
                 "best_params": grid[best], "psr_best": psr, "dsr_best": dsr, "dsr_benchmark_ann": bench * np.sqrt(bars_per_year),
                 **pbo.to_dict()}


def main() -> None:
    banner("05 — Trend following: BTC daily grid, deflated Sharpe, PBO")
    d = btc_daily_ohlc("2014-01-01")
    close = d["close"]
    taker = CostModel()
    grid_tab, grid_info = grid_scan(close, DAILY, taker, "BTC daily TSMOM grid")

    banner("05 — Pre-specified composite trend on BTC (net of taker costs)")
    params = TrendParams()
    tgt = trend_positions(close, params, DAILY)
    res = run_backtest(close, tgt, taker, DAILY, high=d["high"], low=d["low"])
    bh = close.pct_change().fillna(0.0)
    s = res.summary
    print(f"Full sample {close.index[0].date()}→{close.index[-1].date()}: Sharpe {s.sharpe:.2f}, CAGR {s.cagr:.1%}, vol {s.ann_vol:.1%}, "
          f"MDD {s.max_drawdown:.1%}, Calmar {s.calmar:.2f}, avg |lev| {s.avg_gross_leverage:.2f}, turnover {s.turnover_per_year:.0f}x/yr, "
          f"cost drag {res.costs.sum()/s.years:.2%}/yr")
    print(f"Buy&hold BTC: Sharpe {sharpe_ratio(bh, DAILY):.2f}, CAGR {cagr(bh, DAILY):.1%}, MDD {max_drawdown(bh):.1%}")
    ts = res.trade_summary
    total_trade_pnl = sum(t.pnl for t in res.trades)
    print(f"Trades: {ts.n_trades}, win rate {ts.win_rate:.1%}, payoff {ts.payoff_ratio:.2f}, PF {ts.profit_factor:.2f}, "
          f"avg hold {ts.avg_bars:.0f}d, sum of trade pnl {total_trade_pnl:+.2f} → without top-3 trades {ts.pnl_without_top3:+.2f} "
          f"(top-3 share {(total_trade_pnl - ts.pnl_without_top3) / total_trade_pnl:.0%})")
    worst = res.returns.idxmin()
    print(f"Worst day: {worst.date()} {res.returns.min():.1%} (position {res.positions[worst]:+.2f}, BTC {bh[worst]:.1%}); "
          f"max |position| {res.positions.abs().max():.2f}")
    p_boot, _, _ = bootstrap_sharpe_pvalue(res.returns, n_boot=2000, mean_block=20)
    sk, ku = skew_kurt(res.returns)
    psr = probabilistic_sharpe_ratio(s.sharpe / np.sqrt(DAILY), 0.0, len(res.returns), sk, ku)
    mintrl = min_track_record_length(s.sharpe / np.sqrt(DAILY), 0.0, sk, ku) / DAILY
    print(f"Bootstrap p(mean<=0) = {p_boot:.4f} | PSR = {psr:.3f} | MinTRL = {mintrl:.1f} years")
    per = period_table(res.returns, DAILY, label="trend", benchmark=bh)
    print(md_table(per))

    costs_tab = []
    for name, cm in [("zero", CostModel.zero()), ("maker 1.5bp+0.5", CostModel.maker()), ("taker 4.5bp+3", taker),
                     ("2x taker", CostModel(fee_bps=9, slippage_bps=6))]:
        r = run_backtest(close, tgt, cm, DAILY, with_trades=False)
        costs_tab.append({"costs": name, "sharpe": r.summary.sharpe, "cagr": r.summary.cagr, "mdd": r.summary.max_drawdown})
    costs_tab = pd.DataFrame(costs_tab)
    print(md_table(costs_tab))

    lo_params = TrendParams(long_only=True)
    lo = run_backtest(close, trend_positions(close, lo_params, DAILY), taker, DAILY, with_trades=False)
    print(f"Long-only variant: Sharpe {lo.summary.sharpe:.2f}, CAGR {lo.summary.cagr:.1%}, MDD {lo.summary.max_drawdown:.1%}")

    banner("05 — Walk-forward over the single-lookback grid (2y train / 6m test, re-selected each step)")
    wf = walk_forward(close, lambda px, g: simple_tsmom_positions(px, g["lookback"], 0.30, g["vol_lookback"], DAILY),
                      default_grid(DAILY), train_bars=730, test_bars=182, bars_per_year=DAILY, costs=taker, warmup_bars=260)
    wf_s = sharpe_ratio(wf.oos_returns, DAILY)
    print(f"Walk-forward OOS: Sharpe {wf_s:.2f}, CAGR {cagr(wf.oos_returns, DAILY):.1%}, MDD {max_drawdown(wf.oos_returns):.1%} "
          f"over {len(wf.segments)} segments; median IS→OOS Sharpe {np.median([x.is_sharpe for x in wf.segments]):.2f}→"
          f"{np.median([x.oos_sharpe for x in wf.segments]):.2f}; worst OOS day {wf.oos_returns.min():.1%} on {wf.oos_returns.idxmin().date()}")
    # the same window for the fixed ensemble, for a like-for-like comparison
    ens_same = res.returns.reindex(wf.oos_returns.index)
    print(f"Fixed ensemble over the same OOS window: Sharpe {sharpe_ratio(ens_same, DAILY):.2f}, MDD {max_drawdown(ens_same):.1%}")

    banner("05 — Multi-asset composite trend, equal risk (Coin Metrics daily, to 2026-05)")
    closes = load_daily_closes(["btc", "eth", "bnb", "xrp", "ada", "doge", "ltc", "link", "dot"], min_price=0.0001).loc["2017-01-01":]
    strat_rets = {}
    for a in closes:
        px = closes[a].dropna()
        if len(px) < 400:
            continue
        r = run_backtest(px, trend_positions(px, params, DAILY), taker, DAILY, with_trades=False)
        strat_rets[a] = r.returns
    SR = pd.DataFrame(strat_rets)
    w = risk_parity_weights(SR, 60, DAILY).shift(1)
    port = (SR * w).sum(axis=1, min_count=1).dropna()
    port = port.loc["2018-01-01":]
    btc_bh = closes["btc"].pct_change().reindex(port.index).fillna(0)
    print(f"Portfolio {port.index[0].date()}→{port.index[-1].date()}: Sharpe {sharpe_ratio(port, DAILY):.2f}, CAGR {cagr(port, DAILY):.1%}, "
          f"MDD {max_drawdown(port):.1%} | BTC B&H same window: Sharpe {sharpe_ratio(btc_bh, DAILY):.2f}, MDD {max_drawdown(btc_bh):.1%} "
          f"| corr(port, BTC) {np.corrcoef(port, btc_bh)[0,1]:.2f}")
    per_asset = pd.DataFrame({a: {"sharpe": sharpe_ratio(r, DAILY), "mdd": max_drawdown(r), "years": len(r) / DAILY} for a, r in strat_rets.items()}).T
    print(md_table(per_asset.reset_index().rename(columns={"index": "asset"})))
    port_per = period_table(port, DAILY, bounds=["2018-01-01", "2020-01-01", "2022-01-01", "2024-01-01", "2027-01-01"], label="port", benchmark=btc_bh)
    print(md_table(port_per))

    banner("05 — Hourly TSMOM (does speed survive costs?)")
    h = load_hourly_btc("2017-01-01")["close"]
    h_tab, h_info = grid_scan(h, HOURLY, taker, "BTC hourly TSMOM grid (taker)")
    h_tab_m, h_info_m = grid_scan(h, HOURLY, CostModel.maker(), "BTC hourly TSMOM grid (maker)")

    md = ("# Trend following\n\n## BTC daily single-lookback grid (taker costs)\n\n" + md_table(grid_tab) +
          f"\n\nPSR(best)={grid_info['psr_best']:.3f}, DSR(best)={grid_info['dsr_best']:.3f}, PBO={grid_info['pbo']:.2f}\n\n"
          f"## Composite (pre-specified) on BTC\n\n{md_table(pd.DataFrame([res.to_dict()]).T.reset_index().rename(columns={'index':'metric',0:'value'}))}\n\n"
          f"Bootstrap p={p_boot:.4f}, PSR={psr:.3f}, MinTRL={mintrl:.1f}y\n\n### Sub-periods\n\n{md_table(per)}\n\n### Cost sensitivity\n\n{md_table(costs_tab)}\n\n"
          f"### Walk-forward OOS\n\n{md_table(wf.table())}\n\nOOS Sharpe {wf_s:.2f}\n\n## Multi-asset\n\n{md_table(per_asset.reset_index())}\n\n{md_table(port_per)}\n\n"
          f"## Hourly grid (taker)\n\n{md_table(h_tab)}\n\n## Hourly grid (maker)\n\n{md_table(h_tab_m)}\n")
    save_results("05_trend_following", {
        "grid": grid_tab.to_dict(orient="records"), "grid_info": grid_info, "composite_btc": res.to_dict(),
        "composite_bootstrap_p": p_boot, "composite_psr": psr, "composite_mintrl_years": mintrl,
        "composite_periods": per.to_dict(orient="records"), "cost_sensitivity": costs_tab.to_dict(orient="records"),
        "long_only": lo.summary.to_dict(), "walk_forward_oos_sharpe": wf_s, "walk_forward_segments": wf.table().to_dict(orient="records"),
        "portfolio": {"sharpe": sharpe_ratio(port, DAILY), "cagr": cagr(port, DAILY), "mdd": max_drawdown(port),
                      "start": str(port.index[0].date()), "end": str(port.index[-1].date()),
                      "corr_btc": float(np.corrcoef(port, btc_bh)[0, 1])},
        "per_asset": per_asset.to_dict(orient="index"), "portfolio_periods": port_per.to_dict(orient="records"),
        "hourly_grid_taker": h_info, "hourly_grid_maker": h_info_m, "params": params.to_dict(),
    }, md)
    # persist the daily return streams for the ensemble script
    pd.DataFrame({"trend_btc": res.returns, "bh_btc": bh}).to_parquet("results/_trend_btc_returns.parquet")
    port.rename("trend_portfolio").to_frame().to_parquet("results/_trend_portfolio_returns.parquet")


if __name__ == "__main__":
    main()
