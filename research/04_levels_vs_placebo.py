"""Myth test #4 — price levels vs placebo levels of equal density.

Round numbers (a documented order-clustering effect: Osler 2003), Fibonacci
retracements, and the PO3 / Goldbach dealing-range levels from the user's own
framework, all measured the same way on hourly BTC 2017→2026:

  touch  = close comes within `tol` of a level after being further away
  bounce = k bars later price is back on the side it came from
  rejection = return away from the level over k bars (positive = barrier)

Each is compared with the *same* measurement on (a) the grid shifted by a
random offset and (b) random level sets of identical count. If the named
levels do not beat their placebos, the lines are decoration.
"""
from __future__ import annotations

import numpy as np
import pandas as pd

from _common import load_hourly_btc, save_results, md_table, banner, verdict
from quant22.features.levels import (grid_touch_reaction, round_number_step, goldbach_levels_reaction, GOLDBACH_PCTS,
                                     fib_retracement_reaction, FIB_RATIOS)

N_PLACEBO = 200


def main() -> None:
    banner("04 — Round numbers, Fibonacci and PO3/Goldbach levels vs placebo")
    h = load_hourly_btc("2017-01-01")
    close, high, low = h["close"].values, h["high"].values, h["low"].values
    rng = np.random.default_rng(0)
    rows = []

    # ---- round numbers: fixed grids
    for step in [1000.0, 5000.0, 10000.0]:
        for k in [6, 24]:
            tol = 0.001
            real = grid_touch_reaction(close, round_number_step(step), 0.0, tol, k)
            plac = [grid_touch_reaction(close, round_number_step(step), rng.uniform(0.05, 0.95), tol, k) for _ in range(N_PLACEBO)]
            pb = np.array([p.bounce_rate for p in plac]); pr = np.array([p.mean_rejection for p in plac])
            lab, p = verdict(real.bounce_rate, pb)
            rows.append({"family": f"round ${step:,.0f}", "k_bars": k, "n_touches": real.n_touches, "bounce_rate": real.bounce_rate,
                         "placebo_bounce": np.nanmean(pb), "placebo_bounce_hi95": np.nanpercentile(pb, 97.5),
                         "mean_rejection_bp": real.mean_rejection * 1e4, "placebo_rejection_bp": np.nanmean(pr) * 1e4,
                         "t_stat": real.t_stat, "p_vs_placebo": p, "verdict": lab})
            print(f"round {step:>7,.0f} k={k:>2}: bounce {real.bounce_rate:.3f} (n={real.n_touches}) vs placebo {np.nanmean(pb):.3f} "
                  f"[hi95 {np.nanpercentile(pb, 97.5):.3f}] | rejection {real.mean_rejection*1e4:+.1f}bp vs {np.nanmean(pr)*1e4:+.1f}bp → {lab}")

    # ---- PO3 / Goldbach internal levels
    for po3 in [729, 2187, 6561]:
        for k in [6, 24]:
            tol = 0.0007
            real = goldbach_levels_reaction(close, po3, GOLDBACH_PCTS, tol, k)
            plac = []
            for _ in range(N_PLACEBO):
                pcts = np.sort(np.r_[0.0, 1.0, rng.uniform(0.01, 0.99, size=len(GOLDBACH_PCTS) - 2)])
                plac.append(goldbach_levels_reaction(close, po3, pcts, tol, k))
            uni = goldbach_levels_reaction(close, po3, np.linspace(0, 1, len(GOLDBACH_PCTS)), tol, k)
            pb = np.array([p.bounce_rate for p in plac]); pr = np.array([p.mean_rejection for p in plac])
            lab, p = verdict(real.bounce_rate, pb)
            rows.append({"family": f"Goldbach PO3={po3}", "k_bars": k, "n_touches": real.n_touches, "bounce_rate": real.bounce_rate,
                         "placebo_bounce": np.nanmean(pb), "placebo_bounce_hi95": np.nanpercentile(pb, 97.5),
                         "mean_rejection_bp": real.mean_rejection * 1e4, "placebo_rejection_bp": np.nanmean(pr) * 1e4,
                         "t_stat": real.t_stat, "p_vs_placebo": p, "verdict": lab, "uniform_grid_bounce": uni.bounce_rate})
            print(f"PO3 {po3:>5} k={k:>2}: bounce {real.bounce_rate:.3f} (n={real.n_touches}) vs random-% placebo {np.nanmean(pb):.3f} "
                  f"[hi95 {np.nanpercentile(pb, 97.5):.3f}], uniform grid {uni.bounce_rate:.3f} → {lab}")

    # ---- Fibonacci retracements
    for sw in [120, 240, 720]:
        for k in [12, 48]:
            tol = 0.002
            real = fib_retracement_reaction(close, high, low, sw, FIB_RATIOS, tol, k)
            plac = [fib_retracement_reaction(close, high, low, sw, np.sort(rng.uniform(0.15, 0.85, size=len(FIB_RATIOS))), tol, k)
                    for _ in range(N_PLACEBO // 2)]
            pb = np.array([p.bounce_rate for p in plac]); pr = np.array([p.mean_rejection for p in plac])
            lab, p = verdict(real.bounce_rate, pb)
            rows.append({"family": f"Fibonacci swing={sw}h", "k_bars": k, "n_touches": real.n_touches, "bounce_rate": real.bounce_rate,
                         "placebo_bounce": np.nanmean(pb), "placebo_bounce_hi95": np.nanpercentile(pb, 97.5),
                         "mean_rejection_bp": real.mean_rejection * 1e4, "placebo_rejection_bp": np.nanmean(pr) * 1e4,
                         "t_stat": real.t_stat, "p_vs_placebo": p, "verdict": lab})
            print(f"fib swing={sw:>3}h k={k:>2}: bounce {real.bounce_rate:.3f} (n={real.n_touches}) vs random-ratio placebo {np.nanmean(pb):.3f} "
                  f"[hi95 {np.nanpercentile(pb, 97.5):.3f}] → {lab}")

    table = pd.DataFrame(rows)
    n = len(table)
    table["p_bonferroni"] = (table["p_vs_placebo"] * n).clip(upper=1.0)
    table["survives_multiple_testing"] = table["p_bonferroni"] < 0.05
    print(f"\n{n} level hypotheses tested; {int((table['p_vs_placebo'] < 0.05).sum())} nominally p<0.05 "
          f"(expected by chance: {0.05 * n:.1f}); {int(table['survives_multiple_testing'].sum())} survive Bonferroni")
    rows = table.to_dict(orient="records")
    save_results("04_levels_vs_placebo", {"table": rows}, "# Levels vs placebo (hourly BTC 2017-2026)\n\n" + md_table(table) + "\n")


if __name__ == "__main__":
    main()
