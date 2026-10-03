# What actually makes money, how fast, and how to run it

*Quant research memo — 3 October 2026. Every number is either produced by a script in
`research/` (data in `data/processed/`, outputs in `results/`) or cited in
[`SOURCES.md`](SOURCES.md). Labels: **VALIDATED** = survives the full protocol (costs, placebo or
null model, multiple-testing correction, out-of-sample check); **PLAUSIBLE** = real mechanism,
not yet proven on our data; **NOT PROVEN** = no better than chance on our data.*

> **Résumé en français.** Le seul edge qui survit à tous nos tests est le *suivi de tendance
> journalier* sur BTC/ETH, avec un sizing par volatilité (Sharpe ≈ 1 net de frais sur 2014-2026,
> mais **plat en 2025-26**). Le *carry de funding* est réel mais dormant en 2026. Les FVG/ICT, les
> niveaux Fibonacci, les chiffres ronds, les niveaux PO3/Goldbach, la Lune, Mercure rétrograde et la
> numérologie font **jeu égal avec le hasard**. La vitesse maximale d'enrichissement est fixée par
> les maths : croissance optimale = Sharpe²/2 par an. Aller plus vite que Kelly fait aller *moins*
> vite et mène à la ruine. Instrument recommandé : BTC puis ETH en futures *réglementés* CME
> (24/7 depuis mai 2026) pour un résident français, Hyperliquid seulement en connaissance du risque
> réglementaire MiCA. Les prop firms sont une option convexe, rentable *uniquement* avec un edge
> mesuré (Sharpe ≳ 0,75).

---

## 0. The answer in ten lines

1. **Instrument:** BTC first, then ETH. **Venue:** CME Micro Bitcoin / Bitcoin Friday futures
   via a regulated futures broker if you are tax-resident in the EU (24/7 trading since
   29 May 2026). Hyperliquid perps have the best microstructure for small accounts, but they are
   an unauthorised venue for EU residents after MiCA's 1 July 2026 cut-off (§5).
2. **Core system:** a *pre-specified, un-optimised* daily trend-following ensemble with volatility
   targeting. On BTC 2014→2026 it returns **Sharpe 1.05 net of taker costs, CAGR 25.5%, max
   drawdown −29%**, against −86% for buy-and-hold (bootstrap p = 0.001, Deflated Sharpe 0.997).
3. **But:** it was **flat in 2025-26** (Sharpe −0.09 over 21 months). A true Sharpe-1 strategy does
   that about 9% of the time, so this is a yellow flag, not a death certificate. Plan on a forward
   Sharpe of about **0.6–0.9**, not 1.05.
4. **Second sleeve:** funding-rate carry earned 8–11% a year net from 2020 to 2024 with almost no
   drawdown. In 2026 BTC funding is about 1.3% annualised, so the sleeve is **dormant**. It switches
   itself back on when leverage demand returns.
5. **Everything retail "edge" culture sells failed the placebo test on 85,000 hourly BTC bars (2017-26):**
   FVG fills (real BTC fills gaps *less* often than a random walk), Fibonacci, round numbers,
   PO3/Goldbach levels, lunar phase, Mercury retrograde and numerology. 26 hypotheses were tested;
   **0 survive multiple-testing correction.**
6. **Speed limit:** the maximum long-run growth rate is **SR²/2** per year. With a realistic
   SR ≈ 0.8 that is 32%/yr at full Kelly, with a 50% chance of halving your account along the way.
   The sane setting (¼–½ Kelly) gives 14–24%/yr. **No honest trading system turns a small account
   into a large one quickly.** Large money comes from more capital, other people's capital, or
   selling the skill.
7. **Prop firms** (Apex 100K EOD at the $399 list price) cost a **zero-edge** trader −$212 per
   attempt, and only 5.6% of attempts ever get a payout. That matches the industry figure of about
   7%. With a real Sharpe of 1 the EV is +$282; at Sharpe 2 it is +$1,468. At promotional prices
   the payoff becomes a cheap call option on your edge (§4.4).
8. **Size by volatility, never by conviction.** Volatility is forecastable (correlation 0.44 month
   to month); direction barely is (0.10).
