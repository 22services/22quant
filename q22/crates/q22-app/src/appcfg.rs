//! Application config: the engine section (account, guard, allocator, instruments, strategies)
//! plus `[runtime]` (broker, feed) and `[dashboard]`. One TOML file drives everything.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use q22_engine::EngineConfig;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrokerKind {
    /// Simulated fills on the incoming bars (same fill model as the backtester).
    Paper,
    /// TopstepX (or another ProjectX Gateway deployment). Needs Q22_PROJECTX_USER / Q22_PROJECTX_KEY.
    Projectx,
    /// Bybit v5 (HyroTrader sub-account or personal). Needs Q22_BYBIT_KEY / Q22_BYBIT_SECRET.
    Bybit,
    /// Rithmic R|Protocol (Lucid Trading and other Rithmic prop firms). Needs Q22_RITHMIC_* and a
    /// conformance-approved app name.
    Rithmic,
    /// Tradovate REST + market-data WebSocket (personal Tradovate accounts only). Needs Q22_TRADOVATE_*.
    Tradovate,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedKind {
    /// Replay CSV files at `replay_speed` bars per second (demo / rehearsal, no account needed).
    Replay,
    /// Bars polled from the configured broker (ProjectX or Bybit).
    Broker,
    /// Free public Bybit klines (crypto paper trading without any key).
    BybitPublic,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub broker: BrokerKind,
    pub feed: FeedKind,
    #[serde(default)]
    pub replay_files: BTreeMap<String, String>,
    #[serde(default = "d_speed")]
    pub replay_speed: f64,
    #[serde(default)]
    pub replay_from: Option<String>,
    #[serde(default = "d_poll")]
    pub poll_secs: u64,
    #[serde(default = "d_warmup")]
    pub warmup_days: i64,
    #[serde(default)]
    pub projectx_api: Option<String>,
    #[serde(default)]
    pub projectx_account: Option<String>,
    #[serde(default = "d_bybit")]
    pub bybit_base: String,
    #[serde(default = "d_state")]
    pub state_dir: String,
    #[serde(default = "yes")]
    pub flatten_on_exit: bool,
    /// What to do with a broker position the engine did not open: "alert" (pause) or "flatten".
    #[serde(default = "d_unmanaged")]
    pub on_unmanaged_position: String,
}

fn d_speed() -> f64 {
    50.0
}
fn d_poll() -> u64 {
    5
}
fn d_warmup() -> i64 {
    90
}
fn d_bybit() -> String {
    q22_broker::bybit::DEMO.to_string()
}
fn d_state() -> String {
    "state".into()
}
fn yes() -> bool {
    true
}
fn d_unmanaged() -> String {
    "alert".into()
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            broker: BrokerKind::Paper,
            feed: FeedKind::Replay,
            replay_files: BTreeMap::new(),
            replay_speed: d_speed(),
            replay_from: None,
            poll_secs: d_poll(),
            warmup_days: d_warmup(),
            projectx_api: None,
            projectx_account: None,
            bybit_base: d_bybit(),
            state_dir: d_state(),
            flatten_on_exit: true,
            on_unmanaged_position: d_unmanaged(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DashboardConfig {
    #[serde(default = "d_bind")]
    pub bind: String,
    #[serde(default = "d_reports")]
    pub reports_dir: String,
}

fn d_bind() -> String {
    "127.0.0.1:8722".into()
}
fn d_reports() -> String {
    "reports".into()
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self { bind: d_bind(), reports_dir: d_reports() }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Sections {
    #[serde(default)]
    runtime: RuntimeConfig,
    #[serde(default)]
    dashboard: DashboardConfig,
}

pub struct AppConfig {
    pub engine: EngineConfig,
    pub runtime: RuntimeConfig,
    pub dashboard: DashboardConfig,
    pub path: PathBuf,
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let engine: EngineConfig = toml::from_str(&text).with_context(|| format!("parsing engine config in {}", path.display()))?;
        let s: Sections = toml::from_str(&text).with_context(|| format!("parsing [runtime]/[dashboard] in {}", path.display()))?;
        Ok(Self { engine, runtime: s.runtime, dashboard: s.dashboard, path: path.to_path_buf() })
    }

    /// Resolve a path relative to the config file's directory.
    pub fn rel(&self, p: &str) -> PathBuf {
        let pb = PathBuf::from(p);
        if pb.is_absolute() {
            pb
        } else {
            self.path.parent().unwrap_or(Path::new(".")).join(pb)
        }
    }
}
