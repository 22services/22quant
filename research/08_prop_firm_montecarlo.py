"""Prop-firm evaluation as a bet: expected value given an honest edge.

Apex 100K (EOD trailing, 2026 rules): $6,000 target, $3,000 trailing drawdown
(freezes at +$100), $1,500 daily loss limit, consistency 50%, $399 list fee
(or ~$60 with a promo), $149 activation, payout ladder capped at $18k. We vary the trader's true annual Sharpe and daily P&L
volatility, and compute P(pass), P(blow-up), expected days and the EV of
one evaluation net of the fee.
"""
from __future__ import annotations

import pandas as pd

from _common import save_results, md_table, banner
from quant22.risk.prop_firm import PropRules, TraderModel, simulate_evaluation


def main() -> None:
    banner("08 — Apex 100K EOD evaluation Monte Carlo")
    rows = []
    for fee in (399.0, 60.0):          # list price vs typical 85%-off promo
        rules = PropRules(challenge_fee=fee)
        for sr in (-1.0, -0.5, 0.0, 0.5, 1.0, 1.5, 2.0, 3.0):
            for dv in (250.0, 500.0, 1000.0, 1500.0):
                res = simulate_evaluation(rules, TraderModel(sharpe_ann=sr, daily_vol_usd=dv), n_sims=20000)
                rows.append({"fee": fee, "sharpe_ann": sr, "daily_vol_usd": dv, "p_pass": res.p_pass, "p_blow_eval": res.p_fail,
                             "days_to_pass": res.mean_days_to_pass, "p_any_payout_if_funded": res.p_at_least_one_payout,
                             "mean_payouts_if_funded": res.mean_total_payouts, "p_blow_funded_1y": res.p_blow_funded_within_year,
                             "ev_usd": res.ev_per_challenge_usd})
    tab = pd.DataFrame(rows)
    print(md_table(tab[tab.fee == 399.0]))
    print(md_table(tab[(tab.fee == 60.0) & (tab.daily_vol_usd == 1000.0)]))
    intraday = PropRules(eod_trailing=False)
    r2 = simulate_evaluation(intraday, TraderModel(sharpe_ann=1.0, daily_vol_usd=1000.0))
    r1 = simulate_evaluation(PropRules(), TraderModel(sharpe_ann=1.0, daily_vol_usd=1000.0))
    print(f"Sharpe 1.0, $1k/day vol: EOD trailing P(pass) {r1.p_pass:.2f}, EV ${r1.ev_per_challenge_usd:,.0f} vs intraday trailing "
          f"P(pass) {r2.p_pass:.2f}, EV ${r2.ev_per_challenge_usd:,.0f}")
    # own capital comparison: same trader, same $ risk, no rules — expected 1y P&L = Sharpe × vol × sqrt(252)... minus nothing
    for sr in (1.0, 2.0):
        own = sr * 1000.0 * (252 ** 0.5)
        print(f"Same trader (Sharpe {sr}, $1k/day vol) on own capital: expected 1y P&L ≈ ${own:,.0f} with no evaluation risk "
              f"(needs the margin to run $1k/day vol: ~$10-20k on MNQ)")
    save_results("08_prop_firm_montecarlo", {"table": rows, "eod_vs_intraday": {"eod": r1.to_dict(), "intraday": r2.to_dict()}},
                 "# Apex 100K evaluation Monte Carlo\n\n" + md_table(tab) + "\n")


if __name__ == "__main__":
    main()
