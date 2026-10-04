//! VWAP mean reversion for range days.
//!
//! Between `start_min` and `end_min`, when price is more than `k_entry` session-σ away from
//! VWAP *and* the bar closes back toward VWAP (rejection), fade the extension: target VWAP,
//! stop `k_stop` σ beyond the entry, time-stop after `max_hold_bars`. Refuses to fire when the
//! intraday efficiency ratio says the day is trending. This is liquidity provision — it is
//! supposed to be switched off by the allocator outside ranging/neutral regimes.

use anyhow::Result;
use serde_json::{json, Map, Value};

use q22_core::Side;

use super::param;
use crate::regime::Regime;
use crate::strategy::{EntrySignal, Manage, OpenPosition, Strategy, StrategyCtx};

pub struct VwapReversion {
    id: String,
    start: i64,
    end: i64,
    k_entry: f64,
    k_stop: f64,
    max_er: f64,
    max_hold_bars: usize,
    max_trades_per_day: usize,
    trades_today: usize,
    last_z: Option<f64>,
}

impl VwapReversion {
    pub fn from_params(id: &str, p: &Map<String, Value>) -> Result<Self> {
        Ok(Self {
            id: id.to_string(),
            start: param(p, "start_min", 60.0) as i64,
            end: param(p, "end_min", 330.0) as i64,
            k_entry: param(p, "k_entry", 2.0),
            k_stop: param(p, "k_stop", 1.0),
            max_er: param(p, "max_intraday_er", 0.3),
            max_hold_bars: param(p, "max_hold_bars", 12.0) as usize,
            max_trades_per_day: param(p, "max_trades_per_day", 2.0) as usize,
            trades_today: 0,
            last_z: None,
        })
    }
}

impl Strategy for VwapReversion {
    fn id(&self) -> &str {
        &self.id
    }
    fn family(&self) -> &'static str {
        "mean reversion"
    }
    fn affinity(&self, r: Regime) -> f64 {
        match r {
            Regime::Ranging => 1.0,
            Regime::Neutral => 0.6,
            Regime::Volatile => 0.3,
            Regime::TrendingUp | Regime::TrendingDown => 0.1,
            Regime::Shock => 0.0,
            Regime::Unknown => 0.4,
        }
    }
    fn on_session_start(&mut self, _m: &crate::market::MarketState) {
        self.trades_today = 0;
        self.last_z = None;
    }
    fn entry(&mut self, ctx: &StrategyCtx) -> Option<EntrySignal> {
        let s = ctx.m.session.as_ref()?;
        let (vwap, sd) = (s.vwap()?, s.vwap_std()?);
        if sd <= 0.0 {
            return None;
        }
        let b = ctx.bar;
        let z = (b.close - vwap) / sd;
        self.last_z = Some(z);
        if ctx.offset < self.start || ctx.offset > self.end || self.trades_today >= self.max_trades_per_day {
            return None;
        }
        if ctx.m.er_bar_value().is_none_or(|er| er > self.max_er) {
            return None;
        }
        let (side, stop) = if z <= -self.k_entry && b.close > b.open {
            (Side::Long, b.close - self.k_stop * sd)
        } else if z >= self.k_entry && b.close < b.open {
            (Side::Short, b.close + self.k_stop * sd)
        } else {
            return None;
        };
        self.trades_today += 1;
        Some(EntrySignal {
            side,
            stop,
            target: Some(vwap),
            confidence: ((z.abs() - self.k_entry) / 2.0 + 0.6).clamp(0.5, 1.0),
            reason: format!("{z:+.2}σ from VWAP {vwap:.2} with rejection bar"),
            max_notional_frac: None,
        })
    }
    fn manage(&mut self, _ctx: &StrategyCtx, pos: &OpenPosition) -> Manage {
        if pos.bars_held >= self.max_hold_bars {
            Manage::Exit(format!("time stop after {} bars", pos.bars_held))
        } else {
            Manage::Hold
        }
    }
    fn status(&self) -> Value {
        json!({"vwap_z": self.last_z, "trades_today": self.trades_today})
    }
}

impl crate::market::MarketState {
    /// Intraday efficiency ratio of the last 12 session bars (None while warming up).
    pub fn er_bar_value(&self) -> Option<f64> {
        let closes: Vec<f64> = self.recent.iter().rev().take(13).map(|b| b.close).collect();
        if closes.len() < 13 {
            return None;
        }
        let net = (closes[0] - closes[12]).abs();
        let path: f64 = closes.windows(2).map(|w| (w[0] - w[1]).abs()).sum();
        Some(if path > 0.0 { net / path } else { 0.0 })
    }
}
