//! Bar loading and resampling.
//!
//! Supported out of the box (auto-detected from the header):
//! * `datetime,open,high,low,close,volume` with RFC-3339 timestamps (getdata.finance, ProjectX exports)
//! * Dukascopy-style tab-separated `Time Open High Low Close Volume` (GMT)
//! * CryptoCompare `UNIX_TIMESTAMP,DATETIME,OPEN,HIGH,CLOSE,LOW,...` (columns found by name)
//! * Any CSV/TSV/SSV with a time column (`time|datetime|timestamp|date[+time]|unix_timestamp|open_time`)
//!   and OHLC(V) columns; unix seconds or milliseconds; naive timestamps are read in `naive_tz`.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

use crate::instrument::MarketKind;
use crate::session::trading_day;
use crate::types::Bar;

pub fn load_bars(path: &Path, naive_tz: Option<Tz>) -> Result<Vec<Bar>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse_bars(&text, naive_tz).with_context(|| format!("parsing {}", path.display()))
}

pub fn parse_bars(text: &str, naive_tz: Option<Tz>) -> Result<Vec<Bar>> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next().ok_or_else(|| anyhow!("empty file"))?;
    let delim = if header.contains('\t') {
        '\t'
    } else if header.contains(';') && !header.contains(',') {
        ';'
    } else {
        ','
    };
    let cols: Vec<String> = header.split(delim).map(|c| c.trim().trim_matches('"').to_ascii_lowercase()).collect();
    let find = |names: &[&str]| cols.iter().position(|c| names.contains(&c.as_str()));
    let o = find(&["open", "o"]).ok_or_else(|| anyhow!("no open column in {header:?}"))?;
    let h = find(&["high", "h"]).ok_or_else(|| anyhow!("no high column"))?;
    let l = find(&["low", "l"]).ok_or_else(|| anyhow!("no low column"))?;
    let c = find(&["close", "c", "last"]).ok_or_else(|| anyhow!("no close column"))?;
    let v = find(&["volume", "vol", "v", "volume_btc", "tick_volume"]);
    let unix = find(&["unix_timestamp", "unix", "open_time", "timestamp_ms"]);
    let datetime = find(&["datetime", "time", "timestamp", "date_time", "t", "gmt time", "local time"]);
    let date = find(&["date"]);
    enum TimeCol {
        Unix(usize),
        One(usize),
        DateTime(usize, usize),
    }
    let tcol = if let Some(u) = unix {
        TimeCol::Unix(u)
    } else if let (Some(d), Some(t)) = (date, datetime) {
        if cols[t] == "time" { TimeCol::DateTime(d, t) } else { TimeCol::One(t) }
    } else if let Some(t) = datetime {
        TimeCol::One(t)
    } else if let Some(d) = date {
        TimeCol::One(d)
    } else {
        bail!("no time column in header {header:?}");
    };

    let mut out = Vec::with_capacity(text.len() / 50);
    for (i, line) in lines.enumerate() {
        let f: Vec<&str> = line.split(delim).map(|x| x.trim().trim_matches('"')).collect();
        let get = |k: usize| f.get(k).copied().unwrap_or("");
        let ts = match tcol {
            TimeCol::Unix(k) => parse_time(get(k), naive_tz),
            TimeCol::One(k) => parse_time(get(k), naive_tz),
            TimeCol::DateTime(d, t) => parse_time(&format!("{} {}", get(d), get(t)), naive_tz),
        };
        let Some(ts) = ts else {
            if i < 3 {
                bail!("cannot parse time in line {}: {line:?}", i + 2);
            }
            continue;
        };
        let p = |k: usize| get(k).parse::<f64>().unwrap_or(f64::NAN);
        let bar = Bar { ts, open: p(o), high: p(h), low: p(l), close: p(c), volume: v.map(p).filter(|x| x.is_finite()).unwrap_or(0.0) };
        if bar.is_valid() {
            out.push(bar);
        }
    }
    out.sort_by_key(|b| b.ts);
    out.dedup_by_key(|b| b.ts);
    if out.is_empty() {
        bail!("no valid bars");
    }
    Ok(out)
}

pub fn parse_time(s: &str, naive_tz: Option<Tz>) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        let x: f64 = s.parse().ok()?;
        let secs = if x > 1e11 { x / 1000.0 } else { x };
        return Utc.timestamp_opt(secs as i64, 0).single();
    }
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    const FMTS: &[&str] = &[
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M",
        "%Y.%m.%d %H:%M:%S",
        "%d.%m.%Y %H:%M:%S%.f",
        "%Y%m%d %H%M%S",
        "%Y%m%d %H:%M:%S",
        "%m/%d/%Y %H:%M:%S",
        "%m/%d/%Y %H:%M",
    ];
    for fmt in FMTS {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, fmt) {
            return localize(n, naive_tz);
        }
    }
    for fmt in ["%Y-%m-%d", "%Y%m%d", "%m/%d/%Y"] {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            return localize(d.and_hms_opt(0, 0, 0)?, naive_tz);
        }
    }
    None
}

fn localize(n: NaiveDateTime, tz: Option<Tz>) -> Option<DateTime<Utc>> {
    match tz {
        None => Some(Utc.from_utc_datetime(&n)),
        Some(tz) => tz.from_local_datetime(&n).earliest().map(|t| t.with_timezone(&Utc)),
    }
}

