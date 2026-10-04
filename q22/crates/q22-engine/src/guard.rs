//! Compliance guard — the part that keeps the account alive *and* un-banned.
//!
//! Every entry passes [`ComplianceGuard::check_entry`]; every bar (and every few seconds in
//! live mode) [`ComplianceGuard::continuous`] may force a flatten or a halt. Each decision is
//! returned with a human-readable reason that the engine logs and the dashboard displays.
//!
//! Rules enforced (all configurable, defaults are stricter than the firms'):
//! * session window for new entries, forced flatten before the firm's cut-off;
//! * personal daily loss stop below the firm's limit; buffer to the max-loss threshold;
//! * consistency-aware daily profit lock (stop for the day before one day becomes too big);
//! * max trades per day, max consecutive losses, cool-down between entries;
//! * order-rate limiter (anti-HFT) and minimum hold time (anti micro-scalping);
//! * scheduled-news blackout windows;
//! * one net direction per correlated group (no hedging NQ vs ES);
//! * max contracts (firm and personal), max leverage, max risk per trade;
//! * operator heartbeat ("actively monitored" automation): no heartbeat → no new entries.

use std::collections::{HashMap, VecDeque};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use q22_core::instrument::MarketKind;
use q22_core::session::{ct_minute_of_day, et_minute_of_day, fmt_hhmm, parse_hhmm};
use q22_core::{InstrumentSpec, Side};

