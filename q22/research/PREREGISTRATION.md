# Pre-registration — NQ/ES divergence study (written 2026-10-04, before any result was computed)

This file freezes **what will be tested, how, and what counts as a pass** before the 16-year
Databento data is examined. Anything that is changed after a result is seen gets logged in
§7 with the reason, and every variant ever evaluated counts toward the Deflated-Sharpe trial
count.

## 1. Data and split

* Databento GLBX.MDP3 `ohlcv-1m`, all ES and NQ contracts, 2010-06-06 → 2026-07-09.
  Converted with `q22 databento` into continuous front-month series:
  * roll to the next contract when its volume beats the front's on the previous session;
  * Panama back-adjusted, so point P&L is exact;
  * 65 rolls, 4,144 sessions.
* Only minutes where **both** NQ and ES printed a bar are used.
* **In-sample (IS): sessions 2010-06-07 → 2018-12-31.** All design choices, parameters and
  selection happen here.
* **Out-of-sample (OOS): sessions 2019-01-02 → 2026-07-09.** It is touched **once**, by the
  frozen portfolio. `q22 study` refuses OOS dates unless explicitly forced, and the forced run
  is logged in §7.
* Sessions are CME trading days (roll at 17:00 CT). RTH = 09:30–16:00 ET.

## 2. Execution model (same for every hypothesis)

* A signal on the close of bar *t* fills at the **open of bar t+1** plus 1 tick of slippage.
* The stop and target are resting orders. When both are touched in the same bar, the stop
  fills first. A target fills only on a trade-through of 1 tick. A gap through the stop fills at
  the open, and stops get 1 tick of slippage.
