"""Loaders for the datasets shipped in ``data/processed`` (and their raw sources).

Sources (all public, all re-fetchable with :mod:`quant22.data.fetch`):

* Coin Metrics Community data — daily price, volume and on-chain metrics for
  BTC, ETH and 9 other assets (https://github.com/coinmetrics/data, CC BY-NC 4.0).
* Hourly BTC OHLCV 2010→today — CryptoCompare aggregate, mirrored on GitHub by
  mouadja02/bitcoin-technical-indicators-dataset (Git LFS).
* Binance BTC/ETH spot, USDT-perp daily OHLCV and 8-hour funding rates
  2020→2026-04 — zwmjj/funding-rate-arb.
"""
from __future__ import annotations

from pathlib import Path

import pandas as pd

ROOT = Path(__file__).resolve().parents[2]
PROCESSED = ROOT / "data" / "processed"
RAW = ROOT / "data" / "raw"

ASSETS = ["btc", "eth", "sol", "bnb", "xrp", "ada", "doge", "ltc", "link", "avax", "dot"]


def load_daily(asset: str = "btc") -> pd.DataFrame:
    """Daily Coin Metrics frame indexed by UTC date with at least a ``close`` column."""
    df = pd.read_parquet(PROCESSED / f"{asset}_daily.parquet")
    return df


def load_daily_closes(assets: list[str] | None = None, min_price: float = 0.0) -> pd.DataFrame:
    assets = assets or ASSETS
    cols = {}
    for a in assets:
        s = load_daily(a)["close"]
        cols[a] = s[s > min_price]
    return pd.DataFrame(cols).sort_index()


def load_hourly_btc(start: str | None = "2014-01-01") -> pd.DataFrame:
    """Hourly BTC OHLCV. Early years (2010-2013) contain zero/missing bars and
    thin exchanges; the default ``start`` drops them."""
    df = pd.read_parquet(PROCESSED / "btc_hourly.parquet")
    if start:
        df = df.loc[start:]
    return df


def load_daily_latest(asset: str = "btc") -> pd.Series:
    """Most recent daily closes available locally. For BTC the hourly file (to 2026-10-03)
    is resampled to UTC days; other assets fall back to Coin Metrics (to 2026-05-24)."""
    if asset.lower() == "btc":
        h = load_hourly_btc("2014-01-01")["close"]
        d = h.resample("1D").last().dropna()
        return d.iloc[:-1] if h.index[-1].hour != 23 else d   # drop the still-forming day
    return load_daily(asset.lower())["close"]


def load_funding(asset: str = "btc") -> pd.Series:
    """8-hour Binance USDT-perp funding rate as a fraction (1e-4 = 1bp), UTC index."""
    s = pd.read_parquet(PROCESSED / f"{asset}_funding_8h.parquet")["funding_rate"]
    return s


def load_binance_daily(asset: str = "btc", kind: str = "perp") -> pd.DataFrame:
    return pd.read_parquet(PROCESSED / f"{asset}_{kind}_daily.parquet")
