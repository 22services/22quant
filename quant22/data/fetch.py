"""Fetch and normalise the public datasets used by this repository.

Run ``python -m quant22.data.fetch`` to (re)build ``data/processed``.

Only GitHub-hosted mirrors are used, so this works from locked-down networks
(exchange APIs are often blocked from cloud sandboxes). When you run on a
machine with open network access you can also pull live data from Hyperliquid
with :func:`fetch_hyperliquid_candles` / :func:`fetch_hyperliquid_funding`.
"""
from __future__ import annotations

import json
import sys
import urllib.request
from pathlib import Path

import numpy as np
import pandas as pd

from .loaders import ASSETS, PROCESSED, RAW

COINMETRICS = "https://raw.githubusercontent.com/coinmetrics/data/master/csv/{asset}.csv"
HOURLY_BTC = "https://media.githubusercontent.com/media/mouadja02/bitcoin-technical-indicators-dataset/main/bitcoin-hourly-ohlcv.csv"
FUNDING_REPO = "https://raw.githubusercontent.com/zwmjj/funding-rate-arb/main/data/raw/{name}.csv"
HL_INFO = "https://api.hyperliquid.xyz/info"

DAILY_KEEP = {
    "PriceUSD": "close", "ReferenceRateUSD": "ref_rate", "volume_reported_spot_usd_1d": "volume_usd",
    "CapMrktCurUSD": "mcap_usd", "CapMVRVCur": "mvrv", "AdrActCnt": "active_addresses", "TxCnt": "tx_count",
    "HashRate": "hashrate", "FlowInExUSD": "exch_inflow_usd", "FlowOutExUSD": "exch_outflow_usd",
    "SplyCur": "supply", "IssTotUSD": "issuance_usd", "FeeTotNtv": "fees_native",
}


def _download(url: str, dest: Path, force: bool = False) -> Path:
    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.exists() and not force:
        return dest
    print(f"GET {url}")
    with urllib.request.urlopen(url, timeout=180) as r, open(dest, "wb") as f:
        f.write(r.read())
    return dest


def build_daily(asset: str, force: bool = False) -> pd.DataFrame:
    raw = _download(COINMETRICS.format(asset=asset), RAW / f"{asset}_coinmetrics.csv", force)
    df = pd.read_csv(raw, low_memory=False)
    df["time"] = pd.to_datetime(df["time"], utc=True)
    df = df.set_index("time")
    keep = {k: v for k, v in DAILY_KEEP.items() if k in df.columns}
    out = df[list(keep)].rename(columns=keep).astype(float)
    if "close" not in out:
        out["close"] = np.nan
    if "ref_rate" in out:
        out["close"] = out["close"].fillna(out["ref_rate"])
    out = out[out["close"].notna() & (out["close"] > 0)]
    PROCESSED.mkdir(parents=True, exist_ok=True)
    out.to_parquet(PROCESSED / f"{asset}_daily.parquet")
    return out


def build_hourly_btc(force: bool = False) -> pd.DataFrame:
    raw = _download(HOURLY_BTC, RAW / "btc_hourly_cryptocompare.csv", force)
    df = pd.read_csv(raw)
    df = df.rename(columns={"UNIX_TIMESTAMP": "ts", "OPEN": "open", "HIGH": "high", "LOW": "low", "CLOSE": "close",
                            "VOLUME_USD": "volume_usd", "VOLUME_BTC": "volume_btc"})
    df["time"] = pd.to_datetime(df["ts"], unit="s", utc=True)
    df = df.set_index("time")[["open", "high", "low", "close", "volume_usd", "volume_btc"]].astype(float)
    df = df[(df["close"] > 0) & (df["high"] >= df["low"])]
    df = df[~df.index.duplicated(keep="last")].sort_index()
    df.to_parquet(PROCESSED / "btc_hourly.parquet")
    return df


def build_funding(asset: str, force: bool = False) -> None:
    f = pd.read_csv(_download(FUNDING_REPO.format(name=f"{asset}_funding_rate"), RAW / f"{asset}_funding_rate.csv", force))
    f["time"] = pd.to_datetime(f["timestamp"], utc=True, format="mixed").dt.floor("h")
    s = f.set_index("time")["fundingRate"].astype(float).rename("funding_rate")
    s = s[~s.index.duplicated(keep="last")].sort_index()
    s.to_frame().to_parquet(PROCESSED / f"{asset}_funding_8h.parquet")
    for kind in ("perp", "spot"):
        d = pd.read_csv(_download(FUNDING_REPO.format(name=f"{asset}_{kind}"), RAW / f"{asset}_{kind}_daily.csv", force))
        d["time"] = pd.to_datetime(d["timestamp"], utc=True, format="mixed")
        d = d.set_index("time")[["open", "high", "low", "close", "volume"]].astype(float).sort_index()
        d.to_parquet(PROCESSED / f"{asset}_{kind}_daily.parquet")


def fetch_hyperliquid_candles(coin: str, interval: str, start_ms: int, end_ms: int) -> pd.DataFrame:
    """Live candles from Hyperliquid's public info endpoint (weight 20 per call, 1200/min/IP)."""
    body = json.dumps({"type": "candleSnapshot", "req": {"coin": coin, "interval": interval,
                                                         "startTime": start_ms, "endTime": end_ms}}).encode()
    req = urllib.request.Request(HL_INFO, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        rows = json.loads(r.read())
    df = pd.DataFrame(rows).rename(columns={"t": "ts", "o": "open", "h": "high", "l": "low", "c": "close", "v": "volume"})
    df["time"] = pd.to_datetime(df["ts"], unit="ms", utc=True)
    return df.set_index("time")[["open", "high", "low", "close", "volume"]].astype(float)


def fetch_hyperliquid_funding(coin: str, start_ms: int, end_ms: int | None = None) -> pd.Series:
    body = {"type": "fundingHistory", "coin": coin, "startTime": start_ms}
    if end_ms:
        body["endTime"] = end_ms
    req = urllib.request.Request(HL_INFO, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        rows = json.loads(r.read())
    s = pd.Series({pd.to_datetime(x["time"], unit="ms", utc=True): float(x["fundingRate"]) for x in rows})
    return s.sort_index().rename("funding_rate")


def main(argv: list[str]) -> None:
    force = "--force" in argv
    for a in ASSETS:
        d = build_daily(a, force)
        print(f"{a}: {len(d)} days {d.index[0].date()} → {d.index[-1].date()}")
    h = build_hourly_btc(force)
    print(f"btc hourly: {len(h)} bars {h.index[0]} → {h.index[-1]}")
    for a in ("btc", "eth"):
        build_funding(a, force)
        print(f"{a} funding built")


if __name__ == "__main__":
    main(sys.argv[1:])
