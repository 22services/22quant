#![recursion_limit = "512"]
//! q22-engine — the decision core shared *verbatim* by the backtester and the live runner.
//!
//! ```text
//!  bar ─► MarketState ─► Regime ─► Strategies ─► Allocator ─► Sizing ─► ComplianceGuard ─► Commands
//!                                                      ▲                    ▲
//!                                           strategy health        PropTracker (firm rules)
//! ```
//!
//! Nothing in here knows whether it is running on history or on a live account: the
//! [`engine::Engine`] consumes bars and fills and emits [`engine::Command`]s. The
//! backtester ([`sim`]) and the live runner (q22-app) are thin adapters around it, which is
//! the single best protection against "it worked in the backtest" bugs.

pub mod allocator;
pub mod config;
pub mod engine;
pub mod guard;
pub mod market;
pub mod prop;
pub mod regime;
pub mod sim;
pub mod snapshot;
pub mod strategies;
pub mod strategy;

pub use config::EngineConfig;
pub use engine::{Command, Engine, Fill};
pub use regime::Regime;
