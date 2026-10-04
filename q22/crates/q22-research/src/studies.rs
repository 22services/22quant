//! The pre-registered hypotheses (research/PREREGISTRATION.md §3) turned into trade lists.
//! Every parameter here is the registered one; nothing is fitted.

use crate::pair::{Pair, ES, NQ, SM_FLAT};
use crate::sim::{Trade, MICRO};

const TICK: f64 = 0.25;

pub struct StudyTrades {
    pub id: String,
    pub description: String,
    pub trades: Vec<Trade>,
    /// Family A: the same trigger on "confirmed" sweeps (both indices swept) — the null.
    pub null_trades: Vec<Trade>,
}

fn other(a: usize) -> usize {
    1 - a
}

// ------------------------------------------------------------------ family A — SMT / SSMT

#[derive(Clone, Copy, PartialEq)]
pub enum Leg {
    /// trade the index that swept (fade the sweep)
    X,
    /// trade the index that failed, in the reversal direction
    Y,
}

/// Bearish case = highs (`dir = +1`), bullish = lows (`dir = −1`). Returns the trade if the MSS
/// confirms within 30 bars. `ext` is the running extreme of each index since the window start
/// (inclusive of the bars scanned so far).
#[allow(clippy::too_many_arguments)]
fn confirm_and_trade(p: &Pair, d: usize, dir: f64, x: usize, level_x: f64, sweep_i: usize, w_start: usize, leg: Leg) -> Option<Trade> {
    let day = &p.days[d];
    let flat = day.flat_idx?;
    let px = &p.px;
    // MSS: X closes back through its level within 30 bars of the sweep
    let mut j = None;
    for k in sweep_i..=(sweep_i + 30).min(flat) {
        let c = px[x][k].c;
        if (dir > 0.0 && c < level_x) || (dir < 0.0 && c > level_x) {
            j = Some(k);
            break;
        }
    }
    let j = j?;
    let entry_idx = j + 1;
    if entry_idx > flat {
        return None;
    }
    let traded = if leg == Leg::X { x } else { other(x) };
    // stop: traded index's extreme since the window start (through the MSS bar) + 1 tick
    let ext = if dir > 0.0 { (w_start..=j).map(|k| px[traded][k].h).fold(f64::MIN, f64::max) + TICK } else { (w_start..=j).map(|k| px[traded][k].l).fold(f64::MAX, f64::min) - TICK };
    let side = -dir; // reversal
    let o = px[traded][entry_idx].o;
    let risk = side * (o - ext);
    if risk <= 0.0 {
        return None;
    }
    Some(Trade { day: d, asset: traded, side, entry_idx, stop: ext, target: Some(o + side * 2.0 * risk), exit_idx: flat })
}

/// A1–A3: static levels per index per day.
pub fn smt_static(p: &Pair, days: &[usize], levels: &dyn Fn(usize) -> Option<[(f64, f64); 2]>, w0: u16, w1: u16, leg: Leg, id: &str, desc: &str) -> StudyTrades {
    let mut trades = vec![];
    let mut null = vec![];
    for &d in days {
        let Some(lv) = levels(d) else { continue };
        let Some(start) = p.at_minute(d, w0) else { continue };
        let day = &p.days[d];
        for dir in [1.0, -1.0] {
            let level = |a: usize| if dir > 0.0 { lv[a].0 } else { lv[a].1 };
            let mut ext = [if dir > 0.0 { f64::MIN } else { f64::MAX }; 2];
            for i in start..day.end {
                if p.sm[i] >= w1 {
                    break;
                }
                for a in [NQ, ES] {
                    ext[a] = if dir > 0.0 { ext[a].max(p.px[a][i].h) } else { ext[a].min(p.px[a][i].l) };
                }
                let swept = |a: usize| if dir > 0.0 { ext[a] >= level(a) + TICK } else { ext[a] <= level(a) - TICK };
                let (s0, s1) = (swept(NQ), swept(ES));
                if !(s0 || s1) {
                    continue;
                }
                if s0 && s1 {
                    // confirmed sweep (null): X = the larger excess in ATR units
                    let ex = |a: usize| dir * (ext[a] - level(a)) / day.atr[a];
                    let x = if ex(NQ) >= ex(ES) { NQ } else { ES };
                    if let Some(t) = confirm_and_trade(p, d, dir, x, level(x), i, start, leg) {
                        null.push(t);
                    }
                } else {
                    let x = if s0 { NQ } else { ES };
                    if let Some(t) = confirm_and_trade(p, d, dir, x, level(x), i, start, leg) {
                        trades.push(t);
                    }
                }
                break; // first sweep of the window defines the event
            }
        }
    }
    StudyTrades { id: id.into(), description: desc.into(), trades, null_trades: null }
}