/// Median spacing between consecutive bars, in minutes (used to detect the timeframe).
pub fn detect_timeframe_minutes(bars: &[Bar]) -> i64 {
    let mut d: Vec<i64> = bars.windows(2).map(|w| (w[1].ts - w[0].ts).num_minutes()).filter(|&m| m > 0).take(5000).collect();
    if d.is_empty() {
        return 1;
    }
    d.sort_unstable();
    d[d.len() / 2]
}

/// Aggregate to `minutes`-minute bars aligned on the UTC epoch (works for 1→5, 5→15, 60→240 …).
pub fn resample(bars: &[Bar], minutes: i64) -> Vec<Bar> {
    let secs = minutes * 60;
    let mut out: Vec<Bar> = Vec::with_capacity(bars.len() / minutes.max(1) as usize + 1);
    for b in bars {
        let bucket = b.ts.timestamp().div_euclid(secs) * secs;
        let ts = Utc.timestamp_opt(bucket, 0).single().unwrap();
        match out.last_mut() {
            Some(last) if last.ts == ts => {
                last.high = last.high.max(b.high);
                last.low = last.low.min(b.low);
                last.close = b.close;
                last.volume += b.volume;
            }
            _ => out.push(Bar { ts, ..*b }),
        }
    }
    out
}

/// One bar per trading day (CME roll at 17:00 CT, or UTC day for crypto). `ts` = first bar's open.
pub fn daily_bars(bars: &[Bar], kind: MarketKind) -> Vec<(NaiveDate, Bar)> {
    let mut m: BTreeMap<NaiveDate, Bar> = BTreeMap::new();
    for b in bars {
        let d = trading_day(kind, b.ts);
        m.entry(d)
            .and_modify(|x| {
                x.high = x.high.max(b.high);
                x.low = x.low.min(b.low);
                x.close = b.close;
                x.volume += b.volume;
            })
            .or_insert(*b);
    }
    m.into_iter().collect()
}

/// Keep bars in [from, to).
pub fn slice_dates(bars: &[Bar], from: Option<NaiveDate>, to: Option<NaiveDate>) -> Vec<Bar> {
    bars.iter()
        .filter(|b| from.is_none_or(|f| b.ts.date_naive() >= f) && to.is_none_or(|t| b.ts.date_naive() < t))
        .copied()
        .collect()
}

pub fn bar_end(b: &Bar, tf_minutes: i64) -> DateTime<Utc> {
    b.ts + Duration::minutes(tf_minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_getdata_format() {
        let t = "datetime,open,high,low,close,volume\n2026-04-01T18:10:00+00:00,24514.75,24524.5,24513,24516.25,1417\n2026-04-01T18:11:00+00:00,24516.5,24528.25,24514,24520.25,1118\n";
        let b = parse_bars(t, None).unwrap();
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].ts.to_rfc3339(), "2026-04-01T18:10:00+00:00");
        assert_eq!(b[1].close, 24520.25);
    }

    #[test]
    fn parses_dukascopy_tab() {
        let t = "Time\tOpen\tHigh\tLow\tClose\tVolume\n2023-02-07 00:06:00\t12488.389\t12491.419\t12488.259\t12491.239\t1\n";
        let b = parse_bars(t, None).unwrap();
        assert_eq!(b[0].ts.to_rfc3339(), "2023-02-07T00:06:00+00:00");
        assert_eq!(b[0].high, 12491.419);
    }

    #[test]
    fn parses_cryptocompare_columns_by_name() {
        let t = "UNIX_TIMESTAMP,DATETIME,OPEN,HIGH,CLOSE,LOW,VOLUME_USD,VOLUME_BTC\n1790982000,2026-10-02 23:00:00,84559.99,84590.01,84518.01,84488.0,35973632.9,425.4\n";
        let b = parse_bars(t, None).unwrap();
        assert_eq!(b[0].close, 84518.01);
        assert_eq!(b[0].low, 84488.0);
        assert_eq!(b[0].volume, 425.4);
    }

    #[test]
    fn naive_timestamps_in_given_tz() {
        let t = "Date;Time;Open;High;Low;Close;Volume\n2026-07-01;09:30:00;100;101;99;100.5;10\n";
        let b = parse_bars(t, Some(chrono_tz::America::New_York)).unwrap();
        assert_eq!(b[0].ts.to_rfc3339(), "2026-07-01T13:30:00+00:00");
    }

    #[test]
    fn resample_and_daily() {
        let mut v = vec![];
        for i in 0..10 {
            let ts = Utc.timestamp_opt(1_700_000_000 - 1_700_000_000 % 300 + i * 60, 0).unwrap();
            v.push(Bar { ts, open: 1.0 + i as f64, high: 2.0 + i as f64, low: 0.5, close: 1.5 + i as f64, volume: 1.0 });
        }
        let r = resample(&v, 5);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].open, 1.0);
        assert_eq!(r[0].close, 5.5);
        assert_eq!(r[0].high, 6.0);
        assert_eq!(r[0].volume, 5.0);
        assert_eq!(detect_timeframe_minutes(&v), 1);
        assert_eq!(daily_bars(&v, MarketKind::Crypto).len(), 1);
    }
}
