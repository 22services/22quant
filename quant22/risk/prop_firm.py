"""Monte Carlo of a funded-trader ("prop firm") evaluation **and** the funded stage.

Two stages, because evaluation profits are never paid out:

1. Evaluation: reach ``profit_target`` before the trailing drawdown is hit,
   within ``max_days``, with ``min_days`` traded.
2. Funded (PA) account: the trailing threshold keeps following the high-water
   mark until it freezes at ``account_size + drawdown_freezes_at``; payouts need
   ≥ ``payout_min_qualifying_days`` days with ≥ ``qualifying_day_profit`` since
   the last payout, a balance above ``payout_min_balance``, the consistency rule,
   and are capped by a ladder of at most ``len(payout_caps)`` payouts.

Each trading day is simulated as an **intraday path** (``steps_per_day`` steps),
so that the daily loss limit and the liquidation threshold trigger *when they
are touched*, not at the close. This matters: flooring a day's close-to-close
P&L at the DLL (the naive shortcut) silently adds positive drift, because it
removes losses without removing the recoveries that a real stop would have cut.
With a real stop, optional stopping guarantees a zero-edge trader stays zero-edge.

Daily P&L is a scale mixture of normals (Gamma-distributed daily variance,
``vol_of_vol_shape`` ≈ 2 gives Student-t(4)-like tails) with the drift implied
by ``sharpe_ann``. A trader with no edge has sharpe_ann = 0; the 97%-lose
literature implies typical retail realised Sharpe is *negative* after costs.

Rule values default to Apex 100K EOD "4.0" (2026) as summarised by third-party
reviews of the Apex help centre: $6k target, $3k trailing drawdown, $1.5k DLL,
$399 list evaluation fee, $149 PA activation, payout ladder 2.0/2.5/2.5/3.0/
4.0/4.0k. Re-check every number against the firm before relying on it.
"""
from __future__ import annotations

from dataclasses import dataclass, asdict, field

import numpy as np


@dataclass
class PropRules:
    account_size: float = 100_000.0
    profit_target: float = 6_000.0
    trailing_drawdown: float = 3_000.0
    eod_trailing: bool = True             # True: threshold updated from the day's *closing* balance; False: from intraday peaks
    drawdown_freezes_at: float = 100.0    # threshold stops trailing at account_size + this
    daily_loss_limit: float | None = 1_500.0
    min_days: int = 7
    max_days: int = 60
    challenge_fee: float = 399.0          # list price 2026 ($399-599); 80-90% discount codes are common
    pa_activation_fee: float = 149.0
    consistency_max_day_share: float | None = 0.5
    # funded stage
    pa_max_days: int = 250
    payout_min_balance: float = 103_600.0
    safety_net: float = 103_100.0
    payout_min: float = 500.0
    payout_min_qualifying_days: int = 5
    qualifying_day_profit: float = 300.0
    payout_caps: tuple = (2_000.0, 2_500.0, 2_500.0, 3_000.0, 4_000.0, 4_000.0)   # Apex 100K EOD ladder (2026, $18k max)
    payout_split: float = 1.0


@dataclass
class TraderModel:
    sharpe_ann: float = 1.0
    daily_vol_usd: float = 800.0
    vol_of_vol_shape: float = 2.0      # Gamma shape of daily variance (lower = fatter tails)
    trading_days_per_year: int = 252
    steps_per_day: int = 16


@dataclass
class PropResult:
    p_pass: float
    p_fail: float
    p_timeout: float
    mean_days_to_pass: float
    p_consistency_block_at_pass: float
    p_at_least_one_payout: float          # conditional on being funded
    mean_total_payouts: float             # conditional on being funded
    mean_n_payouts: float
    p_blow_funded_within_year: float
    mean_funded_lifetime_days: float
    ev_per_challenge_usd: float
    ev_detail: dict = field(default_factory=dict)

    def to_dict(self) -> dict:
        return asdict(self)


