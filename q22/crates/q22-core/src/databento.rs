//! Databento `ohlcv-1m` CSV (GLBX.MDP3, all contracts) → one continuous front-month series.
//!
//! * Calendar spreads (`ESM0-ESU0`) are dropped; only outrights are kept.
//! * **Roll by volume, without look-ahead:** the session of day *d* trades the contract that
//!   had the most volume on day *d − 1*, and the roll only ever moves to a later expiry.
//! * **Panama (difference) back-adjustment:** at each roll the gap between the new and the old
//!   contract is measured on the last minute of the previous session where both traded, and
//!   every earlier bar is shifted by the sum of the later gaps. Point differences — and so the
//!   dollar P&L of any trade that does not span a roll — are exactly those of the traded
//!   contract; the most recent segment carries real prices.
//!
//! Databento's `ts_event` for OHLCV bars is the bar's open time (UTC), which is q22's convention.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, Write};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, NaiveDate, Utc};

use crate::instrument::MarketKind;
use crate::session::trading_day;
use crate::types::Bar;

#[derive(Clone, Debug)]
pub struct Roll {
    /// First session traded on the new contract.
    pub day: NaiveDate,
    pub from: String,
    pub to: String,
    /// new − old, measured at `at`.
    pub gap: f64,
    /// new ÷ old, measured at `at`.
    pub ratio: f64,
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct ContinuousBar {
    pub bar: Bar,
    pub contract: String,
    /// Panama: points added (raw = adjusted − adj). Ratio: factor applied (raw = adjusted ÷ adj).
    pub adj: f64,
}

#[derive(Debug, Default)]
pub struct ConvertStats {
    pub rows: usize,
    pub spread_rows: usize,
    pub other_root_rows: usize,
    pub contracts: usize,
    pub sessions: usize,
}

const MONTHS: &str = "FGHJKMNQUVXZ";

/// `ESM0` → ("ES", 6, "0"). Databento's year is a single digit, so `ESM0` is June 2010 *and*
/// June 2020 — see [`expiry_year`].
fn split_symbol(sym: &str) -> Option<(&str, u32, &str)> {
    let n = sym.len();
    if n < 3 || sym.contains('-') || !sym.as_bytes()[n - 1].is_ascii_digit() {
        return None;
    }
    let digits = sym.bytes().rev().take_while(|b| b.is_ascii_digit()).count();
    let mpos = n - digits - 1;
    let m = MONTHS.find(sym.as_bytes()[mpos] as char)? as u32 + 1;
    Some((&sym[..mpos], m, &sym[mpos + 1..]))
}

/// Expiry year of a contract seen trading at `ts`: the first year ≥ the bar's year whose last
/// digit(s) match the symbol (contracts are listed at most a few years ahead).
fn expiry_year(year_digits: &str, ts: DateTime<Utc>) -> Option<i32> {
    let y = chrono::Datelike::year(&ts.date_naive());
    let d: i32 = year_digits.parse().ok()?;
    let modulus = 10_i32.pow(year_digits.len() as u32);
    Some(y + (d - y).rem_euclid(modulus))
}

/// Canonical contract name with a 4-digit year, e.g. `ESM2020`.
fn canonical(sym: &str, ts: DateTime<Utc>) -> Option<String> {
    let (root, m, yd) = split_symbol(sym)?;
    Some(format!("{root}{}{}", &MONTHS[(m - 1) as usize..m as usize], expiry_year(yd, ts)?))
}

/// Read every outright of `root` (e.g. "NQ") from a Databento OHLCV CSV.
pub fn read_contracts(reader: impl BufRead, root: &str) -> Result<(HashMap<String, Vec<Bar>>, ConvertStats)> {
    let mut lines = reader.lines();
    let header = lines.next().ok_or_else(|| anyhow!("empty file"))??;
    let cols: Vec<&str> = header.split(',').collect();
    let idx = |name: &str| cols.iter().position(|c| *c == name).ok_or_else(|| anyhow!("column {name} missing in {header:?}"));
    let (its, io, ih, il, ic, iv, isym) = (idx("ts_event")?, idx("open")?, idx("high")?, idx("low")?, idx("close")?, idx("volume")?, idx("symbol")?);
    let mut out: HashMap<String, Vec<Bar>> = HashMap::new();
    let mut st = ConvertStats::default();
    for (n, line) in lines.enumerate() {
        let line = line?;
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < cols.len() {
            continue;
        }
        st.rows += 1;
        let sym = f[isym];
        if sym.contains('-') {
            st.spread_rows += 1;
            continue;
        }
        match split_symbol(sym) {
            Some((r, _, _)) if r == root => {}
            _ => {
                st.other_root_rows += 1;
                continue;
            }
        }
        let ts = DateTime::parse_from_rfc3339(f[its]).with_context(|| format!("line {}: bad ts {:?}", n + 2, f[its]))?.with_timezone(&Utc);
        let p = |k: usize| f[k].parse::<f64>().unwrap_or(f64::NAN);
        let bar = Bar { ts, open: p(io), high: p(ih), low: p(il), close: p(ic), volume: p(iv) };
        if bar.is_valid() {
            let key = canonical(sym, ts).ok_or_else(|| anyhow!("line {}: bad symbol {sym:?}", n + 2))?;
            out.entry(key).or_default().push(bar);
        }
    }
    for v in out.values_mut() {
        v.sort_by_key(|b| b.ts);
        v.dedup_by_key(|b| b.ts);
    }
    st.contracts = out.len();
    Ok((out, st))
}

/// Build the continuous series. Returns the bars (with the contract each came from) and the rolls.
/// How rolls are stitched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adjust {
    /// Add the roll gap to earlier bars: exact point (dollar) P&L, distorted percentages far back.
    Panama,
    /// Multiply earlier bars by the roll ratio: exact percentage moves, prices proportional to the
    /// traded contracts (use with costs in basis points for long-history research).
    Ratio,
}

