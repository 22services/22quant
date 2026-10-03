"""Vectorised bar backtester with explicit, itemised costs.

Conventions (the single most common source of fake edges is getting these wrong):

* ``prices`` are bar closes indexed by time.
* ``target`` is the exposure (fraction of equity, +long/-short, may exceed 1 for
  leverage) that the strategy *decides* at the close of bar t using only data up
  to and including bar t.
* That exposure earns the return from close t to close t+1. Internally we shift
  the target by one bar, so there is no look-ahead by construction.
* Costs are charged on turnover: |Δposition| × (fee + slippage), in return
  units. Funding (perpetual swaps) is charged per bar on the held position:
  longs pay when funding is positive.

The engine is intentionally not event-driven: for daily/hourly research the
vectorised form is faster, easier to audit and good enough. Intrabar stops are
modelled separately in :func:`apply_stop_loss` using highs/lows where available.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Optional

import numpy as np
import pandas as pd

from .metrics import Summary, Trade, extract_trades, summarize, trade_stats, TradeStats


@dataclass
class CostModel:
    """All quantities are in basis points of notional per side unless stated.

    Defaults approximate a retail taker on Hyperliquid perps in 2026
    (0.045% taker fee) plus a conservative slippage/impact allowance.
    Use ``maker()`` for a resting-limit-order profile (0.015% maker fee, and
    near-zero slippage *but* fill uncertainty, which this engine does not model).
    """
    fee_bps: float = 4.5
    slippage_bps: float = 3.0
    funding: Optional[pd.Series] = None     # per-bar funding rate as a fraction (e.g. 0.0001 = 1bp)
    borrow_bps_per_year: float = 0.0        # cost of leverage above 1x, annualised

    @property
    def per_side(self) -> float:
        return (self.fee_bps + self.slippage_bps) / 1e4

    @classmethod
    def maker(cls, slippage_bps: float = 0.5, **kw) -> "CostModel":
        return cls(fee_bps=1.5, slippage_bps=slippage_bps, **kw)

    @classmethod
    def zero(cls) -> "CostModel":
        return cls(fee_bps=0.0, slippage_bps=0.0)


@dataclass
class BacktestResult:
    returns: pd.Series            # net returns per bar
    gross_returns: pd.Series      # before costs
    positions: pd.Series          # exposure held during each bar's return
    turnover: pd.Series           # |Δposition| per bar
    costs: pd.Series              # total cost per bar (return units)
    equity: pd.Series
    bars_per_year: float
    summary: Summary
    trades: list[Trade] = field(default_factory=list)
    trade_summary: Optional[TradeStats] = None

    def to_dict(self) -> dict:
        d = self.summary.to_dict()
        if self.trade_summary is not None:
            d.update({f"trade_{k}": v for k, v in self.trade_summary.__dict__.items()})
        d["total_cost_drag"] = float(self.costs.sum())
        return d


def run_backtest(prices: pd.Series, target: pd.Series, costs: CostModel | None = None,
                 bars_per_year: float = 365.25, max_leverage: float = 5.0,
                 high: Optional[pd.Series] = None, low: Optional[pd.Series] = None,
                 with_trades: bool = True) -> BacktestResult:
    costs = costs or CostModel()
    prices = prices.astype(float)
    target = target.reindex(prices.index).fillna(0.0).clip(-max_leverage, max_leverage)

    asset_ret = prices.pct_change().fillna(0.0)
    pos = target.shift(1).fillna(0.0)          # exposure during bar t's return
    gross = pos * asset_ret

    turnover = (pos - pos.shift(1).fillna(0.0)).abs()
    trade_cost = turnover * costs.per_side

    funding_cost = pd.Series(0.0, index=prices.index)
    if costs.funding is not None:
        f = costs.funding.reindex(prices.index).fillna(0.0)
        funding_cost = pos * f                 # long pays positive funding, short receives

    borrow = pd.Series(0.0, index=prices.index)
    if costs.borrow_bps_per_year:
        excess = (pos.abs() - 1.0).clip(lower=0.0)
        borrow = excess * costs.borrow_bps_per_year / 1e4 / bars_per_year

    total_cost = trade_cost + funding_cost + borrow
    net = gross - total_cost
    equity = (1.0 + net).cumprod()

    summ = summarize(net, bars_per_year, positions=pos, turnover=turnover)
    trades: list[Trade] = []
    tstats = None
    if with_trades:
        trades = extract_trades(prices, pos, net_returns=net, high=high, low=low)
        tstats = trade_stats(trades)
    return BacktestResult(net, gross, pos, turnover, total_cost, equity, bars_per_year, summ, trades, tstats)


def apply_stop_loss(prices: pd.Series, target: pd.Series, stop_pct: float,
                    high: Optional[pd.Series] = None, low: Optional[pd.Series] = None) -> pd.Series:
    """Flatten the target after an intrabar adverse move of ``stop_pct`` from the
    trade's entry price, until the signal flips or re-enters.

    Conservative convention: if the bar's low (for longs) breaches the stop we
    assume the stop filled at the stop price *and* we do not re-enter until the
    target changes sign. We approximate the stop fill by flattening from the next
    bar (the bar's own return is still taken, which *overstates* the loss on gap
    bars — conservative).
    """
    p = prices.values.astype(float)
    hi = p if high is None else high.values.astype(float)
    lo = p if low is None else low.values.astype(float)
    tgt = target.reindex(prices.index).fillna(0.0).values.astype(float)
    out = tgt.copy()
    entry = np.nan
    stopped = False
    prev_sign = 0
    for t in range(len(p)):
        s = int(np.sign(tgt[t]))
        if s != prev_sign:
            stopped = False
            entry = p[t]           # we enter at close t
            prev_sign = s
        if s == 0:
            continue
        if stopped:
            out[t] = 0.0
            continue
        if t > 0:
            adverse = (lo[t] / entry - 1.0) if s > 0 else (1.0 - hi[t] / entry)
            if adverse <= -stop_pct:
                stopped = True
                out[t] = 0.0
    return pd.Series(out, index=prices.index)
