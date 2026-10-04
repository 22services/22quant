# q22 — automated multi-strategy trading for prop-firm accounts (Rust)

q22 is an automated trading engine written in Rust. It runs several published strategies, reads
the market regime every session, and wraps every order in a **compliance guard** built from the
prop firm's rules, so the account is neither blown nor banned. You supervise it in a **local web
dashboard** with pause, flatten and kill controls. The *same* engine runs the backtest, the
replay demo, paper trading and live trading, so what you test is what trades.

Everything is free and open source except the broker's own fees: the TopstepX API costs
$14.50/month with the Topstep code (otherwise $29). Bybit's API, the market data used here and all
libraries are free. **No Python anywhere**: Rust (tokio, axum, reqwest) plus a vanilla-JS
dashboard using TradingView Lightweight Charts.

> **The honest bottom line (16 years of real NQ/ES futures, details in [§7](#7-results--16-years-of-nq-and-es-futures-20102026))**
> * **NQ/ES divergence (SMT/SSMT) as a reversal signal has no edge.**
>   * Pre-registered tests: 8 SMT/SSMT variants plus spread reversion/momentum, lead–lag and
>     gap divergence, on 2,178 in-sample sessions. Every one failed.
>   * After an SMT event, price drifts by **0.00 ATR** at 30, 60 and 120 minutes, the same as
>     random entries.
> * **Divergence is useful as a *filter*.**
>   * A momentum breakout the other index *does not confirm* is a much worse trade, in all four
>     tests.
>   * The shipped bots only take **breakouts confirmed by both NQ and ES**.
> * **The combined bot** (MNQ and MES intraday momentum, each confirmed by the other index) was
>   frozen in-sample, then tested once on 2019-01 → 2026-07 with real micro costs:
>
>   | OOS result | Value |
>   |---|---|
>   | Net | **+$11.0k**, PF 1.14, Sharpe 0.51 |
>   | Max drawdown | −$2.4k |
>   | Worst day | −$461 |
>   | Positive years | 5 of 8 |
>   | Topstep/Lucid 50K pass rate (rolling) | **≈45%** (CI 22–70%) |
>   | Median time to pass | ~6 months |
>
>   **Label: PLAUSIBLE, not proven.** The deflated Sharpe after 30 trials is 0.24, and 2025–2026
>   YTD is flat to negative.
> * **Lucid works through Rithmic** (after Rithmic's one-time conformance test). Tradovate's API
>   is closed to prop accounts. LucidFlex (one-time fee) makes each attempt about 3× cheaper
>   than Topstep's monthly billing.
> * Nothing here makes money *quickly*. Run `signals` first, then `assist`, before `auto`.

---

## 1. Two-minute demo (no account, no key)

```bash
curl https://sh.rustup.rs -sSf | sh          # once: install Rust (stable ≥ 1.85)
cd q22
cargo run --release -p q22-app -- run -c config/demo_replay.toml
# → open the http://127.0.0.1:8722/?token=… URL printed in the terminal
```

The demo replays real NQ futures minutes (Apr→Sep 2026, bundled) through the full engine under
Topstep 50K rules at 50 bars per second, with simulated fills. You'll see the regime, every
signal, every block and its reason, fills, the account buffer and the equity curve.

## 2. Commands

| Command | What it does |
|---|---|
| `q22 run -c config/….toml` | Engine + dashboard. Paper, replay or live depending on `[runtime]`. |
| `q22 check -c config/….toml` | Validates a config and prints the compliance checklist for its firm. |
| `q22 rules [name]` | Lists the prop-firm presets (or prints one in full). |
| `q22 backtest -c … --data SYM=file.csv[@TZ]` | Backtest with a prop-evaluation replay. JSON report in `reports/`. |
| `q22 compare -c … --data …` | Ablation: all strategies with and without regime gating/adaptive health, then each one alone, with Deflated Sharpe. |
| `q22 passrate -c … --data …` | Starts a fresh evaluation every N sessions: probability of passing, time to pass or fail, CI. |
| `q22 serve --reports reports` | Dashboard for reports only (no engine). |
| `q22 fetch-data --out data` | Downloads the free research datasets (index 5-min 2020-23, BTC hourly). |
| `q22 databento --input … --root NQ --out … [--adjust panama\|ratio]` | Databento all-contract OHLCV-1m → continuous front month: volume roll without look-ahead, Panama or ratio back-adjustment, plus a roll log. |
| `q22 study [--cost contract\|bps]` | The pre-registered NQ/ES event studies (SMT/SSMT, relative value, lead–lag, gaps), with placebo and null groups. **In-sample only** unless `--force-oos`. |
| `q22 broker-check -c …` | Connects to the configured broker (ProjectX, Rithmic, Tradovate, Bybit) and prints the account, positions and recent bars. **Read-only.** |

All commands are run as `cargo run --release -p q22-app -- <command> …`, or use
`target/release/q22` after one build. Reproduce every number in this README:

```bash
q22 fetch-data --out data
q22 compare  -c config/research_ablation.toml --data MNQ=data/USATECHIDXUSD_M5.csv --label ndx5m
q22 compare  -c config/research_ablation.toml --data MES=data/USA500IDXUSD_M5.csv  --label spx5m
q22 compare  -c config/research_ablation.toml --data MNQ=data/NQ_1m_sample_2026.csv --label nq2026
q22 passrate -c config/backtest_futures.toml --data MNQ=data/USATECHIDXUSD_M5.csv --label passrate_topstep50k_mnq_ndx2020_23
q22 passrate -c config/backtest_es.toml      --data MES=data/USA500IDXUSD_M5.csv  --label passrate_topstep50k_mes_spx2020_23
q22 backtest -c config/hyrotrader_btc.toml   --data BTCUSDT=data/btc_hourly.csv --from 2014-01-01 --label btc_hyrotrader_daily_trend
q22 passrate -c config/hyrotrader_btc.toml   --data BTCUSDT=data/btc_hourly.csv --from 2014-01-01 --warmup 200 --every 10 --label passrate_hyrotrader_btc_2014_26
```

## 3. Which prop firms can run it without a ban

Automation rules differ by firm, and some differ between the evaluation and the funded account.
q22 encodes the firm's stance in each preset (`automation = Full | EvaluationOnly | AssistOnly`).
It **refuses to start live `auto` mode on a firm that does not allow it**.

| Firm | Bots allowed? | How q22 connects | Status in q22 |
|---|---|---|---|
| **Topstep** (Combine → Express Funded) | **Yes, through the TopstepX API**, which must be actively monitored. **No VPS, VPN or remote servers**: run it on your own computer. HFT, micro-scalping and max size into tier-1 news are banned. | ProjectX Gateway REST (`api.topstepx.com`), API key from your Topstep dashboard | **Implemented** (`config/topstep_50k_live.toml`) |
| **HyroTrader** (crypto, Bybit) | Marketed as allowed, **but** the terms ban bots "except where expressly permitted". **Get written confirmation from support**, or use `assist` mode. The challenge trades a Bybit **demo** sub-account. | Bybit v5 REST (`api-demo.bybit.com` for the challenge, `api.bybit.com` when funded) | **Implemented** (`config/hyrotrader_btc.toml`) |
| **Lucid Trading** (LucidFlex / LucidPro) | **Yes**, in evaluation and funded accounts; HFT is the only automation ban. Flat by 4:45 PM ET. | **Rithmic R \| Protocol** (WebSocket + protobuf, via `rithmic-rs`). It needs Rithmic's one-time **conformance test**, which gives you the app-name prefix and URL. | **Implemented** (`config/nq_es_lucidflex_50k.toml`, presets `lucidflex_50k` and `lucidpro_50k`) |
| Tradovate-hosted prop accounts (Lucid, Apex, Tradeify…) | Tradovate itself **does not give API access to prop or evaluation accounts**. A personal API key cannot reach them. | Only authorised partner bridges (TradingView webhooks via PickMyTrade or TradersPost) | Not supported. Use Rithmic for Lucid. |
| Your own Tradovate account | Yes: funded account of at least $1,000 plus the $25/month API add-on (CME data for the API is extra) | Tradovate REST + market-data WebSocket | **Implemented** (`broker = "tradovate"`) |
| **Apex Trader Funding** | Evaluation yes; **fully automated trading prohibited on PA/Live** | — | Preset `apex_100k_eod` → use `mode = "assist"` (you click every entry) |

If you trade your own Bybit account, use the `personal` preset: a 25% kill-switch drawdown and a
3% daily stop are the only rules.

## 4. Architecture

```
 bars (CSV replay | TopstepX | Rithmic | Tradovate | Bybit | Bybit public)
   │   all instruments of one timestamp are ingested together, then each decides
   ▼
 MarketState ── daily layer: ATR, ADX, efficiency ratio, vol percentile → Regime
   │            session layer: VWAP, noise profile σ(time-of-day), opening range, gap
   ▼
 Strategies (noise_breakout, orb, last_half_hour, vwap_reversion, daily_trend)
   │  each proposes: side, stop, target, confidence, reason — and may read its peers
   │  (e.g. noise_breakout's NQ ↔ ES confirmation filter)
   ▼
 Allocator  score = regime affinity × health × confidence; one net position per instrument
   ▼
 Sizing     risk $ (or % equity) ÷ stop distance, capped by 15% of the buffer and the firm's max contracts
   ▼
 ComplianceGuard ── refuses or flattens with a written reason (see §6)
   ▼
 Commands → SimBroker (backtest/paper)  or  Broker (ProjectX / Rithmic / Tradovate / Bybit) → fills → PropTracker
   ▼
 Snapshot → dashboard (SSE stream) + journal (state/trades.jsonl)
```

| Crate | Contents |
|---|---|
| `q22-core` | Bars, instruments (NQ/MNQ/ES/MES/YM/MYM/RTY/M2K/MBT/BTC/ETH/SOL), CME/crypto sessions, CME front-month calendar, CSV loaders (auto-detected formats and time zones), the Databento continuous-series builder, indicators, statistics (PSR, DSR, MinTRL). |
| `q22-engine` | Regime classifier, strategies (cross-asset aware), allocator, prop rules and tracker, compliance guard, engine, backtester and pass-rate study, dashboard snapshot. |
| `q22-broker` | `Broker` trait. **ProjectX** (TopstepX) with rate limiting and token refresh. **Rithmic** R \| Protocol (Lucid) via `rithmic-rs`, with exchange-side brackets. **Tradovate** REST + market-data WebSocket. **Bybit** v5 with HMAC signing and exchange-side stops. |
| `q22-research` | Pre-registered event studies on aligned NQ/ES minutes: trade simulation with costs, time-matched placebo, null groups, forward-drift diagnostics. |
| `q22-app` | `q22` CLI, live runner (timestamp synchronisation across instruments, reconciliation, unmanaged-position handling, CRITICAL flatten on errors), axum dashboard, embedded UI. |

About 10,900 lines of Rust and JS. 48 unit tests (`cargo test --workspace`) cover:
* the HMAC test vectors and the Rithmic/Tradovate message handling;
* the Databento roll and back-adjustment, the CME roll calendar and sessions;
* indicators and DSR;
* prop rules, guard rules and the allocator;
* the fill model (stop before target inside one bar), the research simulator, end-to-end
  backtests and the pass-rate study.

## 5. Strategies and market conditions

Every strategy is a published idea with **fixed, un-optimised defaults**. Each declares which
regimes it suits.

| id | Idea (source) | Built for | Shipped |
|---|---|---|---|
| `noise_breakout` | Intraday momentum out of a time-of-day "noise area" (bands from the last 14 sessions' σ at each minute). Checks every 30 min from 10:00 ET; trailing stop at max(band, VWAP). [Zarattini, Aziz & Barbon 2024] | trend, volatile | **on** |
| `orb` | 15-min opening-range breakout, close-confirmed, 2R target, break-even at 1R [Crabel 1990; Zarattini & Aziz 2023] | trend, volatile | off (weak) |
| `last_half_hour` | Overnight + first half-hour return predicts the last half-hour [Gao, Han, Li & Zhou 2018] | all but shock | off (lost money) |
| `vwap_reversion` | Fade 2σ VWAP extensions on non-trending days | ranging, neutral | off (no edge after costs) |
| `daily_trend` | BTC/ETH time-series momentum ensemble (20/60/120-day returns, EWMA crossovers, 55-day breakout), Chandelier stop, vol targeting [Moskowitz, Ooi & Pedersen 2012; q22 research] | any, 24/7 | on for crypto |

`noise_breakout` has two options:
* `exit_mode`: `resting` is a trailing stop at max(band, VWAP); `checks` is the paper's
  30-minute exits with a protective stop at `protect_atr` × daily ATR.
* `peer_filter`: `confirm` trades a breakout only if the other index is also outside its own
  noise area on the same side; `diverge` trades only unconfirmed breakouts and exists for
  measurement.

The shipped NQ/ES portfolio uses `confirm` on both bots (§7.3).

**Regime** is classified once per session from completed daily bars, with no look-ahead:
Shock → Trending up/down → Volatile → Ranging → Neutral, first match wins. The inputs are
ADX(14), the 10-day efficiency ratio, the 252-day ATR percentile and the opening gap. The
thresholds are round numbers fixed in advance. Every regime and every decision is logged and
shown in the dashboard.

**Why gating ships off:** the ablation (§7) shows that gating strategies by daily regime lowered
Sharpe every time. Intraday momentum already selects trend days by itself: it only triggers when
price leaves the noise area. A daily filter mostly removes the best trend days that start out of
a quiet regime. To re-enable it, set `[allocator] regime_gating = true` and
`adaptive_health = true`.

## 6. The compliance guard

These rules keep the account alive and inside the firm's terms. They are all in
`crates/q22-engine/src/guard.rs`, and each refusal is logged with its reason. The defaults are
stricter than the firms'.

* **Session discipline (CME):** new entries 09:35–15:45 ET only. Everything is flattened by
  15:58 ET, or by the firm's own cut-off if earlier (Topstep: 15:10 CT).
* **Personal daily loss stop:** 30% of the max loss (≤ 80% of any firm daily loss limit). It
  halts the day and flattens.
* **Buffer floor:** no new risk when the buffer to the max-loss threshold is below 25% of the max
  loss. Per-trade risk is capped at 15% of the remaining buffer, so size shrinks automatically
  in a drawdown.
* **Consistency-aware profit lock:** stops for the day at 80% of the largest day the firm's
  consistency rule allows without raising the target. For Topstep that is $1,200 on day one; it
  widens as profit accumulates. Locking helped in testing (§7).
* **Behaviour limits:** max trades per day, max consecutive losses, a 120 s cool-down between
  entries, an order-rate limiter (anti-HFT), a 30 s minimum hold (anti micro-scalping), news
  blackout windows, and no opposite positions in correlated instruments (no hedging).
* **Operator heartbeat:** in live mode, no new entries unless the dashboard has been visible
  within the last N seconds (Topstep requires automation to be actively monitored).
* **Exchange-side protection:** every entry gets a resting stop (and a target when the strategy
  has one). On Bybit, stop-loss and take-profit are attached to the order itself.
  Reconciliation cancels orphan orders. An unexpected position raises an alert (or is
  flattened), and an order error triggers a CRITICAL flatten.
* **Contract limits:** the firm's maximum contracts (a micro counts as 0.1 mini) and an optional
  personal cap.

Firm rules modelled in `prop.rs`:
* drawdown: static, intraday trailing, or end-of-day trailing with a lock at the starting balance;
* daily loss: from the day's start or from the day's high (HyroTrader); it either halts the day
  or fails the account;
* consistency: share of the target, or share of total profit (a big day raises the target);
* profit target, minimum trading days, overnight permission, flat-by time, max risk per
  position, max leverage.

## 7. Results — 16 years of NQ and ES futures (2010–2026)

### 7.1 Data and method

* **Data:** Databento GLBX `ohlcv-1m`, every ES and NQ contract, 2010-06-06 → 2026-07-09.
  `q22 databento` turns it into a continuous front-month series:
  * Databento's one-digit years are resolved (ESM0 means 2010 *or* 2020);
  * the series rolls when the next contract out-traded the front on the previous session;
  * it produces 65 rolls and 4,144 sessions.
* **Back-adjustment:**
  * Panama (exact point P&L) for the registered tests;
  * ratio (exact percentages) for the cost model below.
* **Pre-registration:** [`research/PREREGISTRATION.md`](research/PREREGISTRATION.md) fixed
  everything before any result was computed, and was committed to git:
  * the hypotheses and their parameters;
  * the fill model;
  * the in-sample period, **2010-06 → 2018-12**;
  * the out-of-sample period, **2019-01 → 2026-07**, touched once by a frozen portfolio;
  * the pass criteria.

  Every later change is in its deviation log. All **30 trials** count toward the Deflated Sharpe.
* **Two cost models, both always reported:**
  * the registered one: $0.75 commission plus 1 tick per side per micro;
  * a **today-calibrated basis-point model** (MNQ 0.25 bp, MES 0.615 bp per side). It shows
    whether a rule would pay at 2026 costs, because a micro's fixed cost was 3–10× larger
    relative to price when NQ traded at 2,000.

### 7.2 Divergence as a reversal signal: rejected

In-sample results, 2,178 sessions (files: `results/study_is.txt` and `results/study_is_bps.txt`):

| Hypothesis | Trades | Net R per trade (registered costs / bps) | Gross R | vs placebo t | Verdict |
|---|---|---|---|---|---|
| A1–A4: SMT at previous-session, overnight, opening-range and 1-min fractal extremes (8 variants) | 977–2,692 | −0.33…−0.52 / −0.13…−0.23 | −0.08…−0.02 | −1.3…+1.4 | **fail** |
| B1/B2: intraday NQ–ES spread z ≥ 2, fade or follow (4) | 1,158 | about −0.12 / −0.03 | about 0 | ≤ 0.6 | **fail** |
| C1: lead–lag at 5 minutes | 254 | −0.34 / −0.10 | 0.00 | −0.5 | **fail** |
| D2: fade breakouts the other index does not confirm | 1,454 | −0.08 / +0.01 | +0.04 | 1.1 | **fail** |
| E1: opposite-sign opening gaps | 21 | | | | too rare |
| A1n: *both* indices sweep the previous-session extreme, then fail (snooped from a null group) | 198 | +0.12 (bps) | +0.17 | 1.6 | **fail** (t 1.2) |

The forward drift after SMT events is under **0.01 daily ATR** at +30, +60 and +120 minutes.
SMT carries no directional information on NQ/ES 1-minute data.

### 7.3 Divergence as a filter: the useful part

The same momentum rule (noise-area breakout) was split by whether the other index confirms the
breakout (in-sample, bps costs):

| Variant | Confirmed: Sharpe | Divergent: Sharpe |
|---|---|---|
| MNQ, resting trail | 1.17 | 0.51 |
| MNQ, 30-min exits | 0.91 | 0.03 |
| MES, resting trail | 0.51 | −0.32 |
| MES, 30-min exits | 0.68 | −0.17 |

**Unconfirmed breakouts are much worse every time.** The shipped bots therefore trade only
breakouts confirmed by both NQ and ES. The dashboard's "NQ ↔ ES confirmation" panel shows the
state live.

### 7.4 The frozen portfolio

Two bots, frozen before the OOS run:
* **nb_mnq:** MNQ momentum, resting trail, confirmed by MES;
* **nb_mes:** MES momentum, the paper's 30-minute exits, confirmed by MNQ.

Both risk $250 per trade, inside the guard's 15%-of-buffer cap.

| Period | Costs | Net | Sharpe | Max DD | Worst day | Years + | Topstep 50K pass (CI) | Median sessions to pass |
|---|---|---|---|---|---|---|---|---|
| IS 2010–18 | bps | +$41.9k | 1.16 | −$4.6k | −$557 | | 52% (36–68%) | 54 |
| **OOS 2019–26** | **micro, registered** | **+$11.0k** | **0.51** | −$2.4k | −$461 | **5/8** | **45% (22–70%)** | 125 |
| OOS 2019–26 | bps | +$14.6k | 0.75 | −$2.7k | −$427 | 5/8 | 75% (48–91%) | 133 |

OOS pass rates under Lucid's rules (micro costs / bps):
* **LucidFlex 50K:** 45% / 75%;
* **LucidPro 50K:** 42% / 71%.

**What to take from this:**
* **It wins, modestly.** The sum of the bots is positive out-of-sample under both cost
  models, and the worst day is well inside every firm's daily limit.
* **It is not proven.**
  * The deflated Sharpe after 30 trials is 0.24 with micro costs and 0.51 with bps costs.
  * About half of the OOS profit comes from 2022.
  * 2025 and 2026 YTD are flat to negative.
* **The drawdown misses my own pre-registered "prop-friendly" bar.** The 7.5-year max DD
  ($2.4k at $250 per trade) is above the $1k target. Meeting it needs about $100 risk per
  trade, at which most signals cannot be sized.
* **Two known weaknesses, left unfixed so the OOS stays clean:**
  * The MES bot cannot size a trade once ES is above about 7,000 (its 0.5-ATR stop is more than
    the $300 cap), so it did not trade in 2026.
  * After a drawdown, the shrinking 15%-of-buffer cap can make even MNQ trades unsizable
    (visible in the replay), which freezes the account until it recovers.
* **The bot was evaluated as one account.** The bots share the guard (cool-down,
  anti-hedge), exactly as they would live.

### 7.5 Cost of one funded account (OOS, micro costs)

Assumes a 45% pass rate and about 6 months per evaluation:

| Firm | Fee model | Expected fees per funded account |
|---|---|---|
| Topstep 50K | $49/month + $149 activation | ≈ **$800** (+ API $14.50/month) |
| LucidFlex 50K | one-time ~$136 | ≈ **$300** (Rithmic data/API fees per your plan) |

Fees change often; check them before you buy.

Reproduce:
```bash
q22 databento --input "../NQ DATA/glbx-mdp3-20100606-20260709.ohlcv-1m.csv" --root NQ --out data/databento/NQ_c1_1m.csv
q22 databento --input "../ES DATA/glbx-mdp3-20100606-20260709.ohlcv-1m.csv" --root ES --out data/databento/ES_c1_1m.csv
#   (repeat both with --adjust ratio --out data/databento/{NQ,ES}_c1_1m_ratio.csv for the bps model)
q22 study                     # registered costs, in-sample
q22 study --cost bps          # today-calibrated costs, in-sample
research/run_is_matrix.sh     # engine variants, in-sample
research/run_is_portfolios.sh # portfolio selection, in-sample
q22 backtest -c config/research/frozen_P3_contract.toml --data MNQ=data/databento/NQ_c1_1m.csv \
  --data MES=data/databento/ES_c1_1m.csv --from 2019-01-02 --to 2026-07-09 --trials 30 --label oos_P3_contract
q22 passrate -c config/research/frozen_P3_contract.toml --data MNQ=data/databento/NQ_c1_1m.csv \
  --data MES=data/databento/ES_c1_1m.csv --from 2019-01-02 --to 2026-07-09
```

## 8. Earlier results on free data (2020–23 index CFDs, BTC)

The data is public and free (see [`data/README.md`](data/README.md)). Costs are always
included: MNQ/MES commission per side plus 1 tick of slippage on every market fill. Fills happen
at the next bar's open. A stop is assumed to fill first when stop and target are both inside one
bar, and a target needs a one-tick trade-through.

### 8.1 Ablation — 11 variants, Deflated Sharpe corrected for 11 trials

The design was pre-registered in `config/research_ablation.toml`: four intraday strategies,
Topstep 50K rules, $250 risk budget, 5-minute bars. Full tables are in
[`results/`](results).

| Variant | NDX 2020-09→2023-09 Sharpe (net $) | S&P 500 same period | NQ 2026-04→09 |
|---|---|---|---|
| All four · gating + adaptive health (the original design) | 0.21 (+$1.5k) | 0.18 (+$1.5k) | −0.90 |
| All four · no gating | 0.77 (+$8.2k) | 0.11 (+$1.2k) | −1.22 |
| **noise_breakout alone · no gating** | **1.26** (+$13.9k, 754 trades, PF 1.30, DSR 0.75) | **1.02** (+$11.5k, 797 trades, PF 1.22, DSR 0.57) | **−1.53** (−$1.1k, 49 trades) |
| noise_breakout alone · gated | 0.92 | 0.43 | −0.95 |
| orb alone | 0.65 | 0.14 | −0.39 (4 trades) |
| last_half_hour alone | −1.50 | −2.22 | +0.93 (13 trades) |
| vwap_reversion alone | 0.10 | −1.13 | +2.52 (15 trades) |

How to read it:
* The noise breakout is the only strategy positive on both 2020-23 indices after costs and
  deflation.
* The S&P 500 period overlaps the paper's own sample (2007–2024), so it is not out-of-sample for
  the idea. Nasdaq-100 is a different index, which makes it a partial replication.
* The 2026 sample is about 110 sessions. Over that span the annualised Sharpe has a standard
  error of about 1.5, so −1.53 sits about 1.9 standard errors below the 2020-23 estimate. That
  is a real warning sign, not proof that the edge has decayed. **Treat the first live months
  as the real test**, starting in `signals` or `assist` mode.
* `last_half_hour` failed on both indices, even though the Gao et al. effect was published.
  Intraday edges decay once they are published (McLean & Pontiff 2016).

### 8.2 Probability of passing — rolling-start evaluations (`q22 passrate`)

A sequential replay only contains 5–12 evaluations in three years. So `q22 passrate` starts a
fresh Topstep 50K Combine **every 5 sessions** and lets the engine trade it until it passes or
fails. Neighbouring runs overlap, so the 95% interval uses the number of *independent* attempts.

| Shipped config | Pass rate (95% CI) | Sessions to pass: median (IQR) | Expected time to a funded account | Expected fees per funded account* |
|---|---|---|---|---|
| MNQ, $250 risk, NDX 2020-23 | **59%** (30–84%, ≈9 independent) | 61 (43–116) ≈ 3 months | ≈ 6.4 months | ≈ $490 (+ API ≈ $590) |
| MES, $250 risk, S&P 2020-23 | **50%** (20–79%, ≈7 independent) | 88 (62–150) ≈ 4 months | ≈ 9.5 months | ≈ $640 (+ API ≈ $780) |

\* Topstep Standard path: $49 per month of Combine plus a $149 activation when you pass. The API
adds $14.50 per month. These figures include the failed attempts (≈ 1/p evaluations per
funded account).

A "failure" here includes q22's own **guard stop**: once less than 25% of the max loss remains,
the guard stops trading, and the study books the evaluation as failed. A human might keep trading
it. Topstep would not yet close it.

**Risk per trade** ([`results/passrate_risk_sweep.txt`](results/passrate_risk_sweep.txt)):

| Risk per trade | Effect |
|---|---|
| ≤ $150 | Starves the strategy: the stops are too wide to size even one micro, so evaluations drag on for 6–17 months. |
| $250–300 | About the same pass rate as $200, about 30 sessions faster. |
| above $300 | Changes nothing: the guard's 15%-of-buffer cap binds. |

**The consistency lock helps.** With the lock: 59% on NDX and 50% on S&P. Without it: 58% and
41%.

### 8.3 Crypto: BTC daily trend on HyroTrader rules

These numbers are for 2014-01→2026-10, hourly bars, HyroTrader 2-step phase 1 ($10k, +10%
target, 10% max loss, 5% daily from the day's high), 1.5% risk per trade with an exchange-side
stop.

| Metric | Result |
|---|---|
| Trades / win rate / PF | 193 / 37% / 1.90 (+0.36R average) |
| Sharpe (daily) | 0.68 |
| Max drawdown | −$550 |
| Passed phase 1 | 99% of decided rolling starts |
| Calendar days to pass (median) | **537 (≈18 months)** |
| Rolling starts still unfinished at the data end | 158 of 446 |

It almost never fails, but it is slow. The $159 fee is refundable after the first payout and
there is no time limit, so it can work as a patient side position. It is not a fast path.

The guard caps risk at 15% of the buffer, which is $150 (1.5%). Raising the risk setting further
changed nothing.

### 8.4 What is *not* modelled

* The funded stage: Topstep Express Funded payout rules (changed in April 2026), scaling plans,
  HyroTrader's funded-stage limits (25% margin cap, 2× notional).
* Queue position and partial fills.
* Real CME futures microstructure. The 2020-23 files are index CFD prices with tick volume, a
  close proxy for MNQ/MES but not the futures themselves. The bundled 2026 file *is* NQ futures.
* News days, unless you list the events (`news_events`).

## 9. Going live (Topstep, Lucid)

The NQ/ES portfolio ships as `config/nq_es_topstep_50k.toml` and `config/nq_es_lucidflex_50k.toml`
(both start in `mode = "signals"`). `config/nq_es_replay.toml` replays it on your Databento
files with no account.

**Topstep:**

1. Run the demo. Read the decision log until every block reason makes sense to you.
2. Buy a 50K Combine and subscribe to the TopstepX API (code `topstep`: $14.50/month). Create an
   API key.
3. Set your credentials (environment variables only, never in config files):
   `export Q22_PROJECTX_USER=… Q22_PROJECTX_KEY=…`
4. Check the config: `q22 check -c config/nq_es_topstep_50k.toml`, then
   `q22 broker-check -c config/nq_es_topstep_50k.toml` (read-only). Add the week's tier-1 news
   times (UTC) to `news_events`.
5. Run it **on your own computer** (no VPS, VPN or cloud). Start with `mode = "signals"` for a
   week, then `assist`, then `auto`.
6. Keep the dashboard tab visible. New entries pause if it has been hidden for 3 minutes.
7. When you restart mid-evaluation, copy the balance and the Maximum Loss Limit level from
   Topstep into `resume_balance` and `resume_threshold`.
8. Kill switch: the dashboard button or Ctrl-C. Both flatten everything when
   `flatten_on_exit = true`.

**Lucid (through Rithmic):**

1. **Conformance (one time).**
   * Ask Rithmic for the R | Protocol API dev kit and pass their conformance test, run on their
     test system.
   * Rithmic then gives you the **app-name prefix** and the production/paper **WebSocket URL**.
   * Without it, Rithmic refuses the login. That is Rithmic's rule, not q22's.
2. **Credentials** (environment only):
   ```
   export Q22_RITHMIC_URL=wss://…  Q22_RITHMIC_SYSTEM="<the system name shown in R|Trader Pro for your Lucid login>"
   export Q22_RITHMIC_USER=…  Q22_RITHMIC_PASSWORD=…  Q22_RITHMIC_APP_NAME=<prefix>:q22
   ```
   Optional: `Q22_RITHMIC_ACCOUNT`, if your login has several accounts.
3. **Read-only check.** `q22 broker-check -c config/nq_es_lucidflex_50k.toml` should list your
   account and front-month bars for MNQ and MES.
4. **Run.** `q22 run -c config/nq_es_lucidflex_50k.toml`.
   * Use `signals` for a week, then `assist`, then `auto`.
   * Every entry is an exchange-side market bracket with the CME automated-order flag.
   * Positions are flat by 15:58 ET, earlier than Lucid's 4:45 PM cut-off.

The Rithmic and Tradovate adapters are tested against message-level unit tests and their
library's documented API. **They have not been run against a live Rithmic or Tradovate server
from this environment**, because no credentials are available here. Do the read-only check, then
paper-trade, before risking an evaluation.

**HyroTrader:** use `config/hyrotrader_btc.toml` and switch to `broker = "bybit"`. Set
`bybit_base = "https://api-demo.bybit.com"` for the challenge, export `Q22_BYBIT_KEY` and
`Q22_BYBIT_SECRET` (a trade-only key with no withdrawal permission), and get HyroTrader's written
OK for bots first.

## 10. Dashboard and security

* **Live tab:** KPIs (balance, buffer meter, today's P&L against the loss stop and the profit
  lock, trades), candles with VWAP, entry and exit markers and the stop and target lines,
  position, regime and its features, the strategy table (fit × health → allocation), daily
  equity, the closed trades, and a filterable decision log. With two index instruments it also
  shows the **NQ ↔ ES confirmation panel**: each index's % move since the open, where it sits
  against its noise area, and whether a breakout is confirmed, divergent (skipped) or absent.
  All times are New York time.
* **Backtests tab:** any report in `reports/`: P&L curve, breakdowns by strategy, regime and exit
  reason, prop attempts, trades, and pass-rate studies.
* **Rules tab:** the compliance checklist, firm rules, guard limits and sources.
* **Security:** the dashboard binds to `127.0.0.1` by default. Every API call needs the session
  token printed at start-up (or set `Q22_DASH_TOKEN`). Controls ask for confirmation. There are
  no third-party scripts at runtime: the chart library is vendored, Apache-2.0 with the
  attribution shown.

## 11. Sources

**Strategies and statistics**
* Zarattini, Aziz & Barbon (2024), *Beat the Market: An Effective Intraday Momentum Strategy for
  S&P500 ETF (SPY)*, SSRN 4824172 — https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4824172
* Gao, Han, Li & Zhou (2018), *Market Intraday Momentum*, Journal of Financial Economics —
  https://www.sciencedirect.com/science/article/abs/pii/S0304405X18301351
* Moskowitz, Ooi & Pedersen (2012), *Time Series Momentum*, JFE —
  https://doi.org/10.1016/j.jfineco.2011.11.003
* Bailey & López de Prado (2014), *The Deflated Sharpe Ratio* —
  https://sdm.lbl.gov/oapapers/ssrn-id2507040-bailey.pdf
* McLean & Pontiff (2016), *Does Academic Research Destroy Stock Return Predictability?* —
  https://doi.org/10.1111/jofi.12365

**Prop-firm rules (checked October 2026; verify before you buy — firms change rules often)**
* Topstep:
  * TopstepX API access — https://help.topstep.com/en/articles/11187768-topstepx-api-access
  * Pricing — https://help.topstep.com/en/articles/14289835-topstep-pricing-and-payment-questions
  * Pricing and activation fee — https://www.fundedfuturesfamily.com/topstep-activation-fee/
  * Consistency rule — https://proptradingvibes.com/blog/topstep-consistency-rule
  * Rule changes and the DLL removal on TopstepX — https://tradecovex.com/guides/topstep-rule-changes-2026
  * VPN/VPS policy — https://proptradingvibes.com/blog/topstep-vpn-policy
  * Automation — https://blog.pickmytrade.io/topstepx-automation-2026/
  * Rules overview — https://www.futureshive.com/blog/topstep-rules-explained
* ProjectX Gateway API —
  https://gateway.docs.projectx.com/docs/api-reference/order/order-place/ ·
  https://gateway.docs.projectx.com/docs/api-reference/market-data/retrieve-bars/
* HyroTrader:
  * Trading rules — https://www.hyrotrader.com/trading-rules/
  * Max loss per trade — https://www.hyrotrader.com/faq/rules/what-is-the-maximum-loss-per-trade-rule/
  * Bybit API setup — https://www.hyrotrader.com/faq/bybit-platform/how-to-correctly-set-up-bybit-api-and-why-am-i-getting-an-error-when-connecting-the-api/
  * Reviews — https://cryptoslate.com/prop-firms/hyrotrader-review/ ·
    https://thetrustedprop.com/blogs/hyrotrader-review-2026-rules-fees-payouts
  * Rules overview — https://www.proptradingvibes.com/blog/hyrotrader-rules-overview
* Bybit v5 API — https://bybit-exchange.github.io/docs/v5/position/trading-stop
* Lucid Trading:
  * Rules and automation — https://tradetanto.com/learn/lucid-trading-rules-explained-every-plan-rule-and-limit ·
    https://velotrade.com/blog/lucid-trading-review
  * 50K plans — https://proptradingvibes.com/blog/lucid-trading-50k-account-rules ·
    https://damnpropfirms.com/prop-firms/lucid-trading-rules-payouts/
  * Platforms — https://proptradingvibes.com/blog/lucid-trading-platforms
* Tradovate API access (prop and evaluation accounts not eligible; $25/month add-on) —
  https://support.tradovate.com/s/article/Tradovate-API-Access?language=en_US ·
  https://partner.tradovate.com/api/rest-api-endpoints/authentication/access-token-request
* Rithmic:
  * R | Protocol API — https://www.rithmic.com/apis
  * Conformance requirement — https://blog.pickmytrade.io/rithmic-access-denied-api-access-level-error-fix/
  * `rithmic-rs` Rust client (MIT/Apache-2.0) — https://crates.io/crates/rithmic-rs
* Apex PA automation ban —
  https://support.apextraderfunding.com/hc/en-us/articles/31519788944411-Performance-Account-PA-and-Compliance

**Data**
* Databento GLBX.MDP3 OHLCV-1m (ES and NQ, 2010–2026; your licensed copy, not redistributed) —
  https://databento.com/datasets/GLBX.MDP3
* NQ 1-minute 2026 sample (MIT) — https://github.com/getdata-finance/nq-1m-ohlcv-stocks-historical-data
* Index 5-minute 2020-23 (MIT) — https://github.com/TheSnowGuru/Stocks-Futures-Financial-Time-series-Tick-Bar-Data
* BTC hourly — https://github.com/mouadja02/bitcoin-technical-indicators-dataset

---

*Research software, not investment advice. Prop-firm evaluations are paid products, and most
traders fail them. Never risk money you cannot afford to lose, and re-read your firm's current
rules before turning on `auto`.*
