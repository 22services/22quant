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

## Paid history: Databento (16 years of ES and NQ)

The repository root holds licensed Databento files (Git LFS):
`ES DATA/glbx-mdp3-20100606-20260709.ohlcv-1m.csv` and `NQ DATA/…`. These are all contracts and
calendar spreads, 1-minute, 2010-06-06 → 2026-07-09.

Build the continuous series (not committed, 100–650 MB each):

```bash
q22 databento --input "../NQ DATA/glbx-mdp3-20100606-20260709.ohlcv-1m.csv" --root NQ --out data/databento/NQ_c1_1m.csv
q22 databento --input "../ES DATA/glbx-mdp3-20100606-20260709.ohlcv-1m.csv" --root ES --out data/databento/ES_c1_1m.csv
# exact percentages (pair with basis-point costs):
q22 databento --input "../NQ DATA/glbx-mdp3-20100606-20260709.ohlcv-1m.csv" --root NQ --out data/databento/NQ_c1_1m_ratio.csv --adjust ratio
q22 databento --input "../ES DATA/glbx-mdp3-20100606-20260709.ohlcv-1m.csv" --root ES --out data/databento/ES_c1_1m_ratio.csv --adjust ratio
```

How the series is built:
* **Roll:** to the next contract when its volume beat the front's on the *previous* session,
  never backwards. This gives 65 rolls, listed in `*.rolls.csv`.
* **Panama adjustment:** adds the roll gaps, so point/dollar P&L is exact. The last column
  `offset` gives the raw price as `price − offset`.
* **Ratio adjustment:** multiplies by the roll ratios, so percentages are exact. Its `factor`
  column gives the raw price as `price ÷ factor`.
