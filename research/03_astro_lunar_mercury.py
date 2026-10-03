"""Myth test #3 — astrology & numerology vs mechanical calendar effects.

Tested on BTC (2011→2026) and ETH (2016→2026) daily returns, each against a
*placebo calendar* (the same calendar shifted by a random offset, 2000 times):

  * Lunar phase: new-moon window vs full-moon window (Dichev & Janes 2003).
  * Mercury retrograde vs direct (Hang & Wang 2021).
  * Numerology: "master number" dates (digit sum of the date = 11 or 22) and
    prices whose integer part has digit sum 9 (Gann folklore).
Then, for contrast, two *mechanical* calendar effects with a cause:
  * Hour-of-day return/volatility concentration in US hours (2022→2026).
  * Day-of-week volatility (weekend liquidity).
"""
from __future__ import annotations

import numpy as np
import pandas as pd

from _common import load_daily, load_hourly_btc, btc_daily_ohlc, save_results, md_table, banner, verdict
from quant22.features.astro import days_from_new_moon, days_from_full_moon, mercury_retrograde, SYNODIC_MONTH

N_PLACEBO = 2000


def window_diff(r: pd.Series, offset_days: float, half_window: int = 7) -> float:
    idx = r.index + pd.Timedelta(days=offset_days)
    dn = days_from_new_moon(idx).values
    df_ = days_from_full_moon(idx).values
    new = r.values[np.abs(dn) <= half_window]
    full = r.values[np.abs(df_) <= half_window]
    return float(new.mean() - full.mean()) * 365.25


