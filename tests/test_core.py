import numpy as np
import pandas as pd
import pytest

from quant22.backtest.engine import CostModel, run_backtest, apply_stop_loss
from quant22.backtest.metrics import sharpe_ratio, max_drawdown, extract_trades, trade_stats, skew_kurt
from quant22.validation.sharpe_tests import (probabilistic_sharpe_ratio, deflated_sharpe_ratio, expected_max_sharpe,
                                             min_track_record_length)
from quant22.validation.cscv import pbo_cscv
from quant22.validation.placebo import bootstrap_sharpe_pvalue, gbm_surrogates, bootstrap_surrogate_prices
from quant22.risk.kelly import kelly_binary, kelly_continuous, growth_rate, prob_drawdown_to, time_to_multiply
from quant22.features.astro import moon_phase, retrograde_periods, mercury_retrograde
from quant22.features.levels import fvg_events, grid_touch_reaction, round_number_step


def _prices(n=500, seed=0, drift=0.0, vol=0.02):
    rng = np.random.default_rng(seed)
    r = rng.normal(drift, vol, n)
    idx = pd.date_range("2020-01-01", periods=n, freq="D", tz="UTC")
    return pd.Series(100 * np.exp(np.cumsum(r)), index=idx)


def test_engine_no_lookahead_and_costs():
    p = _prices(n=3000)
    # A signal built from the *current* bar's return must not be paid that return:
    # with the correct one-bar shift it earns the next (independent) return -> no edge.
    cur = np.sign(p / p.shift(1) - 1).fillna(0)
    res_ok = run_backtest(p, cur, CostModel.zero())
    assert abs(res_ok.summary.sharpe) < 2.0
    # Sanity: the same signal *without* the shift would be perfect foresight (huge Sharpe).
    cheat = (cur * p.pct_change().fillna(0))
    assert sharpe_ratio(cheat, 365.25) > 10
    # a constant long position with zero costs reproduces buy & hold exactly
    bh = run_backtest(p, pd.Series(1.0, index=p.index), CostModel.zero())
    assert np.isclose(bh.equity.iloc[-1], p.iloc[-1] / p.iloc[0], rtol=1e-9)
    # costs: entering once and exiting once costs exactly 2 x per_side
    tgt = pd.Series(0.0, index=p.index)
    tgt.iloc[10:20] = 1.0
    cm = CostModel(fee_bps=10, slippage_bps=0)
    res = run_backtest(p, tgt, cm)
    assert np.isclose(res.costs.sum(), 2 * 10 / 1e4)


def test_funding_sign_convention():
    p = _prices()
    f = pd.Series(1e-4, index=p.index)  # longs pay 1bp per bar
    long = run_backtest(p, pd.Series(1.0, index=p.index), CostModel(fee_bps=0, slippage_bps=0, funding=f))
    short = run_backtest(p, pd.Series(-1.0, index=p.index), CostModel(fee_bps=0, slippage_bps=0, funding=f))
    assert long.costs.sum() > 0 and short.costs.sum() < 0


def test_trade_extraction_and_stats():
    p = _prices(100)
    tgt = pd.Series(0.0, index=p.index)
    tgt.iloc[5:15] = 1.0
    tgt.iloc[30:40] = -1.0
    res = run_backtest(p, tgt, CostModel.zero())
    assert len(res.trades) == 2
    assert res.trades[0].side == 1 and res.trades[1].side == -1
    st = trade_stats(res.trades)
    assert st.n_trades == 2 and 0 <= st.win_rate <= 1


def test_stop_loss_flattens():
    idx = pd.date_range("2020-01-01", periods=10, freq="D", tz="UTC")
    p = pd.Series([100, 100, 100, 90, 90, 90, 90, 90, 90, 90], index=idx, dtype=float)
    tgt = pd.Series(1.0, index=idx)
    out = apply_stop_loss(p, tgt, 0.05)
    assert out.iloc[3] == 0.0 and out.iloc[9] == 0.0 and out.iloc[1] == 1.0


def test_psr_dsr_mintrl():
    assert probabilistic_sharpe_ratio(0.1, 0.0, 1000) > 0.99
    assert probabilistic_sharpe_ratio(0.0, 0.0, 1000) == pytest.approx(0.5)
    # more trials -> higher bar
    assert expected_max_sharpe(100, 0.01) > expected_max_sharpe(10, 0.01) > 0
    rng = np.random.default_rng(0)
    trials = rng.normal(0, 0.05, 200)
    dsr, bench = deflated_sharpe_ratio(trials.max(), 500, trials)
    assert dsr < 0.9  # best of 200 noise strategies should not pass
    assert min_track_record_length(0.05, 0.0) > min_track_record_length(0.2, 0.0)


def test_pbo_random_trials_near_half():
    # A single noise matrix can contain a column that is lucky over the *whole* sample
    # (CSCV then rightly reports low PBO for that sample), so average over seeds.
    pbos = [pbo_cscv(np.random.default_rng(s).normal(0, 0.01, size=(2000, 30)), n_blocks=8).pbo for s in range(6)]
    assert 0.35 < np.mean(pbos) < 0.65


def test_pbo_real_edge_low():
    rng = np.random.default_rng(2)
    m = rng.normal(0, 0.01, size=(2000, 10))
    m[:, 3] += 0.003  # one genuinely good variant
    res = pbo_cscv(m, n_blocks=8)
    assert res.pbo < 0.2


