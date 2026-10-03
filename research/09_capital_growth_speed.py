"""How fast can money be made? The physics of compounding under uncertainty.

Given a strategy's true Sharpe ratio, the maximum long-run growth rate is
g* = SR²/2 per year (full Kelly), with a 50% chance of halving your capital
along the way. Betting more than Kelly makes you slower *and* likelier to
go broke. We tabulate time-to-double and drawdown odds for the Sharpe levels
that actually exist, then Monte-Carlo a fat-tailed year at several leverages.
"""
from __future__ import annotations

import json
from pathlib import Path

import numpy as np
import pandas as pd

from _common import save_results, md_table, banner
from quant22.risk.kelly import (kelly_from_sharpe, optimal_growth_ann, time_to_multiply, prob_drawdown_to, growth_rate)


def main() -> None:
    banner("09 — Growth speed limits set by the Sharpe ratio")
    vol = 0.30  # strategy vol at 1x (our trend strategy targets ~30%)
    rows = []
    for sr in (0.5, 0.75, 1.0, 1.5, 2.0, 3.0):
        f = kelly_from_sharpe(sr, vol)
        for k in (1.0, 0.5, 0.25):
            rows.append({"sharpe": sr, "kelly_fraction": k, "leverage_on_30%vol_strategy": f * k,
                         "growth_per_year": (k - 0.5 * k**2) * sr**2, "years_to_double": time_to_multiply(2, sr, k),
                         "years_to_10x": time_to_multiply(10, sr, k), "p_halve_capital": prob_drawdown_to(0.5, k),
                         "p_lose_80pct": prob_drawdown_to(0.2, k)})
    tab = pd.DataFrame(rows)
    print(md_table(tab))

    banner("09 — One fat-tailed year at various leverages (Student-t, df=3.5), strategy Sharpe from results if available")
    sr = 1.0
    p = Path("results/05_trend_following.json")
    if p.exists():
        sr = float(json.loads(p.read_text())["composite_btc"]["sharpe"])
        print(f"Using measured composite trend Sharpe {sr:.2f} (full-sample, net of taker costs)")
    rng = np.random.default_rng(0)
    df = 3.5
    n_days, n_sims = 365, 20000
    daily_mu = sr * vol / 365.0                  # annual excess return = SR × vol
    daily_sd = vol / np.sqrt(365.0)
    t = rng.standard_t(df, size=(n_sims, n_days)) * np.sqrt((df - 2) / df)
    base = daily_mu + daily_sd * t
    mc = []
    for lev in (0.5, 1.0, 2.0, 3.0, 5.0, 10.0):
        r = np.clip(lev * base, -0.99, None)           # cannot lose more than everything per day
        eq = np.cumprod(1 + r, axis=1)
        peak = np.maximum.accumulate(eq, axis=1)
        dd = (eq / peak - 1).min(axis=1)
        mc.append({"leverage": lev, "median_final": float(np.median(eq[:, -1])), "mean_final": float(eq[:, -1].mean()),
                   "p_double": float((eq[:, -1] >= 2).mean()), "p_lose_half": float((eq.min(axis=1) <= 0.5).mean()),
                   "p_ruin_90pct": float((eq.min(axis=1) <= 0.1).mean()), "median_max_dd": float(np.median(dd)),
                   "geometric_growth": float(np.median(np.log(eq[:, -1])))})
    mc = pd.DataFrame(mc)
    print(md_table(mc))
    capital = pd.DataFrame([{"start_capital": c, "half_kelly_1y_median": c * np.exp(0.375 * sr**2),
                             "half_kelly_3y_median": c * np.exp(3 * 0.375 * sr**2)} for c in (1_000, 5_000, 10_000, 50_000)])
    print(md_table(capital))
    save_results("09_capital_growth_speed", {"kelly_table": rows, "monte_carlo": mc.to_dict(orient="records"), "sharpe_used": sr,
                                             "capital_paths": capital.to_dict(orient="records")},
                 "# Growth speed\n\n" + md_table(tab) + "\n\n## Fat-tailed year by leverage\n\n" + md_table(mc) + "\n\n" + md_table(capital) + "\n")


if __name__ == "__main__":
    main()