pub fn a1_prev_session(p: &Pair, days: &[usize], leg: Leg) -> StudyTrades {
    let lv = |d: usize| -> Option<[(f64, f64); 2]> {
        let q = &p.days[d - 1];
        Some([(q.high[NQ], q.low[NQ]), (q.high[ES], q.low[ES])])
    };
    let tag = if leg == Leg::X { "X" } else { "Y" };
    smt_static(p, days, &lv, 930, 1080, leg, &format!("A1-{tag}"), "SMT at the previous session high/low, 09:30–12:00")
}

/// A1n (deviation D-4, data-snooped): both indices sweep the previous-session high/low and
/// the larger sweeper closes back through within 30 bars → reversal on that index. The SMT
/// events become the comparison group.
pub fn a1n_confirmed_failure(p: &Pair, days: &[usize]) -> StudyTrades {
    let mut s = a1_prev_session(p, days, Leg::X);
    std::mem::swap(&mut s.trades, &mut s.null_trades);
    s.id = "A1n".into();
    s.description = "both indices sweep the previous-session extreme and fail back → reversal (snooped)".into();
    s
}

pub fn a2_overnight(p: &Pair, days: &[usize], leg: Leg) -> StudyTrades {
    let lv = |d: usize| -> Option<[(f64, f64); 2]> {
        let day = &p.days[d];
        let rs = day.rth_start?;
        if rs <= day.start + 60 {
            return None;
        }
        let mut out = [(f64::MIN, f64::MAX); 2];
        for a in [NQ, ES] {
            for k in day.start..rs {
                out[a].0 = out[a].0.max(p.px[a][k].h);
                out[a].1 = out[a].1.min(p.px[a][k].l);
            }
        }
        Some(out)
    };
    let tag = if leg == Leg::X { "X" } else { "Y" };
    smt_static(p, days, &lv, 930, 1050, leg, &format!("A2-{tag}"), "SMT at the overnight (18:00–09:30) high/low, 09:30–11:30")
}

pub fn a3_opening_range(p: &Pair, days: &[usize], leg: Leg) -> StudyTrades {
    let lv = |d: usize| -> Option<[(f64, f64); 2]> {
        let s = p.at_minute(d, 930)?;
        let e = p.at_minute(d, 960)?;
        if e <= s + 20 {
            return None;
        }
        let mut out = [(f64::MIN, f64::MAX); 2];
        for a in [NQ, ES] {
            for k in s..e {
                out[a].0 = out[a].0.max(p.px[a][k].h);
                out[a].1 = out[a].1.min(p.px[a][k].l);
            }
        }
        Some(out)
    };
    let tag = if leg == Leg::X { "X" } else { "Y" };
    smt_static(p, days, &lv, 960, 1200, leg, &format!("A3-{tag}"), "SMT at the 09:30–10:00 opening-range high/low, 10:00–14:00")
}

/// Fractal swing points of one index inside a day: (index, price), confirmed `n` bars later.
fn fractals(p: &Pair, a: usize, s: usize, e: usize, n: usize, high: bool) -> Vec<(usize, f64)> {
    let px = &p.px[a];
    let mut out = vec![];
    if e < s + 2 * n + 1 {
        return out;
    }
    for j in (s + n)..(e - n) {
        let v = if high { px[j].h } else { px[j].l };
        let left = (j - n..j).all(|k| if high { px[k].h < v } else { px[k].l > v });
        let right = (j + 1..=j + n).all(|k| if high { px[k].h <= v } else { px[k].l >= v });
        if left && right {
            out.push((j, v));
        }
    }
    out
}

