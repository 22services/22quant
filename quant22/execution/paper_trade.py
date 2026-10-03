"""Daily paper-trading loop for the validated trend sleeve (dry-run by default).

    python -m quant22.execution.paper_trade --coins BTC ETH --equity 10000
    python -m quant22.execution.paper_trade --coins BTC --equity 10000 --live --mainnet   # requires QUANT22_LIVE sentinel

Every run:
1. loads daily closes (Hyperliquid candles if the API is reachable, else the local parquet),
2. computes the pre-specified composite trend exposure per coin (TrendParams),
3. risk-parity weights the coins and applies the portfolio limits (RiskLimits),
4. hands target notionals to the executor, which logs or sends orders.

Run it once a day after 00:00 UTC (daily bar close). State for dry-run mode is kept in
``paper_state.json`` so the paper book persists between runs.
"""
from __future__ import annotations

import argparse
import json
import time
from pathlib import Path

import numpy as np
import pandas as pd

from ..data.fetch import fetch_hyperliquid_candles
from ..data.loaders import load_daily_latest
from ..risk.vol_target import RiskLimits, risk_parity_weights
from ..strategies.trend import TrendParams, trend_positions
from .hyperliquid import AccountState, HyperliquidExecutor

DAILY = 365.25
STATE = Path("paper_state.json")


def load_closes(coin: str, days: int = 500) -> pd.Series:
    try:
        end = int(time.time() * 1000)
        start = end - days * 86_400_000
        df = fetch_hyperliquid_candles(coin, "1d", start, end)
        return df["close"].rename(coin)
    except Exception as exc:  # network blocked, API change, etc.
        s = load_daily_latest(coin.lower()).rename(coin).iloc[-days:]
        print(f"[{coin}] Hyperliquid candles unavailable ({exc.__class__.__name__}); using local data up to {s.index[-1].date()}")
        return s


def load_state(equity_default: float) -> tuple[AccountState, dict[str, float]]:
    if STATE.exists():
        d = json.loads(STATE.read_text())
        return (AccountState(d["equity_usd"], d["positions"], d["day_start_equity"], d["high_water_mark"]),
                d.get("last_prices", {}))
    return AccountState(equity_default, {}, equity_default, equity_default), {}


def mark_to_market(st: AccountState, last_prices: dict[str, float], prices: dict[str, float]) -> float:
    """Revalue paper positions (signed USD notionals) from the last run's prices to today's."""
    pnl = 0.0
    for c, notional in st.positions.items():
        if c in last_prices and c in prices and last_prices[c] > 0:
            r = prices[c] / last_prices[c] - 1.0
            pnl += notional * r
            st.positions[c] = notional * (1.0 + r)
    st.day_start_equity = st.equity_usd
    st.equity_usd += pnl
    st.high_water_mark = max(st.high_water_mark, st.equity_usd)
    return pnl


def save_state(st: AccountState, prices: dict[str, float]) -> None:
    STATE.write_text(json.dumps({**st.__dict__, "last_prices": prices}, indent=2))


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--coins", nargs="+", default=["BTC", "ETH"])
    ap.add_argument("--equity", type=float, default=10_000.0, help="paper equity if no state file / no live account")
    ap.add_argument("--live", action="store_true")
    ap.add_argument("--mainnet", action="store_true")
    ap.add_argument("--target-vol", type=float, default=0.20, help="portfolio vol target")
    args = ap.parse_args()

    limits = RiskLimits(target_vol_ann=args.target_vol, max_gross_leverage=1.5, max_single_asset_leverage=1.0)
    ex = HyperliquidExecutor(limits=limits, dry_run=not args.live, testnet=not args.mainnet)
    params = TrendParams()

    closes = pd.DataFrame({c: load_closes(c) for c in args.coins}).dropna(how="all")
    stale = {c: closes[c].dropna().index[-1] for c in closes}
    if len(set(stale.values())) > 1:
        print(f"WARNING: last bars differ across coins {dict((k, str(v.date())) for k, v in stale.items())} — refresh data before trading")
    exposures, rets = {}, {}
    for c in closes:
        px = closes[c].dropna()
        pos = trend_positions(px, params, DAILY)
        exposures[c] = float(pos.iloc[-1])
        rets[c] = px.pct_change() * pos.shift(1)
    w = risk_parity_weights(pd.DataFrame(rets).dropna(), 60, DAILY).iloc[-1].fillna(1.0 / len(args.coins))

    prices = {c: float(closes[c].dropna().iloc[-1]) for c in args.coins}
    paper, last_prices = load_state(args.equity)
    if ex.dry_run:
        pnl = mark_to_market(paper, last_prices, prices)
        if last_prices:
            print(f"paper P&L since last run: ${pnl:+,.2f}")
    st = ex.account_state(fallback=paper)
    ex.guard.check_account(st)
    print(f"equity ${st.equity_usd:,.0f} | HWM ${st.high_water_mark:,.0f} | halted={ex.guard.halted} {ex.guard.halt_reason}")
    rows = []
    for c in args.coins:
        px = prices[c]
        target = exposures[c] * w[c] * len(args.coins) * st.equity_usd   # per-coin vol-target × risk-parity share
        intent = ex.rebalance(c, target, st, ref_price=ex.mid(c, fallback=px), reason="daily trend rebalance")
        rows.append({"coin": c, "close": px, "trend_exposure": exposures[c], "rp_weight": float(w[c]),
                     "target_notional": target, "current": st.positions.get(c, 0.0),
                     "order": f"{intent.side} {intent.size} (${intent.notional_usd:,.0f})" if intent else "none"})
        if intent and ex.dry_run:   # book exactly what the guard let through (fills assumed at the reference price)
            st.positions[c] = st.positions.get(c, 0.0) + (intent.notional_usd if intent.side == "buy" else -intent.notional_usd)
    print(pd.DataFrame(rows).to_string(index=False))
    if ex.dry_run:
        save_state(st, prices)
        print(f"paper state saved to {STATE}; intents appended to {ex.log_path}")


if __name__ == "__main__":
    main()
