//! Broker adapters.
//!
//! * [`projectx`] — ProjectX Gateway REST API: **TopstepX** (default) and other ProjectX
//!   deployments (e.g. TheFuturesDesk). Topstep allows automated trading only through this API
//!   ($14.50–29/month), from your own device (no VPS/VPN), actively monitored.
//! * [`bybit`] — Bybit v5 REST API (linear perpetuals). **HyroTrader** funded accounts are Bybit
//!   sub-accounts operated through this API; it is also the free public data feed for crypto.
//!
//! Both adapters are deliberately thin, polling-based and auditable: money-moving calls are
//! never retried automatically, every call is rate-limited below the provider's published
//! limits, and protective stops are always resting on the broker's side so a crash of this
//! program never leaves a position unprotected.

pub mod bybit;
pub mod projectx;

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