def _day_paths(trader: TraderModel, n: int, rng: np.random.Generator) -> np.ndarray:
    """Cumulative intraday P&L paths, shape (n, steps)."""
    k = trader.steps_per_day
    mu_d = trader.sharpe_ann * trader.daily_vol_usd / np.sqrt(trader.trading_days_per_year)
    g = rng.gamma(trader.vol_of_vol_shape, 1.0 / trader.vol_of_vol_shape, size=(n, 1))   # mean-1 daily variance multiplier
    steps = mu_d / k + trader.daily_vol_usd * np.sqrt(g / k) * rng.standard_normal((n, k))
    return np.cumsum(steps, axis=1)


def _trade_day(bal0: np.ndarray, thr0: np.ndarray, alive: np.ndarray, rules: PropRules, trader: TraderModel,
               rng: np.random.Generator) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Simulate one day for every account. Returns (day_pnl, breached, threshold_after_day)."""
    n = bal0.size
    path = _day_paths(trader, n, rng)
    k = path.shape[1]
    frozen = rules.account_size + rules.drawdown_freezes_at
    # daily loss limit: flatten at the first touch (keep the overshoot of the discrete step — gap realism)
    stop_hit = np.zeros(n, dtype=bool)
    if rules.daily_loss_limit:
        hit = path <= -rules.daily_loss_limit
        stop_hit = hit.any(axis=1)
        first = np.where(stop_hit, hit.argmax(axis=1), k - 1)
        mask = np.arange(k)[None, :] > first[:, None]
        path = np.where(mask, path[np.arange(n), first][:, None], path)
    equity = bal0[:, None] + path
    if rules.eod_trailing:
        thr_path = np.broadcast_to(thr0[:, None], equity.shape)
    else:
        runmax = np.maximum.accumulate(np.maximum(equity, bal0[:, None]), axis=1)
        thr_path = np.minimum(np.maximum(thr0[:, None], runmax - rules.trailing_drawdown), frozen)
    breached = alive & (equity <= thr_path).any(axis=1)
    pnl = np.where(alive, path[:, -1], 0.0)
    close = bal0 + pnl
    if rules.eod_trailing:
        thr = np.minimum(np.maximum(thr0, close - rules.trailing_drawdown), frozen)
    else:
        thr = np.minimum(np.maximum(thr_path[:, -1], close - rules.trailing_drawdown), frozen)
    thr = np.where(thr0 >= frozen, thr0, thr)
    return pnl, breached, thr


def simulate_evaluation_stage(rules: PropRules, trader: TraderModel, n_sims: int, rng: np.random.Generator) -> dict:
    bal = np.full(n_sims, rules.account_size)
    thr = np.full(n_sims, rules.account_size - rules.trailing_drawdown)
    alive = np.ones(n_sims, dtype=bool)
    passed = np.zeros(n_sims, dtype=bool)
    failed = np.zeros(n_sims, dtype=bool)
    first_pass = np.full(n_sims, rules.max_days + 1)
    max_day = np.zeros(n_sims)
    cons_block = np.zeros(n_sims, dtype=bool)
    for d in range(rules.max_days):
        active = alive & ~passed
        pnl, breached, thr_new = _trade_day(bal, thr, active, rules, trader, rng)
        bal = bal + pnl
        thr = np.where(active, thr_new, thr)
        newly_failed = active & breached
        failed |= newly_failed
        alive &= ~newly_failed
        max_day = np.where(active, np.maximum(max_day, pnl), max_day)
        reach = active & ~newly_failed & (bal - rules.account_size >= rules.profit_target) & (d + 1 >= rules.min_days)
        if rules.consistency_max_day_share:
            cons_block |= reach & (max_day / np.maximum(bal - rules.account_size, 1e-9) > rules.consistency_max_day_share)
        first_pass = np.where(reach, d, first_pass)
        passed |= reach
    return {"passed": passed, "failed": failed, "timeout": ~passed & ~failed, "first_pass": first_pass, "cons_block": cons_block}


def simulate_funded_stage(rules: PropRules, trader: TraderModel, n_sims: int, rng: np.random.Generator) -> dict:
    bal = np.full(n_sims, rules.account_size)
    thr = np.full(n_sims, rules.account_size - rules.trailing_drawdown)
    alive = np.ones(n_sims, dtype=bool)
    payouts = np.zeros(n_sims)
    n_pay = np.zeros(n_sims, dtype=int)
    profit_since = np.zeros(n_sims)
    max_day_since = np.zeros(n_sims)
    qual = np.zeros(n_sims, dtype=int)
    lifetime = np.zeros(n_sims, dtype=int)
    caps = np.array(rules.payout_caps)
    for _ in range(rules.pa_max_days):
        pnl, breached, thr_new = _trade_day(bal, thr, alive, rules, trader, rng)
        bal = bal + pnl
        thr = np.where(alive, thr_new, thr)
        alive &= ~breached
        lifetime += alive
        profit_since += pnl
        max_day_since = np.maximum(max_day_since, pnl)
        qual += (alive & (pnl >= rules.qualifying_day_profit)).astype(int)
        idx = np.minimum(n_pay, len(caps) - 1)
        amount = np.minimum(caps[idx], bal - rules.safety_net) * rules.payout_split
        ok_cons = np.ones(n_sims, dtype=bool) if not rules.consistency_max_day_share else \
            (max_day_since / np.maximum(profit_since, 1e-9) < rules.consistency_max_day_share)
        can = alive & (n_pay < len(caps)) & (qual >= rules.payout_min_qualifying_days) & (bal >= rules.payout_min_balance) \
            & (profit_since > 0) & ok_cons & (amount >= rules.payout_min)
        payouts += np.where(can, amount, 0.0)
        bal -= np.where(can, amount / rules.payout_split, 0.0)
        n_pay += can.astype(int)
        profit_since = np.where(can, 0.0, profit_since)
        max_day_since = np.where(can, 0.0, max_day_since)
        qual = np.where(can, 0, qual)
    return {"payouts": payouts, "n_pay": n_pay, "alive": alive, "lifetime": lifetime}


def simulate_evaluation(rules: PropRules, trader: TraderModel, n_sims: int = 20000, seed: int = 0) -> PropResult:
    rng = np.random.default_rng(seed)
    ev_ = simulate_evaluation_stage(rules, trader, n_sims, rng)
    fu = simulate_funded_stage(rules, trader, n_sims, rng)
    p_pass = float(ev_["passed"].mean())
    mean_pay = float(fu["payouts"].mean())
    ev = -rules.challenge_fee + p_pass * (mean_pay - rules.pa_activation_fee)
    return PropResult(
        p_pass=p_pass, p_fail=float(ev_["failed"].mean()), p_timeout=float(ev_["timeout"].mean()),
        mean_days_to_pass=float(ev_["first_pass"][ev_["passed"]].mean() + 1) if ev_["passed"].any() else np.nan,
        p_consistency_block_at_pass=float(ev_["cons_block"][ev_["passed"]].mean()) if ev_["passed"].any() else 0.0,
        p_at_least_one_payout=float((fu["n_pay"] > 0).mean()), mean_total_payouts=mean_pay,
        mean_n_payouts=float(fu["n_pay"].mean()), p_blow_funded_within_year=float((~fu["alive"]).mean()),
        mean_funded_lifetime_days=float(fu["lifetime"].mean()), ev_per_challenge_usd=float(ev),
        ev_detail={"challenge_fee": rules.challenge_fee, "pa_activation_fee": rules.pa_activation_fee,
                   "median_total_payouts_if_funded": float(np.median(fu["payouts"])),
                   "p_paid_per_challenge": float(p_pass * (fu["n_pay"] > 0).mean())},
    )