pub fn build_continuous(contracts: &HashMap<String, Vec<Bar>>, mode: Adjust) -> Result<(Vec<ContinuousBar>, Vec<Roll>, usize)> {
    let kind = MarketKind::CmeEquityIndex;
    // expiry order from canonical names (ESM2020 → (2020, 6))
    let mut expiry: HashMap<&str, (i32, u32)> = HashMap::new();
    for sym in contracts.keys() {
        let (_, m, y) = split_symbol(sym).ok_or_else(|| anyhow!("bad symbol {sym}"))?;
        let y: i32 = y.parse().map_err(|_| anyhow!("bad year in {sym}"))?;
        expiry.insert(sym.as_str(), (y, m));
    }
    // per-session volume and bars
    let mut vol: BTreeMap<NaiveDate, HashMap<&str, f64>> = BTreeMap::new();
    let mut by_day: HashMap<(&str, NaiveDate), (usize, usize)> = HashMap::new(); // index range in contract vec
    for (sym, bars) in contracts {
        let mut start = 0;
        for i in 0..bars.len() {
            let d = trading_day(kind, bars[i].ts);
            *vol.entry(d).or_default().entry(sym.as_str()).or_default() += bars[i].volume;
            let next_d = bars.get(i + 1).map(|b| trading_day(kind, b.ts));
            if next_d != Some(d) {
                by_day.insert((sym.as_str(), d), (start, i + 1));
                start = i + 1;
            }
        }
    }
    let days: Vec<NaiveDate> = vol.keys().copied().collect();
    if days.is_empty() {
        return Err(anyhow!("no sessions"));
    }
    let mut current: &str = vol[&days[0]].iter().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map(|(s, _)| *s).unwrap();
    let mut segments: Vec<(NaiveDate, &str)> = vec![(days[0], current)];
    let mut rolls: Vec<Roll> = vec![];
    for w in days.windows(2) {
        let (prev, day) = (w[0], w[1]);
        let pv = &vol[&prev];
        let cur_v = pv.get(current).copied().unwrap_or(0.0);
        let later = pv.iter().filter(|(s, v)| expiry[**s] > expiry[current] && **v > cur_v).max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map(|(s, _)| *s);
        if let Some(next) = later {
            // gap on the last minute of `prev` where both traded
            let (a0, a1) = by_day.get(&(current, prev)).copied().unwrap_or((0, 0));
            let (b0, b1) = by_day.get(&(next, prev)).copied().unwrap_or((0, 0));
            let old = &contracts[current][a0..a1];
            let new = &contracts[next][b0..b1];
            let new_at: HashMap<DateTime<Utc>, f64> = new.iter().map(|b| (b.ts, b.close)).collect();
            let common = old.iter().rev().find_map(|b| new_at.get(&b.ts).map(|nc| (b.ts, *nc - b.close, *nc / b.close)));
            let (at, gap, ratio) = match common {
                Some(x) => x,
                None => match (old.last(), new.last()) {
                    (Some(o), Some(n)) => (o.ts.max(n.ts), n.close - o.close, n.close / o.close),
                    _ => return Err(anyhow!("roll {current}→{next} on {day}: no overlapping bars on {prev}")),
                },
            };
            rolls.push(Roll { day, from: current.to_string(), to: next.to_string(), gap, ratio, at });
            current = next;
            segments.push((day, current));
        }
    }
    // segment k: Panama adds the sum of every later gap, Ratio multiplies by every later ratio
    let panama = mode == Adjust::Panama;
    let mut offsets = vec![if panama { 0.0 } else { 1.0 }; segments.len()];
    for k in (0..segments.len().saturating_sub(1)).rev() {
        offsets[k] = if panama { offsets[k + 1] + rolls[k].gap } else { offsets[k + 1] * rolls[k].ratio };
    }
    let mut out = Vec::new();
    let mut seg = 0;
    let mut sessions = 0;
    for &d in &days {
        while seg + 1 < segments.len() && segments[seg + 1].0 <= d {
            seg += 1;
        }
        let sym = segments[seg].1;
        if let Some(&(a, b)) = by_day.get(&(sym, d)) {
            sessions += 1;
            let off = offsets[seg];
            for bar in &contracts[sym][a..b] {
                let f = |x: f64| if panama { x + off } else { x * off };
                out.push(ContinuousBar { bar: Bar { ts: bar.ts, open: f(bar.open), high: f(bar.high), low: f(bar.low), close: f(bar.close), volume: bar.volume }, contract: sym.to_string(), adj: off });
            }
        }
    }
    Ok((out, rolls, sessions))
}