/// A4: micro SMT on 1-minute fractal swings (n = 10) formed within ±5 bars of each other.
pub fn a4_micro(p: &Pair, days: &[usize], leg: Leg) -> StudyTrades {
    const N: usize = 10;
    let mut trades = vec![];
    let mut null = vec![];
    for &d in days {
        let day = &p.days[d];
        let (Some(w_start), Some(_flat)) = (p.at_minute(d, 945), day.flat_idx) else { continue };
        let w_end = p.at_minute(d, 1260).unwrap_or(day.end); // 15:00
        for dir in [1.0, -1.0] {
            let high = dir > 0.0;
            let sw = [fractals(p, NQ, day.start, day.end, N, high), fractals(p, ES, day.start, day.end, N, high)];
            let beyond = |a: usize, i: usize, lvl: f64| if high { p.px[a][i].h >= lvl + TICK } else { p.px[a][i].l <= lvl - TICK };
            'scan: for i in w_start..w_end {
                for x in [NQ, ES] {
                    let y = other(x);
                    // latest confirmed swing of X
                    let Some(&(jx, lx)) = sw[x].iter().rev().find(|(j, _)| j + N <= i) else { continue };
                    if !beyond(x, i, lx) || (jx + 1..i).any(|k| beyond(x, k, lx)) {
                        continue; // not the first break of that swing
                    }
                    // Y's swing formed within ±5 bars of X's, confirmed by now
                    let Some(&(jy, ly)) = sw[y].iter().rev().find(|(j, _)| j + N <= i && (*j as i64 - jx as i64).abs() <= 5) else { continue };
                    let y_broke = (jy + 1..=i).any(|k| beyond(y, k, ly));
                    let t = confirm_and_trade(p, d, dir, x, lx, i, w_start, leg);
                    if y_broke {
                        if let Some(t) = t {
                            null.push(t);
                        }
                    } else if let Some(t) = t {
                        trades.push(t);
                    }
                    break 'scan; // first qualifying break per side per day
                }
            }
        }
    }
    let tag = if leg == Leg::X { "X" } else { "Y" };
    StudyTrades { id: format!("A4-{tag}"), description: "micro SMT on 1-min fractal swings (n=10, ±5 bars), 09:45–15:00".into(), trades, null_trades: null }
}

// ------------------------------------------------------------------ family B — relative value

/// End-of-day vol-normalised spread s = ΔNQ/ATR_NQ − ΔES/ATR_ES (from the RTH open to 15:55).
fn spread_at(p: &Pair, d: usize, i: usize) -> Option<f64> {
    let day = &p.days[d];
    let o = day.rth_open;
    let a = day.atr;
    let s = (p.px[NQ][i].c - o[NQ]) / a[NQ] - (p.px[ES][i].c - o[ES]) / a[ES];
    s.is_finite().then_some(s)
}

#[derive(Clone, Copy, PartialEq)]
pub enum BVariant {
    ReversionShortLeader,
    ReversionLongLaggard,
    MomentumLongLeader,
    MomentumShortLaggard,
}

pub fn b_spread(p: &Pair, days: &[usize], v: BVariant) -> StudyTrades {
    let mut trades = vec![];
    let eod: Vec<Option<f64>> = (0..p.days.len()).map(|d| p.days[d].flat_idx.and_then(|f| p.days[d].rth_start.and_then(|_| spread_at(p, d, f)))).collect();
    for &d in days {
        // σ of the end-of-day spread over the previous 20 sessions
        let hist: Vec<f64> = (d.saturating_sub(20)..d).filter_map(|k| eod[k]).collect();
        if hist.len() < 15 {
            continue;
        }
        let m = hist.iter().sum::<f64>() / hist.len() as f64;
        let sd = (hist.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (hist.len() - 1) as f64).sqrt();
        let (Some(s0), Some(flat)) = (p.at_minute(d, 960), p.days[d].flat_idx) else { continue };
        for i in s0..flat {
            if p.sm[i] >= 1110 {
                break; // 14:30
            }
            let Some(s) = spread_at(p, d, i) else { continue };
            let elapsed = (p.sm[i] as f64 + 1.0 - 930.0).max(1.0);
            let z = s / (sd * (elapsed / 385.0).sqrt());
            if z.abs() < 2.0 {
                continue;
            }
            let leader = if z > 0.0 { NQ } else { ES };
            let (asset, side) = match v {
                BVariant::ReversionShortLeader => (leader, -1.0),
                BVariant::ReversionLongLaggard => (other(leader), 1.0),
                BVariant::MomentumLongLeader => (leader, 1.0),
                BVariant::MomentumShortLaggard => (other(leader), -1.0),
            };
            let e = i + 1;
            if e <= flat {
                let o = p.px[asset][e].o;
                let atr = p.days[d].atr[asset];
                trades.push(Trade { day: d, asset, side, entry_idx: e, stop: o - side * 0.25 * atr, target: Some(o + side * 0.5 * atr), exit_idx: flat });
            }
            break;
        }
    }
    let (id, desc) = match v {
        BVariant::ReversionShortLeader => ("B1-short-leader", "spread |z|≥2: fade by shorting the leader"),
        BVariant::ReversionLongLaggard => ("B1-long-laggard", "spread |z|≥2: fade by buying the laggard"),
        BVariant::MomentumLongLeader => ("B2-long-leader", "spread |z|≥2: follow by buying the leader"),
        BVariant::MomentumShortLaggard => ("B2-short-laggard", "spread |z|≥2: follow by shorting the laggard"),
    };
    // the sign convention above is for an up-leader; a down-move leader (z<0 means ES led *up*
    // relative, i.e. NQ lagged) is handled by `leader`, sides are about relative strength.
    StudyTrades { id: id.into(), description: desc.into(), trades, null_trades: vec![] }
}

