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

> **The honest bottom line (details in [Results](#results))**
> * One strategy survived the testing: **intraday momentum with a noise area**
>   (Zarattini–Aziz–Barbon 2024). Net of costs on 2020-09→2023-09 5-minute data it made Sharpe
>   **1.26** on Nasdaq-100 and **1.02** on S&P 500. Deflated for the 11 variants tried, that is
>   a DSR of 0.75 and 0.57: probably real, not proven.
> * The other three intraday ideas lost money or added nothing after costs. **Regime gating
>   lowered Sharpe in every variant**, so it ships disabled. You can switch it on; it is
>   measured, not assumed.
> * Under Topstep 50K rules, rolling-start replays pass **≈59% of evaluations on MNQ**
>   (95% CI 30–84%). A pass takes a median **61 sessions (≈3 months)**. Expect about **6 months
>   and $500–600 in fees** to reach one funded account. The **2026 NQ sample (110 sessions) lost
>   money** (Sharpe −1.5, 49 trades): too short to judge, but it is the most recent data.
> * Nothing here makes money *quickly*. It is a disciplined way to buy a call option on a modest
>   edge, with fixed, known costs.

---

## 1. Two-minute demo (no account, no key)

```bash
curl https://sh.rustup.rs -sSf | sh          # once: install Rust (stable ≥ 1.82)
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
| **Lucid Trading** | Yes, in evaluation and funded accounts (HFT banned) | Rithmic / Tradovate | Not implemented (needs a Rithmic or Tradovate adapter) |
| **Apex Trader Funding** | Evaluation yes; **fully automated trading prohibited on PA/Live** | — | Preset `apex_100k_eod` → use `mode = "assist"` (you click every entry) |

If you trade your own Bybit account, use the `personal` preset: a 25% kill-switch drawdown and a
3% daily stop are the only rules.

## 4. Architecture

```
 bars (CSV replay | TopstepX | Bybit | Bybit public)
   │
   ▼
 MarketState ── daily layer: ATR, ADX, efficiency ratio, vol percentile → Regime
   │            session layer: VWAP, noise profile σ(time-of-day), opening range, gap
   ▼
 Strategies (noise_breakout, orb, last_half_hour, vwap_reversion, daily_trend)
   │  each proposes: side, stop, target, confidence, reason
   ▼
 Allocator  score = regime affinity × health × confidence; one net position per instrument
   ▼
 Sizing     risk $ (or % equity) ÷ stop distance, capped by 15% of the buffer and the firm's max contracts
   ▼
 ComplianceGuard ── refuses or flattens with a written reason (see §6)
   ▼
 Commands → SimBroker (backtest/paper)  or  Broker (ProjectX / Bybit) → fills → PropTracker
   ▼
 Snapshot → dashboard (SSE stream) + journal (state/trades.jsonl)
```

| Crate | Contents |
|---|---|
| `q22-core` | Bars, instruments (NQ/MNQ/ES/MES/YM/MYM/RTY/M2K/MBT/BTC/ETH/SOL), CME/crypto sessions, CSV loaders (auto-detected formats and time zones), indicators, statistics (PSR, DSR, MinTRL). |
| `q22-engine` | Regime classifier, strategies, allocator, prop rules and tracker, compliance guard, engine, backtester and pass-rate study, dashboard snapshot. |
| `q22-broker` | `Broker` trait. ProjectX (TopstepX / The Futures Desk) with rate limiting and token refresh. Bybit v5 with HMAC signing and exchange-side stops. |
| `q22-app` | `q22` CLI, live runner (reconciliation, unmanaged-position handling, CRITICAL flatten on errors), axum dashboard, embedded UI. |

About 7,900 lines of Rust and JS. 36 unit tests (`cargo test --workspace`) cover the HMAC test
vectors, sessions, indicators and DSR, prop rules, guard rules, the allocator, the fill model
(stop before target inside one bar), end-to-end backtests and the pass-rate study.

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

## 7. Results

The data is public and free (see [`data/README.md`](data/README.md)). Costs are always
included: MNQ/MES commission per side plus 1 tick of slippage on every market fill. Fills happen
at the next bar's open. A stop is assumed to fill first when stop and target are both inside one
bar, and a target needs a one-tick trade-through.

### 7.1 Ablation — 11 variants, Deflated Sharpe corrected for 11 trials

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

### 7.2 Probability of passing — rolling-start evaluations (`q22 passrate`)

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

### 7.3 Crypto: BTC daily trend on HyroTrader rules

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

### 7.4 What is *not* modelled

* The funded stage: Topstep Express Funded payout rules (changed in April 2026), scaling plans,
  HyroTrader's funded-stage limits (25% margin cap, 2× notional).
* Queue position and partial fills.
* Real CME futures microstructure. The 2020-23 files are index CFD prices with tick volume, a
  close proxy for MNQ/MES but not the futures themselves. The bundled 2026 file *is* NQ futures.
* News days, unless you list the events (`news_events`).

## 8. Going live on Topstep (checklist)

1. Run the demo. Read the decision log until every block reason makes sense to you.
2. Buy a 50K Combine and subscribe to the TopstepX API (code `topstep`: $14.50/month). Create an
   API key.
3. Set your credentials (environment variables only, never in config files):
   `export Q22_PROJECTX_USER=… Q22_PROJECTX_KEY=…`
4. Check the config: `q22 check -c config/topstep_50k_live.toml`. Add the week's tier-1 news
   times (UTC) to `news_events`.
5. Run it **on your own computer** (no VPS, VPN or cloud). Start with `mode = "signals"` for a
   week, then `assist`, then `auto`.
6. Keep the dashboard tab visible. New entries pause if it has been hidden for 3 minutes.
7. When you restart mid-evaluation, copy the balance and the Maximum Loss Limit level from
   Topstep into `resume_balance` and `resume_threshold`.
8. Kill switch: the dashboard button or Ctrl-C. Both flatten everything when
   `flatten_on_exit = true`.

For HyroTrader, use `config/hyrotrader_btc.toml` and switch to `broker = "bybit"`. Set
`bybit_base = "https://api-demo.bybit.com"` for the challenge, export `Q22_BYBIT_KEY` and
`Q22_BYBIT_SECRET` (a trade-only key with no withdrawal permission), and get HyroTrader's written
OK for bots first.

## 9. Dashboard and security

* **Live tab:** KPIs (balance, buffer meter, today's P&L against the loss stop and the profit
  lock, trades), candles with VWAP, entry and exit markers and the stop and target lines,
  position, regime and its features, the strategy table (fit × health → allocation), daily
  equity, the closed trades, and a filterable decision log.
* **Backtests tab:** any report in `reports/`: P&L curve, breakdowns by strategy, regime and exit
  reason, prop attempts, trades, and pass-rate studies.
* **Rules tab:** the compliance checklist, firm rules, guard limits and sources.
* **Security:** the dashboard binds to `127.0.0.1` by default. Every API call needs the session
  token printed at start-up (or set `Q22_DASH_TOKEN`). Controls ask for confirmation. There are
  no third-party scripts at runtime: the chart library is vendored, Apache-2.0 with the
  attribution shown.

## 10. Sources

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
* Lucid Trading automation — https://tradetanto.com/learn/lucid-trading-rules-explained-every-plan-rule-and-limit ·
  https://velotrade.com/blog/lucid-trading-review
* Apex PA automation ban —
  https://support.apextraderfunding.com/hc/en-us/articles/31519788944411-Performance-Account-PA-and-Compliance

**Data**
* NQ 1-minute 2026 sample (MIT) — https://github.com/getdata-finance/nq-1m-ohlcv-stocks-historical-data
* Index 5-minute 2020-23 (MIT) — https://github.com/TheSnowGuru/Stocks-Futures-Financial-Time-series-Tick-Bar-Data
* BTC hourly — https://github.com/mouadja02/bitcoin-technical-indicators-dataset

---

*Research software, not investment advice. Prop-firm evaluations are paid products, and most
traders fail them. Never risk money you cannot afford to lose, and re-read your firm's current
rules before turning on `auto`.*
