//! q22-core — the shared vocabulary of the q22 trading system.
//!
//! * [`types`]      bars, sides, fills
//! * [`instrument`] contract specifications (tick size/value, commissions, sessions)
//! * [`session`]    CME equity-index and 24/7 crypto calendars (ET/CT aware)
//! * [`data`]       CSV loaders for the common free/paid bar formats + resampling
//! * [`ta`]         streaming (O(1) per bar) indicators — no look-ahead by construction
//! * [`stats`]      performance statistics incl. Probabilistic / Deflated Sharpe

pub mod data;
pub mod instrument;
pub mod session;
pub mod stats;
pub mod ta;
pub mod types;

pub use instrument::{InstrumentSpec, MarketKind};
pub use types::{Bar, Side};
