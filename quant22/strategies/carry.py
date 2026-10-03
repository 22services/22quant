"""Perpetual-swap funding carry.

Mechanism: perpetual swaps anchor to spot via a funding payment between longs
and shorts every 8h (Binance, Bybit) or 1h (Hyperliquid). When retail
leverage is crowded long, funding is positive and the short perp earns it.
Long spot + short perp is delta-neutral: the return is the funding stream
minus fees, basis drift and the opportunity cost of cash (the spot leg is
fully funded).

Two uses here:
1. :func:`carry_backtest` – the pure delta-neutral trade with realistic costs.
2. :func:`funding_overlay` – a *filter* on a directional strategy: do not pay
   extreme funding to be long a perp; when funding is extreme the carry is
   large enough to be a cost worth modelling explicitly (the engine charges it).
"""
from __future__ import annotations

from dataclasses import dataclass

import numpy as np
import pandas as pd

from ..backtest.metrics import summarize, Summary


@dataclass
class CarryParams:
    entry_bps: float = 1.0        # enter when the (smoothed) 8h funding exceeds this
    exit_bps: float = 0.3         # exit when it falls below this
    smooth_periods: int = 3       # EWM span on funding (periods) to avoid whipsaw
    fee_bps_per_leg_side: float = 4.5     # taker fee+slippage per leg per side (two legs!)
    cash_rate_ann: float = 0.04   # opportunity cost of the capital locked in the spot leg
    periods_per_year: int = 3 * 365


@dataclass
class CarryResult:
    returns: pd.Series          # net return per funding period on the notional
    positions: pd.Series        # 1 when in the trade
    summary: Summary
    gross_funding_ann: float
    n_trades: int
    time_in_market: float


def carry_positions(funding: pd.Series, p: CarryParams) -> pd.Series:
    f_bps = funding.ewm(span=p.smooth_periods, min_periods=1).mean() * 1e4
    pos = np.zeros(len(f_bps))
    state = 0
    vals = f_bps.values
    for i in range(len(vals)):
        if state == 0 and vals[i] > p.entry_bps:
            state = 1
        elif state == 1 and vals[i] < p.exit_bps:
            state = 0
        pos[i] = state
    return pd.Series(pos, index=funding.index)


def carry_backtest(funding: pd.Series, p: CarryParams, basis_drift: pd.Series | None = None) -> CarryResult:
    pos = carry_positions(funding, p)
    held = pos.shift(1).fillna(0.0)                  # decision at t applies to the next funding payment
    gross = held * funding                           # short perp receives positive funding
    if basis_drift is not None:
        gross = gross + held * basis_drift.reindex(funding.index).fillna(0.0)
    turnover = (pos - pos.shift(1).fillna(0.0)).abs()
    # entering/exiting touches 2 legs (spot + perp): 2 × per-side cost
    cost = turnover * 2.0 * p.fee_bps_per_leg_side / 1e4
    cash = held * p.cash_rate_ann / p.periods_per_year
    net = gross - cost - cash
    summ = summarize(net, p.periods_per_year, positions=held, turnover=turnover)
    n_trades = int((pos.diff() == 1).sum())
    return CarryResult(net, held, summ, float(funding.mean() * p.periods_per_year), n_trades, float(held.mean()))


def funding_overlay(target: pd.Series, funding_per_bar: pd.Series, max_pay_bps_per_bar: float = 3.0) -> pd.Series:
    """Cut long exposure when you would pay more than ``max_pay_bps_per_bar``,
    cut shorts when funding is very negative. Expectation: fewer crowded positions."""
    f = funding_per_bar.reindex(target.index).fillna(0.0) * 1e4
    out = target.copy()
    out[(f > max_pay_bps_per_bar) & (out > 0)] = 0.0
    out[(f < -max_pay_bps_per_bar) & (out < 0)] = 0.0
    return out
