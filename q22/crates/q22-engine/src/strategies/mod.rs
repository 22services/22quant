//! The strategy book. Each one is a published, mechanically-motivated idea with fixed default
//! parameters (no curve fitting), and each declares which regimes it is built for.
//!
//! | id               | idea / source                                                      | regimes          |
//! |------------------|--------------------------------------------------------------------|------------------|
//! | `noise_breakout` | Zarattini, Aziz & Barbon (2024) intraday momentum "noise area"     | trend, volatile  |
//! | `orb`            | Opening-range breakout (Crabel 1990; Zarattini & Aziz 2023)        | trend, volatile  |
//! | `last_half_hour` | Gao, Han, Li & Zhou (2018) market intraday momentum                | all but shock    |
//! | `vwap_reversion` | Fade 2σ VWAP extensions on range days (liquidity provision)        | ranging, neutral |
//! | `daily_trend`    | Time-series momentum ensemble (Moskowitz et al. 2012; q22 research)| any, 24/7 assets |

pub mod daily_trend;
pub mod last_half_hour;
pub mod noise_breakout;
pub mod orb;
pub mod vwap_reversion;

use anyhow::{bail, Result};

use crate::config::StrategyConfig;
use crate::strategy::Strategy;

pub fn build(cfg: &StrategyConfig) -> Result<Box<dyn Strategy>> {
    let p = &cfg.params;
    Ok(match cfg.kind.as_str() {
        "noise_breakout" => Box::new(noise_breakout::NoiseBreakout::from_params(&cfg.id, p)?),
        "orb" => Box::new(orb::OpeningRangeBreakout::from_params(&cfg.id, p)?),
        "last_half_hour" => Box::new(last_half_hour::LastHalfHour::from_params(&cfg.id, p)?),
        "vwap_reversion" => Box::new(vwap_reversion::VwapReversion::from_params(&cfg.id, p)?),
        "daily_trend" => Box::new(daily_trend::DailyTrend::from_params(&cfg.id, p)?),
        other => bail!("unknown strategy kind {other:?}"),
    })
}

/// Read a numeric parameter with a default.
pub(crate) fn param(p: &serde_json::Map<String, serde_json::Value>, key: &str, default: f64) -> f64 {
    p.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
}

pub(crate) fn param_bool(p: &serde_json::Map<String, serde_json::Value>, key: &str, default: bool) -> bool {
    p.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}