def lunar_test(r: pd.Series, name: str, rng: np.random.Generator) -> dict:
    obs = window_diff(r, 0.0)
    plac = np.array([window_diff(r, rng.uniform(0, SYNODIC_MONTH)) for _ in range(N_PLACEBO // 4)])
    label, p = verdict(obs, plac, higher_is_better=obs >= 0)
    print(f"[{name}] new-moon minus full-moon window, annualised: {obs:+.1%}  | placebo 95% band "
          f"[{np.percentile(plac, 2.5):+.1%}, {np.percentile(plac, 97.5):+.1%}]  → {label} (p={p:.3f})")
    return {"asset": name, "effect_ann": obs, "placebo_lo": np.percentile(plac, 2.5), "placebo_hi": np.percentile(plac, 97.5), "p": p, "verdict": label}


def mercury_test(r: pd.Series, name: str, rng: np.random.Generator) -> dict:
    retro = mercury_retrograde(r.index).values
    obs = float(r.values[retro].mean() - r.values[~retro].mean()) * 365.25
    n = len(r)
    plac = []
    for _ in range(N_PLACEBO):
        k = rng.integers(1, n)
        sh = np.roll(retro, k)
        plac.append(float(r.values[sh].mean() - r.values[~sh].mean()) * 365.25)
    plac = np.array(plac)
    label, p = verdict(obs, plac, higher_is_better=obs >= 0)
    print(f"[{name}] Mercury retrograde minus direct, annualised: {obs:+.1%} (retro share {retro.mean():.1%}) | "
          f"placebo band [{np.percentile(plac, 2.5):+.1%}, {np.percentile(plac, 97.5):+.1%}] → {label} (p={p:.3f})")
    return {"asset": name, "effect_ann": obs, "retro_share": float(retro.mean()), "placebo_lo": np.percentile(plac, 2.5),
            "placebo_hi": np.percentile(plac, 97.5), "p": p, "verdict": label}


def numerology_test(r: pd.Series, price: pd.Series, name: str, rng: np.random.Generator) -> list[dict]:
    out = []
    digits = r.index.strftime("%Y%m%d")
    dsum = np.array([sum(int(ch) for ch in s) for s in digits])
    master = np.isin(dsum, [11, 22, 33])
    prev = price.shift(1).reindex(r.index).fillna(1).values
    # integer part ≥ 10 so that the rule is about digits, not about "price below $10" (a regime, not numerology)
    pdig = np.array([(p >= 10) and (sum(int(ch) for ch in str(int(p))) % 9 == 0) for p in prev])
    for mask, lab in [(master, "master-number dates (digit sum 11/22/33)"), (pdig, "prev close (≥$10) digit-sum ≡ 0 mod 9")]:
        if mask.sum() < 30:
            continue
        obs = float(r.values[mask].mean() - r.values[~mask].mean()) * 365.25
        plac = np.array([(lambda m: float(r.values[m].mean() - r.values[~m].mean()) * 365.25)(np.roll(mask, rng.integers(1, len(r))))
                         for _ in range(N_PLACEBO // 2)])
        label, p = verdict(obs, plac, higher_is_better=obs >= 0)
        print(f"[{name}] {lab}: {obs:+.1%} ann. (n={mask.sum()}) → {label} (p={p:.3f})")
        out.append({"asset": name, "rule": lab, "effect_ann": obs, "n": int(mask.sum()), "p": p, "verdict": label})
    return out


def mechanical_effects() -> tuple[pd.DataFrame, pd.DataFrame]:
    banner("03 — for contrast: mechanical calendar effects with a cause")
    h = load_hourly_btc("2022-01-01")
    r = np.log(h["close"]).diff().dropna()
    hod = pd.DataFrame({"ret_bp": r.groupby(r.index.hour).mean() * 1e4, "vol_bp": r.groupby(r.index.hour).std() * 1e4,
                        "share_of_variance": (r**2).groupby(r.index.hour).sum() / (r**2).sum()})
    hod.index.name = "hour_utc"
    us = hod.loc[13:21, "share_of_variance"].sum()
    print(f"13:00–21:59 UTC (US cash hours, 37.5% of the day) carries {us:.1%} of BTC's realised variance 2022→2026")
    d = np.log(btc_daily_ohlc("2018-01-01")["close"]).diff().dropna()
    dow = pd.DataFrame({"ret_bp": d.groupby(d.index.dayofweek).mean() * 1e4, "vol_bp": d.groupby(d.index.dayofweek).std() * 1e4,
                        "n": d.groupby(d.index.dayofweek).size()})
    dow.index = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
    print(md_table(dow.reset_index().rename(columns={"index": "day"})))
    return hod.reset_index(), dow.reset_index().rename(columns={"index": "day"})


def main() -> None:
    banner("03 — Astrology & numerology vs placebo calendars")
    rng = np.random.default_rng(0)
    series = {}
    btc = btc_daily_ohlc("2014-01-01")["close"]
    cm_btc = load_daily("btc")["close"].loc["2011-01-01":"2013-12-31"]
    series["BTC 2011-2026"] = pd.concat([cm_btc, btc]).sort_index()
    series["ETH 2016-2026"] = load_daily("eth")["close"].loc["2016-01-01":]
    lunar, merc, numer = [], [], []
    for name, px in series.items():
        px = px[~px.index.duplicated()]
        r = np.log(px).diff().dropna()
        lunar.append(lunar_test(r, name, rng))
        merc.append(mercury_test(r, name, rng))
        numer += numerology_test(r, px, name, rng)
    n_tests = len(lunar) + len(merc) + len(numer)
    print(f"\n{n_tests} esoteric hypotheses tested → Bonferroni threshold for 5% family-wise error: p < {0.05 / n_tests:.4f}")
    for group in (lunar, merc, numer):
        for row in group:
            row["p_bonferroni"] = min(1.0, row["p"] * n_tests)
            row["survives_multiple_testing"] = bool(row["p_bonferroni"] < 0.05)
            if not row["survives_multiple_testing"]:
                row["verdict"] = "NOT PROVEN (fails multiple-testing correction)"
    hod, dow = mechanical_effects()
    md = ("# Astrology / numerology vs placebo\n\n## Lunar\n\n" + md_table(pd.DataFrame(lunar)) +
          "\n\n## Mercury retrograde\n\n" + md_table(pd.DataFrame(merc)) +
          "\n\n## Numerology\n\n" + md_table(pd.DataFrame(numer)) +
          "\n\n## Mechanical: hour of day (UTC), BTC 2022-2026\n\n" + md_table(hod) +
          "\n\n## Mechanical: day of week, BTC 2018-2026\n\n" + md_table(dow) + "\n")
    save_results("03_astro_numerology", {"lunar": lunar, "mercury": merc, "numerology": numer,
                                         "hour_of_day": hod.to_dict(orient="records"), "day_of_week": dow.to_dict(orient="records")}, md)


if __name__ == "__main__":
    main()