def test_bootstrap_pvalue():
    rng = np.random.default_rng(3)
    r = rng.normal(0.0, 0.01, 1000)
    p, _, _ = bootstrap_sharpe_pvalue(r, n_boot=300)
    assert p > 0.05
    r2 = rng.normal(0.002, 0.01, 1000)
    p2, _, _ = bootstrap_sharpe_pvalue(r2, n_boot=300)
    assert p2 < 0.05


def test_surrogates_shape():
    lr = np.random.default_rng(0).normal(0, 0.01, 300)
    g = gbm_surrogates(lr, 5)
    b = bootstrap_surrogate_prices(lr, 5)
    assert g.shape == (5, 301) and b.shape == (5, 301)
    assert np.all(g > 0) and np.all(b > 0)


def test_kelly():
    assert kelly_binary(0.6, 1.0) == pytest.approx(0.2)
    assert kelly_continuous(0.01, 0.1) == pytest.approx(1.0)
    f = kelly_continuous(0.01, 0.1)
    assert growth_rate(f, 0.01, 0.1) > growth_rate(2 * f, 0.01, 0.1)
    assert prob_drawdown_to(0.5, 1.0) == pytest.approx(0.5)
    assert prob_drawdown_to(0.5, 0.5) == pytest.approx(0.125)
    assert time_to_multiply(2, 1.0, 1.0) == pytest.approx(np.log(2) / 0.5)


def test_moon_phase_known_dates():
    # New moon 2024-01-11 11:57 UTC ; Full moon 2024-01-25 17:54 UTC (USNO)
    nm = moon_phase(pd.DatetimeIndex(["2024-01-11 12:00"], tz="UTC")).iloc[0]
    fm = moon_phase(pd.DatetimeIndex(["2024-01-25 18:00"], tz="UTC")).iloc[0]
    assert min(nm, 1 - nm) < 0.03
    assert abs(fm - 0.5) < 0.03


def test_mercury_retrograde_known_period():
    # Mercury was retrograde roughly 2024-04-01 .. 2024-04-25 (astronomical almanacs)
    periods = retrograde_periods("2024-01-01", "2024-12-31")
    assert 3 <= len(periods) <= 4
    hit = [p for p in periods if abs((p[0] - pd.Timestamp("2024-04-01", tz="UTC")).days) <= 3
           and abs((p[1] - pd.Timestamp("2024-04-25", tz="UTC")).days) <= 3]
    assert hit, periods
    r = mercury_retrograde(pd.DatetimeIndex(["2024-04-10", "2024-06-10"], tz="UTC"))
    assert bool(r.iloc[0]) and not bool(r.iloc[1])


def test_fvg_and_grid():
    rng = np.random.default_rng(0)
    close = 100 * np.exp(np.cumsum(rng.normal(0, 0.01, 2000)))
    high = close * (1 + np.abs(rng.normal(0, 0.005, 2000)))
    low = close * (1 - np.abs(rng.normal(0, 0.005, 2000)))
    st = fvg_events(high, low, close, horizon=24)
    assert st.n_events > 0 and 0 <= st.fill_rate <= 1
    lr = grid_touch_reaction(close, round_number_step(5.0), 0.0, 0.002, 6)
    assert lr.n_touches > 0


def test_skew_kurt_normal():
    r = np.random.default_rng(0).normal(0, 1, 200000)
    s, k = skew_kurt(r)
    assert abs(s) < 0.05 and abs(k - 3) < 0.1


def test_paper_mark_to_market_and_guard():
    from quant22.execution.paper_trade import mark_to_market
    from quant22.execution.hyperliquid import AccountState, HyperliquidExecutor
    from quant22.risk.vol_target import RiskLimits
    st = AccountState(10_000.0, {"BTC": 5_000.0, "ETH": -2_000.0}, 10_000.0, 10_000.0)
    pnl = mark_to_market(st, {"BTC": 100.0, "ETH": 10.0}, {"BTC": 110.0, "ETH": 11.0})
    assert pnl == pytest.approx(500.0 - 200.0)
    assert st.equity_usd == pytest.approx(10_300.0) and st.positions["BTC"] == pytest.approx(5_500.0)
    ex = HyperliquidExecutor(RiskLimits(max_single_asset_leverage=1.0, max_gross_leverage=1.5), dry_run=True,
                             log_path="/dev/null")
    st2 = AccountState(10_000.0, {"ETH": -8_000.0}, 10_000.0, 10_000.0)
    it = ex.rebalance("BTC", 50_000.0, st2, ref_price=100.0)
    assert it.notional_usd == pytest.approx(7_000.0)   # gross cap 1.5x minus 0.8x already used
    halted = AccountState(8_000.0, {"BTC": 4_000.0}, 8_000.0, 10_000.0)  # -20% from HWM > 15% kill
    it2 = ex.rebalance("BTC", 4_000.0, halted, ref_price=100.0)
    assert ex.guard.halted and it2.side == "sell" and it2.notional_usd == pytest.approx(4_000.0)
    with pytest.raises(RuntimeError):
        HyperliquidExecutor(dry_run=False)


def test_prop_firm_sanity():
    from quant22.risk.prop_firm import PropRules, TraderModel, simulate_evaluation
    bad = simulate_evaluation(PropRules(), TraderModel(sharpe_ann=-1.0, daily_vol_usd=1000), n_sims=4000)
    good = simulate_evaluation(PropRules(), TraderModel(sharpe_ann=2.0, daily_vol_usd=1000), n_sims=4000)
    assert good.p_pass > bad.p_pass and good.ev_per_challenge_usd > bad.ev_per_challenge_usd
    assert 0 <= bad.p_pass <= 1 and abs(bad.p_pass + bad.p_fail + bad.p_timeout - 1) < 1e-9
