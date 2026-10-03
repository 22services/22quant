"""Short-horizon mean reversion on hourly bars.

Mechanism: liquidity provision. After an outsized move pushed by aggressive
taker flow (often liquidation cascades), the book re-forms and part of the move
is given back within hours. This is the edge *market makers* harvest; a retail
taker pays the spread twice to attempt it, which is why the test matters.
"""
from __future__ import annotations

from dataclasses import dataclass

import numpy as np
import pandas as pd

from ..features.signals import log_returns


@dataclass
class ReversalParams:
    move_window: int = 4          # bars over which the "shock" return is measured
    vol_window: int = 168         # bars for the vol normaliser (1 week of hours)
    z_entry: float = 2.0          # enter when |z| exceeds this
    hold_bars: int = 6            # hold for this many bars then flatten
    size: float = 1.0             # exposure per signal (fraction of equity)


def reversal_positions(prices: pd.Series, p: ReversalParams) -> pd.Series:
    r = log_returns(prices)
    move = r.rolling(p.move_window).sum()
    vol = r.rolling(p.vol_window, min_periods=p.vol_window // 2).std() * np.sqrt(p.move_window)
    z = (move / vol.replace(0, np.nan)).fillna(0.0)
    entries = np.where(z > p.z_entry, -1.0, np.where(z < -p.z_entry, 1.0, 0.0))
    pos = np.zeros(len(z))
    remaining = 0
    side = 0.0
    for i in range(len(z)):
        if entries[i] != 0 and remaining == 0:
            side, remaining = entries[i], p.hold_bars
        if remaining > 0:
            pos[i] = side * p.size
            remaining -= 1
    return pd.Series(pos, index=prices.index)
