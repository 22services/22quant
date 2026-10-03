"""Hyperliquid execution adapter — paper/dry-run by default.

Why Hyperliquid: on-chain perp DEX with free L2 order-book data, 1-hour
funding, 0.045%/0.015% taker/maker fees (2026 schedule), a documented REST +
WebSocket API (1200 weight/min/IP) and an official Python SDK
(``pip install hyperliquid-python-sdk``). No KYC wall between a small account
and institutional-grade data, which is exactly the asymmetry the interview
pointed at.

Safety model
------------
* ``dry_run=True`` (default) never signs anything; every intended order is
  appended to ``paper_log.jsonl`` so a paper track record can be audited.
* Live mode requires **both** ``dry_run=False`` and the environment variable
  ``QUANT22_LIVE=I_UNDERSTAND_THE_RISK``. Use an *API wallet* (agent key) with
  trade-only permission, never your main wallet key.
* :class:`RiskGuard` enforces hard limits before any order: max leverage, max
  daily loss, max drawdown kill-switch, minimum notional.

This file is intentionally thin: strategy logic lives in :mod:`quant22.strategies`,
and the executor only turns *target exposures* into orders.
"""
from __future__ import annotations

import json
import os
import time
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import Any, Optional

from ..risk.vol_target import RiskLimits

LIVE_SENTINEL = "I_UNDERSTAND_THE_RISK"


@dataclass
class AccountState:
    equity_usd: float
    positions: dict[str, float]          # coin -> signed notional USD
    day_start_equity: float
    high_water_mark: float


@dataclass
class OrderIntent:
    ts: float
    coin: str
    side: str                 # "buy" | "sell"
    notional_usd: float
    size: float
    ref_price: float
    reason: str
    dry_run: bool
    status: str = "intended"
    response: Optional[Any] = None


class RiskGuard:
    def __init__(self, limits: RiskLimits):
        self.limits = limits
        self.halted = False
        self.halt_reason = ""

    def check_account(self, st: AccountState) -> None:
        if st.high_water_mark > 0 and st.equity_usd / st.high_water_mark - 1.0 <= -self.limits.max_drawdown_kill:
            self.halted, self.halt_reason = True, f"drawdown kill-switch ({st.equity_usd:.0f} vs HWM {st.high_water_mark:.0f})"
        if st.day_start_equity > 0 and st.equity_usd / st.day_start_equity - 1.0 <= -self.limits.max_daily_loss:
            self.halted, self.halt_reason = True, "daily loss limit"

    def clamp_target(self, st: AccountState, coin: str, target_notional: float) -> float:
        if self.halted:
            return 0.0
        cap_single = self.limits.max_single_asset_leverage * st.equity_usd
        target = max(-cap_single, min(cap_single, target_notional))
        others = sum(abs(v) for c, v in st.positions.items() if c != coin)
        room = self.limits.max_gross_leverage * st.equity_usd - others
        if abs(target) > room:
            target = (1 if target > 0 else -1) * max(room, 0.0)
        return target


class HyperliquidExecutor:
    def __init__(self, limits: RiskLimits | None = None, dry_run: bool = True, log_path: str | Path = "paper_log.jsonl",
                 testnet: bool = True, account_address: str | None = None, secret_key: str | None = None):
        self.limits = limits or RiskLimits()
        self.guard = RiskGuard(self.limits)
        self.dry_run = dry_run
        self.log_path = Path(log_path)
        self.testnet = testnet
        self._info = None
        self._exchange = None
        self.address = account_address or os.environ.get("HL_ACCOUNT_ADDRESS")
        if not dry_run:
            if os.environ.get("QUANT22_LIVE") != LIVE_SENTINEL:
                raise RuntimeError("Refusing live mode: set QUANT22_LIVE=I_UNDERSTAND_THE_RISK and use an API wallet.")
            self._connect(secret_key or os.environ.get("HL_SECRET_KEY"))

    # ------------------------------------------------------------------ connectivity
    def _connect(self, secret_key: str | None) -> None:
        from hyperliquid.info import Info                 # type: ignore
        from hyperliquid.exchange import Exchange         # type: ignore
        from hyperliquid.utils import constants           # type: ignore
        import eth_account                                # type: ignore

        url = constants.TESTNET_API_URL if self.testnet else constants.MAINNET_API_URL
        self._info = Info(url, skip_ws=True)
        if secret_key:
            wallet = eth_account.Account.from_key(secret_key)
            self._exchange = Exchange(wallet, url, account_address=self.address)

    def mid(self, coin: str, fallback: float | None = None) -> float:
        if self._info is None:
            if fallback is None:
                raise RuntimeError("dry-run executor needs a reference price (fallback)")
            return fallback
        return float(self._info.all_mids()[coin])

    def account_state(self, fallback: AccountState | None = None) -> AccountState:
        if self._info is None or not self.address:
            if fallback is None:
                raise RuntimeError("dry-run executor needs an AccountState (fallback)")
            return fallback
        us = self._info.user_state(self.address)
        eq = float(us["marginSummary"]["accountValue"])
        pos = {}
        for ap in us.get("assetPositions", []):
            p = ap["position"]
            szi = float(p["szi"])
            if szi != 0:
                pos[p["coin"]] = szi * float(p["entryPx"]) if p.get("entryPx") else szi * self.mid(p["coin"])
        return AccountState(eq, pos, fallback.day_start_equity if fallback else eq,
                            max(eq, fallback.high_water_mark) if fallback else eq)

    # ------------------------------------------------------------------ orders
    def rebalance(self, coin: str, target_notional_usd: float, state: AccountState, ref_price: float,
                  reason: str = "", slippage: float = 0.002, size_decimals: int = 4) -> OrderIntent | None:
        self.guard.check_account(state)
        target = self.guard.clamp_target(state, coin, target_notional_usd)
        current = state.positions.get(coin, 0.0)
        delta = target - current
        if abs(delta) < self.limits.min_notional_usd:
            return None
        side = "buy" if delta > 0 else "sell"
        size = round(abs(delta) / ref_price, size_decimals)
        intent = OrderIntent(time.time(), coin, side, abs(delta), size, ref_price,
                             reason or (self.guard.halt_reason if self.guard.halted else "rebalance"), self.dry_run)
        if self.dry_run or self._exchange is None:
            intent.status = "paper"
        else:
            # IOC-style market order with slippage cap (SDK: market_open(coin, is_buy, sz, px=None, slippage))
            resp = self._exchange.market_open(coin, side == "buy", size, None, slippage)
            intent.status = resp.get("status", "unknown") if isinstance(resp, dict) else "sent"
            intent.response = resp
        self._log(intent)
        return intent

    def flatten_all(self, state: AccountState, prices: dict[str, float], reason: str = "flatten") -> list[OrderIntent]:
        out = []
        for coin, notional in list(state.positions.items()):
            if notional != 0:
                it = self.rebalance(coin, 0.0, state, prices[coin], reason=reason)
                if it:
                    out.append(it)
        return out

    def _log(self, intent: OrderIntent) -> None:
        self.log_path.parent.mkdir(parents=True, exist_ok=True)
        with open(self.log_path, "a") as f:
            f.write(json.dumps(asdict(intent), default=str) + "\n")
