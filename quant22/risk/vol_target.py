"""Portfolio-level risk controls used by the live/paper executor and the research scripts."""
from __future__ import annotations

from dataclasses import dataclass

import numpy as np
import pandas as pd


@dataclass
class RiskLimits:
    target_vol_ann: float = 0.20        # portfolio volatility target
    max_gross_leverage: float = 2.0
    max_single_asset_leverage: float = 1.5
    max_daily_loss: float = 0.03        # fraction of equity; hitting it flattens for the day
    max_drawdown_kill: float = 0.15     # fraction from HWM; hitting it stops trading entirely
    min_notional_usd: float = 10.0


def risk_parity_weights(returns: pd.DataFrame, lookback: int, bars_per_year: float) -> pd.DataFrame:
    """Inverse-volatility weights (equal risk contribution ignoring correlations)."""
    vol = returns.ewm(span=lookback, min_periods=lookback // 2).std() * np.sqrt(bars_per_year)
    inv = 1.0 / vol.replace(0, np.nan)
    return inv.div(inv.sum(axis=1), axis=0)


def portfolio_vol_scalar(port_returns: pd.Series, target_vol_ann: float, lookback: int, bars_per_year: float,
                         max_leverage: float) -> pd.Series:
    rv = port_returns.ewm(span=lookback, min_periods=lookback // 2).std() * np.sqrt(bars_per_year)
    return (target_vol_ann / rv.replace(0, np.nan)).clip(upper=max_leverage).shift(1).fillna(0.0)


def apply_kill_switch(returns: pd.Series, max_dd: float) -> pd.Series:
    """Zero out returns after the equity curve breaches ``max_dd`` from its peak (until the end).
    Used to show what a hard account-level stop does to a strategy's distribution."""
    eq = (1 + returns.fillna(0)).cumprod()
    dd = eq / eq.cummax() - 1
    killed = (dd <= -max_dd).cummax().shift(1).fillna(False)
    return returns.where(~killed, 0.0)
