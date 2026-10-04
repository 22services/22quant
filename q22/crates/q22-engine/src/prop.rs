//! Prop-firm rule models and the account tracker that enforces them.
//!
//! Presets encode the published 2026 rules of firms that **allow automated trading**
//! (sources in `docs/PROP_FIRMS.md`). Firms change rules often: every preset can be
//! overridden field-by-field from the TOML config, and the dashboard shows which rules are
//! active. Treat presets as defaults to verify, not as legal advice.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DrawdownMode {
    /// Threshold = highest end-of-day balance − max_loss (checked against live equity).
    EodTrailing,
    /// Threshold = highest live equity − max_loss.
    IntradayTrailing,
    /// Threshold = starting balance − max_loss, never moves.
    Static,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DailyLossMode {
    /// Loss measured from the day's starting balance.
    FromDayStart,
    /// Loss measured from the day's highest equity (HyroTrader).
    FromDayHigh,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConsistencyMode {
    /// Best day must be ≤ share × profit target: a hard cap on any single day.
    ShareOfTarget,
    /// Best day must be ≤ share × total profit, so a big day raises the effective target to
    /// best day ÷ share instead of failing the account (Topstep Combine 50%, HyroTrader 40%).
    ShareOfTotalProfit,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Automation {
    /// Bots allowed in evaluation and funded accounts.
    Full,
    /// Bots allowed in evaluation only; funded accounts need a human in the loop.
    EvaluationOnly,
    /// Signals only — a human must place every order.
    AssistOnly,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PropRules {
    pub name: String,
    pub firm: String,
    pub account_size: f64,
    pub profit_target: Option<f64>,
    pub max_loss: f64,
    pub drawdown_mode: DrawdownMode,
    /// The trailing threshold stops rising once it reaches this balance (None = never stops).
    pub threshold_lock_at: Option<f64>,
    pub daily_loss_limit: Option<f64>,
    pub daily_loss_mode: DailyLossMode,
    /// True if breaching the daily limit fails the account (HyroTrader); false = day halted (Topstep/Apex).
    pub daily_loss_fails_account: bool,
    pub consistency_share: Option<f64>,
    pub consistency_mode: ConsistencyMode,
    pub min_trading_days: u32,
    /// Max size in *mini-equivalent* contracts (1 micro = 0.1 mini). Crypto: max leverage × equity is used instead.
    pub max_contracts_mini: Option<f64>,
    pub max_leverage: Option<f64>,
    /// Positions must be flat by this Chicago time on trading days (e.g. "15:10" for Topstep).
    pub flat_by_ct: Option<String>,
    pub overnight_allowed: bool,
    /// A stop-loss must be attached within this many seconds of entry (HyroTrader: 300).
    pub stop_required_within_s: Option<u64>,
    pub max_risk_per_trade_pct: Option<f64>,
    pub automation: Automation,
    pub vps_vpn_prohibited: bool,
    pub hft_prohibited: bool,
    pub notes: Vec<String>,
    pub sources: Vec<String>,
}

impl PropRules {
    pub fn preset(name: &str, account_size: Option<f64>) -> Option<PropRules> {
        let topstep = |size: f64, target: f64, mll: f64, minis: f64, dll: f64| PropRules {
            name: format!("topstep_{}k", (size / 1000.0) as i64),
            firm: "Topstep (TopstepX / ProjectX API)".into(),
            account_size: size,
            profit_target: Some(target),
            max_loss: mll,
            drawdown_mode: DrawdownMode::EodTrailing,
            threshold_lock_at: Some(size),
            daily_loss_limit: None,
            daily_loss_mode: DailyLossMode::FromDayStart,
            daily_loss_fails_account: false,
            consistency_share: Some(0.5),
            consistency_mode: ConsistencyMode::ShareOfTotalProfit,
            min_trading_days: 2,
            max_contracts_mini: Some(minis),
            max_leverage: None,
            flat_by_ct: Some("15:10".into()),
            overnight_allowed: false,
            stop_required_within_s: None,
            max_risk_per_trade_pct: None,
            automation: Automation::Full,
            vps_vpn_prohibited: true,
            hft_prohibited: true,
            notes: vec![
                "Consistency: best day ≤ 50% of total profit — a bigger day raises the target to best day ÷ 0.5; it does not fail the Combine.".into(),
                format!("Optional Daily Loss Limit add-on: ${dll:.0} — set daily_loss_limit if you bought it."),
                "Automation only through the TopstepX API ($14.50–29/month); must be actively monitored.".into(),
                "VPS / VPN / remote servers prohibited for order routing: run q22 on your own computer.".into(),
                "Banned: HFT, latency arbitrage, paired straddles, 1–2 tick micro-scalping, max size into tier-1 news.".into(),
                "Verify the MLL lock level and min-days in the Topstep help center before going live.".into(),
            ],
            sources: vec![
                "https://www.futureshive.com/blog/topstep-rules-explained".into(),
                "https://proptradingvibes.com/blog/topstep-consistency-rule".into(),
                "https://tradecovex.com/guides/topstep-rule-changes-2026".into(),
                "https://help.topstep.com/en/articles/11187768-topstepx-api-access".into(),
                "https://proptradingvibes.com/blog/topstep-vpn-policy".into(),
                "https://blog.pickmytrade.io/topstepx-automation-2026/".into(),
            ],
        };
        let hyro = |size: f64, target_pct: f64, max_pct: f64, daily_pct: f64, label: &str| PropRules {
            name: format!("hyrotrader_{label}"),
            firm: "HyroTrader (Bybit API)".into(),
            account_size: size,
            profit_target: Some(size * target_pct),
            max_loss: size * max_pct,
            drawdown_mode: DrawdownMode::Static,
            threshold_lock_at: None,
            daily_loss_limit: Some(size * daily_pct),
            daily_loss_mode: DailyLossMode::FromDayHigh,
            daily_loss_fails_account: true,
            consistency_share: Some(0.4),
            consistency_mode: ConsistencyMode::ShareOfTotalProfit,
            min_trading_days: 0,
            max_contracts_mini: None,
            max_leverage: Some(2.0),
            flat_by_ct: None,
            overnight_allowed: true,
            // The 5-minute stop-loss rule was withdrawn in 2026; q22 still attaches an exchange-side
            // stop to every Bybit entry, which keeps the 3% per-position loss cap mechanical.
            stop_required_within_s: None,
            max_risk_per_trade_pct: Some(0.03),
            automation: Automation::Full,
            vps_vpn_prohibited: false,
            hft_prohibited: false,
            notes: vec![
                "Realised loss on one position must stay ≤ 3% of the initial balance (reviewed manually); q22 sizes to ≤ 1.5% and always sends an exchange-side stop.".into(),
                "Daily drawdown trails the day's highest balance and resets at 00:00 UTC.".into(),
                "Evaluation: no single day may exceed 40% of total profit.".into(),
                "The challenge trades a Bybit DEMO sub-account (api-demo.bybit.com) with your own API key; funded = real Bybit sub-account.".into(),
                "Bots: marketed as allowed, but the terms ban bots 'except where expressly permitted' — get written confirmation from support before running auto mode.".into(),
            ],
            sources: vec![
                "https://www.hyrotrader.com/trading-rules/".into(),
                "https://www.hyrotrader.com/faq/rules/what-is-the-maximum-loss-per-trade-rule/".into(),
                "https://www.hyrotrader.com/faq/bybit-platform/how-to-correctly-set-up-bybit-api-and-why-am-i-getting-an-error-when-connecting-the-api/".into(),
                "https://cryptoslate.com/prop-firms/hyrotrader-review/".into(),
                "https://www.proptradingvibes.com/blog/hyrotrader-rules-overview".into(),
            ],
        };
        let size = account_size;
        Some(match name {
            "topstep_50k" => topstep(50_000.0, 3_000.0, 2_000.0, 5.0, 1_000.0),
            "topstep_100k" => topstep(100_000.0, 6_000.0, 3_000.0, 10.0, 2_000.0),
            "topstep_150k" => topstep(150_000.0, 9_000.0, 4_500.0, 15.0, 3_000.0),
            "hyrotrader_2step_phase1" => hyro(size.unwrap_or(10_000.0), 0.10, 0.10, 0.05, "2step_phase1"),
            "hyrotrader_2step_phase2" => hyro(size.unwrap_or(10_000.0), 0.05, 0.10, 0.05, "2step_phase2"),
            "hyrotrader_1step" => hyro(size.unwrap_or(10_000.0), 0.10, 0.06, 0.04, "1step"),
            "apex_100k_eod" => PropRules {
                name: "apex_100k_eod".into(),
                firm: "Apex Trader Funding (assist mode only on PA)".into(),
                account_size: 100_000.0,
                profit_target: Some(6_000.0),
                max_loss: 3_000.0,
                drawdown_mode: DrawdownMode::EodTrailing,
                threshold_lock_at: Some(100_100.0),
                daily_loss_limit: Some(1_500.0),
                daily_loss_mode: DailyLossMode::FromDayStart,
                daily_loss_fails_account: false,
                consistency_share: Some(0.5),
                consistency_mode: ConsistencyMode::ShareOfTotalProfit,
                min_trading_days: 0,
                max_contracts_mini: Some(10.0),
                max_leverage: None,
                flat_by_ct: Some("15:59".into()),
                overnight_allowed: false,
                stop_required_within_s: None,
                max_risk_per_trade_pct: None,
                automation: Automation::EvaluationOnly,
                vps_vpn_prohibited: false,
                hft_prohibited: true,
                notes: vec!["Fully automated trading is prohibited on PA/Live accounts: use mode = \"assist\" (you approve every entry).".into()],
                sources: vec!["https://support.apextraderfunding.com/hc/en-us/articles/31519788944411-Performance-Account-PA-and-Compliance".into()],
            },
            "none" | "personal" => PropRules {
                name: "personal".into(),
                firm: "own capital (no firm rules)".into(),
                account_size: size.unwrap_or(10_000.0),
                profit_target: None,
                max_loss: size.unwrap_or(10_000.0) * 0.25,
                drawdown_mode: DrawdownMode::IntradayTrailing,
                threshold_lock_at: None,
                daily_loss_limit: Some(size.unwrap_or(10_000.0) * 0.03),
                daily_loss_mode: DailyLossMode::FromDayStart,
                daily_loss_fails_account: false,
                consistency_share: None,
                consistency_mode: ConsistencyMode::ShareOfTotalProfit,
                min_trading_days: 0,
                max_contracts_mini: None,
                max_leverage: Some(2.0),
                flat_by_ct: None,
                overnight_allowed: true,
                stop_required_within_s: None,
                max_risk_per_trade_pct: Some(0.01),
                automation: Automation::Full,
                vps_vpn_prohibited: false,
                hft_prohibited: false,
                notes: vec!["Personal account: a 25% kill-switch drawdown and 3% daily stop are the only rules.".into()],
                sources: vec![],
            },
            _ => return None,
        })
    }

    pub fn preset_names() -> &'static [&'static str] {
        &["topstep_50k", "topstep_100k", "topstep_150k", "hyrotrader_2step_phase1", "hyrotrader_2step_phase2", "hyrotrader_1step", "apex_100k_eod", "personal"]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AccountStatus {
    Active,
    Passed { date: NaiveDate },
    Failed { date: NaiveDate, reason: String },
}

/// Live state of a (simulated or real) prop account under `rules`.
#[derive(Clone, Debug, Serialize)]
pub struct PropTracker {
    pub rules: PropRules,
    pub start_balance: f64,
    pub balance: f64,
    pub equity: f64,
    pub threshold: f64,
    pub high_eod_balance: f64,
    pub high_equity: f64,
    pub day: Option<NaiveDate>,
    pub day_start_balance: f64,
    pub day_high_equity: f64,
    pub day_halted: bool,
    pub daily_pnl: BTreeMap<NaiveDate, f64>,
    pub status: AccountStatus,
}

impl PropTracker {
    pub fn new(rules: PropRules) -> Self {
        let b = rules.account_size;
        let threshold = b - rules.max_loss;
        Self { start_balance: b, balance: b, equity: b, threshold, high_eod_balance: b, high_equity: b, day: None, day_start_balance: b, day_high_equity: b, day_halted: false, daily_pnl: BTreeMap::new(), status: AccountStatus::Active, rules }
    }

    /// Start from a known live state (balance and the firm-reported threshold).
    pub fn resume(rules: PropRules, balance: f64, threshold: Option<f64>, high_eod_balance: Option<f64>) -> Self {
        let mut t = Self::new(rules);
        t.balance = balance;
        t.equity = balance;
        t.high_eod_balance = high_eod_balance.unwrap_or(balance.max(t.start_balance));
        t.threshold = threshold.unwrap_or_else(|| t.trail_threshold(t.high_eod_balance));
        t.day_start_balance = balance;
        t.day_high_equity = balance;
        t
    }

    fn trail_threshold(&self, high: f64) -> f64 {
        let raw = high - self.rules.max_loss;
        match self.rules.threshold_lock_at {
            Some(lock) => raw.min(lock),
            None => raw,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.status, AccountStatus::Active)
    }

    /// Call at the start of every trading day (before any trade of that day).
    pub fn begin_day(&mut self, day: NaiveDate) {
        if self.day == Some(day) {
            return;
        }
        if let Some(prev) = self.day {
            self.end_day(prev);
        }
        self.day = Some(day);
        self.day_start_balance = self.balance;
        self.day_high_equity = self.equity;
        self.day_halted = false;
    }

    fn end_day(&mut self, day: NaiveDate) {
        let pnl = self.balance - self.day_start_balance;
        self.daily_pnl.insert(day, pnl);
        if self.rules.drawdown_mode == DrawdownMode::EodTrailing && self.balance > self.high_eod_balance {
            self.high_eod_balance = self.balance;
            self.threshold = self.threshold.max(self.trail_threshold(self.high_eod_balance));
        }
        self.check_pass(day);
    }

    /// Flush the current day (end of a backtest).
    pub fn close_books(&mut self) {
        if let Some(d) = self.day {
            self.end_day(d);
        }
    }

    pub fn on_realized(&mut self, pnl: f64) {
        self.balance += pnl;
    }

    /// Mark-to-market with open P&L. Returns a failure reason if the account just failed.
    pub fn mark(&mut self, open_pnl: f64) -> Option<String> {
        if !self.is_active() {
            return None;
        }
        self.equity = self.balance + open_pnl;
        self.high_equity = self.high_equity.max(self.equity);
        self.day_high_equity = self.day_high_equity.max(self.equity);
        if self.rules.drawdown_mode == DrawdownMode::IntradayTrailing {
            self.threshold = self.threshold.max(self.trail_threshold(self.high_equity));
        }
        let day = self.day.unwrap_or_default();
        if self.equity <= self.threshold {
            let reason = format!("max loss breached: equity {:.0} ≤ threshold {:.0}", self.equity, self.threshold);
            self.status = AccountStatus::Failed { date: day, reason: reason.clone() };
            return Some(reason);
        }
        if let Some(dll) = self.rules.daily_loss_limit {
            if self.daily_loss() >= dll {
                if self.rules.daily_loss_fails_account {
                    let reason = format!("daily loss limit breached: {:.0} ≥ {:.0}", self.daily_loss(), dll);
                    self.status = AccountStatus::Failed { date: day, reason: reason.clone() };
                    return Some(reason);
                }
                self.day_halted = true;
            }
        }
        None
    }

    pub fn daily_loss(&self) -> f64 {
        let reference = match self.rules.daily_loss_mode {
            DailyLossMode::FromDayStart => self.day_start_balance,
            DailyLossMode::FromDayHigh => self.day_high_equity,
        };
        (reference - self.equity).max(0.0)
    }

    pub fn day_pnl(&self) -> f64 {
        self.equity - self.day_start_balance
    }

    pub fn total_profit(&self) -> f64 {
        self.balance - self.start_balance
    }

    pub fn trading_days(&self) -> usize {
        self.daily_pnl.values().filter(|v| **v != 0.0).count()
    }

    pub fn best_day(&self) -> f64 {
        self.daily_pnl.values().cloned().fold(0.0, f64::max)
    }

    /// Is the consistency rule currently satisfied?
    pub fn consistency_ok(&self) -> bool {
        let Some(share) = self.rules.consistency_share else { return true };
        match self.rules.consistency_mode {
            ConsistencyMode::ShareOfTarget => self.rules.profit_target.is_none_or(|t| self.best_day() <= share * t + 1e-6),
            ConsistencyMode::ShareOfTotalProfit => {
                let tp = self.total_profit();
                tp <= 0.0 || self.best_day() <= share * tp + 1e-6
            }
        }
    }

    /// Largest profit today that keeps the consistency rule from raising the bar (None = no cap).
    pub fn consistency_day_cap(&self) -> Option<f64> {
        let share = self.rules.consistency_share?;
        let target = self.rules.profit_target?;
        match self.rules.consistency_mode {
            ConsistencyMode::ShareOfTarget => Some(share * target),
            ConsistencyMode::ShareOfTotalProfit => {
                // today d is fine while d ≤ share × (prior + d)  ⇔  d ≤ share ÷ (1 − share) × prior
                let prior = (self.day_start_balance - self.start_balance).max(0.0);
                let from_prior = if share < 1.0 { share / (1.0 - share) * prior } else { f64::INFINITY };
                Some((share * target).max(from_prior))
            }
        }
    }

    pub fn buffer(&self) -> f64 {
        self.equity - self.threshold
    }

    fn check_pass(&mut self, day: NaiveDate) {
        if !self.is_active() {
            return;
        }
        if let Some(t) = self.rules.profit_target {
            if self.total_profit() >= t && self.consistency_ok() && self.trading_days() as u32 >= self.rules.min_trading_days {
                self.status = AccountStatus::Passed { date: day };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, n).unwrap()
    }

    #[test]
    fn eod_trailing_threshold_and_lock() {
        let mut t = PropTracker::new(PropRules::preset("topstep_50k", None).unwrap());
        assert_eq!(t.threshold, 48_000.0);
        t.begin_day(d(5));
        t.on_realized(1_000.0);
        t.mark(0.0);
        // intraday gains don't move an EOD threshold
        assert_eq!(t.threshold, 48_000.0);
        t.begin_day(d(6));
        assert_eq!(t.threshold, 49_000.0);
        t.on_realized(2_500.0);
        t.begin_day(d(7));
        // locks at the starting balance
        assert_eq!(t.threshold, 50_000.0);
    }

    #[test]
    fn fails_on_breach_and_passes_on_target_with_consistency() {
        let mut t = PropTracker::new(PropRules::preset("topstep_50k", None).unwrap());
        t.begin_day(d(5));
        assert!(t.mark(-2_000.0).is_some());
        assert!(!t.is_active());

        let mut t = PropTracker::new(PropRules::preset("topstep_50k", None).unwrap());
        t.begin_day(d(5));
        t.on_realized(1_600.0); // best day 1,600 > 50% of 3,000 → consistency blocks the pass
        t.begin_day(d(6));
        t.on_realized(1_500.0);
        t.begin_day(d(7));
        assert!(t.is_active(), "consistency rule must block: {:?}", t.status);
        t.on_realized(0.0);
        t.on_realized(100.0); // total 3,200 → best day 1,600 is now exactly 50% → passes at the close
        t.close_books();
        assert!(matches!(t.status, AccountStatus::Passed { .. }), "a big day raises the target, it does not fail: {:?}", t.status);
        let mut t2 = PropTracker::new(PropRules::preset("topstep_50k", None).unwrap());
        for (i, p) in [1_000.0, 1_000.0, 1_100.0].iter().enumerate() {
            t2.begin_day(d(5 + i as u32));
            t2.on_realized(*p);
        }
        t2.close_books();
        assert!(matches!(t2.status, AccountStatus::Passed { .. }), "{:?}", t2.status);
    }

    #[test]
    fn hyrotrader_daily_from_high_fails() {
        let mut t = PropTracker::new(PropRules::preset("hyrotrader_1step", Some(10_000.0)).unwrap());
        t.begin_day(d(5));
        assert!(t.mark(300.0).is_none());
        // 4% of 10k = 400 from the day's high (10,300) → 9,900 equity is a breach
        assert!(t.mark(-100.0).is_some());
    }
}
