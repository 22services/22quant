//! Per-instrument market state: daily features (for the regime), the current session
//! (open, VWAP, high/low, previous close) and the time-of-day "noise" profile used by the
//! intraday-momentum strategy. Updated once per completed bar.

use std::collections::{BTreeMap, VecDeque};

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

use q22_core::instrument::MarketKind;
use q22_core::session::{is_rth, minutes_from_rth_open, to_et, trading_day};
use q22_core::ta::{Adx, Atr, EfficiencyRatio, Ema, Rolling, SessionVwap};
use q22_core::Bar;

use crate::regime::{classify, Regime, RegimeFeatures, RegimeThresholds};

#[derive(Clone, Copy, Debug, Serialize)]
pub struct DayBar {
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionState {
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub last: f64,
    pub bars: usize,
    #[serde(skip)]
    pub vwap: SessionVwap,
    /// Previous completed session close (None on the first session).
    pub prev_close: Option<f64>,
}

impl SessionState {
    pub fn vwap(&self) -> Option<f64> {
        self.vwap.vwap()
    }
    pub fn vwap_std(&self) -> Option<f64> {
        self.vwap.std()
    }
}

pub struct MarketState {
    pub kind: MarketKind,
    pub tf_minutes: i64,
    pub thresholds: RegimeThresholds,
    // ---- daily layer (completed sessions only)
    pub days: Vec<DayBar>,
    atr_d: Atr,
    adx_d: Adx,
    er_d: EfficiencyRatio,
    ema20_d: Ema,
    vol_hist: Rolling,
    pub features: RegimeFeatures,
    pub regime: Regime,
    // ---- session layer
    pub session: Option<SessionState>,
    finalized: bool,
    /// True on the first bar of a new session (strategies use it for daily decisions).
    pub new_session: bool,
    // ---- intraday layer (session bars only)
    pub atr_bar: Atr,
    pub er_bar: EfficiencyRatio,
    // ---- noise profile: |close/open − 1| at each minute-offset (bar end) over the last N sessions
    noise: BTreeMap<i64, Rolling>,
    today_moves: Vec<(i64, f64)>,
    pub noise_lookback: usize,
    // ---- recent bars for charts
    pub recent: VecDeque<Bar>,
    pub last_bar_end: Option<DateTime<Utc>>,
}

impl MarketState {
    pub fn new(kind: MarketKind, tf_minutes: i64, thresholds: RegimeThresholds) -> Self {
        Self {
            kind,
            tf_minutes,
            thresholds,
            days: Vec::new(),
            atr_d: Atr::new(14),
            adx_d: Adx::new(14),
            er_d: EfficiencyRatio::new(10),
            ema20_d: Ema::new(20),
            vol_hist: Rolling::new(252),
            features: RegimeFeatures::default(),
            regime: Regime::Unknown,
            session: None,
            finalized: true,
            new_session: false,
            atr_bar: Atr::new(14),
            er_bar: EfficiencyRatio::new(12),
            noise: BTreeMap::new(),
            today_moves: Vec::new(),
            noise_lookback: 14,
            recent: VecDeque::with_capacity(600),
            last_bar_end: None,
        }
    }

    /// Does this bar belong to the tradable session for this market?
    pub fn in_session(&self, ts: DateTime<Utc>) -> bool {
        match self.kind {
            MarketKind::CmeEquityIndex => is_rth(ts),
            MarketKind::Crypto => true,
        }
    }

    fn session_date(&self, ts: DateTime<Utc>) -> NaiveDate {
        match self.kind {
            MarketKind::CmeEquityIndex => to_et(ts).date_naive(),
            MarketKind::Crypto => trading_day(MarketKind::Crypto, ts),
        }
    }

    /// Minutes from the session open to the **end** of this bar.
    pub fn offset_end(&self, b: &Bar) -> i64 {
        match self.kind {
            MarketKind::CmeEquityIndex => minutes_from_rth_open(b.ts) + self.tf_minutes,
            MarketKind::Crypto => {
                let secs = b.ts.timestamp().rem_euclid(86_400);
                secs / 60 + self.tf_minutes
            }
        }
    }

    /// Average absolute move from the open at `offset` (bar end) over prior sessions.
    pub fn noise_sigma(&self, offset: i64) -> Option<f64> {
        let r = self.noise.get(&offset)?;
        (r.len() >= self.noise_lookback.min(10)).then(|| r.mean().unwrap())
    }

