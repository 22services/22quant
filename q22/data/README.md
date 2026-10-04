# Sample data

| File | What | Source / licence |
|---|---|---|
| `NQ_1m_sample_2026.csv` | E-mini Nasdaq-100 futures, 1-minute OHLCV, 2026-04-01 → 2026-09-02 (55,440 bars, UTC) | getdata.finance free sample, MIT — https://github.com/getdata-finance/nq-1m-ohlcv-stocks-historical-data |

More history (not committed, fetched on demand with `q22 fetch-data --out data`):

| File | What | Source / licence |
|---|---|---|
| `USATECHIDXUSD_M5.csv` | Nasdaq-100 index CFD, 5-minute, 2020-09-24 → 2023-09-11 (GMT, tick volume) | TheSnowGuru/Stocks-Futures-Financial-Time-series-Tick-Bar-Data, MIT |
| `USA500IDXUSD_M5.csv` | S&P 500 index CFD, 5-minute, same period | same |
| `btc_hourly.csv` | BTC/USD hourly OHLCV 2010 → today (CryptoCompare aggregate) | mouadja02/bitcoin-technical-indicators-dataset |

For live futures data use your broker feed (TopstepX API includes it). For deeper futures
history, buy pay-as-you-go history from a vendor (e.g. Databento) and point `--data` at the CSV.
