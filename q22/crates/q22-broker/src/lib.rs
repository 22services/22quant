//! Broker adapters.
//!
//! * [`projectx`] — ProjectX Gateway REST API: **TopstepX** (default) and other ProjectX
//!   deployments (e.g. TheFuturesDesk). Topstep allows automated trading only through this API
//!   ($14.50–29/month), from your own device (no VPS/VPN), actively monitored.
//! * [`bybit`] — Bybit v5 REST API (linear perpetuals). **HyroTrader** funded accounts are Bybit
//!   sub-accounts operated through this API; it is also the free public data feed for crypto.
//! * [`rithmic`] — Rithmic R | Protocol (WebSocket + protobuf, via `rithmic-rs`): **Lucid
//!   Trading** and other Rithmic-cleared prop firms, after Rithmic's conformance test.
//! * [`tradovate`] — Tradovate REST + market-data WebSocket, for *personal* Tradovate accounts
//!   (Tradovate does not give API access to prop/evaluation accounts).
//!
//! The adapters are deliberately thin and auditable: money-moving calls are never retried
//! automatically, calls are rate-limited below the provider's published limits, automated
//! orders carry the exchange's automated-order flag, and protective stops always rest on the
//! broker's side so a crash of this program never leaves a position unprotected.

pub mod bybit;
pub mod projectx;
#[cfg(feature = "rithmic")]
pub mod rithmic;
pub mod tradovate;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;

use q22_core::{Bar, Side};

#[derive(Clone, Debug, Serialize)]
pub struct AccountInfo {
    pub id: String,
    pub name: String,
    pub balance: f64,
    pub can_trade: bool,
    pub simulated: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BrokerPosition {
    pub symbol: String,
    /// Signed quantity (+ long, − short).
    pub qty: f64,
    pub avg_price: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct EntryAck {
    pub order_id: String,
    /// Average fill price if the broker reported it synchronously.
    pub fill_price: Option<f64>,
    pub filled_qty: f64,
    pub stop_order_id: Option<String>,
    pub target_order_id: Option<String>,
}

#[async_trait]
pub trait Broker: Send + Sync {
    fn name(&self) -> &str;
    /// Authenticate and select the trading account.
    async fn connect(&mut self) -> Result<AccountInfo>;
    async fn account(&self) -> Result<AccountInfo>;
    /// Completed bars with open time ≥ `since` (oldest first).
    async fn bars(&self, symbol: &str, tf_minutes: i64, since: DateTime<Utc>) -> Result<Vec<Bar>>;
    /// Market entry with a protective stop (and optional target) attached or placed right after.
    async fn enter(&self, symbol: &str, side: Side, qty: f64, stop: f64, target: Option<f64>, tag: &str) -> Result<EntryAck>;
    async fn modify_stop(&self, symbol: &str, side: Side, qty: f64, new_stop: f64) -> Result<()>;
    /// Flatten the symbol at market and cancel its working orders. Returns the exit price if known.
    async fn close(&self, symbol: &str) -> Result<Option<f64>>;
    async fn positions(&self) -> Result<Vec<BrokerPosition>>;
    async fn cancel_all(&self, symbol: &str) -> Result<()>;
    /// Price of the most recent fill that closed a position in `symbol`, if available.
    async fn last_exit_price(&self, symbol: &str) -> Result<Option<f64>>;
}
