//! Engine configuration (deserialised from TOML by the app).

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use q22_core::InstrumentSpec;

use crate::allocator::AllocatorConfig;
use crate::guard::GuardConfig;
use crate::prop::PropRules;
use crate::regime::RegimeThresholds;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Mode {
    /// Orders are sent automatically (firms that allow bots: Topstep via API, HyroTrader, Lucid…).
    #[default]
    Auto,
    /// Every entry waits for a human click in the dashboard (Apex PA and similar).
    Assist,
    /// Nothing is sent; signals are shown and logged (alerting / paper study).
    Signals,
}


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountConfig {
    /// Preset name (see `q22 rules`), e.g. "topstep_50k".
    pub rules: String,
    #[serde(default)]
    pub size: Option<f64>,
    /// Field-by-field overrides of the preset, e.g. `{ daily_loss_limit = 1000 }`.
    #[serde(default)]
    pub overrides: Map<String, Value>,
    /// Resume a live account mid-evaluation: current balance and the firm's current threshold.
    #[serde(default)]
    pub resume_balance: Option<f64>,
    #[serde(default)]
    pub resume_threshold: Option<f64>,
    #[serde(default)]
    pub resume_high_eod_balance: Option<f64>,
}

impl AccountConfig {
    pub fn resolve_rules(&self) -> Result<PropRules> {
        let base = PropRules::preset(&self.rules, self.size).ok_or_else(|| anyhow!("unknown rules preset {:?}; try one of {:?}", self.rules, PropRules::preset_names()))?;
        if self.overrides.is_empty() {
            return Ok(base);
        }
        let mut v = serde_json::to_value(&base)?;
        let obj = v.as_object_mut().unwrap();
        for (k, val) in &self.overrides {
            if !obj.contains_key(k) {
                return Err(anyhow!("unknown rule override {k:?}"));
            }
            obj.insert(k.clone(), val.clone());
        }
        serde_json::from_value(v).context("applying rule overrides")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstrumentConfig {
    /// Built-in symbol (NQ, MNQ, ES, MES, MBT, BTCUSDT …).
    pub symbol: String,
    #[serde(default = "default_tf")]
    pub timeframe_min: i64,
    /// Broker-side search text / contract id (e.g. "MNQ" or "CON.F.US.MNQ.Z26"; "BTCUSDT").
    #[serde(default)]
    pub broker_symbol: Option<String>,
    #[serde(default = "default_slip")]
    pub slippage_ticks: f64,
    /// Field overrides for the instrument spec (commission_per_side, fee_rate …).
    #[serde(default)]
    pub spec_overrides: Map<String, Value>,
}

fn default_tf() -> i64 {
    5
}
fn default_slip() -> f64 {
    1.0
}

impl InstrumentConfig {
    pub fn spec(&self) -> Result<InstrumentSpec> {
        let base = InstrumentSpec::builtin(&self.symbol).ok_or_else(|| anyhow!("unknown instrument {:?}", self.symbol))?;
        if self.spec_overrides.is_empty() {
            return Ok(base);
        }
        let mut v = serde_json::to_value(&base)?;
        for (k, val) in &self.spec_overrides {
            v.as_object_mut().unwrap().insert(k.clone(), val.clone());
        }
        Ok(serde_json::from_value(v)?)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StrategyConfig {
    pub id: String,
    pub kind: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Symbols this strategy trades (empty = all instruments).
    #[serde(default)]
    pub instruments: Vec<String>,
    #[serde(default)]
    pub params: Map<String, Value>,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EngineConfig {
    #[serde(default)]
    pub mode: Mode,
    pub account: AccountConfig,
    #[serde(default)]
    pub guard: GuardConfig,
    #[serde(default)]
    pub allocator: AllocatorConfig,
    #[serde(default)]
    pub regime: RegimeThresholds,
    #[serde(rename = "instrument")]
    pub instruments: Vec<InstrumentConfig>,
    #[serde(rename = "strategy")]
    pub strategies: Vec<StrategyConfig>,
    /// Minutes an assist-mode intent waits for approval before expiring.
    #[serde(default = "default_approval")]
    pub approval_timeout_min: i64,
}

fn default_approval() -> i64 {
    3
}

impl EngineConfig {
    /// A ready-made config: Topstep 50K, MNQ 5-minute, the four intraday strategies.
    pub fn example_futures() -> Self {
        let mut guard = GuardConfig::default();
        guard.risk_per_trade_usd = Some(250.0);
        guard.risk_per_trade_frac = None;
        let s = |id: &str| StrategyConfig { id: id.into(), kind: id.into(), enabled: true, instruments: vec![], params: Map::new() };
        Self {
            mode: Mode::Auto,
            account: AccountConfig { rules: "topstep_50k".into(), size: None, overrides: Map::new(), resume_balance: None, resume_threshold: None, resume_high_eod_balance: None },
            guard,
            allocator: AllocatorConfig::default(),
            regime: RegimeThresholds::default(),
            instruments: vec![InstrumentConfig { symbol: "MNQ".into(), timeframe_min: 5, broker_symbol: None, slippage_ticks: 1.0, spec_overrides: Map::new() }],
            strategies: vec![s("noise_breakout"), s("orb"), s("last_half_hour"), s("vwap_reversion")],
            approval_timeout_min: 3,
        }
    }
}
