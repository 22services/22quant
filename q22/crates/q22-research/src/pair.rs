//! Two continuous minute series (NQ = 0, ES = 1) aligned on common timestamps, cut into CME
//! sessions, with the per-session facts every hypothesis needs (levels, ATR, RTH bounds).
//!
//! Time is expressed as the **session minute** `sm` = minutes since 18:00 ET of the previous
//! calendar day (the Globex open), so it increases monotonically inside a session:
//! 09:30 ET = 930, 10:00 = 960, 15:55 = 1315, 16:00 = 1320.

use std::io::BufRead;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::America::New_York;

use q22_core::session::trading_day;
use q22_core::MarketKind;

pub const NQ: usize = 0;
pub const ES: usize = 1;
pub const NAMES: [&str; 2] = ["NQ", "ES"];

pub const SM_RTH_OPEN: u16 = 930;
pub const SM_RTH_CLOSE: u16 = 1320;
pub const SM_FLAT: u16 = 1315; // 15:55 ET

#[derive(Clone, Copy, Debug, Default)]
pub struct Ohlc {
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
}

/// One continuous series as written by `q22 databento`.
pub struct Series {
    pub ts: Vec<i64>,
    pub px: Vec<Ohlc>,
}

pub fn load_series(path: &Path) -> Result<Series> {
    let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut lines = std::io::BufReader::with_capacity(1 << 20, f).lines();
    let header = lines.next().ok_or_else(|| anyhow!("empty file"))??;
    if !header.starts_with("unix_timestamp,open,high,low,close") {
        return Err(anyhow!("{}: expected a `q22 databento` file, got header {header:?}", path.display()));
    }
    let (mut ts, mut px) = (Vec::with_capacity(6_000_000), Vec::with_capacity(6_000_000));
    for line in lines {
        let line = line?;
        let mut it = line.split(',');
        let mut next = || it.next().ok_or_else(|| anyhow!("short line {line:?}"));
        let t: i64 = next()?.parse()?;
        let o: f64 = next()?.parse()?;
        let h: f64 = next()?.parse()?;
        let l: f64 = next()?.parse()?;
        let c: f64 = next()?.parse()?;
        ts.push(t);
        px.push(Ohlc { o, h, l, c });
    }
    Ok(Series { ts, px })
}

#[derive(Clone, Debug)]
pub struct Day {
    pub date: NaiveDate,
    /// `[start, end)` bar indices of the whole Globex session.
    pub start: usize,
    pub end: usize,
    /// First bar at/after 09:30 ET and first bar at/after 16:00 ET (exclusive end of RTH).
    pub rth_start: Option<usize>,
    pub rth_end: usize,
    /// Last bar at or before 15:55 ET (the time exit).
    pub flat_idx: Option<usize>,
    /// ATR(14) of completed sessions *before* this one, per index.
    pub atr: [f64; 2],
    /// Full-session high/low/close per index (known only after the session — use the previous day's).
    pub high: [f64; 2],
    pub low: [f64; 2],
    pub close: [f64; 2],
    /// RTH open / last RTH close per index.
    pub rth_open: [f64; 2],
    pub rth_close: [f64; 2],
}

pub struct Pair {
    pub ts: Vec<DateTime<Utc>>,
    pub px: [Vec<Ohlc>; 2],
    pub sm: Vec<u16>,
    pub day_of: Vec<u32>,
    pub days: Vec<Day>,
    /// Cost model used by the simulator (default: the registered per-contract model).
    pub cost: crate::sim::CostModel,
}

fn session_minute(ts: DateTime<Utc>) -> u16 {
    let et = ts.with_timezone(&New_York);
    let m = et.hour() * 60 + et.minute();
    ((m + 360) % 1440) as u16
}

impl Pair {
    pub fn load(nq: &Path, es: &Path) -> Result<Pair> {
        let a = load_series(nq)?;
        let b = load_series(es)?;
        Ok(Self::align(&a, &b))
    }