// ------------------------------------------------------------------ family C — lead–lag

pub fn c1_lead_lag(p: &Pair, days: &[usize]) -> StudyTrades {
    // 5-minute RTH slots: r5[a][slot] in daily-ATR units
    let slot_ret = |d: usize, a: usize, k: u16| -> Option<f64> {
        let s = p.at_minute(d, 930 + 5 * k)?;
        let e = p.at_minute(d, 935 + 5 * k)?;
        if p.sm[s] != 930 + 5 * k || e == 0 || e - 1 < s {
            return None;
        }
        Some((p.px[a][e - 1].c - p.px[a][s].o) / p.days[d].atr[a])
    };
    let mut trades = vec![];
    for &d in days {
        let Some(flat) = p.days[d].flat_idx else { continue };
        let mut done = [false; 2];
        for k in 3..72u16 {
            // slot k covers [930+5k, 935+5k); 09:45 → k=3 … 15:30 → k=72
            let (Some(rn), Some(re)) = (slot_ret(d, NQ, k), slot_ret(d, ES, k)) else { continue };
            let sigma = |a: usize| -> Option<f64> {
                let h: Vec<f64> = (d.saturating_sub(20)..d).filter_map(|q| p.days[q].atr[a].is_finite().then(|| slot_ret(q, a, k)).flatten()).collect();
                if h.len() < 15 {
                    return None;
                }
                let m = h.iter().sum::<f64>() / h.len() as f64;
                Some((h.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (h.len() - 1) as f64).sqrt())
            };
            for (x, rx, ry) in [(NQ, rn, re), (ES, re, rn)] {
                if done[x] {
                    continue;
                }
                let (Some(sx), Some(sy)) = (sigma(x), sigma(other(x))) else { continue };
                if rx.abs() >= 2.0 * sx && ry * rx.signum() < 0.5 * sy {
                    let y = other(x);
                    let Some(e) = p.at_minute(d, 935 + 5 * k) else { continue };
                    if e > flat {
                        continue;
                    }
                    let side = rx.signum();
                    let o = p.px[y][e].o;
                    let atr = p.days[d].atr[y];
                    let exit_idx = p.at_minute(d, p.sm[e] + 14).map(|z| z.min(flat)).unwrap_or(flat);
                    trades.push(Trade { day: d, asset: y, side, entry_idx: e, stop: o - side * 0.10 * atr, target: Some(o + side * 0.20 * atr), exit_idx });
                    done[x] = true;
                }
            }
        }
    }
    StudyTrades { id: "C1".into(), description: "lead–lag: X moves ≥2σ in 5 min, Y <0.5σ → trade Y in X's direction for 15 min".into(), trades, null_trades: vec![] }
}

// ------------------------------------------------------------------ family D2 — fade divergent noise breakouts

