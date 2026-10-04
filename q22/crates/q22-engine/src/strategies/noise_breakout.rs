//! Intraday momentum with a "noise area" (Zarattini, Aziz & Barbon 2024, *Beat the Market*).
//!
//! For each time-of-day `m`, σ_m is the average |close_m / open − 1| over the last 14 sessions.
//! The noise area is `[min(open, prev_close)·(1−σ_m), max(open, prev_close)·(1+σ_m)]`.
//! Every 30 minutes (from 10:00 ET) a close above the upper band goes long, below the lower
//! band goes short. The stop trails at max(upper band, VWAP) for longs (min(lower, VWAP) for
//! shorts), updated at each check; positions are closed before the session end.
//!
//! Deviation from the paper (deliberate, for prop accounts): the trailing level rests as a
//! real stop order (intrabar protection) instead of being checked only every 30 minutes, and
//! a minimum stop distance of `min_stop_atr` × bar-ATR avoids microscopic stops.

use anyhow::Result;
use serde_json::{json, Map, Value};

use q22_core::Side;

use super::param;
use crate::regime::Regime;
use crate::strategy::{EntrySignal, Manage, OpenPosition, Strategy, StrategyCtx};

pub struct NoiseBreakout {
    id: String,
    check_every: i64,
    first_check: i64,
    last_entry: i64,
    band_mult: f64,
    min_stop_atr: f64,
    max_trades_per_day: usize,
    trades_today: usize,
    last_bands: Option<(f64, f64)>,
}

impl NoiseBreakout {
    pub fn from_params(id: &str, p: &Map<String, Value>) -> Result<Self> {
        Ok(Self {
            id: id.to_string(),
            check_every: param(p, "check_every_min", 30.0) as i64,
            first_check: param(p, "first_check_min", 30.0) as i64,
            last_entry: param(p, "last_entry_min", 360.0) as i64,
            band_mult: param(p, "band_mult", 1.0),
            min_stop_atr: param(p, "min_stop_atr", 1.0),
            max_trades_per_day: param(p, "max_trades_per_day", 3.0) as usize,
            trades_today: 0,
            last_bands: None,
        })
    }

    fn bands(&self, ctx: &StrategyCtx) -> Option<(f64, f64)> {
        let s = ctx.m.session.as_ref()?;
        let sigma = ctx.m.noise_sigma(ctx.offset)? * self.band_mult;
        let pc = s.prev_close.unwrap_or(s.open);
        Some((s.open.max(pc) * (1.0 + sigma), s.open.min(pc) * (1.0 - sigma)))
    }

    fn is_check(&self, offset: i64) -> bool {
        offset >= self.first_check && offset % self.check_every == 0
    }
}

impl Strategy for NoiseBreakout {
    fn id(&self) -> &str {
        &self.id
    }
    fn family(&self) -> &'static str {
        "intraday momentum"
    }
    fn affinity(&self, r: Regime) -> f64 {
        match r {
            Regime::TrendingUp | Regime::TrendingDown => 1.0,
            Regime::Volatile => 0.9,
            Regime::Neutral => 0.7,
            Regime::Ranging => 0.3,
            Regime::Shock => 0.4,
            Regime::Unknown => 0.5,
        }
    }
    fn on_session_start(&mut self, _m: &crate::market::MarketState) {
        self.trades_today = 0;
        self.last_bands = None;
    }
    fn entry(&mut self, ctx: &StrategyCtx) -> Option<EntrySignal> {
        if !self.is_check(ctx.offset) || ctx.offset > self.last_entry || self.trades_today >= self.max_trades_per_day {
            return None;
        }
        let (ub, lb) = self.bands(ctx)?;
        self.last_bands = Some((ub, lb));
        let s = ctx.m.session.as_ref()?;
        let vwap = s.vwap()?;
        let atr = ctx.m.atr_bar.value()?;
        let c = ctx.bar.close;
        let min_d = self.min_stop_atr * atr;
        let sig = if c > ub {
            let stop = ub.max(vwap).min(c - min_d);
            Some((Side::Long, stop, (c - ub) / (ub - lb).max(1e-9)))
        } else if c < lb {
            let stop = lb.min(vwap).max(c + min_d);
            Some((Side::Short, stop, (lb - c) / (ub - lb).max(1e-9)))
        } else {
            None
        }?;
        self.trades_today += 1;
        Some(EntrySignal {
            side: sig.0,
            stop: sig.1,
            target: None,
            confidence: (0.6 + sig.2).min(1.0),
            reason: format!("close {c:.2} outside noise area [{lb:.2}, {ub:.2}] at +{}m", ctx.offset),
            max_notional_frac: None,
        })
    }
    fn manage(&mut self, ctx: &StrategyCtx, pos: &OpenPosition) -> Manage {
        if !self.is_check(ctx.offset) {
            return Manage::Hold;
        }
        let (Some((ub, lb)), Some(vwap)) = (self.bands(ctx), ctx.m.session.as_ref().and_then(|s| s.vwap())) else {
            return Manage::Hold;
        };
        self.last_bands = Some((ub, lb));
        let c = ctx.bar.close;
        match pos.side {
            Side::Long => {
                let trail = ub.max(vwap);
                if c < trail {
                    Manage::Exit(format!("close {c:.2} back below trail {trail:.2}"))
                } else if trail > pos.stop {
                    Manage::MoveStop(trail)
                } else {
                    Manage::Hold
                }
            }
            Side::Short => {
                let trail = lb.min(vwap);
                if c > trail {
                    Manage::Exit(format!("close {c:.2} back above trail {trail:.2}"))
                } else if trail < pos.stop {
                    Manage::MoveStop(trail)
                } else {
                    Manage::Hold
                }
            }
        }
    }
    fn status(&self) -> Value {
        json!({"upper_band": self.last_bands.map(|b| b.0), "lower_band": self.last_bands.map(|b| b.1), "trades_today": self.trades_today})
    }
}