    pub fn align(a: &Series, b: &Series) -> Pair {
        let (mut i, mut j) = (0, 0);
        let mut ts = vec![];
        let (mut pa, mut pb) = (vec![], vec![]);
        while i < a.ts.len() && j < b.ts.len() {
            match a.ts[i].cmp(&b.ts[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    ts.push(Utc.timestamp_opt(a.ts[i], 0).single().expect("valid ts"));
                    pa.push(a.px[i]);
                    pb.push(b.px[j]);
                    i += 1;
                    j += 1;
                }
            }
        }
        let sm: Vec<u16> = ts.iter().map(|t| session_minute(*t)).collect();
        let mut days: Vec<Day> = vec![];
        let mut day_of = vec![0u32; ts.len()];
        let mut k = 0;
        while k < ts.len() {
            let d = trading_day(MarketKind::CmeEquityIndex, ts[k]);
            let start = k;
            while k < ts.len() && trading_day(MarketKind::CmeEquityIndex, ts[k]) == d {
                day_of[k] = days.len() as u32;
                k += 1;
            }
            let end = k;
            let rth_start = (start..end).find(|&x| sm[x] >= SM_RTH_OPEN && sm[x] < SM_RTH_CLOSE);
            let rth_end = (start..end).find(|&x| sm[x] >= SM_RTH_CLOSE).unwrap_or(end);
            let flat_idx = rth_start.and_then(|s| (s..rth_end).rev().find(|&x| sm[x] <= SM_FLAT));
            let mut day = Day { date: d, start, end, rth_start, rth_end, flat_idx, atr: [f64::NAN; 2], high: [f64::MIN; 2], low: [f64::MAX; 2], close: [f64::NAN; 2], rth_open: [f64::NAN; 2], rth_close: [f64::NAN; 2] };
            for (x, p) in [&pa, &pb].iter().enumerate() {
                for bar in &p[start..end] {
                    day.high[x] = day.high[x].max(bar.h);
                    day.low[x] = day.low[x].min(bar.l);
                }
                day.close[x] = p[end - 1].c;
                if let Some(s) = rth_start {
                    day.rth_open[x] = p[s].o;
                    day.rth_close[x] = p[rth_end - 1].c;
                }
            }
            days.push(day);
        }
        // Wilder ATR(14) from completed sessions; day d sees the ATR as of the end of day d-1.
        for x in 0..2 {
            let mut atr: Option<f64> = None;
            let mut seed = vec![];
            for d in 0..days.len() {
                days[d].atr[x] = atr.unwrap_or(f64::NAN);
                let (h, l) = (days[d].high[x], days[d].low[x]);
                let tr = if d == 0 { h - l } else { (h - l).max((h - days[d - 1].close[x]).abs()).max((l - days[d - 1].close[x]).abs()) };
                match atr {
                    Some(a) => atr = Some(a + (tr - a) / 14.0),
                    None => {
                        seed.push(tr);
                        if seed.len() == 14 {
                            atr = Some(seed.iter().sum::<f64>() / 14.0);
                        }
                    }
                }
            }
        }
        Pair { ts, px: [pa, pb], sm, day_of, days, cost: crate::sim::CostModel::Contract }
    }

    /// Index of the bar at session minute `sm` (or the first after it) inside day `d`.
    pub fn at_minute(&self, d: usize, sm: u16) -> Option<usize> {
        let day = &self.days[d];
        let off = self.sm[day.start..day.end].partition_point(|&m| m < sm);
        let i = day.start + off;
        (i < day.end).then_some(i)
    }

    /// Days whose date is inside `[from, to]`, with an ATR and an RTH session.
    pub fn usable_days(&self, from: NaiveDate, to: NaiveDate) -> Vec<usize> {
        (0..self.days.len())
            .filter(|&d| {
                let x = &self.days[d];
                x.date >= from && x.date <= to && x.atr.iter().all(|a| a.is_finite() && *a > 0.0) && x.rth_start.is_some() && x.flat_idx.is_some() && d > 0
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_minutes() {
        let t = |h, m| Utc.from_utc_datetime(&NaiveDate::from_ymd_opt(2024, 7, 10).unwrap().and_hms_opt(h, m, 0).unwrap());
        // EDT = UTC-4: 13:30 UTC = 09:30 ET → 930; 22:00 UTC = 18:00 ET → 0
        assert_eq!(session_minute(t(13, 30)), 930);
        assert_eq!(session_minute(t(22, 0)), 0);
        assert_eq!(session_minute(t(19, 55)), 1315);
    }

    #[test]
    fn align_and_days() {
        let mk = |ts: Vec<i64>| Series { px: ts.iter().map(|_| Ohlc { o: 1.0, h: 2.0, l: 0.5, c: 1.5 }).collect(), ts };
        // 2024-07-10 13:30 UTC = 1720618200
        let a = mk(vec![1720618200, 1720618260, 1720618320]);
        let b = mk(vec![1720618260, 1720618320, 1720618380]);
        let p = Pair::align(&a, &b);
        assert_eq!(p.ts.len(), 2);
        assert_eq!(p.days.len(), 1);
        assert_eq!(p.days[0].rth_start, Some(0));
    }
}
