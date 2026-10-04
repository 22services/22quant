//! Regime-aware meta-allocator.
//!
//! When several strategies want to trade the same instrument on the same bar, the allocator
//! picks one (prop accounts hold one net position per instrument and forbid hedging):
//!
//! `score = affinity(regime) × health × confidence`
//!
//! * `affinity` — the strategy's declared fit for today's regime (fixed in code, not fitted);
//! * `health`   — 1 + 0.25·tanh(t/2) from the t-stat of the strategy's last N trades in R
//!   (0.75 … 1.25; neutral until 10 trades). It de-risks what is not working, like a CTA's
//!   drawdown control, without switching anything fully off on noise;
//! * `confidence` — the strategy's own setup quality.
//!
//! The best candidate is taken if its score ≥ `min_score`; opposite-side candidates whose
//! scores are within `conflict_margin` of each other cancel out (no trade). The score also
//! scales the risk budget (clamped to [0.5, 1]).

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::regime::Regime;
use crate::strategy::EntrySignal;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AllocatorConfig {
    pub regime_gating: bool,
    pub adaptive_health: bool,
    pub min_score: f64,
    pub conflict_margin: f64,
    pub health_window: usize,
}

impl Default for AllocatorConfig {
    fn default() -> Self {
        Self { regime_gating: true, adaptive_health: true, min_score: 0.5, conflict_margin: 0.15, health_window: 20 }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StrategyHealth {
    pub recent_r: VecDeque<f64>,
    pub trades: usize,
    pub sum_r: f64,
    pub wins: usize,
}

impl StrategyHealth {
    pub fn health(&self) -> f64 {
        let n = self.recent_r.len();
        if n < 10 {
            return 1.0;
        }
        let m = self.recent_r.iter().sum::<f64>() / n as f64;
        let sd = (self.recent_r.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n as f64 - 1.0)).sqrt().max(0.25);
        let t = m / (sd / (n as f64).sqrt());
        1.0 + 0.25 * (t / 2.0).tanh()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    pub strategy: String,
    pub signal: EntrySignal,
    pub affinity: f64,
    pub health: f64,
    pub score: f64,
}

#[derive(Clone, Debug, Serialize)]
pub enum Decision {
    Take { candidate: Candidate, size_mult: f64 },
    None { reason: String, candidates: Vec<Candidate> },
}

pub struct Allocator {
    pub cfg: AllocatorConfig,
    pub health: HashMap<String, StrategyHealth>,
}

impl Allocator {
    pub fn new(cfg: AllocatorConfig) -> Self {
        Self { cfg, health: HashMap::new() }
    }

    pub fn record(&mut self, strategy: &str, r: f64) {
        let w = self.cfg.health_window;
        let h = self.health.entry(strategy.to_string()).or_default();
        h.recent_r.push_back(r);
        if h.recent_r.len() > w {
            h.recent_r.pop_front();
        }
        h.trades += 1;
        h.sum_r += r;
        if r > 0.0 {
            h.wins += 1;
        }
    }

    pub fn health_of(&self, strategy: &str) -> f64 {
        if !self.cfg.adaptive_health {
            return 1.0;
        }
        self.health.get(strategy).map_or(1.0, |h| h.health())
    }

    /// `signals`: (strategy id, affinity for the current regime, signal).
    pub fn decide(&self, regime: Regime, signals: Vec<(String, f64, EntrySignal)>) -> Decision {
        let _ = regime;
        let mut c: Vec<Candidate> = signals
            .into_iter()
            .map(|(id, aff, s)| {
                let affinity = if self.cfg.regime_gating { aff } else { 1.0 };
                let health = self.health_of(&id);
                let score = affinity * health * s.confidence.clamp(0.0, 1.0);
                Candidate { strategy: id, signal: s, affinity, health, score }
            })
            .collect();
        if c.is_empty() {
            return Decision::None { reason: "no signal".into(), candidates: c };
        }
        c.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        let best = &c[0];
        if best.score < self.cfg.min_score {
            return Decision::None { reason: format!("best score {:.2} < {:.2} ({})", best.score, self.cfg.min_score, best.strategy), candidates: c };
        }
        if let Some(opp) = c.iter().skip(1).find(|x| x.signal.side != best.signal.side) {
            if best.score - opp.score < self.cfg.conflict_margin {
                return Decision::None { reason: format!("conflict {} vs {}", best.strategy, opp.strategy), candidates: c };
            }
        }
        let size_mult = best.score.clamp(0.5, 1.0);
        Decision::Take { candidate: c.remove(0), size_mult }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use q22_core::Side;

    fn sig(side: Side, conf: f64) -> EntrySignal {
        EntrySignal { side, stop: 0.0, target: None, confidence: conf, reason: String::new(), max_notional_frac: None }
    }

    #[test]
    fn picks_best_and_respects_threshold_and_conflicts() {
        let a = Allocator::new(AllocatorConfig::default());
        match a.decide(Regime::Ranging, vec![("mom".into(), 0.3, sig(Side::Long, 1.0)), ("mr".into(), 1.0, sig(Side::Short, 0.8))]) {
            Decision::Take { candidate, .. } => assert_eq!(candidate.strategy, "mr"),
            d => panic!("{d:?}"),
        }
        assert!(matches!(a.decide(Regime::Ranging, vec![("mom".into(), 0.3, sig(Side::Long, 1.0))]), Decision::None { .. }));
        assert!(matches!(a.decide(Regime::Neutral, vec![("a".into(), 0.8, sig(Side::Long, 1.0)), ("b".into(), 0.8, sig(Side::Short, 0.95))]), Decision::None { .. }));
        let mut ng = Allocator::new(AllocatorConfig { regime_gating: false, ..Default::default() });
        assert!(matches!(ng.decide(Regime::Ranging, vec![("mom".into(), 0.3, sig(Side::Long, 1.0))]), Decision::Take { .. }));
        for _ in 0..20 {
            ng.record("mom", -1.0);
        }
        ng.record("mom", 0.5);
        assert!(ng.health_of("mom") < 0.8);
    }
}
