//! Market intraday momentum (Gao, Han, Li & Zhou 2018, JFE): the return from the previous
//! close to 10:00 ET (overnight + first half-hour) predicts the return of the last half-hour.
//!
//! At `entry_min` (default 360 = 15:30 ET) go in the direction of that early return if it is
//! larger than `min_move_atr` × daily ATR; the engine's session flatten closes the trade.
//! A protective stop at `stop_atr` × daily ATR caps the tail.

use anyhow::Result;
use serde_json::{json, Map, Value};

use q22_core::Side;

use super::param;
use crate::regime::Regime;
use crate::strategy::{EntrySignal, Strategy, StrategyCtx};

pub struct LastHalfHour {
    id: String,
    signal_min: i64,
    entry_min: i64,
    min_move_atr: f64,
    stop_atr: f64,
    early_return: Option<f64>,
    early_move: Option<f64>,
    done_today: bool,
}

impl LastHalfHour {
    pub fn from_params(id: &str, p: &Map<String, Value>) -> Result<Self> {
        Ok(Self {
            id: id.to_string(),
            signal_min: param(p, "signal_min", 30.0) as i64,
            entry_min: param(p, "entry_min", 360.0) as i64,
            min_move_atr: param(p, "min_move_atr", 0.10),
            stop_atr: param(p, "stop_atr", 0.25),
            early_return: None,
            early_move: None,
            done_today: false,
        })
    }
}

impl Strategy for LastHalfHour {
    fn id(&self) -> &str {
        &self.id
    }
    fn family(&self) -> &'static str {
        "intraday momentum (close)"
    }
    fn affinity(&self, r: Regime) -> f64 {
        match r {
            Regime::Volatile => 1.0,
            Regime::TrendingUp | Regime::TrendingDown => 0.9,
            Regime::Neutral => 0.8,
            Regime::Ranging => 0.6,
            Regime::Shock => 0.5,
            Regime::Unknown => 0.5,
        }
    }
    fn on_session_start(&mut self, _m: &crate::market::MarketState) {
        self.early_return = None;
        self.early_move = None;
        self.done_today = false;
    }
    fn entry(&mut self, ctx: &StrategyCtx) -> Option<EntrySignal> {
        let s = ctx.m.session.as_ref()?;
        if ctx.offset == self.signal_min {
            if let Some(pc) = s.prev_close {
                self.early_return = Some(ctx.bar.close / pc - 1.0);
                self.early_move = Some(ctx.bar.close - pc);
            }
            return None;
        }
        if self.done_today || ctx.offset != self.entry_min {
            return None;
        }
        self.done_today = true;
        let (r, mv) = (self.early_return?, self.early_move?);
        let atr = ctx.m.features.atr_d?;
        if mv.abs() < self.min_move_atr * atr {
            return None;
        }
        let side = if r > 0.0 { Side::Long } else { Side::Short };
        let c = ctx.bar.close;
        Some(EntrySignal {
            side,
            stop: c - side.sign() * self.stop_atr * atr,
            target: None,
            confidence: (0.6 + (mv.abs() / atr)).min(1.0),
            reason: format!("prev close→10:00 move {:+.2} ({:+.2}%) ⇒ ride the close", mv, r * 100.0),
            max_notional_frac: None,
        })
    }
    fn status(&self) -> Value {
        json!({"early_return_pct": self.early_return.map(|r| r * 100.0), "done_today": self.done_today})
    }
}
