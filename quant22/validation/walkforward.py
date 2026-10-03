"""Walk-forward (anchored or rolling) parameter selection with purge/embargo.

The pattern: for each window, choose the parameter set that maximises an
objective *in-sample*, then apply only that set to the following out-of-sample
window. Concatenating the OOS segments gives a single honest return stream that
already contains the cost of re-selection.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Callable, Iterable, Sequence

import numpy as np
import pandas as pd

from ..backtest.engine import CostModel, run_backtest
from ..backtest.metrics import sharpe_ratio


@dataclass
class WFSegment:
    train_start: pd.Timestamp
    train_end: pd.Timestamp
    test_start: pd.Timestamp
    test_end: pd.Timestamp
    chosen: dict
    is_sharpe: float
    oos_sharpe: float


@dataclass
class WalkForwardResult:
    oos_returns: pd.Series
    segments: list[WFSegment]

    def table(self) -> pd.DataFrame:
        return pd.DataFrame([{**s.chosen, "train_start": s.train_start.date(), "test_start": s.test_start.date(),
                              "test_end": s.test_end.date(), "is_sharpe": s.is_sharpe, "oos_sharpe": s.oos_sharpe}
                             for s in self.segments])


def rolling_windows(index: pd.DatetimeIndex, train_bars: int, test_bars: int, embargo_bars: int = 0,
                    anchored: bool = False) -> Iterable[tuple[np.ndarray, np.ndarray]]:
    n = len(index)
    start = 0
    while True:
        tr_end = start + train_bars
        te_start = tr_end + embargo_bars
        te_end = te_start + test_bars
        if te_start >= n:
            break
        tr_start = 0 if anchored else start
        yield np.arange(tr_start, tr_end), np.arange(te_start, min(te_end, n))
        start += test_bars


def walk_forward(prices: pd.Series, signal_fn: Callable[[pd.Series, dict], pd.Series], param_grid: Sequence[dict],
                 train_bars: int, test_bars: int, bars_per_year: float, costs: CostModel | None = None,
                 embargo_bars: int = 0, anchored: bool = False, objective: Callable[[pd.Series], float] | None = None,
                 warmup_bars: int = 0) -> WalkForwardResult:
    """``signal_fn(prices_window, params) -> target exposure series`` must use only past data.

    ``warmup_bars`` extra history is prepended to each window so indicators are
    warmed up; the objective is still evaluated on the window proper.
    """
    costs = costs or CostModel()
    objective = objective or (lambda r: sharpe_ratio(r, bars_per_year))
    # Pre-compute every variant's net return over the full sample once (signals are causal,
    # so the return on bar t is identical whether computed on a window or the full series,
    # up to indicator warm-up which we handle by warmup_bars).
    variant_returns = []
    for params in param_grid:
        tgt = signal_fn(prices, params)
        res = run_backtest(prices, tgt, costs, bars_per_year, with_trades=False)
        variant_returns.append(res.returns.values)
    R = np.column_stack(variant_returns)
    segs: list[WFSegment] = []
    oos = pd.Series(np.nan, index=prices.index)
    for tr_idx, te_idx in rolling_windows(prices.index, train_bars, test_bars, embargo_bars, anchored):
        tr_idx = tr_idx[tr_idx >= warmup_bars]
        if len(tr_idx) < max(30, train_bars // 2):
            continue
        scores = [objective(pd.Series(R[tr_idx, k])) for k in range(R.shape[1])]
        k = int(np.nanargmax(scores))
        oos.iloc[te_idx] = R[te_idx, k]
        segs.append(WFSegment(prices.index[tr_idx[0]], prices.index[tr_idx[-1]], prices.index[te_idx[0]],
                              prices.index[te_idx[-1]], dict(param_grid[k]), float(scores[k]),
                              sharpe_ratio(R[te_idx, k], bars_per_year)))
    return WalkForwardResult(oos.dropna(), segs)
