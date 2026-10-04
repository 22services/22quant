//! Opening-range breakout.
//!
//! The opening range (OR) is the high/low of the first `or_minutes` of the session. The first
//! bar that *closes* beyond it (before `last_entry_min`) triggers an entry in that direction —
//! close-confirmation avoids most single-tick fake-outs. Stop = the other side of the OR, but
//! never further than `max_stop_atr` × daily ATR; target = `target_r` × initial risk (0 = hold
//! to the session exit). Skips days where the OR is already wider than `max_or_atr` × daily ATR
//! (the move has happened) or narrower than `min_or_atr` (noise).

use anyhow::Result;
use serde_json::{json, Map, Value};

use q22_core::Side;

use super::param;
use crate::regime::Regime;
use crate::strategy::{EntrySignal, Manage, OpenPosition, Strategy, StrategyCtx};

pub struct OpeningRangeBreakout {
    id: String,
    or_minutes: i64,
    last_entry: i64,
    max_stop_atr: f64,
    target_r: f64,
    max_or_atr: f64,
    min_or_atr: f64,
    breakeven_r: f64,
    or_high: f64,
    or_low: f64,
    done_today: bool,
}

impl OpeningRangeBreakout {
    pub fn from_params(id: &str, p: &Map<String, Value>) -> Result<Self> {
        Ok(Self {
            id: id.to_string(),
            or_minutes: param(p, "or_minutes", 15.0) as i64,
            last_entry: param(p, "last_entry_min", 120.0) as i64,
            max_stop_atr: param(p, "max_stop_atr", 0.35),
            target_r: param(p, "target_r", 2.0),
            max_or_atr: param(p, "max_or_atr", 0.6),
            min_or_atr: param(p, "min_or_atr", 0.05),
            breakeven_r: param(p, "breakeven_r", 1.0),
            or_high: f64::NAN,
            or_low: f64::NAN,
            done_today: false,
        })
    }
}

impl Strategy for OpeningRangeBreakout {
    fn id(&self) -> &str {
        &self.id
    }
    fn family(&self) -> &'static str {
        "breakout"
    }
    fn affinity(&self, r: Regime) -> f64 {
        match r {
            Regime::TrendingUp | Regime::TrendingDown => 0.9,
            Regime::Volatile => 0.9,
            Regime::Neutral => 0.7,
            Regime::Ranging => 0.3,
            Regime::Shock => 0.3,
            Regime::Unknown => 0.5,
        }
    }
    fn on_session_start(&mut self, _m: &crate::market::MarketState) {
        self.or_high = f64::NAN;
        self.or_low = f64::NAN;
        self.done_today = false;
    }
    fn entry(&mut self, ctx: &StrategyCtx) -> Option<EntrySignal> {
        let b = ctx.bar;
        if ctx.offset <= self.or_minutes {
            self.or_high = if self.or_high.is_nan() { b.high } else { self.or_high.max(b.high) };
            self.or_low = if self.or_low.is_nan() { b.low } else { self.or_low.min(b.low) };
            return None;
        }
        if self.done_today || ctx.offset > self.last_entry || self.or_high.is_nan() {
            return None;
        }
        let atr_d = ctx.m.features.atr_d?;
        let width = self.or_high - self.or_low;
        if width > self.max_or_atr * atr_d || width < self.min_or_atr * atr_d {
            self.done_today = true;
            return None;
        }
        let max_stop = self.max_stop_atr * atr_d;
        let c = b.close;
        let (side, stop) = if c > self.or_high {
            (Side::Long, self.or_low.max(c - max_stop))
        } else if c < self.or_low {
            (Side::Short, self.or_high.min(c + max_stop))
        } else {
            return None;
        };
        self.done_today = true;
        let risk = (c - stop).abs();
        let target = (self.target_r > 0.0).then(|| c + side.sign() * self.target_r * risk);
        Some(EntrySignal {
            side,
            stop,
            target,
            confidence: (1.0 - width / (self.max_or_atr * atr_d)).clamp(0.5, 1.0),
            reason: format!("close {c:.2} broke {}m OR [{:.2}, {:.2}]", self.or_minutes, self.or_low, self.or_high),
            max_notional_frac: None,
        })
    }
    fn manage(&mut self, ctx: &StrategyCtx, pos: &OpenPosition) -> Manage {
        // Move the stop to break-even once the trade has earned `breakeven_r`.
        if self.breakeven_r <= 0.0 {
            return Manage::Hold;
        }
        let r = pos.initial_risk_points();
        let gain = (ctx.bar.close - pos.entry_price) * pos.side.sign();
        let be = pos.entry_price;
        let improves = match pos.side {
            Side::Long => be > pos.stop,
            Side::Short => be < pos.stop,
        };
        if gain >= self.breakeven_r * r && improves {
            Manage::MoveStop(be)
        } else {
            Manage::Hold
        }
    }
    fn status(&self) -> Value {
        json!({"or_high": (!self.or_high.is_nan()).then_some(self.or_high), "or_low": (!self.or_low.is_nan()).then_some(self.or_low), "done_today": self.done_today})
    }
}
