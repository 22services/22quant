# 22quant — an evidence-first trading system (crypto perps)

> "A backtest is not there to reassure the trader; it is there to try to prove the strategy fails."

This repository answers one question honestly: **given a small account, a 2026 market, and
everything the internet says about trading — what actually has a measurable edge, how fast can it
compound, and how should it be run?** The full answer is in
[`docs/STRATEGY_REPORT.md`](docs/STRATEGY_REPORT.md); the sources in [`docs/SOURCES.md`](docs/SOURCES.md).

The code is a small, auditable Python toolkit (`quant22/`) plus ten research scripts (`research/`)
whose outputs are committed in `results/`. Every claim in the report can be regenerated with one
command.

> **Trading system: [`q22/`](q22/README.md)** (Rust, no Python). An automated multi-strategy
> engine for prop-firm accounts: Topstep via the TopstepX API, HyroTrader via Bybit. It
> classifies the market regime every session, enforces each firm's rules through a compliance
> guard, and comes with a local supervision dashboard. The same engine runs the backtest, the
> replay demo, paper and live trading. Try it with no account:
> `cd q22 && cargo run --release -p q22-app -- run -c config/demo_replay.toml`.

## Quick start

```bash
pip install -e ".[dev]"          # numpy, pandas, scipy, pyarrow (+ pytest, matplotlib)
python -m pytest -q              # 16 tests: look-ahead, costs, PSR/DSR, CSCV, Kelly, ephemeris, risk guard, prop MC…
python research/run_all.py       # ~8 min; rewrites results/*.md, results/*.json, results/run_all.log
python -m quant22.data.fetch     # (optional) refresh data/processed from the public GitHub mirrors
python -m quant22.execution.paper_trade --coins BTC ETH --equity 10000   # dry-run daily rebalance
```

## What is in here

| Path | Purpose |
|---|---|
| `quant22/backtest/` | Vectorised bar backtester with itemised costs (fee, slippage, funding, borrow) and a one-bar decision lag enforced by construction; trade extraction with MFE/MAE. |
| `quant22/validation/` | Probabilistic & Deflated Sharpe, Minimum Track Record Length, Probability of Backtest Overfitting (CSCV), walk-forward re-selection, stationary bootstrap, block permutation, random-walk surrogates, placebo level grids. |
| `quant22/risk/` | Kelly / fractional Kelly / growth & drawdown odds, volatility targeting, risk-parity, kill-switch, two-stage prop-firm Monte Carlo (evaluation **and** funded account). |
| `quant22/strategies/` | Trend (TSMOM + EWMA + breakout, vol-targeted), funding carry, hourly reversal. |
| `quant22/features/` | Causal signals; `astro.py` (lunar phase, Mercury retrograde from Keplerian elements); `levels.py` (FVG, round numbers, Fibonacci, PO3/Goldbach with placebo machinery). |
| `quant22/execution/` | Hyperliquid adapter (dry-run by default, live needs an explicit sentinel), `RiskGuard`, daily `paper_trade` loop. |
| `research/01…12` | The experiments (myths, edges, prop-firm EV, growth speed, ensemble, stylized facts, contract granularity). |
| `data/processed/` | BTC hourly 2010→2026-10-03, 11 coins daily (Coin Metrics), Binance funding/perp/spot 2020→2026-04. |

## The three rules this code enforces

1. **No look-ahead.** A target decided on bar *t* earns the return *t→t+1*. The engine shifts for
   you; a test (`tests/test_core.py::test_engine_no_lookahead_and_costs`) proves the cheat does
   not work.
2. **Costs first.** Default costs are a Hyperliquid retail taker (0.045% + 3bp slippage). Every
   result is shown at zero / maker / taker / 2× taker so you can see what the edge is made of.
3. **Placebo or it did not happen.** Levels are compared with shifted grids, calendars with shifted
   calendars, patterns with random-walk surrogates, and the best of a parameter grid with the
   expected best of *N* noise strategies (Deflated Sharpe) and with CSCV (PBO).

## Status of each idea (see the report for numbers)

| Idea | Verdict |
|---|---|
| Daily trend following on BTC with vol targeting (pre-specified ensemble) | **VALIDATED** 2014→2026 (Sharpe ≈1.05 net, bootstrap p=0.001, DSR 0.997, beats re-optimised walk-forward); **flat in 2025-26**; top-3 trades = 73% of P&L |
| Multi-coin trend basket | **VALIDATED-weak** (Sharpe ≈0.7, −0.1 corr to BTC) |
| Funding-rate carry | **VALIDATED historically, dormant now** (regime income; ~1%/yr funding in 2026) |
| Hourly trend as a taker | **NOT PROVEN** (DSR 0.01) · as a maker: PLAUSIBLE, fills not modelled |
| Hourly mean reversion | **DEAD after 2018 / costs** (0% of variants positive after taker fees) |
| FVG / ICT, Fibonacci, round numbers, PO3-Goldbach levels | **INDISTINGUISHABLE from placebo** |
| Lunar phase, Mercury retrograde, numerology | **NOT PROVEN** (0 of 8 survive multiple-testing correction) |
| Fat tails, volatility clustering (econophysics) | **REAL** — Hill α≈2.5 hourly; vol is forecastable (ρ=0.44), direction barely (ρ=0.10) → size by volatility |
| Prop-firm evaluations | A call option on your edge: EV −$212/attempt at zero edge, +$282 at Sharpe 1 (Apex 100K EOD, $399 list); 90% of funded accounts blown within a year at Sharpe 1 |

Nothing here is investment advice. The point of the repository is that you can check.
