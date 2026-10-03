"""Astronomy for testing astrology.

Two "esoteric" calendars that traders talk about, computed from first
principles so the tests cannot be accused of using a lucky almanac:

* Lunar phase from the mean synodic month (29.530588853 d) anchored at the
  new moon of 2000-01-06 18:14 UTC. Accurate to ~±0.5 day over 1900–2100,
  which is more than enough for 7- or 15-day windows (Dichev & Janes 2003;
  Yuan, Zheng & Zhu 2006).
* Mercury's apparent (geocentric) ecliptic longitude from Keplerian elements
  (Standish, JPL "Approximate Positions of the Planets", valid 1800–2050), to
  flag *retrograde* periods — days when the longitude decreases (Hang & Wang
  2021, "Long Live Hermes!"; Qi, Wang & Zhang 2022).

If an effect is real it should survive a placebo: the same test with the
calendar shifted by a random offset.
"""
from __future__ import annotations

import numpy as np
import pandas as pd

SYNODIC_MONTH = 29.530588853
JD_NEW_MOON_2000 = 2451550.1          # 2000-01-06 18:14 UTC
JD_J2000 = 2451545.0

# Standish / JPL approximate elements (epoch J2000, rates per Julian century)
# a (AU), e, I (deg), L (deg), long.peri (deg), long.node (deg)
_ELEMENTS = {
    "mercury": (np.array([0.38709927, 0.20563593, 7.00497902, 252.25032350, 77.45779628, 48.33076593]),
                np.array([0.00000037, 0.00001906, -0.00594749, 149472.67411175, 0.16047689, -0.12534081])),
    "earth":   (np.array([1.00000261, 0.01671123, -0.00001531, 100.46457166, 102.93768193, 0.0]),
                np.array([0.00000562, -0.00004392, -0.01294668, 35999.37244981, 0.32327364, 0.0])),
}


def julian_day(ts: pd.DatetimeIndex | pd.Series) -> np.ndarray:
    t = pd.DatetimeIndex(ts)
    if t.tz is not None:
        t = t.tz_convert("UTC").tz_localize(None)
    # Unix epoch 1970-01-01 00:00 UTC = JD 2440587.5. Cast explicitly: pandas>=3 may
    # store the index at second/microsecond resolution, so ``asi8`` is not nanoseconds.
    secs = t.values.astype("datetime64[us]").astype(np.int64) / 1e6
    return 2440587.5 + secs / 86400.0


def moon_phase(ts: pd.DatetimeIndex) -> pd.Series:
    """Phase in [0,1): 0 = new moon, 0.5 = full moon."""
    jd = julian_day(ts)
    ph = ((jd - JD_NEW_MOON_2000) / SYNODIC_MONTH) % 1.0
    return pd.Series(ph, index=ts, name="moon_phase")


def days_from_new_moon(ts: pd.DatetimeIndex) -> pd.Series:
    """Signed days to the *nearest* new moon, in (−14.77, 14.77]."""
    ph = moon_phase(ts).values
    d = ph * SYNODIC_MONTH
    d = np.where(d > SYNODIC_MONTH / 2, d - SYNODIC_MONTH, d)
    return pd.Series(d, index=ts, name="days_from_new_moon")


def days_from_full_moon(ts: pd.DatetimeIndex) -> pd.Series:
    ph = (moon_phase(ts).values + 0.5) % 1.0
    d = ph * SYNODIC_MONTH
    d = np.where(d > SYNODIC_MONTH / 2, d - SYNODIC_MONTH, d)
    return pd.Series(d, index=ts, name="days_from_full_moon")


def _kepler(M: np.ndarray, e: np.ndarray, iters: int = 30) -> np.ndarray:
    E = M + e * np.sin(M)
    for _ in range(iters):
        E = E - (E - e * np.sin(E) - M) / (1 - e * np.cos(E))
    return E


def heliocentric_ecliptic(body: str, jd: np.ndarray) -> np.ndarray:
    el0, rate = _ELEMENTS[body]
    T = (jd - JD_J2000) / 36525.0
    a, e, I, L, wbar, Om = (el0[i] + rate[i] * T for i in range(6))
    I, L, wbar, Om = np.radians(I), np.radians(L), np.radians(wbar), np.radians(Om)
    w = wbar - Om
    M = (L - wbar + np.pi) % (2 * np.pi) - np.pi
    E = _kepler(M, e)
    xp = a * (np.cos(E) - e)
    yp = a * np.sqrt(1 - e**2) * np.sin(E)
    cw, sw, cO, sO, cI, sI = np.cos(w), np.sin(w), np.cos(Om), np.sin(Om), np.cos(I), np.sin(I)
    x = (cw * cO - sw * sO * cI) * xp + (-sw * cO - cw * sO * cI) * yp
    y = (cw * sO + sw * cO * cI) * xp + (-sw * sO + cw * cO * cI) * yp
    z = (sw * sI) * xp + (cw * sI) * yp
    return np.vstack([x, y, z]).T


def mercury_geocentric_longitude(ts: pd.DatetimeIndex) -> pd.Series:
    jd = julian_day(ts)
    m = heliocentric_ecliptic("mercury", jd)
    e = heliocentric_ecliptic("earth", jd)
    g = m - e
    lon = np.degrees(np.arctan2(g[:, 1], g[:, 0])) % 360.0
    return pd.Series(lon, index=ts, name="mercury_lon")


def mercury_retrograde(ts: pd.DatetimeIndex) -> pd.Series:
    """Boolean: True on days when Mercury's apparent longitude is decreasing.

    Computed on a daily grid spanning the input, then aligned to ``ts``.
    """
    t = pd.DatetimeIndex(ts)
    tz = t.tz
    start = (t.min() - pd.Timedelta(days=2)).normalize()
    end = (t.max() + pd.Timedelta(days=2)).normalize()
    grid = pd.date_range(start, end, freq="1D", tz=tz)
    lon = mercury_geocentric_longitude(grid).values
    dlon = np.diff(np.unwrap(np.radians(lon)))
    retro = pd.Series(np.r_[dlon[0], dlon] < 0, index=grid, name="mercury_retrograde")
    return retro.reindex(t.normalize(), method="nearest").set_axis(t)


def retrograde_periods(start: str, end: str) -> list[tuple[pd.Timestamp, pd.Timestamp]]:
    grid = pd.date_range(start, end, freq="1D", tz="UTC")
    r = mercury_retrograde(grid).values
    out = []
    i = 0
    while i < len(r):
        if r[i]:
            j = i
            while j + 1 < len(r) and r[j + 1]:
                j += 1
            out.append((grid[i], grid[j]))
            i = j + 1
        else:
            i += 1
    return out
