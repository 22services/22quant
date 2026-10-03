"""The physics of the BTC price process (econophysics stylized facts).

What a strategy can exploit is decided by the statistical structure of returns:
  * fat tails — Hill tail exponent α (equities ≈ 3, "inverse cubic law");
  * aggregational Gaussianity — kurtosis falling with horizon;
  * volatility clustering — autocorrelation of |r| (this is what makes vol
    targeting work: risk is forecastable even when returns are not);
  * (non-)predictability of returns — autocorrelation, Lo–MacKinlay variance
    ratios, and a DFA Hurst exponent (H = 0.5 random walk, > 0.5 persistent).
"""
from __future__ import annotations

import numpy as np
import pandas as pd

from _common import load_hourly_btc, save_results, md_table, banner
from quant22.features.signals import log_returns


def hill_alpha(x: np.ndarray, tail_frac: float = 0.05) -> float:
    a = np.sort(np.abs(x[~np.isnan(x)]))[::-1]
    k = max(int(len(a) * tail_frac), 10)
    return float(1.0 / np.mean(np.log(a[:k] / a[k])))


def variance_ratio(r: np.ndarray, q: int) -> tuple[float, float]:
    """Lo–MacKinlay VR(q) with the heteroskedasticity-robust z statistic."""
    r = r[~np.isnan(r)]
    n = len(r)
    mu = r.mean()
    var1 = np.sum((r - mu) ** 2) / (n - 1)
    rq = np.convolve(r, np.ones(q), "valid")
    varq = np.sum((rq - q * mu) ** 2) / (q * (n - q + 1) * (1 - q / n))
    vr = varq / var1
    dev = (r - mu) ** 2
    theta = 0.0
    for j in range(1, q):
        delta = n * np.sum(dev[j:] * dev[:-j]) / (np.sum(dev) ** 2)
        theta += (2 * (q - j) / q) ** 2 * delta
    z = np.sqrt(n) * (vr - 1) / np.sqrt(theta) if theta > 0 else np.nan   # Lo & MacKinlay (1988) z*(q)
    return float(vr), float(z)


def dfa_hurst(r: np.ndarray, scales=(16, 32, 64, 128, 256, 512)) -> float:
    r = r[~np.isnan(r)]
    y = np.cumsum(r - r.mean())
    F = []
    for s in scales:
        n = len(y) // s
        segs = y[: n * s].reshape(n, s)
        t = np.arange(s)
        res = []
        for seg in segs:
            c = np.polyfit(t, seg, 1)
            res.append(np.mean((seg - np.polyval(c, t)) ** 2))
        F.append(np.sqrt(np.mean(res)))
    return float(np.polyfit(np.log(scales), np.log(F), 1)[0])


def main() -> None:
    banner("11 — Stylized facts of BTC returns (2017→2026, hourly-derived)")
    h = load_hourly_btc("2017-01-01")["close"]
    rows = []
    for lab, rule in [("1h", None), ("4h", "4h"), ("1d", "1D"), ("1w", "7D")]:
        px = h if rule is None else h.resample(rule).last().dropna()
        r = log_returns(px).dropna().values
        rows.append({"horizon": lab, "n": len(r), "ann_vol": r.std() * np.sqrt({"1h": 8766, "4h": 2191.5, "1d": 365.25, "1w": 52.18}[lab]),
                     "excess_kurtosis": float(pd.Series(r).kurt()), "skew": float(pd.Series(r).skew()),
                     "hill_alpha_5pct": hill_alpha(r), "acf1_returns": float(pd.Series(r).autocorr(1)),
                     "acf1_abs_returns": float(pd.Series(np.abs(r)).autocorr(1))})
    tab = pd.DataFrame(rows)
    print(md_table(tab))
    d = log_returns(h.resample("1D").last().dropna()).dropna()
    absacf = pd.DataFrame({"lag_days": [1, 5, 10, 20, 60, 120],
                           "acf_abs_return": [float(np.abs(d).autocorr(k)) for k in (1, 5, 10, 20, 60, 120)],
                           "acf_return": [float(d.autocorr(k)) for k in (1, 5, 10, 20, 60, 120)]})
    print(md_table(absacf))
    vr = pd.DataFrame([{"q_days": q, **dict(zip(["VR", "z_robust"], variance_ratio(d.values, q)))} for q in (2, 5, 10, 20, 60)])
    print(md_table(vr))
    hurst_daily = dfa_hurst(d.values)
    hurst_hourly = dfa_hurst(log_returns(h).dropna().values, scales=(24, 48, 96, 192, 384, 768, 1536))
    hurst_absd = dfa_hurst(np.abs(d.values) - np.abs(d.values).mean())
    print(f"DFA Hurst: daily returns {hurst_daily:.3f} | hourly returns {hurst_hourly:.3f} | daily |returns| (volatility) {hurst_absd:.3f}")
    # rolling realised vol → next-month realised vol predictability (why vol targeting works)
    rv = d.rolling(30).std() * np.sqrt(365.25)
    fwd = rv.shift(-30)
    corr = float(pd.concat([rv, fwd], axis=1).dropna().corr().iloc[0, 1])
    rr = d.rolling(30).sum()
    corr_ret = float(pd.concat([rr, rr.shift(-30)], axis=1).dropna().corr().iloc[0, 1])
    print(f"corr(30d realised vol, next 30d realised vol) = {corr:.2f}  vs  corr(30d return, next 30d return) = {corr_ret:.2f}")
    save_results("11_stylized_facts", {"by_horizon": rows, "abs_acf": absacf.to_dict(orient="records"),
                                       "variance_ratios": vr.to_dict(orient="records"), "hurst_daily": hurst_daily,
                                       "hurst_hourly": hurst_hourly, "hurst_abs_daily": hurst_absd,
                                       "vol_forecastability_corr": corr, "return_forecastability_corr": corr_ret},
                 "# Stylized facts\n\n" + md_table(tab) + "\n\n" + md_table(absacf) + "\n\n" + md_table(vr) +
                 f"\n\nDFA Hurst daily {hurst_daily:.3f}, hourly {hurst_hourly:.3f}, |r| {hurst_absd:.3f}; vol forecast corr {corr:.2f}, return {corr_ret:.2f}\n")


if __name__ == "__main__":
    main()
