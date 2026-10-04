//! Market-regime classification.
//!
//! Computed once per session from *completed* daily bars (no look-ahead), plus a gap check
//! at the open. Thresholds are deliberately round numbers fixed in advance, not fitted.
//!
//! | Regime    | Rule (first match wins)                                         | Who trades it          |
//! |-----------|------------------------------------------------------------------|------------------------|
//! | Shock     | ATR percentile ≥ 95% or opening gap ≥ 1.5 daily ATR              | almost nobody, small   |
//! | Trending  | ADX(14) ≥ 25 or 10-day efficiency ratio ≥ 0.40                   | breakout / momentum    |
//! | Volatile  | ATR percentile ≥ 75%                                             | momentum, reduced size |
//! | Ranging   | ADX(14) < 20 and efficiency ratio < 0.25                         | mean reversion         |
//! | Neutral   | anything else                                                    | everyone, normal size  |

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Regime {
    Unknown,
    Shock,
    TrendingUp,
    TrendingDown,
    Volatile,
    Ranging,
    Neutral,
}

impl Regime {
    pub fn is_trending(self) -> bool {
        matches!(self, Regime::TrendingUp | Regime::TrendingDown)
    }
    pub fn label(self) -> &'static str {
        match self {
            Regime::Unknown => "unknown (warm-up)",
            Regime::Shock => "shock",
            Regime::TrendingUp => "trending up",
            Regime::TrendingDown => "trending down",
            Regime::Volatile => "volatile",
            Regime::Ranging => "ranging",
            Regime::Neutral => "neutral",
        }
    }
    pub const ALL: [Regime; 7] = [Regime::Unknown, Regime::Shock, Regime::TrendingUp, Regime::TrendingDown, Regime::Volatile, Regime::Ranging, Regime::Neutral];
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RegimeThresholds {
    pub shock_vol_pct: f64,
    pub shock_gap_atr: f64,
    pub trend_adx: f64,
    pub trend_er: f64,
    pub volatile_vol_pct: f64,
    pub range_adx: f64,
    pub range_er: f64,
    pub min_days: usize,
}

impl Default for RegimeThresholds {
    fn default() -> Self {
        Self { shock_vol_pct: 0.95, shock_gap_atr: 1.5, trend_adx: 25.0, trend_er: 0.40, volatile_vol_pct: 0.75, range_adx: 20.0, range_er: 0.25, min_days: 30 }
    }
}

/// The features behind a classification — shown on the dashboard so a human can audit it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RegimeFeatures {
    pub days: usize,
    pub atr_d: Option<f64>,
    pub atr_pct_of_price: Option<f64>,
    pub vol_percentile: Option<f64>,
    pub adx_d: Option<f64>,
    pub er_d: Option<f64>,
    pub trend_bias: Option<f64>,
    pub gap_atr: Option<f64>,
}

pub fn classify(f: &RegimeFeatures, t: &RegimeThresholds) -> Regime {
    if f.days < t.min_days || f.atr_d.is_none() {
        return Regime::Unknown;
    }
    let vp = f.vol_percentile.unwrap_or(0.5);
    let adx = f.adx_d.unwrap_or(0.0);
    let er = f.er_d.unwrap_or(0.0);
    if vp >= t.shock_vol_pct || f.gap_atr.is_some_and(|g| g.abs() >= t.shock_gap_atr) {
        return Regime::Shock;
    }
    if adx >= t.trend_adx || er >= t.trend_er {
        return if f.trend_bias.unwrap_or(0.0) >= 0.0 { Regime::TrendingUp } else { Regime::TrendingDown };
    }
    if vp >= t.volatile_vol_pct {
        return Regime::Volatile;
    }
    if adx < t.range_adx && er < t.range_er {
        return Regime::Ranging;
    }
    Regime::Neutral
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(vp: f64, adx: f64, er: f64, bias: f64, gap: f64) -> RegimeFeatures {
        RegimeFeatures { days: 100, atr_d: Some(1.0), atr_pct_of_price: Some(0.01), vol_percentile: Some(vp), adx_d: Some(adx), er_d: Some(er), trend_bias: Some(bias), gap_atr: Some(gap) }
    }

    #[test]
    fn rules_in_priority_order() {
        let t = RegimeThresholds::default();
        assert_eq!(classify(&f(0.97, 30.0, 0.5, 1.0, 0.0), &t), Regime::Shock);
        assert_eq!(classify(&f(0.5, 10.0, 0.1, 1.0, 2.0), &t), Regime::Shock);
        assert_eq!(classify(&f(0.5, 30.0, 0.1, -1.0, 0.0), &t), Regime::TrendingDown);
        assert_eq!(classify(&f(0.5, 10.0, 0.5, 1.0, 0.0), &t), Regime::TrendingUp);
        assert_eq!(classify(&f(0.8, 22.0, 0.3, 1.0, 0.0), &t), Regime::Volatile);
        assert_eq!(classify(&f(0.5, 15.0, 0.1, 1.0, 0.0), &t), Regime::Ranging);
        assert_eq!(classify(&f(0.5, 22.0, 0.3, 1.0, 0.0), &t), Regime::Neutral);
        let mut w = f(0.5, 22.0, 0.3, 1.0, 0.0);
        w.days = 5;
        assert_eq!(classify(&w, &t), Regime::Unknown);
    }
}
