//! Single-trade simulation on aligned minute bars, with the pre-registered fill and cost model,
//! plus the time-matched placebo and summary statistics.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::Serialize;

use crate::pair::{Pair, SM_FLAT};

/// Per-side costs of one micro contract.
#[derive(Clone, Copy, Debug)]
pub struct Costs {
    pub tick: f64,
    pub point_value: f64,
    pub commission: f64,
    pub slip_ticks: f64,
}

/// MNQ and MES (index 0 = NQ, 1 = ES), pre-registered: $0.75 commission + 1 tick per side.
pub const MICRO: [Costs; 2] = [Costs { tick: 0.25, point_value: 2.0, commission: 0.75, slip_ticks: 1.0 }, Costs { tick: 0.25, point_value: 5.0, commission: 0.75, slip_ticks: 1.0 }];

#[derive(Clone, Debug)]
pub struct Trade {
    pub day: usize,
    pub asset: usize,
    /// +1 long, −1 short
    pub side: f64,
    /// Bar whose **open** is the entry.
    pub entry_idx: usize,
    pub stop: f64,
    pub target: Option<f64>,
    /// Last bar held; exit at its close if nothing else happened (≤ the day's 15:55 bar).
    pub exit_idx: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Outcome {
    pub r: f64,
    pub r_gross: f64,
    pub usd: f64,
    pub risk_pts: f64,
    pub exit: &'static str,
    pub bars: usize,
}

/// How costs are charged. `Contract`: the registered per-micro commission + slippage ticks.
/// `Bps`: a fraction of notional per side (index 0 = NQ, 1 = ES), calibrated to today's prices —
/// use with ratio-adjusted data (PREREGISTRATION.md D-3).
#[derive(Clone, Copy, Debug)]
pub enum CostModel {
    Contract,
    Bps([f64; 2]),
}

/// Today-calibrated: MNQ $1.25/side at NQ 25,000; MES $2.00/side at ES 6,500.
pub const BPS_TODAY: [f64; 2] = [0.000025, 0.0000615];

pub fn simulate(p: &Pair, t: &Trade, cost_mult: f64) -> Option<Outcome> {
    let c = MICRO[t.asset];
    let px = &p.px[t.asset];
    let (slip, bps) = match p.cost {
        CostModel::Contract => (c.slip_ticks * c.tick * cost_mult, 0.0),
        CostModel::Bps(r) => (0.0, r[t.asset] * cost_mult),
    };
    let commission = if bps > 0.0 { 0.0 } else { c.commission * cost_mult };
    let day = &p.days[t.day];
    let last = t.exit_idx.min(day.flat_idx?);
    if t.entry_idx > last || t.entry_idx >= day.end {
        return None;
    }
    let raw_entry = px[t.entry_idx].o;
    let entry = raw_entry + t.side * slip;
    let risk = t.side * (entry - t.stop);
    if risk < c.tick {
        return None; // stop already through at the entry, or microscopic
    }
    let (mut exit_px, mut raw_exit, mut why, mut held) = (f64::NAN, f64::NAN, "time", 0);
    for i in t.entry_idx..=last {
        let b = px[i];
        held = i - t.entry_idx + 1;
        let (adv_open, adv_ext) = if t.side > 0.0 { (b.o <= t.stop, b.l <= t.stop) } else { (b.o >= t.stop, b.h >= t.stop) };
        if adv_open || adv_ext {
            let fill = if adv_open { b.o } else { t.stop };
            raw_exit = fill;
            exit_px = fill - t.side * slip;
            why = "stop";
            break;
        }
        if let Some(tg) = t.target {
            let (gap, touch) = if t.side > 0.0 { (b.o >= tg, b.h >= tg + c.tick) } else { (b.o <= tg, b.l <= tg - c.tick) };
            if gap || touch {
                let fill = if gap { b.o } else { tg };
                raw_exit = fill;
                exit_px = fill;
                why = "target";
                break;
            }
        }
        if i == last {
            raw_exit = b.c;
            exit_px = b.c - t.side * slip;
        }
    }
    let pts = t.side * (exit_px - entry);
    let usd = pts * c.point_value - 2.0 * commission - bps * (entry + exit_px) * c.point_value;
    Some(Outcome { r: usd / (risk * c.point_value), r_gross: t.side * (raw_exit - raw_entry) / risk, usd, risk_pts: risk, exit: why, bars: held })
}

/// Same side, stop distance (in daily-ATR units) and target multiple, entered at the same
/// session minute on `k` random days from `pool`. Returns the mean placebo R.
pub fn placebo(p: &Pair, t: &Trade, pool: &[usize], k: usize, seed: u64) -> Option<f64> {
    let day = &p.days[t.day];
    let atr = day.atr[t.asset];
    let c = MICRO[t.asset];
    let slip = if matches!(p.cost, CostModel::Contract) { c.slip_ticks * c.tick } else { 0.0 };
    let entry = p.px[t.asset][t.entry_idx].o + t.side * slip;
    let risk = t.side * (entry - t.stop);
    if risk <= 0.0 || !atr.is_finite() {
        return None;
    }
    let stop_atr = risk / atr;
    let tgt_r = t.target.map(|tg| t.side * (tg - entry) / risk);
    let sm_entry = p.sm[t.entry_idx];
    let hold_min = p.sm[t.exit_idx.min(day.flat_idx?)] as i32 - sm_entry as i32;
    let mut rng = StdRng::seed_from_u64(seed);
    let mut acc = vec![];
    let mut tries = 0;
    while acc.len() < k && tries < 20 * k {
        tries += 1;
        let d = pool[rng.gen_range(0..pool.len())];
        if d == t.day {
            continue;
        }
        let Some(i) = p.at_minute(d, sm_entry) else { continue };
        let dd = &p.days[d];
        let Some(flat) = dd.flat_idx else { continue };
        if i > flat || p.sm[i] != sm_entry {
            continue;
        }
        let a = dd.atr[t.asset];
        let e = p.px[t.asset][i].o + t.side * slip;
        let r = stop_atr * a;
        let exit_sm = (sm_entry as i32 + hold_min).clamp(0, SM_FLAT as i32) as u16;
        let exit_idx = p.at_minute(d, exit_sm).map(|x| x.min(flat)).unwrap_or(flat);
        let pt = Trade { day: d, asset: t.asset, side: t.side, entry_idx: i, stop: e - t.side * r, target: tgt_r.map(|m| e + t.side * m * r), exit_idx };
        if let Some(o) = simulate(p, &pt, 1.0) {
            acc.push(o.r);
        }
    }
    (!acc.is_empty()).then(|| acc.iter().sum::<f64>() / acc.len() as f64)
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    pub n: usize,
    pub mean_r: f64,
    pub t: f64,
    pub win_rate: f64,
    pub pf: f64,
    pub usd_per_trade: f64,
    pub usd_total: f64,
}

pub fn stats(r: &[f64], usd: &[f64]) -> Stats {
    let n = r.len();
    if n == 0 {
        return Stats::default();
    }
    let m = r.iter().sum::<f64>() / n as f64;
    let sd = if n > 1 { (r.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt() } else { 0.0 };
    let (gw, gl): (f64, f64) = r.iter().fold((0.0, 0.0), |(w, l), x| if *x > 0.0 { (w + x, l) } else { (w, l - x) });
    let ut: f64 = usd.iter().sum();
    Stats { n, mean_r: m, t: if sd > 0.0 { m / (sd / (n as f64).sqrt()) } else { 0.0 }, win_rate: r.iter().filter(|x| **x > 0.0).count() as f64 / n as f64, pf: if gl > 0.0 { gw / gl } else { f64::INFINITY }, usd_per_trade: ut / n as f64, usd_total: ut }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pair::{Ohlc, Pair, Series};

    fn pair_from(prices: &[(f64, f64, f64, f64)]) -> Pair {
        // one RTH session starting 2024-07-10 13:30 UTC (09:30 ET)
        let ts: Vec<i64> = (0..prices.len() as i64).map(|k| 1720618200 + 60 * k).collect();
        let px: Vec<Ohlc> = prices.iter().map(|&(o, h, l, c)| Ohlc { o, h, l, c }).collect();
        let s = Series { ts: ts.clone(), px: px.clone() };
        let s2 = Series { ts, px };
        Pair::align(&s, &s2)
    }

    #[test]
    fn stop_first_and_costs() {
        // bar 1 touches both stop (99) and target (102): stop first
        let p = pair_from(&[(100.0, 100.0, 100.0, 100.0), (100.0, 103.0, 98.0, 100.0), (100.0, 100.0, 100.0, 100.0)]);
        let t = Trade { day: 0, asset: 0, side: 1.0, entry_idx: 1, stop: 99.0, target: Some(102.0), exit_idx: 2 };
        let o = simulate(&p, &t, 1.0).unwrap();
        assert_eq!(o.exit, "stop");
        // entry 100.25, exit 98.75 → −1.5 pts × $2 − $1.50 = −$4.50 ; risk 1.25 pts = $2.50 → −1.8R
        assert!((o.usd + 4.5).abs() < 1e-9, "{o:?}");
        assert!((o.r + 1.8).abs() < 1e-9, "{o:?}");
    }

    #[test]
    fn target_needs_trade_through_and_time_exit() {
        let p = pair_from(&[(100.0, 100.0, 100.0, 100.0), (100.0, 102.0, 99.5, 101.0), (101.0, 101.5, 100.5, 101.0)]);
        let t = Trade { day: 0, asset: 0, side: 1.0, entry_idx: 1, stop: 99.0, target: Some(102.0), exit_idx: 2 };
        let o = simulate(&p, &t, 1.0).unwrap();
        assert_eq!(o.exit, "time", "a touch of exactly the target is not a fill");
    }
}