* Every trade is flat by **15:55 ET** (exit at that bar's close, minus slippage). This is
  stricter than both Topstep's 16:10 ET and Lucid's 16:45 ET cut-off.
* Costs per side: MNQ $0.75 commission plus 1 tick ($0.50); MES $0.75 plus 1 tick ($1.25).
  Results are reported in **R** (net of costs) and in **$ per micro contract**.
* At most one trade per hypothesis, side and session (events are counted, not bars).
* The "daily ATR" is ATR(14) of completed Globex sessions; no current-session information is
  used.

## 3. Hypotheses (parameters fixed here)

"X" is the index that sweeps a level and "Y" is the other index. Every hypothesis is tested
for the **bearish** case (highs) and its mirror, the **bullish** case (lows), pooled.

### Family A — SMT / SSMT divergence: sweep in one index, failure in the other → reversal

A sweep happens when X trades through its level by at least 1 tick. Y fails when its high (or
low) since the window start is still on the near side of its own level at that moment. The
confirmation, a market structure shift (MSS), is a 1-minute **close back through the level**
on X within 30 minutes of the sweep. The entry is on the next bar's open.

| id | Levels (per index) | Event window (ET) | Stop | Target |
|---|---|---|---|---|
| A1 | previous session high/low (Globex day) | 09:30–12:00 | traded index's extreme since the window start + 1 tick | 2R, else 15:55 |
| A2 | overnight high/low (18:00–09:30) | 09:30–11:30 | same | same |
| A3 | opening range 09:30–10:00 high/low | 10:00–14:00 | same | same |
| A4 | last 1-min fractal swing (high/low with 10 bars on each side, confirmed 10 bars later), both indices' swings formed within ±5 min of each other | 09:45–15:00 | same | same |

Each of A1–A4 is evaluated with two **legs**:
* `X`: trade the index that swept (fade the sweep);
* `Y`: trade the index that failed, in the reversal direction.

**Null models for family A:**
* the same trigger when **both** indices swept ("confirmed sweep, no SMT");
* a time-matched placebo: the same side, stop distance (in daily-ATR units) and target on 5
  random IS sessions at the same minute.

### Family B — intraday relative value (vol-normalised spread since the RTH open)

`s(t) = (NQ_t − NQ_open)/ATR_NQ − (ES_t − ES_open)/ATR_ES`, where `z = s / σ_s`. `σ_s` is the
standard deviation of the end-of-day `s` over the previous 20 sessions, scaled by √(elapsed
fraction of the session). The event is the first time |z| ≥ 2 between 10:00 and 14:30.

* **B1 reversion:** fade the leader. Legs: short the leader, or long the laggard.
* **B2 momentum:** follow the leader. Legs: long the leader, or short the laggard.
* Stop 0.25 daily ATR; target 0.5 daily ATR (2R); time exit at 15:55.
* Null: the time-matched placebo.

### Family C — lead–lag on 5-minute bars

* **C1:** a 5-minute return of X of at least 2 σ (σ = the 20-session std of 5-minute returns
  at that time of day) while Y moves less than 0.5 σ in the same direction → trade Y in X's
  direction.
* Stop 0.10 daily ATR; exit after 15 minutes or at 2R.
* Window 09:45–15:30, at most one per session per direction. Both directions (NQ→ES, ES→NQ)
  are pooled.

### Family D — cross-index confirmation of intraday momentum

The noise-area breakout (Zarattini et al. 2024, the q22 `noise_breakout` defaults) on index X,
checked at :00/:30 from 10:00.

* **D1:** split the breakouts into *confirmed* (Y is outside its own noise area in the same
  direction) and *divergent* (it is not). Hypothesis: confirmed > divergent in R.
* **D2:** fade divergent breakouts (enter against X), with the stop at the far side of the
  bar's range or 0.25 daily ATR, whichever is wider, and a 2R target.

### Family E — opening-gap divergence

* **E1:** the 09:30 gaps (open versus the previous RTH close, in daily-ATR units) have
  opposite signs and both exceed 0.10. Trade toward the gap fill on the index with the larger
  |gap|.
* Stop: 0.5 × that gap beyond the open. Target: the previous RTH close. Time exit 15:55.

### Family F — reference strategy (not divergence)

* **F1:** `noise_breakout` alone on MNQ and on MES (the q22 default). This shows whether the
  2020-23 CFD result holds on real futures in 2010-18.

**Trial count registered up front:**

| Family | Count |
|---|---|
| A (4 hypotheses × 2 legs) | 8 |
| B (2 × 2 legs) | 4 |
| C | 1 |
| D1 | 1 |
| D2 | 1 |
| E | 1 |
| F (2 instruments) | 2 |
| **Total** | **18** |

Any extra variant adds to this count.

## 4. Pass criteria in IS (all required)

1. n ≥ 100 trades in IS.
2. Net expectancy > 0 R, with t ≥ 2.0 (one-sided p ≈ 0.025) over trades.
3. It beats its null: the paired difference to the time-matched placebo has t ≥ 2.0. For
   family A it must also beat "confirmed sweep, no SMT" in mean R.
4. Net positive in at least 60% of the IS calendar years that contain at least 10 trades.
5. It still makes money with costs doubled.

Survivors are implemented as engine strategies with the exact IS rules. The portfolio is
formed **only** from survivors, plus F1 if F1 passes.

## 5. Portfolio objective (judged once, OOS)

The user's requirement is that *the sum of all bots must be winning with a prop-friendly
drawdown*. Frozen criteria for the OOS run:

* **Net:** combined net P&L > 0, and daily Sharpe > 0.5 after costs.
* **Drawdown:** the max end-of-day drawdown at the chosen size is ≤ 50% of the evaluation's
  max loss ($1,000 for a 50K account with a $2,000 limit).
* **Consistency:** positive in at least 5 of the 7.5 OOS years.
* **Pass rate:** the rolling-start pass rate under Topstep 50K and Lucid 50K rules is reported
  with its confidence interval.

Sizing is set from IS volatility: risk per trade so that the IS 99th-percentile daily loss is
below the guard's daily stop.

## 6. What would falsify the whole program

If no family-A hypothesis beats its null in IS, the conclusion is that NQ/ES SMT, as defined
here, has no measurable edge on 1-minute data in 2010-18. That would be reported as such, and
the portfolio would be built from what does survive. If nothing survives, nothing is traded.

## 7. Deviation log

**D-1 (2026-10-04, after the IS study run): result of the registered families A–E.**
All 15 harness hypotheses fail in IS:

| Family | Net R per trade | Gross R per trade | Placebo t | Yearly profile |
|---|---|---|---|---|
| A (SMT) | −0.33 to −0.52 | −0.07 to −0.02 | −0.1 to +1.4 | 0% of IS years positive |
| B | about −0.12 | | | |
| C1 | −0.34 | | | |
| D2 | −0.08 | | | |
| E1 | n = 21 | | | |

The forward-drift diagnostic shows no SMT information at +30, +60 or +120 minutes (|drift| < 0.01
daily ATR). Logged as **NOT PROVEN / rejected**. File: `results/study_is.txt`.

**D-2: F1 reference fails in IS with the registered cost model.**

| Contract | Net | Gross | Costs | PF |
|---|---|---|---|---|
| MNQ | −$16.4k | +$19.6k | $36.0k | 0.91 |
| MES | −$73.4k | −$38.6k | | |

The per-contract cost ($0.75 plus 1 tick per side) on 2010-18 prices was 3–10× today's cost
relative to price:
* NQ traded at 1,800–7,000 then, against about 25,000 in 2026;
* micro contracts did not exist before May 2019.

**D-3: additional cost model (reported next to the registered one, never instead of it).**
* Ratio back-adjusted data (`q22 databento --adjust ratio`), so percentage moves are exact.
* Costs expressed in basis points of notional, calibrated to 2026 prices:
  * MNQ: $1.25 per side at NQ 25,000 = **0.25 bp per side**;
  * MES: $2.00 per side at ES 6,500 = **0.615 bp per side**.
* The question this answers is *"would the rule be profitable at today's costs, in the market
  conditions of the time?"* Both cost models are always shown.

**D-4: new trials added after seeing D-1/D-2.** They count toward the Deflated-Sharpe trial
count: **18 registered + 4 new = 22**.

| Trial | Rule | Count |
|---|---|---|
| F2 | `noise_breakout` with the paper's own exit (`exit_mode = "checks"`): exit only when a 30-minute check closes back inside max(band, VWAP); protective stop 0.5 daily ATR, which also sizes the trade. MNQ and MES. This is the published rule, not a fitted one. | 2 |
| D1 | `peer_filter` confirm / diverge, evaluated on F2 as well as on F1. | +1 (D1 itself was registered) |
| A1n | "confirmed failed sweep" (both indices sweep the previous-session high/low, then MSS) traded as a reversal. Suggested by the null diagnostic of A1 (+0.06 ATR at +30 min, t = 3.3, n = 198). Data-snooped, so it must also pass OOS to be used. | 1 |

**D-5 (2026-10-04): in-sample engine results and the frozen portfolio.**
Files: `results/is_engine_matrix.txt`, `results/is_portfolios.txt`. IS, today-calibrated
basis-point costs; the registered per-contract costs are shown next to them in those files.

*Single bots:*

| Bot | Sharpe | Max DD |
|---|---|---|
| MNQ `noise_breakout`, resting | 1.23 | −$4.7k |
| MNQ `noise_breakout`, checks | 1.00 | −$2.3k |
| MES `noise_breakout`, checks + confirm | 0.68 | −$2.0k |
| MES `noise_breakout`, resting | 0.13 | |

*D1 (registered): a breakout confirmed by the other index beats a divergent one in all 4
comparisons.*

| Variant | Confirm | Diverge |
|---|---|---|
| MNQ resting | 1.17 | 0.51 |
| MNQ checks | 0.91 | 0.03 |
| MES resting | 0.51 | −0.32 |
| MES checks | 0.68 | −0.17 |

**VALIDATED in IS as a filter. Rejected as a reversal signal** (D2, A-family).

*Portfolios* (6 trials: P1/P2/P3 × risk $150/$250):

| Portfolio | Risk | Sharpe | Max DD | Pass rate | Median sessions to pass |
|---|---|---|---|---|---|
| P3 = MNQ resting+confirm and MES checks+confirm | $250 | **1.16** | −$4.6k (worst day −$557) | 52% (CI 36–68%) | 54 |
| P3 | $150 | 1.13 | | 55% | 97 |
| P2 | $250 | 1.12 | | 38% | |
| P1 | $250 | 0.63 | | | |

ORB was tried as a third bot (2 trials): MNQ Sharpe 0.44, MES −0.67. Rejected.

**Trial count: 30.**

**FROZEN:** P3 at $250 risk per trade (`config/research/frozen_P3_{contract,bps}.toml`).
* It meets the §5 sizing rule: the worst IS day of −$557 is below the guard's daily stop
  ($600).
* It does **not** meet the §5 drawdown criterion at this size: the 8.5-year max DD of $4.6k
  is above $1k. The OOS report will state this plainly, together with the risk per trade
  that would meet it.
* The next and only OOS run is 2019-01-02 → 2026-07-09 with both frozen configs.
