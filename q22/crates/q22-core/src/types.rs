use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One OHLCV bar. `ts` is the bar's **open** time (UTC); the bar is complete at
/// `ts + timeframe`. Every decision made "on" a bar uses only completed bars.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Bar {
    pub ts: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl Bar {
    pub fn range(&self) -> f64 {
        self.high - self.low
    }
    pub fn typical(&self) -> f64 {
        (self.high + self.low + self.close) / 3.0
    }
    pub fn is_valid(&self) -> bool {
        self.open.is_finite()
            && self.high.is_finite()
            && self.low.is_finite()
            && self.close.is_finite()
            && self.high >= self.low
            && self.open > 0.0
            && self.close > 0.0
            && self.high >= self.open.max(self.close) - 1e-9
            && self.low <= self.open.min(self.close) + 1e-9
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Side {
    Long,
    Short,
}

impl Side {
    pub fn sign(self) -> f64 {
        match self {
            Side::Long => 1.0,
            Side::Short => -1.0,
        }
    }
    pub fn opposite(self) -> Side {
        match self {
            Side::Long => Side::Short,
            Side::Short => Side::Long,
        }
    }
    pub fn from_sign(x: f64) -> Option<Side> {
        if x > 0.0 {
            Some(Side::Long)
        } else if x < 0.0 {
            Some(Side::Short)
        } else {
            None
        }
    }
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Side::Long => write!(f, "LONG"),
            Side::Short => write!(f, "SHORT"),
        }
    }
}