use crate::prop::PropTracker;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GuardConfig {
    /// New entries only inside [entry_start_et, entry_end_et] (CME futures). "HH:MM" New York.
    pub entry_start_et: String,
    pub entry_end_et: String,
    /// Flatten everything at this New York time (also capped by the firm's flat_by_ct − 2 min).
    pub flatten_at_et: String,
    /// Personal daily loss stop in USD (None → 30% of the firm's max loss, capped by the firm's DLL × 0.8).
    pub daily_loss_stop: Option<f64>,
    /// Stop opening trades when the buffer to the max-loss threshold falls below this fraction of max loss.
    pub min_buffer_frac: f64,
    /// Fraction of the consistency cap at which the day is locked (0.8 → stop at 80% of the allowed
    /// best day; 0 → no lock, a big day simply raises the target on share-of-total rules).
    pub consistency_lock_frac: f64,
    pub max_trades_per_day: u32,
    pub max_consecutive_losses: u32,
    pub min_seconds_between_entries: i64,
    pub max_orders_per_minute: usize,
    pub min_hold_seconds: i64,
    pub max_contracts_personal: Option<f64>,
    /// Risk per trade in USD (prop futures) — the sizing budget before multipliers.
    pub risk_per_trade_usd: Option<f64>,
    /// Risk per trade as a fraction of equity (crypto / personal).
    pub risk_per_trade_frac: Option<f64>,
    /// Fraction of the remaining buffer to the threshold that a single trade may risk.
    pub max_buffer_risk_frac: f64,
    /// News blackout: no new entries from `before` minutes before to `after` minutes after each event.
    pub news_events: Vec<DateTime<Utc>>,
    pub news_before_min: i64,
    pub news_after_min: i64,
    pub flatten_before_news: bool,
    /// Live only: no new entries if the dashboard/operator heartbeat is older than this.
    pub operator_heartbeat_s: Option<i64>,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self {
            entry_start_et: "09:35".into(),
            entry_end_et: "15:45".into(),
            flatten_at_et: "15:58".into(),
            daily_loss_stop: None,
            min_buffer_frac: 0.25,
            consistency_lock_frac: 0.8,
            max_trades_per_day: 6,
            max_consecutive_losses: 3,
            min_seconds_between_entries: 120,
            max_orders_per_minute: 20,
            min_hold_seconds: 30,
            max_contracts_personal: None,
            risk_per_trade_usd: None,
            risk_per_trade_frac: Some(0.005),
            max_buffer_risk_frac: 0.15,
            news_events: vec![],
            news_before_min: 5,
            news_after_min: 5,
            flatten_before_news: false,
            operator_heartbeat_s: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub enum GuardAction {
    FlattenAll(String),
    HaltDay(String),
    HaltAccount(String),
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DayCounters {
    pub trades: u32,
    pub consecutive_losses: u32,
    pub last_entry: Option<DateTime<Utc>>,
    pub halted: Option<String>,
    pub profit_locked: bool,
}

pub struct ComplianceGuard {
    pub cfg: GuardConfig,
    entry_start: u32,
    entry_end: u32,
    flatten_at: u32,
    pub day: DayCounters,
    orders: VecDeque<DateTime<Utc>>,
    pub account_halted: Option<String>,
    pub last_heartbeat: Option<DateTime<Utc>>,
    /// Net direction per correlated group: group -> (side, symbol)
    pub group_exposure: HashMap<String, (Side, String)>,
}

impl ComplianceGuard {
    pub fn new(cfg: GuardConfig, firm_flat_by_ct: Option<&str>) -> Self {
        let entry_start = parse_hhmm(&cfg.entry_start_et).unwrap_or(9 * 60 + 35);
        let entry_end = parse_hhmm(&cfg.entry_end_et).unwrap_or(15 * 60 + 45);
        let mut flatten_at = parse_hhmm(&cfg.flatten_at_et).unwrap_or(15 * 60 + 58);
        // Firm cut-off is in Chicago time; ET = CT + 60 min on every trading day.
        if let Some(ct) = firm_flat_by_ct.and_then(parse_hhmm) {
            flatten_at = flatten_at.min(ct + 60 - 2);
        }
        Self { cfg, entry_start, entry_end, flatten_at, day: DayCounters::default(), orders: VecDeque::new(), account_halted: None, last_heartbeat: None, group_exposure: HashMap::new() }
    }

    pub fn flatten_minute_et(&self) -> u32 {
        self.flatten_at
    }

    pub fn new_day(&mut self) {
        self.day = DayCounters::default();
    }

    pub fn daily_loss_stop(&self, acct: &PropTracker) -> f64 {
        let default = acct.rules.max_loss * 0.30;
        let mut x = self.cfg.daily_loss_stop.unwrap_or(default);
        if let Some(dll) = acct.rules.daily_loss_limit {
            x = x.min(dll * 0.8);
        }
        x
    }

    /// Consistency-aware cap on today's profit (None = no cap): a fraction of the largest day the
    /// firm's consistency rule allows without raising the profit target.
    pub fn profit_lock(&self, acct: &PropTracker) -> Option<f64> {
        if self.cfg.consistency_lock_frac <= 0.0 {
            return None;
        }
        acct.consistency_day_cap().map(|c| c * self.cfg.consistency_lock_frac)
    }

    pub fn note_order(&mut self, now: DateTime<Utc>) {
        self.orders.push_back(now);
        while self.orders.front().is_some_and(|t| now - *t > Duration::seconds(60)) {
            self.orders.pop_front();
        }
    }

    pub fn order_budget_ok(&mut self, now: DateTime<Utc>) -> bool {
        while self.orders.front().is_some_and(|t| now - *t > Duration::seconds(60)) {
            self.orders.pop_front();
        }
        self.orders.len() < self.cfg.max_orders_per_minute
    }

    pub fn in_news_blackout(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.cfg
            .news_events
            .iter()
            .find(|e| now >= **e - Duration::minutes(self.cfg.news_before_min) && now <= **e + Duration::minutes(self.cfg.news_after_min))
            .copied()
    }

    pub fn record_trade_closed(&mut self, pnl: f64) {
        if pnl < 0.0 {
            self.day.consecutive_losses += 1;
        } else {
            self.day.consecutive_losses = 0;
        }
        if self.day.consecutive_losses >= self.cfg.max_consecutive_losses {
            self.day.halted = Some(format!("{} consecutive losses", self.day.consecutive_losses));
        }
    }

    /// Checks before any new entry. `Err(reason)` blocks the trade.
    #[allow(clippy::too_many_arguments)]
    pub fn check_entry(
        &mut self,
        now: DateTime<Utc>,
        kind: MarketKind,
        spec: &InstrumentSpec,
        side: Side,
        holds_overnight: bool,
        acct: &PropTracker,
        live: bool,
    ) -> Result<(), String> {
        if let Some(r) = &self.account_halted {
            return Err(format!("account halted: {r}"));
        }
        if !acct.is_active() {
            return Err(format!("account not active: {:?}", acct.status));
        }
        if let Some(r) = &self.day.halted {
            return Err(format!("day halted: {r}"));
        }
        if acct.day_halted {
            return Err("firm daily loss limit reached".into());
        }
        if kind == MarketKind::CmeEquityIndex {
            let m = et_minute_of_day(now);
            if m < self.entry_start || m > self.entry_end {
                return Err(format!("outside entry window {}–{} ET", fmt_hhmm(self.entry_start), fmt_hhmm(self.entry_end)));
            }
            if m >= self.flatten_at {
                return Err("past flatten time".into());
            }
            if let Some(ct) = acct.rules.flat_by_ct.as_deref().and_then(parse_hhmm) {
                if ct_minute_of_day(now) + 5 >= ct && ct_minute_of_day(now) < 17 * 60 {
                    return Err("within 5 minutes of the firm's mandatory flat time".into());
                }
            }
        }
        if holds_overnight && !acct.rules.overnight_allowed {
            return Err("strategy holds overnight but the firm requires flat each day".into());
        }
        if let Some(e) = self.in_news_blackout(now) {
            return Err(format!("news blackout around {}", e.format("%Y-%m-%d %H:%M UTC")));
        }
        if self.day.trades >= self.cfg.max_trades_per_day {
            return Err(format!("max {} trades per day reached", self.cfg.max_trades_per_day));
        }
        if let Some(last) = self.day.last_entry {
            if (now - last).num_seconds() < self.cfg.min_seconds_between_entries {
                return Err("cool-down between entries".into());
            }
        }
        if !self.order_budget_ok(now) {
            return Err("order-rate limit (anti-HFT)".into());
        }
        let dls = self.daily_loss_stop(acct);
        if -acct.day_pnl() >= dls {
            self.day.halted = Some(format!("personal daily loss stop {dls:.0} reached"));
            return Err(self.day.halted.clone().unwrap());
        }
        if let Some(lock) = self.profit_lock(acct) {
            if acct.day_pnl() >= lock {
                self.day.profit_locked = true;
                return Err(format!("consistency lock: day P&L {:.0} ≥ {:.0}", acct.day_pnl(), lock));
            }
        }
        if acct.buffer() < self.cfg.min_buffer_frac * acct.rules.max_loss {
            return Err(format!("buffer to max-loss threshold {:.0} below {:.0}%", acct.buffer(), self.cfg.min_buffer_frac * 100.0));
        }
        if let Some((s, sym)) = self.group_exposure.get(&spec.group) {
            if *s != side {
                return Err(format!("anti-hedge: {sym} is {s} in group {}", spec.group));
            }
        }
        if live {
            if let Some(max_age) = self.cfg.operator_heartbeat_s {
                let fresh = self.last_heartbeat.is_some_and(|t| (now - t).num_seconds() <= max_age);
                if !fresh {
                    return Err("no operator heartbeat — automation must be actively monitored (open the dashboard)".into());
                }
            }
        }
        Ok(())
    }

    pub fn note_entry(&mut self, now: DateTime<Utc>, group: &str, side: Side, symbol: &str) {
        self.day.trades += 1;
        self.day.last_entry = Some(now);
        self.group_exposure.insert(group.to_string(), (side, symbol.to_string()));
    }

    pub fn note_flat(&mut self, group: &str) {
        self.group_exposure.remove(group);
    }

    /// Continuous checks (every bar / every few seconds). Returns actions to execute now.
    pub fn continuous(&mut self, now: DateTime<Utc>, kind: MarketKind, acct: &PropTracker, has_position: bool) -> Vec<GuardAction> {
        let mut out = vec![];
        if !acct.is_active() {
            if has_position {
                out.push(GuardAction::FlattenAll(format!("account status {:?}", acct.status)));
            }
            return out;
        }
        if kind == MarketKind::CmeEquityIndex && has_position {
            let m = et_minute_of_day(now);
            if m >= self.flatten_at && m < 17 * 60 + 60 {
                out.push(GuardAction::FlattenAll(format!("session flatten at {} ET", fmt_hhmm(self.flatten_at))));
            }
        }
        if self.cfg.flatten_before_news && has_position {
            if let Some(e) = self.in_news_blackout(now) {
                out.push(GuardAction::FlattenAll(format!("flatten before news {}", e.format("%H:%M UTC"))));
            }
        }
        let dls = self.daily_loss_stop(acct);
        if -acct.day_pnl() >= dls && self.day.halted.is_none() {
            self.day.halted = Some(format!("personal daily loss stop {dls:.0} hit (day P&L {:.0})", acct.day_pnl()));
            out.push(GuardAction::HaltDay(self.day.halted.clone().unwrap()));
            if has_position {
                out.push(GuardAction::FlattenAll("daily loss stop".into()));
            }
        }
        if acct.day_halted && has_position {
            out.push(GuardAction::FlattenAll("firm daily loss limit".into()));
        }
        if let (Some(_), Some(cap)) = (self.profit_lock(acct), acct.consistency_day_cap()) {
            // Hard ceiling: never let one day exceed 95% of the consistency cap.
            if has_position && acct.day_pnl() >= 0.95 * cap {
                out.push(GuardAction::FlattenAll("consistency ceiling reached for today".into()));
            }
        }
        out
    }

    pub fn may_exit_discretionary(&self, entry_time: DateTime<Utc>, now: DateTime<Utc>) -> bool {
        (now - entry_time).num_seconds() >= self.cfg.min_hold_seconds
    }
}

/// Position size from a risk budget. Returns 0 when the stop is too wide for the budget.
#[allow(clippy::too_many_arguments)]
pub fn size_position(
    spec: &InstrumentSpec,
    entry: f64,
    stop: f64,
    acct: &PropTracker,
    gcfg: &GuardConfig,
    size_mult: f64,
    max_notional_frac: Option<f64>,
    daily_loss_room: f64,
) -> (f64, String) {
    let stop_pts = (entry - stop).abs();
    if stop_pts <= 0.0 {
        return (0.0, "zero stop distance".into());
    }
    let mut budget = f64::INFINITY;
    let mut why = String::new();
    if let Some(usd) = gcfg.risk_per_trade_usd.filter(|x| *x > 0.0) {
        budget = budget.min(usd);
        why = format!("risk ${usd:.0}");
    }
    if let Some(frac) = gcfg.risk_per_trade_frac.filter(|x| *x > 0.0) {
        let b = acct.equity * frac;
        if b < budget {
            budget = b;
            why = format!("risk {:.2}% equity", frac * 100.0);
        }
    }
    if let Some(maxp) = acct.rules.max_risk_per_trade_pct {
        budget = budget.min(acct.equity * maxp * 0.9);
    }
    let buffer_cap = acct.buffer() * gcfg.max_buffer_risk_frac;
    if buffer_cap < budget {
        budget = buffer_cap;
        why = format!("{:.0}% of buffer", gcfg.max_buffer_risk_frac * 100.0);
    }
    let dl_cap = daily_loss_room * 0.5;
    if dl_cap < budget {
        budget = dl_cap;
        why = "half of remaining daily loss room".into();
    }
    if !budget.is_finite() || budget <= 0.0 {
        return (0.0, "no risk budget".into());
    }
    budget *= size_mult.clamp(0.0, 1.0);
    let per_unit = stop_pts * spec.point_value() + 2.0 * spec.cost_one_side(1.0, entry);
    let mut qty = spec.round_qty_down(budget / per_unit);
    if let Some(minis) = acct.rules.max_contracts_mini {
        // micro contracts count as 0.1 mini
        let per_mini = if spec.symbol.starts_with('M') && spec.kind == MarketKind::CmeEquityIndex && spec.symbol != "MBT" { 10.0 } else { 1.0 };
        qty = qty.min((minis * per_mini).floor());
    }
    if let Some(p) = gcfg.max_contracts_personal {
        qty = qty.min(p);
    }
    let notional_cap = match (max_notional_frac, acct.rules.max_leverage) {
        (Some(f), Some(l)) => Some(f.min(l)),
        (Some(f), None) => Some(f),
        (None, Some(l)) => Some(l),
        (None, None) => None,
    };
    if let Some(f) = notional_cap {
        let max_qty = spec.round_qty_down(acct.equity * f / (entry * spec.point_value()));
        if max_qty < qty {
            qty = max_qty;
            why.push_str(&format!(", notional cap {:.2}× equity", f));
        }
    }
    (qty, format!("{why}; budget ${budget:.0}, {stop_pts:.2} pts stop"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prop::PropRules;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn windows_and_flatten() {
        let rules = PropRules::preset("topstep_50k", None).unwrap();
        let acct = PropTracker::new(rules.clone());
        let mut g = ComplianceGuard::new(GuardConfig::default(), rules.flat_by_ct.as_deref());
        let mnq = InstrumentSpec::builtin("MNQ").unwrap();
        // 10:00 ET in July = 14:00 UTC
        assert!(g.check_entry(utc("2026-07-01T14:00:00Z"), MarketKind::CmeEquityIndex, &mnq, Side::Long, false, &acct, false).is_ok());
        assert!(g.check_entry(utc("2026-07-01T13:31:00Z"), MarketKind::CmeEquityIndex, &mnq, Side::Long, false, &acct, false).is_err());
        assert!(g.check_entry(utc("2026-07-01T14:00:00Z"), MarketKind::CmeEquityIndex, &mnq, Side::Long, true, &acct, false).is_err());
        let acts = g.continuous(utc("2026-07-01T19:58:00Z"), MarketKind::CmeEquityIndex, &acct, true);
        assert!(matches!(acts.first(), Some(GuardAction::FlattenAll(_))));
        assert_eq!(g.flatten_minute_et(), 15 * 60 + 58);
    }

    #[test]
    fn anti_hedge_and_rate_limit_and_heartbeat() {
        let rules = PropRules::preset("topstep_50k", None).unwrap();
        let acct = PropTracker::new(rules.clone());
        let mut cfg = GuardConfig::default();
        cfg.operator_heartbeat_s = Some(60);
        cfg.max_orders_per_minute = 2;
        let mut g = ComplianceGuard::new(cfg, rules.flat_by_ct.as_deref());
        let es = InstrumentSpec::builtin("MES").unwrap();
        let t = utc("2026-07-01T15:00:00Z");
        g.note_entry(t, "us_equity_index", Side::Long, "MNQ");
        assert!(g.check_entry(t + Duration::minutes(10), MarketKind::CmeEquityIndex, &es, Side::Short, false, &acct, false).unwrap_err().contains("anti-hedge"));
        g.note_flat("us_equity_index");
        assert!(g.check_entry(t + Duration::minutes(10), MarketKind::CmeEquityIndex, &es, Side::Short, false, &acct, true).unwrap_err().contains("heartbeat"));
        g.last_heartbeat = Some(t + Duration::minutes(10));
        g.note_order(t + Duration::minutes(10));
        g.note_order(t + Duration::minutes(10));
        assert!(g.check_entry(t + Duration::minutes(10), MarketKind::CmeEquityIndex, &es, Side::Short, false, &acct, true).unwrap_err().contains("order-rate"));
    }

    #[test]
    fn consistency_lock_and_sizing() {
        let rules = PropRules::preset("topstep_50k", None).unwrap();
        let mut acct = PropTracker::new(rules.clone());
        acct.begin_day(chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap());
        let mut g = ComplianceGuard::new(GuardConfig::default(), rules.flat_by_ct.as_deref());
        assert_eq!(g.profit_lock(&acct), Some(1_200.0));
        acct.on_realized(1_250.0);
        acct.mark(0.0);
        let mnq = InstrumentSpec::builtin("MNQ").unwrap();
        assert!(g.check_entry(utc("2026-07-01T15:00:00Z"), MarketKind::CmeEquityIndex, &mnq, Side::Long, false, &acct, false).unwrap_err().contains("consistency"));
        // with 2,000 already banked a 2,000 day keeps best ≤ 50% of total → the cap widens to 2,000
        acct.begin_day(chrono::NaiveDate::from_ymd_opt(2026, 7, 2).unwrap());
        acct.on_realized(750.0);
        acct.begin_day(chrono::NaiveDate::from_ymd_opt(2026, 7, 6).unwrap());
        assert_eq!(acct.consistency_day_cap(), Some(2_000.0));
        assert_eq!(g.profit_lock(&acct), Some(1_600.0));

        let acct = PropTracker::new(rules);
        let mut cfg = GuardConfig::default();
        cfg.risk_per_trade_usd = Some(200.0);
        cfg.risk_per_trade_frac = None;
        // 20-point stop on MNQ = $40 + costs per contract → 4 contracts for $200
        let (q, _) = size_position(&mnq, 20_000.0, 19_980.0, &acct, &cfg, 1.0, None, 600.0);
        assert_eq!(q, 4.0);
        let nq = InstrumentSpec::builtin("NQ").unwrap();
        let (q, _) = size_position(&nq, 20_000.0, 19_980.0, &acct, &cfg, 1.0, None, 600.0);
        assert_eq!(q, 0.0, "a $400 risk on 1 NQ must be refused with a $200 budget");
    }
}