9. **Run it like a fund:** paper-trade daily, then go live tiny with a kill-switch, then scale only
   when live results sit inside the backtest's confidence band (§6).
10. **Your existing NQ framework** stays a hypothesis until it beats placebo levels. The harness to
    test it is in this repo; what's missing is NQ data (Databento historical is pay-as-you-go).

---

## 1. The interview, fact-checked

| Claim (Charlie Haris, MoneyTalk #33) | Verdict | Evidence |
|---|---|---|
| FVG fills happen 67% of the time on real data vs 92% on a random walk, so FVGs have no predictive value | **TRUE (replicated)** | 1h BTC 2017-26, 17,343 FVGs, 24-bar horizon: real 85.3% vs Gaussian random walk 89.4% (95% band 88.9–89.8%) vs shuffled real bars 85.1%. Daily bars: 78.9% vs 88.7%. Gap "fills" are pure geometry. After a fill, the move in the FVG's direction is **+2.7 bp (t = 0.91)** against ~15 bp of round-trip cost. `results/01_*` |
| Price behaves like a random walk | **MOSTLY TRUE** | Lo–MacKinlay variance ratios on daily BTC: 0.95 (2d), 1.02 (10d), 1.30 (60d), none significant at 5% (z = −1.79 … +1.65). Hourly DFA Hurst = 0.503. But volatility is *not* random (Hurst of \|r\| = 0.78). `results/11_*` |
| A monkey picking at random beats the S&P 500 | **TRUE but misleading** | Arnott et al. (2013): 96 of 100 random portfolios beat cap-weighting by about 1.7%/yr, because random picks tilt to small and value stocks. That is a factor tilt, not skill and not luck. |
| Citadel fires a PM at ~7% annual drawdown | **ROUGHLY** | Millennium: −5% halves the book, −7.5% closes it. Citadel and Point72 negotiate the limit per PM. Industry band is a soft stop near −5% and a hard stop at −7% to −10%. |
| Win rates above 60–65% are mathematically unrealistic | **FALSE as stated** | Win rate is a choice of stop/target geometry, not a measure of skill. An 85%-win system at +0.3R/−2.5R has **−0.12R expectancy** and is negative after 500 trades in 100% of simulations. Our best real system wins **29%** of its trades (payoff 9.4, profit factor 3.8). `results/02_*` |
| Set the TP below the average MFE to raise the win rate | **TRUE, and it destroys the edge** | On the 115 real trend trades: TP at ¼ of median MFE lifts the win rate from 38% to 57% and cuts expectancy **from 3.42% to 0.26% per trade (−92%)**. Trend profits live in the tail. |
| Backtest over 10–20 years | **RIGHT, and crypto can't comply** | Liquid BTC history is about 12 years and alts 5–9, which contradicts "crypto is the best playground" unless you lean on statistics (MinTRL, DSR) instead of length. Our trend sleeve needs ≥ 2.3 years of live data just to confirm SR > 0 at 95%. |
| A perfect backtest delivers 80–85% live | **NO UNIVERSAL NUMBER** | It depends on how much you searched. Re-optimising our lookback every 6 months dropped the median Sharpe from **1.91 in-sample to 0.40 out-of-sample (−79%)**, while the un-optimised ensemble held 1.16 on the same window. That is what the Deflated Sharpe and PBO quantify. |
| A 20–25% degradation means the edge is dead | **ARBITRARY** | Use statistics. For a true SR = 1, P(realised Sharpe ≤ 0 over 1 year) = 16%; over 1.75 years = 9%; over 3 years = 4%. Flat 2025-26 is a yellow flag, not proof of death. |
| Crypto is the most inefficient market, the best for algos | **HALF TRUE, decaying fast** | Hourly mean reversion exists (lag-1 autocorrelation −0.023) but **0 of 18 variants survive taker fees**, and gross profits came only from 2017-18. The carry Sharpe fell from 6.45 (2020-25) to 4.06 (2024) to negative (2025). Daily trend still works. |
| Futures data costs ~$1,500/month (Databento) | **OUTDATED** | 2026 CME live tiers are $199 / $1,750 / $4,500; non-professional live starts at $36.50/month; history is pay-as-you-go. |
| Hyperliquid gives free order-book data | **TRUE** | Public info endpoint, 1,200 weight/min/IP; L2 book is weight 2. |
| Below €50k, don't trade actively | **RIGHT on the economics** | SR 0.8 at ¼ Kelly on €10k ≈ €1.5k/yr median (§3). The work only pays if capital, other people's capital, or the skill itself scales. |
| Algos react in 10–200 ms vs 10–13 s for a human | **TRUE but the wrong lesson** | 10–200 ms is *slow* in electronic markets. A retail algo should compete where speed is irrelevant (daily bars), not where it is everything. |

---

## 2. What we tested and what survived

### 2.1 Myths: no better than placebo

Every level or calendar test is measured **against a placebo of equal density** (shifted grids,
random ratios, shifted calendars, 200–2,000 draws), then corrected for the number of hypotheses
tested (Bonferroni).

| Hypothesis | Data | Result | Label |
|---|---|---|---|
| FVG zones attract price | 1h/4h/1D BTC 2017-26, 22k gaps | Real fill rate ≤ random walk at every horizon; post-fill drift +2.7 bp (t = 0.91) | **NOT PROVEN** |
| Round numbers ($1k/$5k/$10k) act as barriers | 1h BTC, 6,257 touches | Bounce 51–55% vs placebo 51–52%; every result inside the 95% placebo band | **NOT PROVEN** (as a trade)* |
| PO3/Goldbach levels (DR 729/2187/6561, 15 levels) | 1h BTC, 30,539 touches | Bounce 50.8–53.0% vs random-ratio placebo 50.7–52.4%; 1 nominal p < 0.05 out of 18 tests (0.9 expected by chance) | **NOT PROVEN** |
| Fibonacci retracements (23.6–78.6%) | 1h BTC, 12,976 pullbacks | 50.3–52.1% vs random ratios 50.3–51.5% | **NOT PROVEN** (agrees with the 2022 Dow/NASDAQ/DAX study) |
| Lunar phase: new moon beats full moon (Dichev & Janes 2003) | BTC 2011-26, ETH 2016-26 | BTC: +51%/yr, raw p = 0.022, **p = 0.18 after correction**. ETH: +9.6%, p = 0.46 | **NOT PROVEN** (direction matches the literature; magnitude is noise) |
| Mercury retrograde is bearish (Hang & Wang 2021) | BTC, ETH | Retrograde days *outperformed*: +28.5%/yr (p = 0.31) and +2.8% (p = 0.53) | **NOT PROVEN** (sign is the opposite of the claim) |
| Numerology: master-number dates, digit-sum-9 prices | BTC, ETH | 3 of 4 null; ETH digit-sum-9 +237%/yr at raw p = 0.009, **p = 0.07 after correction** | **NOT PROVEN** — a textbook false positive you'd "discover" if you tested only that one |

\*Round numbers *are* where orders cluster (Osler 2003: take-profits exactly on the number, stops
just beyond it). The tradeable mechanism is the **stop cascade through** the number, not a bounce
*at* it. That is a testable hypothesis on tick data, not an established edge.

**On astrology specifically:** the published equity effects are small: 3–5%/yr for the Moon
(Yuan et al. 2006) and −3.3%/yr for Mercury retrograde across 48 countries (Hang & Wang 2021).
Their proposed channel is *belief*: people who believe it trade on it. A 2020s replication of the
lunar effect failed (p ≈ 0.7–0.8). On crypto we find nothing that survives correction. Even taking
the literature at face value, 3%/yr of drift cannot pay the costs of trading it on and off twice a
month.

**On your PO3/Goldbach framework:** with ~15 lines per dealing range, *every* price is near a
line. That is why the comparison has to be Goldbach levels vs random levels of the same density,
and the two are indistinguishable on BTC. This does not prove they fail on NQ 1-minute data. It
means the burden of proof sits with the levels, and `quant22.features.levels` now runs exactly
that test once you have NQ data.

### 2.2 Edges: what survived

| Strategy | Sample | Net Sharpe | CAGR | Max DD | Robustness checks | Label |
|---|---|---|---|---|---|---|
| **Daily trend ensemble, BTC** (3 TSMOM + 3 EWMA + breakout, 30% vol target, ≤ 1.5×) | 2014-01 → 2026-10 | **1.05** | 25.5% | −29% | Bootstrap p = 0.001; DSR 0.997 (benchmark SR 0.42 for 27 trials); 27/27 lookbacks positive; costs ×2 → 1.00; sub-periods 0.97 / 1.65 / 1.90 / 0.42 / 1.13 / **−0.09** | **VALIDATED (decaying?)** |
| Same, long/flat only | same | 1.46 | 30.2% | −25% | The short side cost −2.6%/yr against an asset with a 45% CAGR. *This variant was chosen after seeing results*, so treat the gap as optimistic. | **PLAUSIBLE** |
| Trend basket, 9 coins, risk-parity | 2018-01 → 2026-05 | 0.73 | 11.1% | −27% | Correlation −0.10 with BTC; weak 2022-26 (0.06, 0.32); survivorship bias (dead coins missing) | **VALIDATED-weak** |
| Hourly trend (TSMOM), taker | 2017 → 2026 | best 0.96 of 24 | — | — | **DSR 0.009** (benchmark SR 1.72) | **NOT PROVEN** |
| Hourly trend, maker | same | median 0.93 | — | — | DSR 0.98, PBO 0.42, but **fills are assumed**, which flatters it | **PLAUSIBLE** |
| Funding carry (long spot / short perp), BTC & ETH | 2020-01 → 2026-04 | ≈ 11 (inflated: flat 63% of the time) | 8.3% / 10.7% net of fees and 4% cash cost | −0.3% / −0.5% | 13 / 12 trades only; 2026 funding ≈ 1.3% ann. → dormant | **VALIDATED as a regime income; OFF now** |
| Hourly mean reversion after 2σ shocks | 2017 → 2026 | best gross 0.37 | — | — | 0 of 18 variants positive after taker costs; gross profits only in 2017-18 | **NOT PROVEN / dead** |

Three lessons hide in this table:

* **Do not optimise parameters. Average them.** Every daily lookback works, so picking the best
  one is noise-fitting (PBO = 0.69). Walk-forward re-selection delivered OOS Sharpe 0.76 with a
  **−64%** drawdown (it chose a lookback that sat 1.2× short into the +18% day of 2 April 2019).
  The fixed ensemble delivered 1.16 with −29% on the same window.
* **Trend profits are lumpy by construction.** The 3 best trades out of 115 made **73%** of the
  P&L. You cannot cherry-pick which trend to take, so you must take them all.
* **Holding costs matter on perps.** Paying funding cut trend Sharpe from 1.03 to 0.87 over
  2020-26 (−23.5% cumulative). Carrying longs in spot and shorts in the perp recovered it to 1.00.
  CME futures carry the same cost as a *basis*.

### 2.3 Physics: the structure of the price process

| Stylised fact (BTC 2017-26) | Measurement | What it implies |
|---|---|---|
| Fat tails | Hill tail exponent α ≈ **2.45** (1h), 3.2 (1d), 3.95 (1w); excess kurtosis 31 → 12.5 → 3.3 | Gaussian risk models understate crashes. Hourly tails are heavier than the "inverse cubic law" for equities (α ≈ 3). Cap leverage hard. |
| Volatility clustering / long memory | ACF of \|r\| = 0.18 at 1 day, still 0.08 at 60 days; DFA Hurst of \|r\| = **0.78** | Risk is forecastable, so **volatility targeting works**: corr(this month's vol, next month's vol) = 0.44 vs 0.10 for returns. |
| Weak return predictability | Variance ratio 0.95 at 2 days (mild reversal), 1.30 at 60 days (mild trend); daily Hurst 0.58 | Faint persistence at multi-week horizons is exactly where the trend sleeve operates. Nothing exploitable at hourly scale for a taker. |
| Intraday seasonality of risk | 13:00–21:59 UTC (US hours, 37.5% of the day) carries **52.2%** of BTC variance 2022-26 | Schedule rebalances and expect slippage around the US session. This is a *risk* fact, not an alpha. |
| Ergodicity (Peters) | Time-average growth ≠ ensemble average | Maximise log growth, not expected P&L. That is the Kelly criterion (§3). |

### 2.4 Macro backdrop (October 2026), and why the system doesn't forecast it

Fed funds were raised to 3.75–4.00% on 16 September 2026. The 10-year Treasury is at 5.24% and
WTI is above $100 after the Hormuz supply shock. The core PCE outlook is 3.4%. BTC is around $84.5k,
about one-third below its October 2025 high, and funding is near zero, which signals a
**deleveraged** market. Rising real yields have historically been a headwind for long-duration
risk assets. Rather than betting on that narrative, the trend sleeve *reads* it from price. As of
the 2 October 2026 close it holds **+0.67× long BTC** (vol-scaled) after the summer recovery from
$76k, and it will flip if the macro headwind shows up in the tape. This is the system's output, not
a recommendation.

---

## 3. "As much money as quickly as possible": the mathematics

For a strategy with true annual Sharpe **SR** levered by a fraction *k* of Kelly, the long-run
median growth rate is **g = (k − k²/2)·SR²**, and the probability of ever falling to a fraction *x*
of your peak is **x^(2/k − 1)** (Thorp 2006). A useful corollary: **the Kelly-optimal portfolio
volatility equals the Sharpe ratio.** For SR = 0.8 that is 80% portfolio vol, which almost nobody
can stomach.

| True Sharpe | Kelly fraction | Growth / yr | Years to 2× | Years to 10× | P(halve capital) | P(−80%) |
|---|---|---|---|---|---|---|
| 0.75 | 1 (full) | 28% | 2.5 | 8.2 | 50% | 20% |
| 0.75 | ½ | 21% | 3.3 | 10.9 | 12.5% | 0.8% |
| 0.75 | ¼ | 12% | 5.6 | 18.7 | 0.8% | ~0 |
| 1.0 | ½ | 37.5% | 1.8 | 6.1 | 12.5% | 0.8% |
| 2.0 | ½ | 150% | 0.5 | 1.5 | 12.5% | 0.8% |

*(Full table: `results/09_capital_growth_speed.md`.)*

**Over-leverage makes you slower, not faster.** One fat-tailed year of the BTC trend sleeve
(SR 1.05 assumed *known*, 30% vol at 1×, Student-t tails):

| Leverage | Median outcome | P(double) | P(lose half) | P(lose 90%) |
|---|---|---|---|---|
| 1× | ×1.31 | 8% | 0.3% | ~0 |
| 3× | ×1.73 (≈ peak) | 44% | 23% | 0.8% |
| 5× | ×1.58 | 44% | 52% | 9% |
| 10× | **×0.17** | 24% | 87% | **62%** |

The 10× gambler has the highest *average* (×20.9, dragged up by a few lottery winners) and the
worst *typical* outcome. That gap between the ensemble average and the time average is the
ergodicity problem in one line. Remember too that the true Sharpe is *uncertain*, and over-betting
an over-estimated edge is how good traders go broke. Hence ¼–½ Kelly.

**What this means for a real account.** At a forward SR ≈ 0.8 and a 20% vol target (≈ ¼ Kelly),
the expected excess return is ~16%/yr and the median log-growth ~14%: **€10k → ~€11.5k after one
year, €50k → ~€57.5k.** Doubling takes ~5 years. The quickest legitimate routes to a large number are
therefore:

1. **More capital:** income, a business, or the quant-dev skill itself, which is what Charlie did.
2. **Other people's capital:** a prop firm (§4.4) or, much later, a fund with a verified track record.
3. **Higher Sharpe through diversification:** N independent edges of Sharpe S combine to about
   S·√N. Trend plus carry on idle cash gave **1.10–1.26** on 2020-26, against 0.89 for trend alone
   (`results/10_*`). This is the only "free lunch" left.

---

## 4. The system

### 4.1 Instrument and venue

| Option | Pros | Cons | Verdict |
|---|---|---|---|
| **CME Micro Bitcoin (0.1 BTC) / Bitcoin Friday (0.02 BTC)** via a regulated broker | Fully regulated (no exchange/DEX counterparty risk, clear tax status); **24/7 since 29 May 2026**; same broker/tech stack as NQ | Basis (contango) cost on longs; monthly or weekly rolls; contract lumps: MBT needs ≥ ~$25k to track the model (at $10k it sits at 0 contracts on 17% of the days it wants exposure). BFF 0.02 BTC is fine from $5k but rolls weekly. | **Recommended for EU/French residents** |
| **Hyperliquid perps** | $10 minimum orders; 0.045% / 0.015% fees; free L2 data; open API and SDK; 1h funding | Not MiCA-authorised. The 1 July 2026 transition deadline has passed and FinTelegram classes it "Red / Perimeter Watch"; smart-contract and bridge risk; no KYC means no recourse | Best for **research data** and for non-EU residents; for EU residents only as an informed, personal decision |
| Spot BTC/ETH on a MiCA-authorised exchange + cash | Simplest; no funding/basis; long/flat variant only | No shorting; capital-inefficient | Good **starter** implementation of the long/flat variant |

### 4.2 Sleeves and allocation

| Sleeve | Rule (frozen) | Size | When it is on |
|---|---|---|---|
| **Core trend**: BTC, then ETH; add majors only with ≥ $25k | `TrendParams()` defaults: TSMOM 20/60/120d + EWMA 8/24, 16/48, 32/96 + 55d breakout, averaged; vol-scaled to 30% per asset; risk-parity across coins | Portfolio vol target **15–20%** (≈ ¼ Kelly at SR 0.8); gross ≤ 1.5× | Always; rebalance once a day after 00:00 UTC |
| **Carry** | Long spot / short perp when smoothed 8h funding > 1 bp; exit < 0.3 bp | Idle cash of the trend sleeve, 1× notional | Only in high-funding regimes. Dormant now. |
| **Cash** | T-bill / money-market equivalent | Remainder | Always. A trend book sits 60%+ in cash on average. |

The paper/live loop is `python -m quant22.execution.paper_trade --coins BTC ETH --equity 10000`
(dry-run by default; it marks to market and logs every intended order to `paper_log.jsonl`).

### 4.3 Hard risk rules (enforced in code by `RiskGuard`)

* Max gross leverage 1.5×, max per asset 1.0× (live defaults).
* Daily loss limit 3% → flat for the day. Drawdown from the high-water mark of **15% → full stop**
  and a human review. The backtest MDD was 29% at 30% vol, so 15% at 15–20% vol is the comparable
  alarm.
* Never raise size after losses. Raise the vol target only after 12 months of live data whose
  Sharpe lies inside the backtest's 80% confidence band, and never above ½ Kelly.
* **Kill criteria for the edge itself:** stop the sleeve if the live Probabilistic Sharpe Ratio
  vs 0 falls below 5% after ≥ 2 years, or if realised costs exceed the model by 2×.

### 4.4 The convex satellite: prop-firm futures (optional, *after* a validated NQ edge)

Path-simulated Monte Carlo of Apex 100K EOD (2026 rules: $6k target, $3k trailing, $1.5k daily
loss limit, $399 list fee, $149 activation, payout ladder up to $18k). Each day is simulated
intraday, so the daily loss limit and the liquidation threshold trigger *when touched*
(`results/08_*`):

| Trader's true Sharpe | P(pass) | P(paid ever, per attempt) | P(funded account blown within a year) | **EV per attempt @ $399** | EV @ $60 promo |
|---|---|---|---|---|---|
| −1.0 (typical retail after costs) | 13% | 2.4% | 99.7% | **−$357** | −$18 |
| 0.0 (coin-flipper) | 21% | 5.6% | 97% | **−$212** | +$127 |
| 1.0 | 31% | 11.5% | 90% | **+$282** | +$621 |
| 2.0 | 42% | 20% | 78% | **+$1,468** | +$1,807 |

*(daily P&L volatility $1,000; at Sharpe ≥ 2 a $500/day sizing is better. Evaluations are assumed to
allow at most 60 trading days.)*

What this means:

* Prop firms are a **call option on your edge**: the loss is capped at the fee, and the firm's
  drawdown absorbs your bad days. At list price you need a real Sharpe of roughly 0.6–0.75 (depending on sizing) just to break even.
* **Size the daily P&L σ at about ⅙–⅓ of the trailing drawdown** ($500–1,000 on a $3k drawdown).
  At $250 you rarely reach the target within the window; at $1,500 the extra blow-ups outweigh
  the faster passes.
* **EOD trailing beats intraday trailing** (31% vs 27% pass at Sharpe 1, EV $282 vs $132).
* Payout caps limit the upside to about $2.3k (Sharpe 1) to $4.6k (Sharpe 2) per funded account.
  This is a cash-flow side bet, not a path to wealth.
* **Automation:** Apex bans fully automated trading on funded (PA/Live) accounts, so it is
  decision-assisted only. Topstep (TopstepX API, $29/month), Lucid, Tradeify, Bulenox and others
  allow bots in 2026. Get the policy in writing first.
* The NQ edge itself is **not established here** (no NQ data in this sandbox). The best-documented
  candidates to test first are intraday momentum (Gao, Han, Li & Zhou 2018: the first half-hour
  predicts the last, R² 1.6%; Zarattini, Aziz & Barbon 2024: SPY intraday momentum, Sharpe 1.33
  2007-24 net). Then your sweep→displacement→retest sequence against placebo sweeps.

---

## 5. Legal and tax (France, verify with a professional)

* **MiCA:** since 1 July 2026, any crypto service provider serving EU clients without MiCA
  authorisation must stop. Perpetual futures may also fall under MiFID II and the CFD
  product-intervention rules. An unauthorised venue gives you no investor protection and no
  recourse.
* **Tax 2026:** crypto capital gains fall under the PFU at **31.4%** (12.8% income tax + 18.6%
  social contributions, up from 30% on 1 January 2026), or the progressive scale on option. Forms
  **2086**, **2042-C**, and **3916-bis for every foreign account** (exchange or platform). The
  treatment of *derivatives* (perps, CME futures) differs from spot and depends on your status.
  This is a question for a tax adviser before going live, not after. Keep the paper and live logs:
  they are your audit trail.

---

## 6. 90-day execution plan

| When | Do | Exit criterion |
|---|---|---|
| Week 1 | `pip install -e ".[dev]"`, `pytest`, `python research/run_all.py`; read `results/*.md`; open accounts (regulated broker for CME micros, or a spot account for the long/flat variant) | You can reproduce every number in this memo |
| Weeks 1–6 | Run the paper loop daily (cron at 00:05 UTC) on BTC+ETH; compare paper fills with the model | 30+ paper days, zero operational errors |
| Weeks 4–12 | Go live **tiny** (≤ 10–20% of the intended capital, 10–15% vol target); log slippage vs model | Realised costs ≤ 1.5× the model; no rule breaches |
| Month 3+ | Scale to a 15–20% vol target; re-evaluate quarterly with PSR / MinTRL; switch carry on whenever funding > 1 bp/8h | Live Sharpe inside the backtest's 80% band |
| In parallel | Buy NQ 1-min history; use `quant22.features.levels` + `validation/*` to test your framework and the intraday-momentum papers against placebo; only then buy prop evaluations (promo price, EOD trailing, σ_day ≈ ⅙–⅓ of the drawdown) | One rule with DSR > 0.95 and PBO < 0.3, plus 3 months of forward test |

---

## 7. What could make this wrong

* **Regime change.** Trend was flat in 2025-26. ETF-driven institutionalisation may compress crypto
  trends the way decades of CTA money compressed trend returns in futures.
* **Data.** Hourly BTC is a CryptoCompare multi-venue aggregate (2010–2013 dropped). Coin Metrics
  daily data for alts ends 2026-05-24. Funding is Binance-only to 2026-04. The alt universe is
  survivorship-biased: LUNA and FTT are missing.
* **Costs.** Taker fee plus 3 bp slippage is realistic for BTC/ETH at retail size. Basis/funding is
  modelled only in `06`. Maker fills are not modelled.
* **Selection.** Even with DSR/PBO, this memo selected the trend family out of several candidates.
  The long/flat variant was chosen after seeing results. Hence the forward haircut to SR ≈ 0.6–0.9.
* **Venue and counterparty risk** (FTX-style failures, bridge exploits, regulatory shutdowns) is not
  in any backtest. Diversify venues and never keep more on an exchange than the strategy needs.
* **This is research, not investment advice.** The repository exists so that you never have to
  take any number in it on trust.
