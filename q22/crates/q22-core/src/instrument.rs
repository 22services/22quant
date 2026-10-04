use serde::{Deserialize, Serialize};

/// Which trading calendar an instrument follows.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MarketKind {
    /// CME Globex equity-index futures: Sun 17:00 CT → Fri 16:00 CT, daily halt 16:00–17:00 CT,
    /// trading day rolls at 17:00 CT, regular trading hours 09:30–16:00 ET.
    CmeEquityIndex,
    /// 24/7 crypto (perpetuals / spot). Trading day = UTC calendar day.
    Crypto,
}

/// Contract specification. All money is USD.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InstrumentSpec {
    pub symbol: String,
    pub description: String,
    pub kind: MarketKind,
    pub tick_size: f64,
    /// USD value of one tick for one contract (futures) or for one unit of the base asset (crypto).
    pub tick_value: f64,
    /// Smallest tradable quantity increment (1 contract for futures, e.g. 0.001 BTC for crypto).
    pub qty_step: f64,
    pub min_qty: f64,
    /// Round-turn is 2× this. Futures: USD per contract per side (commission + exchange fees).
    pub commission_per_side: f64,
    /// Crypto: fee as a fraction of notional per side (taker). Zero for futures.
    pub fee_rate: f64,
    /// Instruments in the same group are treated as one exposure by the anti-hedging rule.
    pub group: String,
}

impl InstrumentSpec {
    pub fn point_value(&self) -> f64 {
        self.tick_value / self.tick_size
    }
    pub fn round_price(&self, px: f64) -> f64 {
        (px / self.tick_size).round() * self.tick_size
    }
    pub fn round_qty_down(&self, q: f64) -> f64 {
        let steps = (q / self.qty_step + 1e-9).floor();
        let out = steps * self.qty_step;
        if out < self.min_qty {
            0.0
        } else {
            // avoid binary noise like 0.30000000000000004
            (out * 1e8).round() / 1e8
        }
    }
    /// Commission + fees in USD for trading `qty` at `price` on one side.
    pub fn cost_one_side(&self, qty: f64, price: f64) -> f64 {
        qty.abs() * self.commission_per_side + qty.abs() * price * self.point_value() * self.fee_rate
    }
    /// USD P&L of a price move for `qty` contracts/units.
    pub fn pnl(&self, qty: f64, from: f64, to: f64) -> f64 {
        qty * (to - from) * self.point_value()
    }

    /// Built-in specs. Commissions are conservative all-in estimates (commission + exchange + NFA);
    /// override in the config file with your firm's exact schedule.
    pub fn builtin(symbol: &str) -> Option<InstrumentSpec> {
        let s = symbol.to_ascii_uppercase();
        let fut = |sym: &str, desc: &str, tick: f64, tv: f64, comm: f64| InstrumentSpec {
            symbol: sym.into(),
            description: desc.into(),
            kind: MarketKind::CmeEquityIndex,
            tick_size: tick,
            tick_value: tv,
            qty_step: 1.0,
            min_qty: 1.0,
            commission_per_side: comm,
            fee_rate: 0.0,
            group: "us_equity_index".into(),
        };
        let crypto = |sym: &str, desc: &str, tick: f64, step: f64, fee: f64, group: &str| InstrumentSpec {
            symbol: sym.into(),
            description: desc.into(),
            kind: MarketKind::Crypto,
            tick_size: tick,
            tick_value: tick, // 1 unit of base asset moves $1 per $1 of price
            qty_step: step,
            min_qty: step,
            commission_per_side: 0.0,
            fee_rate: fee,
            group: group.into(),
        };
        Some(match s.as_str() {
            "NQ" => fut("NQ", "E-mini Nasdaq-100 ($20/pt)", 0.25, 5.0, 2.50),
            "MNQ" => fut("MNQ", "Micro E-mini Nasdaq-100 ($2/pt)", 0.25, 0.50, 0.75),
            "ES" => fut("ES", "E-mini S&P 500 ($50/pt)", 0.25, 12.50, 2.50),
            "MES" => fut("MES", "Micro E-mini S&P 500 ($5/pt)", 0.25, 1.25, 0.75),
            "YM" => fut("YM", "E-mini Dow ($5/pt)", 1.0, 5.0, 2.50),
            "MYM" => fut("MYM", "Micro E-mini Dow ($0.50/pt)", 1.0, 0.50, 0.75),
            "RTY" => fut("RTY", "E-mini Russell 2000 ($50/pt)", 0.1, 5.0, 2.50),
            "M2K" => fut("M2K", "Micro E-mini Russell 2000 ($5/pt)", 0.1, 0.50, 0.75),
            "MBT" => {
                let mut x = fut("MBT", "CME Micro Bitcoin (0.1 BTC)", 5.0, 0.50, 2.50);
                x.group = "crypto_btc".into();
                x
            }
            "BTCUSDT" => crypto("BTCUSDT", "BTC/USDT linear perpetual", 0.1, 0.001, 0.00055, "crypto_btc"),
            "ETHUSDT" => crypto("ETHUSDT", "ETH/USDT linear perpetual", 0.01, 0.01, 0.00055, "crypto_eth"),
            "SOLUSDT" => crypto("SOLUSDT", "SOL/USDT linear perpetual", 0.01, 0.1, 0.00055, "crypto_sol"),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_values() {
        assert_eq!(InstrumentSpec::builtin("NQ").unwrap().point_value(), 20.0);
        assert_eq!(InstrumentSpec::builtin("mnq").unwrap().point_value(), 2.0);
        assert_eq!(InstrumentSpec::builtin("ES").unwrap().point_value(), 50.0);
        assert!((InstrumentSpec::builtin("MBT").unwrap().point_value() - 0.1).abs() < 1e-12);
        assert_eq!(InstrumentSpec::builtin("BTCUSDT").unwrap().point_value(), 1.0);
    }

    #[test]
    fn rounding() {
        let nq = InstrumentSpec::builtin("NQ").unwrap();
        assert_eq!(nq.round_price(100.13), 100.25);
        assert_eq!(nq.round_qty_down(2.9), 2.0);
        let btc = InstrumentSpec::builtin("BTCUSDT").unwrap();
        assert_eq!(btc.round_qty_down(0.0129), 0.012);
        assert_eq!(btc.round_qty_down(0.0004), 0.0);
        assert!((btc.cost_one_side(0.01, 80000.0) - 0.44).abs() < 1e-9);
    }
}
