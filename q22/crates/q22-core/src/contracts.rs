//! CME equity-index contract calendar: which quarterly contract is "front" on a date.
//!
//! Index futures (ES, NQ, YM, RTY and their micros) list March/June/September/December
//! contracts that expire on the **third Friday** of the month. Volume moves to the next
//! contract about 8 days earlier (the Thursday of the week before expiry), which is the roll
//! date used here — the same behaviour as the volume roll in [`crate::databento`].

use chrono::{Datelike, Duration, NaiveDate, Weekday};

const CODES: [(u32, char); 4] = [(3, 'H'), (6, 'M'), (9, 'U'), (12, 'Z')];

/// Third Friday of a month.
pub fn third_friday(year: i32, month: u32) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
    let to_friday = (Weekday::Fri.num_days_from_monday() + 7 - first.weekday().num_days_from_monday()) % 7;
    first + Duration::days(to_friday as i64 + 14)
}

/// Roll date (first session on the next contract) for the quarterly contract expiring in `month`.
pub fn roll_date(year: i32, month: u32) -> NaiveDate {
    third_friday(year, month) - Duration::days(8)
}

/// Front quarterly contract for `root` on `date`, as `(month code, year)`.
pub fn front_quarterly(date: NaiveDate) -> (char, i32) {
    let mut y = date.year();
    loop {
        for (m, c) in CODES {
            if date < roll_date(y, m) {
                return (c, y);
            }
        }
        y += 1;
    }
}

/// Exchange-style symbol with a one-digit year (`MNQZ6`), as Tradovate and Rithmic spell it.
pub fn front_symbol(root: &str, date: NaiveDate) -> String {
    let (c, y) = front_quarterly(date);
    format!("{root}{c}{}", y.rem_euclid(10))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn third_fridays() {
        assert_eq!(third_friday(2026, 9), d(2026, 9, 18));
        assert_eq!(third_friday(2026, 12), d(2026, 12, 18));
        assert_eq!(third_friday(2027, 3), d(2027, 3, 19));
    }

    #[test]
    fn rolls_eight_days_before_expiry() {
        // Sep-2026 contract expires 2026-09-18 → roll on Thursday 2026-09-10
        assert_eq!(front_symbol("MNQ", d(2026, 9, 9)), "MNQU6");
        assert_eq!(front_symbol("MNQ", d(2026, 9, 10)), "MNQZ6");
        assert_eq!(front_symbol("ES", d(2026, 12, 20)), "ESH7");
        // the Databento volume roll put NQ on the September 2026 contract from 2026-06-16
        assert_eq!(front_symbol("NQ", d(2026, 6, 16)), "NQU6");
    }
}