    pub fn update(&mut self, b: &Bar) {
        self.new_session = false;
        self.last_bar_end = Some(b.ts + chrono::Duration::minutes(self.tf_minutes));
        if self.recent.len() >= 600 {
            self.recent.pop_front();
        }
        self.recent.push_back(*b);
        if !self.in_session(b.ts) {
            // first post-session bar closes the session
            if !self.finalized {
                self.finalize_session();
            }
            return;
        }
        let date = self.session_date(b.ts);
        let is_new = self.session.as_ref().is_none_or(|s| s.date != date);
        if is_new {
            if !self.finalized {
                self.finalize_session();
            }
            let prev_close = self.days.last().map(|d| d.close);
            self.session = Some(SessionState { date, open: b.open, high: b.high, low: b.low, last: b.close, bars: 0, vwap: SessionVwap::default(), prev_close });
            self.finalized = false;
            self.new_session = true;
            self.today_moves.clear();
            // gap check for the regime (known at the open)
            let mut f = self.features.clone();
            f.gap_atr = match (prev_close, f.atr_d) {
                (Some(pc), Some(a)) if a > 0.0 => Some((b.open - pc) / a),
                _ => None,
            };
            self.regime = classify(&f, &self.thresholds);
            self.features = f;
        }
        let s = self.session.as_mut().unwrap();
        s.high = s.high.max(b.high);
        s.low = s.low.min(b.low);
        s.last = b.close;
        s.bars += 1;
        s.vwap.update(b);
        let open = s.open;
        self.atr_bar.update(b);
        self.er_bar.update(b.close);
        let off = self.offset_end(b);
        self.today_moves.push((off, (b.close / open - 1.0).abs()));
    }

    /// Close the current session: append the daily bar, update daily indicators and the noise
    /// profile, and pre-compute tomorrow's regime features (the gap is added at the open).
    pub fn finalize_session(&mut self) {
        self.finalized = true;
        let Some(s) = self.session.as_ref() else { return };
        let d = DayBar { date: s.date, open: s.open, high: s.high, low: s.low, close: s.last };
        // ignore truncated sessions (holiday half-days are fine, a 3-bar fragment is not)
        let min_bars = match self.kind {
            MarketKind::CmeEquityIndex => (120 / self.tf_minutes).max(2) as usize,
            MarketKind::Crypto => (360 / self.tf_minutes).max(1) as usize,
        };
        if s.bars < min_bars {
            return;
        }
        self.days.push(d);
        let db = Bar { ts: Utc::now(), open: d.open, high: d.high, low: d.low, close: d.close, volume: 0.0 };
        let atr = self.atr_d.update(&db);
        let adx = self.adx_d.update(&db);
        let er = self.er_d.update(d.close);
        let ema = self.ema20_d.update(d.close);
        let atr_pct = atr / d.close;
        let vol_pct = if self.vol_hist.len() >= 60 { self.vol_hist.percentile_rank(atr_pct) } else { None };
        self.vol_hist.push(atr_pct);
        for (off, v) in self.today_moves.drain(..) {
            self.noise.entry(off).or_insert_with(|| Rolling::new(self.noise_lookback)).push(v);
        }
        self.features = RegimeFeatures {
            days: self.days.len(),
            atr_d: self.atr_d.value(),
            atr_pct_of_price: Some(atr_pct),
            vol_percentile: vol_pct,
            adx_d: adx,
            er_d: er,
            trend_bias: Some(d.close - ema),
            gap_atr: None,
        };
    }

    pub fn daily_closes(&self) -> impl Iterator<Item = f64> + '_ {
        self.days.iter().map(|d| d.close)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    /// Synthetic RTH-only 5-minute bars, 09:30–16:00 ET (13:30–20:00 UTC in summer).
    pub fn synthetic_days(n_days: usize, drift_per_bar: f64) -> Vec<Bar> {
        let mut out = vec![];
        let mut px = 100.0;
        let start = Utc.with_ymd_and_hms(2025, 6, 2, 13, 30, 0).unwrap();
        let mut day = 0;
        let mut d = 0;
        while day < n_days {
            let date = start + Duration::days(d);
            d += 1;
            let wd = chrono::Datelike::weekday(&date);
            if matches!(wd, chrono::Weekday::Sat | chrono::Weekday::Sun) {
                continue;
            }
            for k in 0..78 {
                let ts = date + Duration::minutes(5 * k);
                let wiggle = if k % 2 == 0 { 0.1 } else { -0.1 };
                let o = px;
                px += drift_per_bar + wiggle;
                out.push(Bar { ts, open: o, high: o.max(px) + 0.05, low: o.min(px) - 0.05, close: px, volume: 100.0 });
            }
            day += 1;
        }
        out
    }

    #[test]
    fn sessions_and_regime() {
        let bars = synthetic_days(60, 0.05);
        let mut m = MarketState::new(MarketKind::CmeEquityIndex, 5, RegimeThresholds::default());
        let mut sessions = 0;
        for b in &bars {
            m.update(b);
            if m.new_session {
                sessions += 1;
            }
        }
        m.finalize_session();
        assert_eq!(sessions, 60);
        assert_eq!(m.days.len(), 60);
        assert!(m.noise_sigma(30).is_some());
        assert!(matches!(m.regime, Regime::TrendingUp), "steady drift must classify as trending up, got {:?}", m.regime);
        let s = m.session.as_ref().unwrap();
        assert!(s.vwap().unwrap() > s.open);
    }
}
