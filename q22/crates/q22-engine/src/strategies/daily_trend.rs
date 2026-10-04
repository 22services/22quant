//! Daily time-series-momentum ensemble for 24/7 assets (BTC, ETH …).
//!
//! The same pre-specified, un-optimised ensemble validated in the q22 research
//! (Sharpe ≈ 1.05 net on BTC 2014-2026): sign of the 20/60/120-day return, three EWMA
//! crossovers normalised by price volatility, and a 55-day breakout — averaged into a score
//! in [−1, 1]. Entry when |score| ≥ `entry_score`, exit when it decays below `exit_score` or
//! flips. Discrete (no daily resizing) so it maps cleanly onto prop accounts that require a
//! stop on every position: a Chandelier stop trails at `stop_atr` × daily ATR from the best
//! close since entry, and the position notional is capped by volatility targeting.
//! Decisions are taken once per day, on the first bar after the UTC day closes.

use std::collections::VecDeque;

use anyhow::Result;
use serde_json::{json, Map, Value};

use q22_core::Side;

use super::{param, param_bool};
use crate::regime::Regime;
use crate::strategy::{EntrySignal, Manage, OpenPosition, Strategy, StrategyCtx};

pub struct DailyTrend {
    id: String,
    entry_score: f64,
    exit_score: f64,
    stop_atr: f64,
    target_vol: f64,
    allow_short: bool,
    score: Option<f64>,
    best_close: f64,
    closes_seen: usize,
}

impl DailyTrend {
    pub fn from_params(id: &str, p: &Map<String, Value>) -> Result<Self> {
        Ok(Self {
            id: id.to_string(),
            entry_score: param(p, "entry_score", 0.3),
            exit_score: param(p, "exit_score", 0.05),
            stop_atr: param(p, "stop_atr", 3.0),
            target_vol: param(p, "target_vol", 0.30),
            allow_short: param_bool(p, "allow_short", true),
            score: None,
            best_close: f64::NAN,
            closes_seen: 0,
        })
    }

    pub fn composite_score(closes: &[f64]) -> Option<f64> {
        let n = closes.len();
        if n < 130 {
            return None;
        }
        let last = closes[n - 1];
        let mut parts = Vec::with_capacity(7);
        for lb in [20usize, 60, 120] {
            parts.push((last / closes[n - 1 - lb] - 1.0).signum());
        }
        // EWMA crossovers normalised by the 63-day std of price, squashed (Baz et al. 2015).
        let tail = &closes[n.saturating_sub(63)..];
        let m = tail.iter().sum::<f64>() / tail.len() as f64;
        let sd = (tail.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (tail.len() as f64 - 1.0)).sqrt();
        for (s, l) in [(8usize, 24usize), (16, 48), (32, 96)] {
            let ema = |span: usize| {
                let a = 2.0 / (span as f64 + 1.0);
                closes.iter().fold(None, |acc: Option<f64>, &x| Some(acc.map_or(x, |p| p + a * (x - p)))).unwrap()
            };
            let x = if sd > 0.0 { (ema(s) - ema(l)) / sd } else { 0.0 };
            parts.push((x * (-x * x / 4.0).exp() / 0.89).clamp(-1.0, 1.0));
        }
        let window = &closes[n - 56..n - 1];
        let hi = window.iter().cloned().fold(f64::MIN, f64::max);
        let lo = window.iter().cloned().fold(f64::MAX, f64::min);
        parts.push(if last >= hi { 1.0 } else if last <= lo { -1.0 } else { 0.0 });
        Some(parts.iter().sum::<f64>() / parts.len() as f64)
    }

    fn realized_vol(closes: &[f64], n: usize) -> Option<f64> {
        if closes.len() < n + 1 {
            return None;
        }
        let r: Vec<f64> = closes[closes.len() - n - 1..].windows(2).map(|w| (w[1] / w[0]).ln()).collect();
        let m = r.iter().sum::<f64>() / r.len() as f64;
        Some((r.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (r.len() as f64 - 1.0)).sqrt() * 365f64.sqrt())
    }
}

impl Strategy for DailyTrend {
    fn id(&self) -> &str {
        &self.id
    }
    fn family(&self) -> &'static str {
        "daily trend"
    }
    fn affinity(&self, r: Regime) -> f64 {
        match r {
            Regime::TrendingUp | Regime::TrendingDown => 1.0,
            Regime::Shock => 0.6,
            _ => 0.8,
        }
    }
    fn holds_overnight(&self) -> bool {
        true
    }
    fn on_session_start(&mut self, m: &crate::market::MarketState) {
        let closes: Vec<f64> = m.daily_closes().collect();
        self.closes_seen = closes.len();
        self.score = Self::composite_score(&closes);
    }
    fn entry(&mut self, ctx: &StrategyCtx) -> Option<EntrySignal> {
        if !ctx.m.new_session {
            return None;
        }
        let score = self.score?;
        if score.abs() < self.entry_score || (score < 0.0 && !self.allow_short) {
            return None;
        }
        let closes: Vec<f64> = ctx.m.daily_closes().collect();
        let atr = ctx.m.features.atr_d?;
        let vol = Self::realized_vol(&closes, 20)?;
        let side = if score > 0.0 { Side::Long } else { Side::Short };
        let c = ctx.bar.close;
        self.best_close = c;
        Some(EntrySignal {
            side,
            stop: c - side.sign() * self.stop_atr * atr,
            target: None,
            confidence: score.abs().clamp(0.5, 1.0),
            reason: format!("trend score {score:+.2}, 20d vol {:.0}%", vol * 100.0),
            max_notional_frac: Some((self.target_vol / vol.max(0.05) * score.abs()).min(1.5)),
        })
    }
    fn manage(&mut self, ctx: &StrategyCtx, pos: &OpenPosition) -> Manage {
        if !ctx.m.new_session {
            return Manage::Hold;
        }
        let Some(score) = self.score else { return Manage::Hold };
        if score * pos.side.sign() < self.exit_score {
            return Manage::Exit(format!("trend score decayed to {score:+.2}"));
        }
        let c = ctx.m.days.last().map(|d| d.close).unwrap_or(ctx.bar.close);
        self.best_close = if self.best_close.is_nan() { c } else if pos.side == Side::Long { self.best_close.max(c) } else { self.best_close.min(c) };
        let Some(atr) = ctx.m.features.atr_d else { return Manage::Hold };
        let trail = self.best_close - pos.side.sign() * self.stop_atr * atr;
        let tighter = match pos.side {
            Side::Long => trail > pos.stop,
            Side::Short => trail < pos.stop,
        };
        if tighter { Manage::MoveStop(trail) } else { Manage::Hold }
    }
    fn status(&self) -> Value {
        json!({"score": self.score, "days": self.closes_seen})
    }
}

/// Keep a bounded history (helper for tests).
pub fn push_bounded(v: &mut VecDeque<f64>, x: f64, cap: usize) {
    v.push_back(x);
    if v.len() > cap {
        v.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_signs() {
        let up: Vec<f64> = (0..200).map(|i| 100.0 * (1.0 + 0.003 * i as f64)).collect();
        let down: Vec<f64> = (0..200).map(|i| 100.0 * (1.0 - 0.003 * i as f64)).collect();
        assert!(DailyTrend::composite_score(&up).unwrap() > 0.8);
        assert!(DailyTrend::composite_score(&down).unwrap() < -0.8);
        assert!(DailyTrend::composite_score(&up[..100]).is_none());
    }
}
