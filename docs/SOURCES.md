# Sources

Everything in `docs/STRATEGY_REPORT.md` is either computed in this repository (scripts in
`research/`, data in `data/processed/`) or cited below. Links were checked on 2026-10-03.

## Data used in the repository

- Coin Metrics Community Network Data (daily price, volume, on-chain) — https://github.com/coinmetrics/data (CC BY-NC 4.0)
- Hourly BTC OHLCV 2010→2026 (CryptoCompare aggregate, mirrored on GitHub) — https://github.com/mouadja02/bitcoin-technical-indicators-dataset
- Binance BTC/ETH spot, USDT-perp and 8h funding rates 2020→2026-04 — https://github.com/zwmjj/funding-rate-arb
- Hyperliquid Python SDK (execution adapter mirrors its API) — https://github.com/hyperliquid-dex/hyperliquid-python-sdk

## Retail trading outcomes

- Chague, De-Losso & Giovannetti, *Day Trading for a Living?* (Brazilian futures day traders 2013-15: 97% lose) — https://www.scribd.com/document/486266428/Chague-Losso-Giovannetti-47WP
- Barber & Odean (2000), *Trading Is Hazardous to Your Wealth* (most active quintile underperforms by ~6.5%/yr) — https://www.researchgate.net/publication/228289199_The_Profitability_of_Day_Traders
- Survey of 30 studies, 8 countries (70–97% loss rates) — https://bananafarmer.app/research/day-trading-failure-rate
- Prop-firm pass/payout statistics 2026 (≈14% pass, ≈7% ever paid, ~1–3% durable) — https://www.quantvps.com/blog/prop-firm-statistics , https://traderssecondbrain.com/guides/prop-firm-pass-rate , https://tradeify.co/post/futures-prop-firm-statistics-2026 , https://www.quantvps.com/blog/prop-firm-statistics

## Multi-manager drawdown rules (fact-check of the "Citadel 7%" claim)

- Millennium: −5% halves capital, −7.5% terminates; Citadel/Point72 negotiated per PM — https://youngandcalculated.substack.com/p/how-pms-actually-get-fired-at-multi , https://www.wallstreetoasis.com/forum/hedge-fund/drawdown-limits-at-mms

## Data costs (fact-check of the "Databento $1,500/month" claim)

- Databento CME live plans 2026: $199 / $1,750 / $4,500 per month; non-pro live from $36.50 — https://databento.com/blog/updates-to-subscription-pricing , https://roadmap.databento.com/announcements/live-cme-data-is-now-open-to-all-users-starting-at-3265month

## Hyperliquid (venue)

- Fees 2026: 0.045% taker / 0.015% maker, volume tiers, maker rebates — https://hyperliquid.gitbook.io/hyperliquid-docs/trading/fees , https://hyperliquidguide.com/guides/fees/fees-explained
- Rate limits: 1200 weight/min/IP REST, 2000 msgs/min WebSocket — https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/rate-limits-and-user-limits
- HLP vault: $136.9M cumulative profit, 15–30% APR windows, 5–12% drawdowns — https://www.coingecko.com/learn/hyperliquid-hlp-vault-analysis , https://www.datawallet.com/crypto/hyperliquid-hlp-explained
- Market impact & adverse selection on Hyperliquid (arXiv 2606.15715) — https://arxiv.org/pdf/2606.15715
- Public trader identity & adverse selection (arXiv 2608.04373) — https://arxiv.org/pdf/2608.04373

## Candidate edges

