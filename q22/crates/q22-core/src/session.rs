//! Trading calendars.
//!
//! CME equity-index futures (Globex): the trading day that a bar belongs to rolls at
//! **17:00 America/Chicago** (a bar at Sunday 18:00 CT belongs to Monday's session).
//! Regular trading hours (RTH) are 09:30–16:00 America/New_York. Prop firms anchor their
//! daily loss limits and end-of-day trailing drawdowns on the 17:00 CT roll, and Topstep
//! requires positions to be flat by 15:10 CT.
//!
//! Crypto: the trading day is the UTC calendar date (HyroTrader resets at 00:00 UTC).
//!
//! Exchange holidays are *data-driven*: no bars → no session. Half-days close early in the
//! data and the time-based exits simply fire on the last bar.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Timelike, Utc, Weekday};
use chrono_tz::America::{Chicago, New_York};
use chrono_tz::Tz;

use crate::instrument::MarketKind;

pub fn to_et(ts: DateTime<Utc>) -> DateTime<Tz> {
    ts.with_timezone(&New_York)
}

pub fn to_ct(ts: DateTime<Utc>) -> DateTime<Tz> {
    ts.with_timezone(&Chicago)
}

/// Minutes since local midnight in New York.
pub fn et_minute_of_day(ts: DateTime<Utc>) -> u32 {
    let t = to_et(ts);
    t.hour() * 60 + t.minute()
}

pub fn ct_minute_of_day(ts: DateTime<Utc>) -> u32 {
    let t = to_ct(ts);
    t.hour() * 60 + t.minute()
}

pub const RTH_OPEN_ET: u32 = 9 * 60 + 30;
pub const RTH_CLOSE_ET: u32 = 16 * 60;

/// The trading day a timestamp belongs to.
pub fn trading_day(kind: MarketKind, ts: DateTime<Utc>) -> NaiveDate {
    match kind {
        MarketKind::Crypto => ts.date_naive(),
        MarketKind::CmeEquityIndex => {
            let ct = to_ct(ts);
            let d = ct.date_naive();
            let d = if ct.hour() >= 17 { d + Duration::days(1) } else { d };
            // A Saturday/Sunday roll belongs to Monday's session.
            match d.weekday() {
                Weekday::Sat => d + Duration::days(2),
                Weekday::Sun => d + Duration::days(1),
                _ => d,
            }
        }
    }
}

/// Is the bar (by its open time) inside regular trading hours (ET, weekdays)?
pub fn is_rth(ts: DateTime<Utc>) -> bool {
    let et = to_et(ts);
    if matches!(et.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }
    let m = et.hour() * 60 + et.minute();
    (RTH_OPEN_ET..RTH_CLOSE_ET).contains(&m)
}

/// Minutes since the 09:30 ET open for a bar that starts at `ts` (negative before the open).
pub fn minutes_from_rth_open(ts: DateTime<Utc>) -> i64 {
    et_minute_of_day(ts) as i64 - RTH_OPEN_ET as i64
}

/// Build a UTC instant from an ET wall-clock time on a given ET date.
pub fn et_instant(date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    New_York.from_local_datetime(&date.and_time(time)).single().map(|t| t.with_timezone(&Utc))
}

pub fn ct_instant(date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    Chicago.from_local_datetime(&date.and_time(time)).single().map(|t| t.with_timezone(&Utc))
}

/// Parse "HH:MM" into minutes of day.
pub fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    (h < 24 && m < 60).then_some(h * 60 + m)
}

pub fn fmt_hhmm(m: u32) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn cme_trading_day_rolls_at_17_ct() {
        // 2026-04-01 is a Wednesday. 16:59 CT (21:59 UTC, CDT = UTC-5) belongs to Wed.
        assert_eq!(trading_day(MarketKind::CmeEquityIndex, utc("2026-04-01T21:59:00Z")), NaiveDate::from_ymd_opt(2026, 4, 1).unwrap());
        // 17:00 CT belongs to Thursday.
        assert_eq!(trading_day(MarketKind::CmeEquityIndex, utc("2026-04-01T22:00:00Z")), NaiveDate::from_ymd_opt(2026, 4, 2).unwrap());
        // Sunday 2026-04-05 18:00 CT → Monday 2026-04-06.
        assert_eq!(trading_day(MarketKind::CmeEquityIndex, utc("2026-04-05T23:00:00Z")), NaiveDate::from_ymd_opt(2026, 4, 6).unwrap());
        // Friday 2026-04-03 17:30 CT (a rare bar after the close) → Monday.
        assert_eq!(trading_day(MarketKind::CmeEquityIndex, utc("2026-04-03T22:30:00Z")), NaiveDate::from_ymd_opt(2026, 4, 6).unwrap());
    }

    #[test]
    fn rth_and_dst() {
        // Summer (EDT, UTC-4): 13:30 UTC = 09:30 ET.
        assert!(is_rth(utc("2026-07-01T13:30:00Z")));
        assert!(!is_rth(utc("2026-07-01T13:29:00Z")));
        assert!(!is_rth(utc("2026-07-01T20:00:00Z")));
        // Winter (EST, UTC-5): 14:30 UTC = 09:30 ET.
        assert!(is_rth(utc("2026-01-07T14:30:00Z")));
        assert!(!is_rth(utc("2026-01-07T14:29:00Z")));
        assert_eq!(minutes_from_rth_open(utc("2026-01-07T15:00:00Z")), 30);
        // Weekend.
        assert!(!is_rth(utc("2026-07-04T15:00:00Z")));
    }

    #[test]
    fn hhmm() {
        assert_eq!(parse_hhmm("15:10"), Some(910));
        assert_eq!(fmt_hhmm(910), "15:10");
        assert_eq!(parse_hhmm("25:00"), None);
        let t = ct_instant(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(), NaiveTime::from_hms_opt(15, 10, 0).unwrap()).unwrap();
        assert_eq!(t, utc("2026-07-01T20:10:00Z"));
    }
}
