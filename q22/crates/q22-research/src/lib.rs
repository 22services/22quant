//! q22-research — pre-registered event studies on two aligned futures (NQ/ES).
//!
//! * [`pair`]    load `q22 databento` series, align minutes, sessions, levels, daily ATR
//! * [`sim`]     single-trade simulation (next-open fills, stop-first, 1-tick trade-through,
//!   commissions + slippage), time-matched placebo, statistics
//! * [`studies`] the registered hypotheses → trade lists
//!
//! [`evaluate`] applies the registered pass criteria (research/PREREGISTRATION.md §4).

pub mod pair;
pub mod sim;
pub mod studies;

use std::collections::BTreeMap;

use chrono::Datelike;
use serde::Serialize;

use pair::{Pair, NAMES};
use sim::{placebo, simulate, stats, Stats};
use studies::StudyTrades;

#[derive(Clone, Debug, Serialize)]
pub struct YearRow {
    pub year: i32,
    pub n: usize,
    pub sum_r: f64,
    pub usd: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct StudyResult {
    pub id: String,
    pub description: String,
    pub net: Stats,
    pub gross_mean_r: f64,
    pub costs_x2: Stats,
    pub placebo_mean_r: f64,
    /// paired (event R − its placebo mean R): mean and t
    pub vs_placebo_diff: f64,
    pub vs_placebo_t: f64,
    pub null: Option<Stats>,
    pub years: Vec<YearRow>,
    pub years_positive_frac: f64,
    pub by_asset: BTreeMap<String, Stats>,
    pub exits: BTreeMap<String, usize>,
    pub pass: BTreeMap<String, bool>,
    pub passed: bool,
}

pub fn evaluate(p: &Pair, st: &StudyTrades, pool: &[usize]) -> StudyResult {
    let mut r = vec![];
    let mut usd = vec![];
    let mut r2 = vec![];
    let mut usd2 = vec![];
    let mut gross = vec![];
    let mut diffs = vec![];
    let mut plac = vec![];
    let mut years: BTreeMap<i32, (usize, f64, f64)> = BTreeMap::new();
    let mut by_asset: BTreeMap<String, (Vec<f64>, Vec<f64>)> = BTreeMap::new();
    let mut exits: BTreeMap<String, usize> = BTreeMap::new();
    for (k, t) in st.trades.iter().enumerate() {
        let Some(o) = simulate(p, t, 1.0) else { continue };
        r.push(o.r);
        usd.push(o.usd);
        gross.push(o.r_gross);
        if let Some(o2) = simulate(p, t, 2.0) {
            r2.push(o2.r);
            usd2.push(o2.usd);
        }
        let seed = (t.day as u64) << 20 ^ (t.entry_idx as u64) ^ ((k as u64) << 40);
        if let Some(pm) = placebo(p, t, pool, 5, seed) {
            diffs.push(o.r - pm);
            plac.push(pm);
        }
        let y = years.entry(p.days[t.day].date.year()).or_default();
        y.0 += 1;
        y.1 += o.r;
        y.2 += o.usd;
        let e = by_asset.entry(NAMES[t.asset].to_string()).or_default();
        e.0.push(o.r);
        e.1.push(o.usd);
        *exits.entry(o.exit.to_string()).or_default() += 1;
    }
    let net = stats(&r, &usd);
    let costs_x2 = stats(&r2, &usd2);
    let dstat = stats(&diffs, &diffs);
    let null = (!st.null_trades.is_empty()).then(|| {
        let (nr, nu): (Vec<f64>, Vec<f64>) = st.null_trades.iter().filter_map(|t| simulate(p, t, 1.0)).map(|o| (o.r, o.usd)).unzip();
        stats(&nr, &nu)
    });
    let years: Vec<YearRow> = years.into_iter().map(|(year, (n, s, u))| YearRow { year, n, sum_r: s, usd: u }).collect();
    let eligible: Vec<&YearRow> = years.iter().filter(|y| y.n >= 10).collect();
    let ypos = if eligible.is_empty() { 0.0 } else { eligible.iter().filter(|y| y.sum_r > 0.0).count() as f64 / eligible.len() as f64 };
    let mut pass = BTreeMap::new();
    pass.insert("1_n>=100".to_string(), net.n >= 100);
    pass.insert("2_mean>0_t>=2".to_string(), net.mean_r > 0.0 && net.t >= 2.0);
    let beats_null = dstat.t >= 2.0 && null.as_ref().map_or(true, |n| net.mean_r > n.mean_r);
    pass.insert("3_beats_null".to_string(), beats_null);
    pass.insert("4_years>=60%".to_string(), ypos >= 0.6);
    pass.insert("5_costs_x2>0".to_string(), costs_x2.mean_r > 0.0);
    let passed = pass.values().all(|v| *v);
    StudyResult {
        id: st.id.clone(),
        description: st.description.clone(),
        gross_mean_r: if gross.is_empty() { 0.0 } else { gross.iter().sum::<f64>() / gross.len() as f64 },
        placebo_mean_r: if plac.is_empty() { 0.0 } else { plac.iter().sum::<f64>() / plac.len() as f64 },
        vs_placebo_diff: dstat.mean_r,
        vs_placebo_t: dstat.t,
        net,
        costs_x2,
        null,
        years_positive_frac: ypos,
        years,
        by_asset: by_asset.into_iter().map(|(k, (r, u))| (k, stats(&r, &u))).collect(),
        exits,
        pass,
        passed,
    }
}

/// Diagnostic (not a trading rule): mean forward move in the trade's direction, in daily-ATR
/// units, from the entry open to 30/60/120 minutes later and to 15:55 — for the events and for
/// the null events. Answers "is there *any* information", independent of stops and costs.
pub fn forward_drift(p: &Pair, trades: &[sim::Trade]) -> (usize, [f64; 4], [f64; 4]) {
    let mut acc: [Vec<f64>; 4] = Default::default();
    for t in trades {
        let day = &p.days[t.day];
        let Some(flat) = day.flat_idx else { continue };
        let e = p.px[t.asset][t.entry_idx].o;
        let atr = day.atr[t.asset];
        for (k, h) in [30u16, 60, 120, 9999].iter().enumerate() {
            let i = if *h == 9999 { flat } else { p.at_minute(t.day, p.sm[t.entry_idx].saturating_add(*h)).map(|x| x.min(flat)).unwrap_or(flat) };
            acc[k].push(t.side * (p.px[t.asset][i].c - e) / atr);
        }
    }
    let n = acc[0].len();
    let mut mean = [0.0; 4];
    let mut tst = [0.0; 4];
    for k in 0..4 {
        let v = &acc[k];
        if v.len() > 1 {
            let m = v.iter().sum::<f64>() / v.len() as f64;
            let sd = (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt();
            mean[k] = m;
            tst[k] = m / (sd / (v.len() as f64).sqrt());
        }
    }
    (n, mean, tst)
}

pub fn print_table(res: &[StudyResult]) {
    println!("{:<18} {:>5} {:>7} {:>6} {:>6} {:>7} {:>8} {:>7} {:>7} {:>6} {:>7} {:>6}  verdict", "study", "n", "netR", "t", "win%", "grossR", "x2costR", "placebo", "diff_t", "null", "$/trade", "yrs+");
    for s in res {
        println!(
            "{:<18} {:>5} {:>+7.3} {:>6.2} {:>5.1}% {:>+7.3} {:>+8.3} {:>+7.3} {:>7.2} {:>6} {:>7.2} {:>5.0}%  {}",
            s.id,
            s.net.n,
            s.net.mean_r,
            s.net.t,
            s.net.win_rate * 100.0,
            s.gross_mean_r,
            s.costs_x2.mean_r,
            s.placebo_mean_r,
            s.vs_placebo_t,
            s.null.as_ref().map(|n| format!("{:+.3}", n.mean_r)).unwrap_or_else(|| "—".into()),
            s.net.usd_per_trade,
            s.years_positive_frac * 100.0,
            if s.passed { "PASS".to_string() } else { format!("fail ({})", s.pass.iter().filter(|(_, v)| !**v).map(|(k, _)| k.split('_').next().unwrap_or("")).collect::<Vec<_>>().join(",")) }
        );
    }
}