- Momentum in crypto 2020–2025, TSMOM 31.96%/yr, converging to modest returns in 2024-25 — https://www.journals.vu.lt/BATP/en/article/view/44540
- Time-series momentum pre/post spot-ETF — https://zenodo.org/records/19671502
- Liu & Tsyvinski, *Risks and Returns of Cryptocurrency* (NBER w24877) — https://www.nber.org/system/files/working_papers/w24877/w24877.pdf
- Moreira & Muir (2017), *Volatility-Managed Portfolios* — https://www.researchgate.net/publication/315972283_Volatility-Managed_Portfolios ; Man Group, *The Impact of Volatility Targeting* — https://www.man.com/insights/the-impact-of-volatility-targeting
- Crypto carry: Sharpe 6.45 (2020-25) falling to 4.06 in 2024 and negative in 2025 — https://arxiv.org/pdf/2510.14435 ; *The Crypto Carry Trade* — https://gerbil.life/papers/CarryTrade.v1.2.pdf ; funding-rate arbitrage risk/return on CEX & DEX — https://www.sciencedirect.com/science/article/pii/S2096720925000818
- Short-horizon mean reversion in crypto (183 pairs, 15-min, decays within hours) — https://arxiv.org/html/2608.21888v1 ; asymmetric mean reversion in BTC — https://shura.shu.ac.uk/23470/1/Asymmetric_Mean_Reversion_of_Bitcoin_Price_Returns.pdf
- BTC weekend effect: no return gap, lower weekend volume (2014-24) — https://www.researchgate.net/publication/396418897_Bitcoin's_Weekend_Effect_Returns_Volatility_and_Volume_2014-2024 ; US-hours concentration of variance — https://cryptoslate.com/crypto-never-closes-but-bitcoin-ethereum-xrp-and-solana-now-move-on-wall-street-time/ ; overnight seasonality — https://quantpedia.com/strategies/intraday-seasonality-in-bitcoin

## Myths tested

- FVG / ICT: ~40,000 FVGs, 4 futures markets, no edge after costs — https://statoasis.com/overfit/research/ict-backtest-what-survives ; https://mpmmarkets.com/research/does-the-fair-value-gap-strategy-work
- Fibonacci: bounce probability indistinguishable from random levels (Dow/NASDAQ/DAX) — https://www.researchgate.net/publication/354702159_Automatic_identification_and_evaluation_of_Fibonacci_retracements_Empirical_evidence_from_three_equity_markets
- Round numbers: Osler (2003), *Stop-Loss Orders and Price Cascades in Currency Markets* — https://www.newyorkfed.org/medialibrary/media/research/staff_reports/sr150.pdf
- Lunar: Dichev & Janes (2003) — https://papers.ssrn.com/sol3/papers.cfm?abstract_id=281665 ; Yuan, Zheng & Zhu (2006) — https://personal.lse.ac.uk/yuan/papers/lunar.pdf ; recent non-replication — https://abouttrading.substack.com/p/does-the-moon-move-markets
- Mercury retrograde: Hang & Wang (2021) *Long Live Hermes!* (48 countries, −3.33%/yr, belief channel) — https://acfr.aut.ac.nz/__data/assets/pdf_file/0004/576994/Hang-Wang-Hermes2021.pdf ; Qi, Wang & Zhang (2022) — https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4074620
- Gann / Square of Nine: no rigorous statistical validation — https://www.mql5.com/en/articles/15566 , https://www.gate.com/learn/articles/what-is-the-gann-square-of-nine-indicator/7301
- Malkiel's monkey (Arnott, Hsu, Kalesnik & Tindall 2013): outperformance = small-cap/value tilt — https://www.marketsentiment.co/p/the-monkeys-that-beat-the-market

## Statistics & physics

- Bailey & López de Prado, *The Deflated Sharpe Ratio* — https://www.researchgate.net/publication/286121118_The_Deflated_Sharpe_Ratio_Correcting_for_Selection_Bias_Backtest_Overfitting_and_Non-Normality ; Bailey, Borwein, López de Prado & Zhu, *Probability of Backtest Overfitting* — https://sdm.lbl.gov/oapapers/ssrn-id2507040-bailey.pdf
- Kelly / fractional Kelly / drawdown odds — MacLean, Thorp & Ziemba, *Good and Bad Properties of the Kelly Criterion* — https://www.stat.berkeley.edu/~aldous/157/Papers/Good_Bad_Kelly.pdf ; Busseti, Ryu & Boyd, *Risk-Constrained Kelly Gambling* — https://stanford.edu/~boyd//papers/pdf/kelly.pdf
- BTC tail exponent 2–2.5 (vs ~3 "inverse cubic law" for equities), aggregational Gaussianity beyond ~2 weeks — https://arxiv.org/pdf/1803.08405 , https://arxiv.org/pdf/1707.07618 ; multifractality & Hurst 2018–2025 — https://doi.org/10.3390/fractalfract10060379
- JPL/Standish approximate planetary elements (used for the Mercury ephemeris in `quant22/features/astro.py`) — https://ssd.jpl.nasa.gov/planets/approx_pos.html

## Macro backdrop, October 2026