pub fn d2_fade_divergent(p: &Pair, days: &[usize]) -> StudyTrades {
    // σ_m at the 12 checks (bar closing at 10:00, 10:30, …, 15:30): |close/open − 1|
    let check_bar = |d: usize, m: u16| -> Option<usize> {
        let i = p.at_minute(d, 930 + m - 1)?;
        (p.sm[i] == 930 + m - 1).then_some(i)
    };
    let mv = |d: usize, a: usize, m: u16| -> Option<f64> {
        let i = check_bar(d, m)?;
        let o = p.days[d].rth_open[a];
        Some((p.px[a][i].c / o - 1.0).abs())
    };
    let mut trades = vec![];
    for &d in days {
        let Some(flat) = p.days[d].flat_idx else { continue };
        let prev_close = p.days[d - 1].rth_close;
        let open = p.days[d].rth_open;
        if !prev_close.iter().all(|x| x.is_finite()) {
            continue;
        }
        let mut done = [false; 2]; // per side: [up-break fade, down-break fade]
        for m in (30..=360u16).step_by(30) {
            let Some(i) = check_bar(d, m) else { continue };
            let mut ub = [0.0; 2];
            let mut lb = [0.0; 2];
            let mut ok = true;
            for a in [NQ, ES] {
                let h: Vec<f64> = (d.saturating_sub(14)..d).filter_map(|q| mv(q, a, m)).collect();
                if h.len() < 10 {
                    ok = false;
                    break;
                }
                let s = h.iter().sum::<f64>() / h.len() as f64;
                ub[a] = open[a].max(prev_close[a]) * (1.0 + s);
                lb[a] = open[a].min(prev_close[a]) * (1.0 - s);
            }
            if !ok {
                continue;
            }
            for x in [NQ, ES] {
                let y = other(x);
                let (cx, cy) = (p.px[x][i].c, p.px[y][i].c);
                for (k, up) in [(0usize, true), (1usize, false)] {
                    if done[k] {
                        continue;
                    }
                    let div = if up { cx > ub[x] && cy <= ub[y] } else { cx < lb[x] && cy >= lb[y] };
                    if !div || i + 1 > flat {
                        continue;
                    }
                    let side = if up { -1.0 } else { 1.0 };
                    let e = i + 1;
                    let o = p.px[x][e].o;
                    let atr = p.days[d].atr[x];
                    let bar = p.px[x][i];
                    let stop = if side < 0.0 { (bar.h + TICK).max(o + 0.25 * atr) } else { (bar.l - TICK).min(o - 0.25 * atr) };
                    let risk = side * (o - stop);
                    trades.push(Trade { day: d, asset: x, side, entry_idx: e, stop, target: Some(o + side * 2.0 * risk), exit_idx: flat });
                    done[k] = true;
                }
            }
        }
    }
    StudyTrades { id: "D2".into(), description: "fade a noise-area breakout the other index does not confirm".into(), trades, null_trades: vec![] }
}

// ------------------------------------------------------------------ family E — gap divergence

pub fn e1_gap_divergence(p: &Pair, days: &[usize]) -> StudyTrades {
    let mut trades = vec![];
    for &d in days {
        let day = &p.days[d];
        let (Some(rs), Some(flat)) = (day.rth_start, day.flat_idx) else { continue };
        let pc = p.days[d - 1].rth_close;
        let g = [(day.rth_open[NQ] - pc[NQ]) / day.atr[NQ], (day.rth_open[ES] - pc[ES]) / day.atr[ES]];
        if !(g[0].is_finite() && g[1].is_finite()) || g[0].signum() == g[1].signum() || g[0].abs() < 0.10 || g[1].abs() < 0.10 {
            continue;
        }
        let x = if g[NQ].abs() >= g[ES].abs() { NQ } else { ES };
        let side = -g[x].signum();
        let e = rs + 1;
        if e > flat {
            continue;
        }
        let gap_pts = day.rth_open[x] - pc[x];
        let stop = day.rth_open[x] + 0.5 * gap_pts; // beyond the open, away from the fill
        trades.push(Trade { day: d, asset: x, side, entry_idx: e, stop, target: Some(pc[x]), exit_idx: flat });
    }
    StudyTrades { id: "E1".into(), description: "opposite-sign opening gaps (≥0.10 ATR each): fade the larger toward the fill".into(), trades, null_trades: vec![] }
}

/// Sanity: the registered cost model, for the report header.
pub fn cost_note() -> String {
    format!(
        "costs per side: MNQ ${:.2} + {} tick (${:.2}); MES ${:.2} + {} tick (${:.2})",
        MICRO[NQ].commission, MICRO[NQ].slip_ticks, MICRO[NQ].tick * MICRO[NQ].point_value, MICRO[ES].commission, MICRO[ES].slip_ticks, MICRO[ES].tick * MICRO[ES].point_value
    )
}

pub const _FLAT: u16 = SM_FLAT;
