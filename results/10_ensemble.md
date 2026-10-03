# Ensemble

| sleeve | sharpe | cagr | ann_vol | mdd | corr_btc |
|---|---|---|---|---|---|
| BTC buy&hold | 0.9099 | 0.4434 | 0.6155 | -0.7667 | 1 |
| trend BTC | 0.8899 | 0.2038 | 0.2407 | -0.2268 | 0.06798 |
| trend portfolio (9 coins) | 0.6526 | 0.09913 | 0.1659 | -0.2685 | -0.04517 |
| carry BTC (1x notional, delta-neutral) | 7.213 | 0.08312 | 0.01108 | -0.002347 | 0.03453 |
| 50/50 trend BTC + trend portfolio | 0.8549 | 0.1543 | 0.1886 | -0.2314 | 0.02351 |
| trend BTC + carry 1x (capital split 50/50) | 1.219 | 0.1501 | 0.1206 | -0.1146 | 0.06941 |
| trend BTC (full) + carry 1x on the idle cash (~65% of capital sits in cash) | 1.104 | 0.2679 | 0.241 | -0.2241 | 0.06893 |
| trend portfolio + carry 2x on idle cash | 1.263 | 0.219 | 0.168 | -0.2634 | -0.04165 |

| index | trend_btc | bh_btc | trend_portfolio | carry_btc |
|---|---|---|---|---|
| trend_btc | 1 | 0.07 | 0.71 | 0.02 |
| bh_btc | 0.07 | 1 | -0.05 | 0.03 |
| trend_portfolio | 0.71 | -0.05 | 1 | 0.1 |
| carry_btc | 0.02 | 0.03 | 0.1 | 1 |