- Fed funds 3.75–4.00% after the 16 Sep 2026 hike; 10y Treasury 5.24% (1 Oct 2026); WTI back above $100; core PCE outlook 3.4% — https://tradingeconomics.com/united-states/interest-rate , https://www.federalreserve.gov/releases/h15/ , https://bellsforex.com/markets/2026/2026-market-brief-october.html , https://www.jpmorgan.com/insights/markets-and-economy/economy/economic-trends
- BTC ≈ $83–86k; spot-ETF inflows record week 21–25 Sep then slowing — https://finance.yahoo.com/markets/crypto/articles/bitcoin-price-prediction-october-2026-141555670.html , https://www.investorideas.com/news/2026/cryptocurrency/09292-bitcoin-etfs-post-2026s-strongest-weekly-inflows-as-markets-turn-to-october-rate-inflation-and-liquidity-risks.asp

## Prop firms, venues, regulation, tax (added for §4–5 of the report)

- Apex 100K EOD 2026 pricing, DLL, activation fee, payout ladder ($2.0k/2.5k/2.5k/3.0k/4.0k/4.0k) — https://propfirmapp.com/prop-firms/apex-trader-funding , https://propfirmsfinder.com/prop-firm/apex-trader-funding/payouts/ , https://www.forexfactory.com/thread/1392737-apex-trader-funding-payout-rules-in-2026
- Apex prohibits fully automated trading on PA/Live accounts — https://support.apextraderfunding.com/hc/en-us/articles/31519788944411-Performance-Account-PA-and-Compliance , https://blog.pickmytrade.trade/apex-funded-automation-rules-2026/
- Futures prop firms allowing automation in 2026 (Topstep API $29/mo, Lucid, Tradeify, Bulenox…) — https://damnpropfirms.com/best-prop-firms-for-algo-trading/ , https://blog.pickmytrade.trade/best-prop-firms-algo-trading-bots-2026/
- CME crypto futures 24/7 since 29 May 2026 — https://www.coindesk.com/markets/2026/05/28/bitcoin-s-famous-cme-gaps-are-about-to-disappear-though-three-remain-unresolved , https://www.investing.com/news/company-news/cme-group-launches-247-cryptocurrency-futures-trading-93CH-4720129
- CME Micro Bitcoin (0.1 BTC) and Bitcoin Friday futures (0.02 BTC) — https://www.cmegroup.com/markets/cryptocurrencies/micro-cryptocurrency-futures-and-options , https://www.cmegroup.com/articles/2024/bitcoin-friday-futures-your-new-bff.html
- MiCA transitional period ends 1 July 2026 (AMF) — https://www.amf-france.org/en/news-publications/news/amf-reminds-digital-asset-service-providers-transitional-period-allowing-them-continue-providing ; Hyperliquid and the EU perimeter — https://fintelegram.com/mica-mifid-ii-perimeter-radar-hyperliquid-and-the-eus-on-chain-perps-problem/
- French crypto taxation 2026 (PFU 31.4%, forms 2086 / 2042-C / 3916-bis) — https://fibo-crypto.fr/en/blog/guide-crypto-tax-france/ , https://blog.nalo.fr/flat-tax-placements/ , https://www.waltio.com/fr/tout-savoir-sur-la-fiscalite-crypto/

## Intraday futures edges to test next (not tested here)

- Gao, Han, Li & Zhou (2018), *Market Intraday Momentum*, JFE — https://www.sciencedirect.com/science/article/abs/pii/S0304405X18301351
- Zarattini, Aziz & Barbon (2024), *Beat the Market: An Effective Intraday Momentum Strategy for SPY* (Sharpe 1.33, 2007–2024) — https://www.researchgate.net/publication/380582442_Beat_the_Market_An_Effective_Intraday_Momentum_Strategy_for_SP500_ETF_SPY

## Classic references used without a link

- Kelly (1956); Thorp (2006), *The Kelly Criterion in Blackjack, Sports Betting and the Stock Market*; Peters (2019), *The ergodicity problem in economics*, Nature Physics 15.
- Lo & MacKinlay (1988), variance-ratio test; Peng et al. (1994), detrended fluctuation analysis; Hill (1975), tail-index estimator.
- Moskowitz, Ooi & Pedersen (2012), *Time Series Momentum*, JFE; Baz et al. (2015), *Dissecting Investment Strategies in the Cross Section and Time Series* (EWMA trend signal).
- Politis & Romano (1994), stationary bootstrap.