/// `unix_timestamp,open,high,low,close,volume,contract,offset|factor` — readable by [`crate::data::parse_bars`].
pub fn write_continuous(w: &mut impl Write, bars: &[ContinuousBar], mode: Adjust) -> Result<()> {
    writeln!(w, "unix_timestamp,open,high,low,close,volume,contract,{}", if mode == Adjust::Panama { "offset" } else { "factor" })?;
    for cb in bars {
        let b = &cb.bar;
        writeln!(w, "{},{},{},{},{},{},{},{}", b.ts.timestamp(), b.open, b.high, b.low, b.close, b.volume, cb.contract, cb.adj)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn bars(start: DateTime<Utc>, n: usize, px: f64, vol: f64) -> Vec<Bar> {
        (0..n).map(|i| Bar { ts: start + Duration::minutes(i as i64), open: px, high: px + 1.0, low: px - 1.0, close: px, volume: vol }).collect()
    }

    #[test]
    fn symbols_parse() {
        assert_eq!(split_symbol("ESM0"), Some(("ES", 6, "0")));
        assert_eq!(split_symbol("NQZ25"), Some(("NQ", 12, "25")));
        let t = |y, m| Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0).unwrap();
        assert_eq!(canonical("ESM0", t(2010, 6)).as_deref(), Some("ESM2010"));
        assert_eq!(canonical("ESM0", t(2019, 9)).as_deref(), Some("ESM2020"), "same symbol a decade later");
        assert_eq!(canonical("ESH1", t(2010, 11)).as_deref(), Some("ESH2011"));
        assert_eq!(canonical("ESZ9", t(2019, 12)).as_deref(), Some("ESZ2019"));
        assert_eq!(split_symbol("ESM0-ESU0"), None);
    }

    #[test]
    fn rolls_on_prior_day_volume_and_back_adjusts() {
        // day 1 (Tue 2026-06-09 session): old dominates; day 2: new dominates; day 3 must be on new
        let d1 = Utc.with_ymd_and_hms(2026, 6, 9, 13, 30, 0).unwrap();
        let d2 = d1 + Duration::days(1);
        let d3 = d2 + Duration::days(1);
        let mut c: HashMap<String, Vec<Bar>> = HashMap::new();
        let mut old = bars(d1, 10, 100.0, 50.0);
        old.extend(bars(d2, 10, 101.0, 10.0));
        old.extend(bars(d3, 10, 102.0, 5.0));
        let mut new = bars(d1, 10, 104.0, 5.0);
        new.extend(bars(d2, 10, 105.0, 60.0));
        new.extend(bars(d3, 10, 106.0, 70.0));
        // make the old contract expire later in the year than its data ends? (June = M)
        c.insert("ESM2026".into(), old);
        c.insert("ESU2026".into(), new);
        let (out, rolls, sessions) = build_continuous(&c, Adjust::Panama).unwrap();
        let (outr, _, _) = build_continuous(&c, Adjust::Ratio).unwrap();
        // ratio: earlier bars × 105/101, the latest segment untouched
        assert!((outr[10].bar.close - 105.0).abs() < 1e-9 && (outr[20].bar.close - 106.0).abs() < 1e-9);
        assert!((outr[0].bar.close - 100.0 * 105.0 / 101.0).abs() < 1e-9 && (outr[0].adj - 105.0 / 101.0).abs() < 1e-12);
        assert_eq!(sessions, 3);
        assert_eq!(rolls.len(), 1);
        assert_eq!(rolls[0].day, trading_day(MarketKind::CmeEquityIndex, d3), "roll uses the previous session's volume");
        assert!((rolls[0].gap - 4.0).abs() < 1e-9);
        // days 1-2 from ESM6 shifted +4, day 3 raw ESU6
        assert!((out[0].bar.close - 104.0).abs() < 1e-9 && out[0].contract == "ESM2026");
        assert!((out[10].bar.close - 105.0).abs() < 1e-9);
        assert!((out[20].bar.close - 106.0).abs() < 1e-9 && out[20].contract == "ESU2026");
    }
}
