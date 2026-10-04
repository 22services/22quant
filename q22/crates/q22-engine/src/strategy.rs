//! The strategy contract.
//!
//! A strategy never sizes, never talks to a broker and never checks firm rules. It only says
//! *"if you are flat, here is a trade with its invalidation level"* and *"for the trade I own,
//! hold / tighten the stop / get out"*. Sizing, allocation and compliance happen downstream.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use q22_core::{Bar, InstrumentSpec, Side};

use crate::market::MarketState;
use crate::regime::Regime;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntrySignal {
    pub side: Side,
    /// Protective stop (absolute price). Mandatory — every trade has an invalidation level.
    pub stop: f64,
    /// Optional take-profit (absolute price).
    pub target: Option<f64>,
    /// 0..1 — how clean the setup is (strategy-specific).
    pub confidence: f64,
    pub reason: String,
    /// Optional cap on position notional as a fraction of equity (vol-targeted strategies).
    pub max_notional_frac: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OpenPosition {
    pub trade_id: u64,
    pub strategy: String,
    pub side: Side,
    pub qty: f64,
    pub entry_price: f64,
    pub entry_time: DateTime<Utc>,
    pub stop: f64,
    pub initial_stop: f64,
    pub target: Option<f64>,
    pub regime_at_entry: Regime,
    pub bars_held: usize,
    pub mfe_price: f64,
    pub mae_price: f64,
}

impl OpenPosition {
    pub fn unrealized(&self, spec: &InstrumentSpec, price: f64) -> f64 {
        spec.pnl(self.qty * self.side.sign(), self.entry_price, price)
    }
    pub fn initial_risk_points(&self) -> f64 {
        (self.entry_price - self.initial_stop).abs()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Manage {
    Hold,
    /// New stop price; the engine only accepts moves that *reduce* risk.
    MoveStop(f64),
    Exit(String),
}

/// Read-only view of another configured instrument, updated with the **same** timestamp as
/// the bar being decided (the engine ingests every bar of a timestamp before any decision).
pub struct PeerRef<'a> {
    pub symbol: &'a str,
    pub m: &'a MarketState,
    pub last_close: f64,
}

pub struct StrategyCtx<'a> {
    pub symbol: &'a str,
    pub spec: &'a InstrumentSpec,
    pub bar: &'a Bar,
    pub bar_end: DateTime<Utc>,
    pub m: &'a MarketState,
    pub regime: Regime,
    /// Minutes from the session open to the end of this bar.
    pub offset: i64,
    /// The other instruments (cross-asset strategies: confirmation, divergence, lead–lag).
    pub peers: &'a [PeerRef<'a>],
}

impl StrategyCtx<'_> {
    /// The named peer, or the first one when `symbol` is None.
    pub fn peer(&self, symbol: Option<&str>) -> Option<&PeerRef<'_>> {
        match symbol {
            Some(s) => self.peers.iter().find(|p| p.symbol.eq_ignore_ascii_case(s)),
            None => self.peers.first(),
        }
    }
}

pub trait Strategy: Send {
    fn id(&self) -> &str;
    fn family(&self) -> &'static str;
    /// How well this strategy's edge fits a regime, 0..1. Used by the allocator.
    fn affinity(&self, r: Regime) -> f64;
    /// Strategies that hold through the session break are refused on prop futures accounts.
    fn holds_overnight(&self) -> bool {
        false
    }
    /// Called on the first bar of every session (after the market state rolled).
    fn on_session_start(&mut self, _m: &MarketState) {}
    /// Called on every completed in-session bar while the instrument is flat.
    fn entry(&mut self, ctx: &StrategyCtx) -> Option<EntrySignal>;
    /// Called on every completed bar for the position this strategy owns.
    fn manage(&mut self, _ctx: &StrategyCtx, _pos: &OpenPosition) -> Manage {
        Manage::Hold
    }
    fn on_trade_closed(&mut self, _r_multiple: f64) {}
    /// Internal levels for the dashboard (opening range, bands, …).
    fn status(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
}
