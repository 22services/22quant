"""Performance metrics for bar-level strategy returns and trade lists.

Everything here is deliberately simple and transparent: a metric you cannot
re-derive by hand is a metric you cannot audit, and un-audited metrics are how
backtests lie to their authors.
"""
from __future__ import annotations

from dataclasses import dataclass, asdict
from typing import Iterable, Optional

import numpy as np
import pandas as pd

HOURS_PER_YEAR = 24 * 365.25
DAYS_PER_YEAR = 365.25  # crypto trades every calendar day


def annualization_factor(bars_per_year: float) -> float:
    return float(np.sqrt(bars_per_year))


def sharpe_ratio(returns: pd.Series | np.ndarray, bars_per_year: float, rf_per_bar: float = 0.0) -> float:
    r = np.asarray(returns, dtype=float)
    r = r[~np.isnan(r)] - rf_per_bar
    if r.size < 2 or r.std(ddof=1) == 0:
        return 0.0
    return float(r.mean() / r.std(ddof=1) * np.sqrt(bars_per_year))


def sortino_ratio(returns: pd.Series | np.ndarray, bars_per_year: float) -> float:
    r = np.asarray(returns, dtype=float)
    r = r[~np.isnan(r)]
    downside = r[r < 0]
    if r.size < 2 or downside.size == 0:
        return 0.0
    dd = np.sqrt(np.mean(downside**2))
    return float(r.mean() / dd * np.sqrt(bars_per_year)) if dd > 0 else 0.0


def equity_curve(returns: pd.Series) -> pd.Series:
    return (1.0 + returns.fillna(0.0)).cumprod()


def max_drawdown(returns_or_equity: pd.Series, is_equity: bool = False) -> float:
    eq = returns_or_equity if is_equity else equity_curve(returns_or_equity)
    peak = eq.cummax()
    dd = eq / peak - 1.0
    return float(dd.min()) if len(dd) else 0.0


def drawdown_series(returns: pd.Series) -> pd.Series:
    eq = equity_curve(returns)
    return eq / eq.cummax() - 1.0


def cagr(returns: pd.Series, bars_per_year: float) -> float:
    r = returns.fillna(0.0)
    if len(r) == 0:
        return 0.0
    total = float((1.0 + r).prod())
    years = len(r) / bars_per_year
    if years <= 0 or total <= 0:
        return -1.0
    return total ** (1.0 / years) - 1.0


def annual_vol(returns: pd.Series, bars_per_year: float) -> float:
    r = returns.dropna()
    return float(r.std(ddof=1) * np.sqrt(bars_per_year)) if len(r) > 1 else 0.0


def calmar_ratio(returns: pd.Series, bars_per_year: float) -> float:
    mdd = abs(max_drawdown(returns))
    return cagr(returns, bars_per_year) / mdd if mdd > 0 else 0.0


def skew_kurt(returns: pd.Series | np.ndarray) -> tuple[float, float]:
    """Return (skewness, *raw* kurtosis). Normal => (0, 3). Used by PSR/DSR."""
    r = np.asarray(returns, dtype=float)
    r = r[~np.isnan(r)]
    if r.size < 4:
        return 0.0, 3.0
    m = r.mean()
    s = r.std(ddof=0)
    if s == 0:
        return 0.0, 3.0
    z = (r - m) / s
    return float(np.mean(z**3)), float(np.mean(z**4))


@dataclass
class Trade:
    entry_idx: int
    exit_idx: int
    side: int          # +1 long, -1 short
    entry_price: float
    exit_price: float
    size: float        # absolute exposure (fraction of equity) at entry
    pnl: float         # return contribution over the trade, net of costs if provided
    mfe: float = np.nan  # max favourable excursion, in return units
    mae: float = np.nan  # max adverse excursion, in return units (negative)
    bars: int = 0


