"""Causal feature construction. Every function returns a series aligned to the
input index where the value at t uses data up to and including t."""
from __future__ import annotations

import numpy as np
import pandas as pd


def log_returns(prices: pd.Series) -> pd.Series:
    return np.log(prices).diff()


def realized_vol(prices: pd.Series, lookback: int, bars_per_year: float, ewm: bool = True) -> pd.Series:
    r = log_returns(prices)
    if ewm:
        v = r.ewm(span=lookback, min_periods=max(5, lookback // 3)).std()
    else:
        v = r.rolling(lookback, min_periods=max(5, lookback // 3)).std()
    return v * np.sqrt(bars_per_year)


def ewma_trend(prices: pd.Series, short: int, long: int, norm_window: int = 63) -> pd.Series:
    """Baz et al. (2015) style trend: (EWMA_s − EWMA_l) / rolling σ(price), then
    squashed through x·exp(−x²/4)/0.89 so extreme values do not dominate.

    Note on the normaliser: we scale by a rolling std of *price* over
    ``norm_window`` (the paper's first normalisation) and skip the second
    normalisation to keep the signal interpretable as "trend in σ units".
    """
    f = prices.ewm(span=short, min_periods=short).mean()
    s = prices.ewm(span=long, min_periods=long).mean()
    sd = prices.rolling(norm_window, min_periods=norm_window // 2).std()
    x = (f - s) / sd.replace(0, np.nan)
    return (x * np.exp(-(x**2) / 4.0) / 0.89).clip(-2, 2)


def tsmom_sign(prices: pd.Series, lookback: int) -> pd.Series:
    """Moskowitz–Ooi–Pedersen (2012) time-series momentum: sign of past return."""
    return np.sign(prices / prices.shift(lookback) - 1.0)


def donchian_breakout(prices: pd.Series, lookback: int) -> pd.Series:
    """+1 when close is at/above the trailing max, −1 at/below the trailing min,
    else carry the previous state (classic turtle-style channel)."""
    hi = prices.rolling(lookback, min_periods=lookback).max().shift(1)
    lo = prices.rolling(lookback, min_periods=lookback).min().shift(1)
    sig = pd.Series(np.nan, index=prices.index)
    sig[prices >= hi] = 1.0
    sig[prices <= lo] = -1.0
    return sig.ffill().fillna(0.0)


def zscore(x: pd.Series, lookback: int) -> pd.Series:
    m = x.rolling(lookback, min_periods=lookback // 2).mean()
    s = x.rolling(lookback, min_periods=lookback // 2).std()
    return (x - m) / s.replace(0, np.nan)


def vol_scaled_position(signal: pd.Series, prices: pd.Series, target_vol: float, vol_lookback: int,
                        bars_per_year: float, max_leverage: float = 3.0) -> pd.Series:
    """Position = signal × target_vol / realised_vol, capped. This is the
    Moreira–Muir / Harvey et al. volatility-targeting overlay."""
    rv = realized_vol(prices, vol_lookback, bars_per_year)
    lev = (target_vol / rv.replace(0, np.nan)).clip(upper=max_leverage)
    return (signal * lev).fillna(0.0)