def extract_trades(prices: pd.Series, positions: pd.Series, net_returns: Optional[pd.Series] = None,
                   high: Optional[pd.Series] = None, low: Optional[pd.Series] = None) -> list[Trade]:
    """Split a position series into round-trip trades.

    A trade starts when the position sign changes from 0 or flips, and ends
    when it returns to 0 or flips. ``positions`` is the exposure held *during*
    bar t's return (i.e. decided on bar t-1). MFE/MAE use high/low if given,
    otherwise closes.
    """
    p = np.asarray(prices, dtype=float)
    pos = np.asarray(positions.fillna(0.0), dtype=float)
    nr = None if net_returns is None else np.asarray(net_returns.fillna(0.0), dtype=float)
    hi = p if high is None else np.asarray(high, dtype=float)
    lo = p if low is None else np.asarray(low, dtype=float)
    trades: list[Trade] = []
    n = len(p)
    i = 0
    while i < n:
        if pos[i] == 0:
            i += 1
            continue
        side = int(np.sign(pos[i]))
        j = i
        while j + 1 < n and np.sign(pos[j + 1]) == side:
            j += 1
        # trade spans bars i..j (position held during these bars' returns)
        entry_price = p[i - 1] if i > 0 else p[i]
        exit_price = p[j]
        seg_hi = hi[i:j + 1]
        seg_lo = lo[i:j + 1]
        if side > 0:
            mfe = float(seg_hi.max() / entry_price - 1.0)
            mae = float(seg_lo.min() / entry_price - 1.0)
        else:
            mfe = float(1.0 - seg_lo.min() / entry_price)
            mae = float(1.0 - seg_hi.max() / entry_price)
        if nr is not None:
            pnl = float(np.prod(1.0 + nr[i:j + 1]) - 1.0)
        else:
            pnl = float(side * (exit_price / entry_price - 1.0) * abs(pos[i]))
        trades.append(Trade(i, j, side, float(entry_price), float(exit_price), float(abs(pos[i])), pnl,
                            mfe, mae, j - i + 1))
        i = j + 1
    return trades


@dataclass
class TradeStats:
    n_trades: int
    win_rate: float
    avg_win: float
    avg_loss: float
    profit_factor: float
    expectancy: float
    payoff_ratio: float
    median_mfe: float
    median_mae: float
    avg_bars: float
    pnl_without_top3: float  # total pnl after removing the 3 best trades (outlier dependence)


def trade_stats(trades: Iterable[Trade]) -> TradeStats:
    tl = list(trades)
    if not tl:
        return TradeStats(0, 0, 0, 0, 0, 0, 0, np.nan, np.nan, 0, 0)
    pnl = np.array([t.pnl for t in tl])
    wins = pnl[pnl > 0]
    losses = pnl[pnl <= 0]
    avg_win = float(wins.mean()) if wins.size else 0.0
    avg_loss = float(losses.mean()) if losses.size else 0.0
    gross_win = float(wins.sum())
    gross_loss = float(-losses.sum())
    pf = gross_win / gross_loss if gross_loss > 0 else np.inf
    wr = wins.size / pnl.size
    payoff = (avg_win / abs(avg_loss)) if avg_loss != 0 else np.inf
    srt = np.sort(pnl)[::-1]
    return TradeStats(
        n_trades=len(tl), win_rate=float(wr), avg_win=avg_win, avg_loss=avg_loss,
        profit_factor=float(pf), expectancy=float(pnl.mean()), payoff_ratio=float(payoff),
        median_mfe=float(np.nanmedian([t.mfe for t in tl])), median_mae=float(np.nanmedian([t.mae for t in tl])),
        avg_bars=float(np.mean([t.bars for t in tl])), pnl_without_top3=float(srt[3:].sum()) if len(srt) > 3 else 0.0,
    )


@dataclass
class Summary:
    bars: int
    years: float
    cagr: float
    ann_vol: float
    sharpe: float
    sortino: float
    max_drawdown: float
    calmar: float
    skew: float
    kurtosis: float
    hit_rate_bars: float
    avg_gross_leverage: float
    turnover_per_year: float
    total_return: float

    def to_dict(self) -> dict:
        return asdict(self)


def summarize(returns: pd.Series, bars_per_year: float, positions: Optional[pd.Series] = None,
              turnover: Optional[pd.Series] = None) -> Summary:
    r = returns.fillna(0.0)
    sk, ku = skew_kurt(r)
    n = len(r)
    years = n / bars_per_year if bars_per_year else np.nan
    lev = float(positions.abs().mean()) if positions is not None else np.nan
    to = float(turnover.sum() / years) if (turnover is not None and years) else np.nan
    return Summary(
        bars=n, years=float(years), cagr=cagr(r, bars_per_year), ann_vol=annual_vol(r, bars_per_year),
        sharpe=sharpe_ratio(r, bars_per_year), sortino=sortino_ratio(r, bars_per_year),
        max_drawdown=max_drawdown(r), calmar=calmar_ratio(r, bars_per_year), skew=sk, kurtosis=ku,
        hit_rate_bars=float((r[r != 0] > 0).mean()) if (r != 0).any() else 0.0,
        avg_gross_leverage=lev, turnover_per_year=to, total_return=float((1 + r).prod() - 1),
    )
