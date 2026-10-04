//! Bar-level backtester driving the *same* [`Engine`] used live.
//!
//! Fill model (conservative by design):
//! * market orders decided at a bar's close fill at the **next bar's open** ± `slippage_ticks`;
//!   entries that would fill in a different trading day are cancelled;
//! * forced exits (`immediate`) fill at the deciding bar's close ± slippage;
//! * resting stop: gap through → fill at the open (− slippage); touch → fill at the stop
//!   (− slippage); resting target: gap through → fill at the open; touch requires a trade
//!   **through** the target by one tick; if stop and target are both inside one bar, the stop
//!   is assumed to fill first.
//! * prop replay: when the account passes or fails, the attempt is recorded and a fresh
//!   evaluation starts the next trading day.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

use q22_core::session::trading_day;
use q22_core::stats;
use q22_core::{Bar, Side};

use crate::config::EngineConfig;
use crate::engine::{ClosedTrade, Command, Engine, EngineEvent, Fill, FillKind};
use crate::prop::AccountStatus;
use crate::regime::Regime;

#[derive(Clone, Debug, Serialize)]
pub struct BacktestOptions {
    pub prop_replay: bool,
    /// Number of configurations tried before picking this one (for the Deflated Sharpe).
    pub n_trials: usize,
    pub label: String,
}

impl Default for BacktestOptions {
    fn default() -> Self {
        Self { prop_replay: true, n_trials: 1, label: "backtest".into() }
    }
}

#[derive(Clone, Debug)]
struct Resting {
    trade_id: u64,
    side: Side,
    qty: f64,
    stop: f64,
    target: Option<f64>,
}

#[derive(Clone, Debug)]
struct PendingMkt {
    cmd: Command,
    day: NaiveDate,
}

#[derive(Default)]
pub struct SimBroker {
    pending: HashMap<String, Vec<PendingMkt>>,
    resting: HashMap<String, Resting>,
}

impl SimBroker {
    fn fill(eng: &mut Engine, trade_id: u64, symbol: &str, kind: FillKind, qty: f64, price: f64, ts: DateTime<Utc>, note: &str) {
        eng.on_fill(&Fill { trade_id, symbol: symbol.to_string(), kind, qty, price, ts, note: note.to_string() });
    }

    /// Start of a new bar for `symbol`: execute queued market orders at the open, then resting orders.
    pub fn on_bar_open(&mut self, eng: &mut Engine, symbol: &str, bar: &Bar) {
        let Some(i) = eng.instrument_index(symbol) else { return };
        let (tick, slip_ticks, kind) = (eng.instruments[i].spec.tick_size, eng.instruments[i].slippage_ticks, eng.instruments[i].spec.kind);
        let slip = tick * slip_ticks;
        let today = trading_day(kind, bar.ts);
        for p in self.pending.remove(symbol).unwrap_or_default() {
            match p.cmd {
                Command::Enter { trade_id, side, qty, stop, target, .. } => {
                    if p.day != today {
                        eng.cancel_pending(symbol, "entry would fill in a new session", bar.ts);
                        continue;
                    }
                    let px = bar.open + side.sign() * slip;
                    Self::fill(eng, trade_id, symbol, FillKind::Entry, qty, px, bar.ts, "market entry");
                    self.resting.insert(symbol.to_string(), Resting { trade_id, side, qty, stop, target });
                }
                Command::Exit { trade_id, ref reason, .. } => {
                    if let Some(r) = self.resting.remove(symbol) {
                        let px = bar.open - r.side.sign() * slip;
                        Self::fill(eng, trade_id, symbol, FillKind::Exit, r.qty, px, bar.ts, reason);
                    }
                }
                Command::ModifyStop { .. } => {}
            }
        }
        if let Some(r) = self.resting.get(symbol).cloned() {
            let s = r.side.sign();
            let stop_hit_at_open = (bar.open - r.stop) * s <= 0.0;
            let target_at_open = r.target.is_some_and(|t| (bar.open - t) * s >= 0.0);
            let stop_touched = if r.side == Side::Long { bar.low <= r.stop } else { bar.high >= r.stop };
            let target_touched = r.target.is_some_and(|t| if r.side == Side::Long { bar.high >= t + tick } else { bar.low <= t - tick });
            let fill = if stop_hit_at_open {
                Some((FillKind::Stop, bar.open - s * slip))
            } else if target_at_open {
                Some((FillKind::Target, bar.open))
            } else if stop_touched {
                Some((FillKind::Stop, r.stop - s * slip))
            } else if target_touched {
                Some((FillKind::Target, r.target.unwrap()))
            } else {
                None
            };
            if let Some((k, px)) = fill {
                self.resting.remove(symbol);
                Self::fill(eng, r.trade_id, symbol, k, r.qty, px, bar.ts, "resting order");
            }
        }
    }

    /// Handle the engine's commands issued at the close of `bar`.
    pub fn on_commands(&mut self, eng: &mut Engine, bar: &Bar, cmds: Vec<Command>) {
        for c in cmds {
            let symbol = c.symbol().to_string();
            let Some(i) = eng.instrument_index(&symbol) else { continue };
            let (tick, slip_ticks, kind, tf) = (eng.instruments[i].spec.tick_size, eng.instruments[i].slippage_ticks, eng.instruments[i].spec.kind, eng.instruments[i].tf_minutes);
            let slip = tick * slip_ticks;
            let bar_end = bar.ts + chrono::Duration::minutes(tf);
            match &c {
                Command::Enter { .. } => {
                    let day = trading_day(kind, bar.ts);
                    self.pending.entry(symbol).or_default().push(PendingMkt { cmd: c, day });
                }
                Command::ModifyStop { stop, .. } => {
                    if let Some(r) = self.resting.get_mut(&symbol) {
                        r.stop = *stop;
                    }
                }
                Command::Exit { trade_id, reason, immediate, .. } => {
                    if *immediate {
                        if let Some(r) = self.resting.remove(&symbol) {
                            let px = eng.instruments[i].last_price - r.side.sign() * slip;
                            Self::fill(eng, *trade_id, &symbol, FillKind::Exit, r.qty, px, bar_end, reason);
                        }
                    } else {
                        let day = trading_day(kind, bar.ts);
                        self.pending.entry(symbol).or_default().push(PendingMkt { cmd: c, day });
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GroupStats {
    pub key: String,
    pub trades: usize,
    pub win_rate: f64,
    pub net: f64,
    pub avg_r: f64,
    pub sum_r: f64,
    pub profit_factor: f64,
    pub avg_bars: f64,
}

fn group_stats(key: String, ts: &[&ClosedTrade]) -> GroupStats {
    let n = ts.len();
    let wins = ts.iter().filter(|t| t.pnl_net > 0.0).count();
    let gw: f64 = ts.iter().filter(|t| t.pnl_net > 0.0).map(|t| t.pnl_net).sum();
    let gl: f64 = -ts.iter().filter(|t| t.pnl_net <= 0.0).map(|t| t.pnl_net).sum::<f64>();
    let sum_r: f64 = ts.iter().map(|t| t.r_multiple).sum();
    GroupStats {
        key,
        trades: n,
        win_rate: if n > 0 { wins as f64 / n as f64 } else { 0.0 },
        net: ts.iter().map(|t| t.pnl_net).sum(),
        avg_r: if n > 0 { sum_r / n as f64 } else { 0.0 },
        sum_r,
        profit_factor: if gl > 0.0 { gw / gl } else if gw > 0.0 { f64::INFINITY } else { 0.0 },
        avg_bars: if n > 0 { ts.iter().map(|t| t.bars_held as f64).sum::<f64>() / n as f64 } else { 0.0 },
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub start_balance: f64,
    pub net_pnl: f64,
    pub gross_pnl: f64,
    pub costs: f64,
    pub trades: usize,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub avg_r: f64,
    pub expectancy_usd: f64,
    pub trading_days: usize,
    pub days_with_trades: usize,
    pub pct_days_positive: f64,
    pub best_day: f64,
    pub worst_day: f64,
    pub max_drawdown_usd: f64,
    pub sharpe_daily_ann: f64,
    pub psr_vs_zero: f64,
    pub dsr: f64,
    pub dsr_trials: usize,
    pub dsr_benchmark_sr_ann: f64,
    pub min_track_record_days: f64,
    pub avg_trade_usd: f64,
    pub return_on_max_loss: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Attempt {
    pub n: usize,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub outcome: String,
    pub reason: String,
    pub trading_days: usize,
    pub profit: f64,
    pub best_day: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PropSummary {
    pub rules: String,
    pub attempts: usize,
    pub passed: usize,
    pub failed: usize,
    pub open: usize,
    pub pass_rate: f64,
    pub median_days_to_pass: Option<f64>,
    pub median_days_to_fail: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DataInfo {
    pub symbol: String,
    pub bars: usize,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub timeframe_min: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct BacktestReport {
    pub label: String,
    pub generated_at: DateTime<Utc>,
    pub config: EngineConfig,
    pub options: BacktestOptions,
    pub data: Vec<DataInfo>,
    pub summary: Summary,
    pub by_strategy: Vec<GroupStats>,
    pub by_regime: Vec<GroupStats>,
    pub by_exit: Vec<GroupStats>,
    pub regime_days: BTreeMap<String, usize>,
    pub prop: PropSummary,
    pub attempts: Vec<Attempt>,
    pub equity: Vec<(NaiveDate, f64)>,
    pub daily_pnl: Vec<(NaiveDate, f64)>,
    pub trades: Vec<ClosedTrade>,
    pub events_tail: Vec<EngineEvent>,
    pub block_counts: BTreeMap<String, usize>,
}

/// Run a backtest over one or more instruments. `data` keys must match configured symbols.
pub fn run_backtest(cfg: EngineConfig, data: Vec<(String, Vec<Bar>)>, opts: BacktestOptions) -> anyhow::Result<BacktestReport> {
    let mut eng = Engine::new(cfg.clone(), false)?;
    let periods: f64 = if eng.is_crypto() { 365.0 } else { 252.0 };
    let mut infos = vec![];
    let mut stream: Vec<(DateTime<Utc>, usize, Bar)> = vec![];
    for (k, (sym, bars)) in data.iter().enumerate() {
        let i = eng.instrument_index(sym).ok_or_else(|| anyhow::anyhow!("data symbol {sym} not configured"))?;
        if let (Some(f), Some(l)) = (bars.first(), bars.last()) {
            infos.push(DataInfo { symbol: sym.clone(), bars: bars.len(), from: f.ts, to: l.ts, timeframe_min: eng.instruments[i].tf_minutes });
        }
        stream.extend(bars.iter().map(|b| (b.ts, k, *b)));
    }
    stream.sort_by_key(|(ts, k, _)| (*ts, *k));
    let names: Vec<String> = data.iter().map(|(s, _)| eng.instruments[eng.instrument_index(s).unwrap()].spec.symbol.clone()).collect();

    let mut sim = SimBroker::default();
    let mut attempts: Vec<Attempt> = vec![];
    let mut attempt_start: Option<NaiveDate> = None;
    let mut wait_for_day: Option<NaiveDate> = None;
    let start_balance = eng.acct.start_balance;
    let kind0 = eng.instruments[0].spec.kind;

    for (_, k, bar) in &stream {
        let sym = &names[*k];
        let day = trading_day(kind0, bar.ts);
        if let Some(d) = wait_for_day {
            if day != d {
                wait_for_day = None;
                eng.paused = false;
            }
        }
        if attempt_start.is_none() {
            attempt_start = Some(day);
        }
        sim.on_bar_open(&mut eng, sym, bar);
        let cmds = eng.on_bar(sym, bar);
        sim.on_commands(&mut eng, bar, cmds);

        // In replay, an account the guard has stopped trading (buffer to the threshold below its
        // minimum) is a dead evaluation: book it as failed and start a fresh one.
        let flat = eng.instruments.iter().all(|r| r.position.is_none());
        if opts.prop_replay && eng.acct.is_active() && flat && eng.acct.buffer() < eng.guard.cfg.min_buffer_frac * eng.acct.rules.max_loss {
            eng.acct.status = AccountStatus::Failed { date: day, reason: format!("guard stop: buffer {:.0} below {:.0}% of max loss", eng.acct.buffer(), eng.guard.cfg.min_buffer_frac * 100.0) };
        }

        if !eng.acct.is_active() {
            // flatten anything left, record the attempt, restart next day
            let cmds = eng.flatten_all("evaluation ended", bar.ts);
            sim.on_commands(&mut eng, bar, cmds);
            eng.acct.close_books();
            let (outcome, reason, end) = match eng.acct.status.clone() {
                AccountStatus::Passed { date } => ("passed".to_string(), String::new(), date),
                AccountStatus::Failed { date, reason } => ("failed".to_string(), reason, date),
                AccountStatus::Active => unreachable!(),
            };
            attempts.push(Attempt { n: attempts.len() + 1, start: attempt_start.unwrap(), end, outcome, reason, trading_days: eng.acct.trading_days(), profit: eng.acct.total_profit(), best_day: eng.acct.best_day() });
            if !opts.prop_replay {
                break;
            }
            eng.reset_account();
            eng.paused = true;
            wait_for_day = Some(day);
            attempt_start = None;
        }
    }
    // close whatever is open at the last price
    if let Some((_, _, last)) = stream.last() {
        let cmds = eng.flatten_all("end of data", last.ts);
        sim.on_commands(&mut eng, last, cmds);
    }
    eng.finish();
    if eng.acct.is_active() {
        if let Some(s) = attempt_start {
            attempts.push(Attempt { n: attempts.len() + 1, start: s, end: eng.current_day.unwrap_or(s), outcome: "open".into(), reason: String::new(), trading_days: eng.acct.trading_days(), profit: eng.acct.total_profit(), best_day: eng.acct.best_day() });
        }
    }

    // ---- statistics
    let trades = eng.trades.clone();
    let mut by_day: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for (d, _) in &eng.daily_equity {
        by_day.entry(*d).or_insert(0.0);
    }
    for t in &trades {
        *by_day.entry(t.day).or_insert(0.0) += t.pnl_net;
    }
    let daily: Vec<f64> = by_day.values().copied().collect();
    let mut cum = 0.0;
    let mut equity = vec![];
    let mut path = vec![0.0];
    for (d, p) in &by_day {
        cum += p;
        equity.push((*d, start_balance + cum));
        path.push(cum);
    }
    let n = daily.len();
    let sr = if stats::std(&daily) > 0.0 { stats::mean(&daily) / stats::std(&daily) } else { 0.0 };
    let (sk, ku) = stats::skew_kurt(&daily);
    let psr = stats::probabilistic_sharpe(sr, 0.0, n, sk, ku);
    let (dsr, bench) = stats::deflated_sharpe(sr, n, opts.n_trials.max(1), 1.0 / n.max(2) as f64, sk, ku);
    let all: Vec<&ClosedTrade> = trades.iter().collect();
    let g = group_stats("all".into(), &all);
    let net: f64 = trades.iter().map(|t| t.pnl_net).sum();
    let summary = Summary {
        start_balance,
        net_pnl: net,
        gross_pnl: trades.iter().map(|t| t.pnl_gross).sum(),
        costs: trades.iter().map(|t| t.costs).sum(),
        trades: trades.len(),
        win_rate: g.win_rate,
        profit_factor: g.profit_factor,
        avg_r: g.avg_r,
        expectancy_usd: if trades.is_empty() { 0.0 } else { net / trades.len() as f64 },
        trading_days: n,
        days_with_trades: by_day.values().filter(|v| **v != 0.0).count(),
        pct_days_positive: if n > 0 { daily.iter().filter(|v| **v > 0.0).count() as f64 / n as f64 } else { 0.0 },
        best_day: daily.iter().cloned().fold(0.0, f64::max),
        worst_day: daily.iter().cloned().fold(0.0, f64::min),
        max_drawdown_usd: stats::max_drawdown(&path),
        sharpe_daily_ann: sr * periods.sqrt(),
        psr_vs_zero: psr,
        dsr,
        dsr_trials: opts.n_trials.max(1),
        dsr_benchmark_sr_ann: bench * periods.sqrt(),
        min_track_record_days: stats::min_track_record(sr, 0.0, sk, ku, 0.95),
        avg_trade_usd: if trades.is_empty() { 0.0 } else { net / trades.len() as f64 },
        return_on_max_loss: net / eng.acct.rules.max_loss,
    };
    let mut groups: BTreeMap<String, Vec<&ClosedTrade>> = BTreeMap::new();
    for t in &trades {
        groups.entry(t.strategy.clone()).or_default().push(t);
    }
    let by_strategy = groups.into_iter().map(|(k, v)| group_stats(k, &v)).collect();
    let mut groups: BTreeMap<String, Vec<&ClosedTrade>> = BTreeMap::new();
    for t in &trades {
        groups.entry(t.regime.label().to_string()).or_default().push(t);
    }
    let by_regime = groups.into_iter().map(|(k, v)| group_stats(k, &v)).collect();
    let mut groups: BTreeMap<String, Vec<&ClosedTrade>> = BTreeMap::new();
    for t in &trades {
        // category = reason without numbers/prices ("close 11043.10 back below trail 11047.25" → "close back below trail")
        let key = t.exit_reason.chars().filter(|c| !(c.is_ascii_digit() || matches!(c, '.' | ':' | '+' | '-' | '%' | '$' | '(' | ')' | '×' | 'σ'))).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
        groups.entry(key).or_default().push(t);
    }
    let by_exit = groups.into_iter().map(|(k, v)| group_stats(k, &v)).collect();
    let mut regime_days: BTreeMap<String, usize> = BTreeMap::new();
    for (_, _, r) in &eng.regime_days {
        *regime_days.entry(r.label().to_string()).or_default() += 1;
    }
    let _ = Regime::ALL;
    let passed: Vec<&Attempt> = attempts.iter().filter(|a| a.outcome == "passed").collect();
    let failed: Vec<&Attempt> = attempts.iter().filter(|a| a.outcome == "failed").collect();
    let median = |v: Vec<f64>| -> Option<f64> {
        if v.is_empty() {
            return None;
        }
        let mut v = v;
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        Some(v[v.len() / 2])
    };
    let decided = passed.len() + failed.len();
    let prop = PropSummary {
        rules: eng.acct.rules.name.clone(),
        attempts: attempts.len(),
        passed: passed.len(),
        failed: failed.len(),
        open: attempts.len() - decided,
        pass_rate: if decided > 0 { passed.len() as f64 / decided as f64 } else { 0.0 },
        median_days_to_pass: median(passed.iter().map(|a| a.trading_days as f64).collect()),
        median_days_to_fail: median(failed.iter().map(|a| a.trading_days as f64).collect()),
    };
    let events_tail = eng.events.iter().rev().take(400).cloned().collect::<Vec<_>>().into_iter().rev().collect();
    Ok(BacktestReport {
        label: opts.label.clone(),
        generated_at: Utc::now(),
        config: cfg,
        options: opts,
        data: infos,
        summary,
        by_strategy,
        by_regime,
        by_exit,
        regime_days,
        prop,
        attempts,
        equity,
        daily_pnl: by_day.into_iter().collect(),
        trades,
        events_tail,
        block_counts: eng.block_counts.clone(),
    })
}

// ---------------------------------------------------------------- rolling-start pass rate

/// Rolling-start evaluation study: a fresh prop evaluation is started every `every_days`
/// sessions (after `warmup_days` of indicator-only warm-up) and run until it passes, fails,
/// hits `max_days` sessions, or the data ends. Sequential replay gives a handful of attempts;
/// this gives one per start date (overlapping, so see `effective_n`).
#[derive(Clone, Debug, Serialize)]
pub struct PassRateOptions {
    pub every_days: usize,
    pub warmup_days: usize,
    /// Sessions per evaluation before it is booked as "open" (0 = until the data ends).
    pub max_days: usize,
    pub label: String,
}

impl Default for PassRateOptions {
    fn default() -> Self {
        Self { every_days: 5, warmup_days: 45, max_days: 0, label: "passrate".into() }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PassRun {
    pub start: NaiveDate,
    pub end: NaiveDate,
    /// passed | failed | open
    pub outcome: String,
    pub reason: String,
    /// Sessions elapsed from the start (what the monthly fee is paid on).
    pub sessions: usize,
    /// Sessions with a closed trade.
    pub trading_days: usize,
    pub trades: usize,
    pub profit: f64,
    pub best_day: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Quartiles {
    pub p25: f64,
    pub median: f64,
    pub p75: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PassRateReport {
    pub label: String,
    pub generated_at: DateTime<Utc>,
    pub rules: String,
    pub options: PassRateOptions,
    pub data: Vec<DataInfo>,
    pub runs: Vec<PassRun>,
    pub passed: usize,
    pub failed: usize,
    pub open: usize,
    /// passed / (passed + failed)
    pub pass_rate: f64,
    /// Runs overlap: roughly (sessions covered) / (mean decided-run length) independent attempts.
    pub effective_n: f64,
    /// Wilson 95% interval of `pass_rate` computed with `effective_n`.
    pub pass_rate_ci95: (f64, f64),
    pub sessions_to_pass: Option<Quartiles>,
    pub sessions_to_fail: Option<Quartiles>,
    pub fail_reasons: BTreeMap<String, usize>,
}

fn quartiles(mut v: Vec<f64>) -> Option<Quartiles> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    Some(Quartiles { p25: q(0.25), median: q(0.5), p75: q(0.75) })
}

/// Wilson score interval (z = 1.96) for a proportion `p` observed on `n` trials.
pub fn wilson95(p: f64, n: f64) -> (f64, f64) {
    if n <= 0.0 {
        return (0.0, 1.0);
    }
    let z = 1.96_f64;
    let d = 1.0 + z * z / n;
    let c = (p + z * z / (2.0 * n)) / d;
    let h = z * ((p * (1.0 - p) / n) + z * z / (4.0 * n * n)).sqrt() / d;
    ((c - h).max(0.0), (c + h).min(1.0))
}

pub fn run_pass_rate(cfg: EngineConfig, data: Vec<(String, Vec<Bar>)>, opts: PassRateOptions) -> anyhow::Result<PassRateReport> {
    let probe = Engine::new(cfg.clone(), false)?;
    let kind0 = probe.instruments[0].spec.kind;
    let rules = probe.acct.rules.name.clone();
    let mut infos = vec![];
    let mut stream: Vec<(DateTime<Utc>, usize, Bar)> = vec![];
    let mut names = vec![];
    for (k, (sym, bars)) in data.iter().enumerate() {
        let i = probe.instrument_index(sym).ok_or_else(|| anyhow::anyhow!("data symbol {sym} not configured"))?;
        if let (Some(f), Some(l)) = (bars.first(), bars.last()) {
            infos.push(DataInfo { symbol: sym.clone(), bars: bars.len(), from: f.ts, to: l.ts, timeframe_min: probe.instruments[i].tf_minutes });
        }
        names.push(probe.instruments[i].spec.symbol.clone());
        stream.extend(bars.iter().map(|b| (b.ts, k, *b)));
    }
    stream.sort_by_key(|(ts, k, _)| (*ts, *k));
    let mut days: Vec<NaiveDate> = stream.iter().map(|(ts, _, _)| trading_day(kind0, *ts)).collect();
    days.dedup();
    let every = opts.every_days.max(1);

    let mut runs = vec![];
    let mut s = opts.warmup_days;
    while s < days.len() {
        let (start_day, warm_day) = (days[s], days[s - opts.warmup_days]);
        let i0 = stream.partition_point(|(ts, _, _)| trading_day(kind0, *ts) < warm_day);
        let mut eng = Engine::new(cfg.clone(), false)?;
        eng.paused = true; // warm-up: indicators only
        let mut sim = SimBroker::default();
        let (mut started, mut sessions, mut last_day) = (false, 0usize, None);
        let mut end = start_day;
        let mut result: Option<(String, String)> = None;
        let mut last_bar: Option<Bar> = None;
        for (_, k, bar) in &stream[i0..] {
            let day = trading_day(kind0, bar.ts);
            if !started && day >= start_day {
                eng.reset_account();
                eng.paused = false;
                started = true;
            }
            if started && last_day != Some(day) {
                if opts.max_days > 0 && sessions >= opts.max_days {
                    break;
                }
                sessions += 1;
                last_day = Some(day);
            }
            sim.on_bar_open(&mut eng, &names[*k], bar);
            let cmds = eng.on_bar(&names[*k], bar);
            sim.on_commands(&mut eng, bar, cmds);
            last_bar = Some(*bar);
            end = day;
            if !started {
                continue;
            }
            let flat = eng.instruments.iter().all(|r| r.position.is_none());
            if eng.acct.is_active() && flat && eng.acct.buffer() < eng.guard.cfg.min_buffer_frac * eng.acct.rules.max_loss {
                eng.acct.status = AccountStatus::Failed { date: day, reason: format!("guard stop: buffer below {:.0}% of max loss", eng.guard.cfg.min_buffer_frac * 100.0) };
            }
            if !eng.acct.is_active() {
                result = Some(match eng.acct.status.clone() {
                    AccountStatus::Passed { .. } => ("passed".into(), String::new()),
                    AccountStatus::Failed { reason, .. } => ("failed".into(), reason),
                    AccountStatus::Active => unreachable!(),
                });
                break;
            }
        }
        if !started {
            break;
        }
        if let Some(b) = last_bar {
            let cmds = eng.flatten_all("study end", b.ts);
            sim.on_commands(&mut eng, &b, cmds);
        }
        eng.acct.close_books();
        let (outcome, reason) = result.unwrap_or_else(|| ("open".into(), String::new()));
        runs.push(PassRun { start: start_day, end, outcome, reason, sessions, trading_days: eng.acct.trading_days(), trades: eng.trades.len(), profit: eng.acct.total_profit(), best_day: eng.acct.best_day() });
        s += every;
    }

    let pick = |o: &str| runs.iter().filter(|r| r.outcome == o).collect::<Vec<_>>();
    let (passed, failed) = (pick("passed"), pick("failed"));
    let decided = passed.len() + failed.len();
    let pass_rate = if decided > 0 { passed.len() as f64 / decided as f64 } else { 0.0 };
    let mean_len = if decided > 0 { passed.iter().chain(failed.iter()).map(|r| r.sessions as f64).sum::<f64>() / decided as f64 } else { 0.0 };
    let covered = days.len().saturating_sub(opts.warmup_days) as f64;
    let effective_n = if mean_len > 0.0 { (covered / mean_len).min(decided as f64) } else { 0.0 };
    let mut fail_reasons: BTreeMap<String, usize> = BTreeMap::new();
    for r in &failed {
        // group by wording: drop amounts ("≥ 500.00", "$1,234") but keep fixed percentages ("25%")
        let key = r.reason.split_whitespace().filter(|w| !w.chars().any(|c| c.is_ascii_digit()) || w.ends_with('%')).collect::<Vec<_>>().join(" ");
        *fail_reasons.entry(key).or_default() += 1;
    }
    Ok(PassRateReport {
        label: opts.label.clone(),
        generated_at: Utc::now(),
        rules,
        options: opts,
        data: infos,
        passed: passed.len(),
        failed: failed.len(),
        open: runs.len() - decided,
        pass_rate,
        effective_n,
        pass_rate_ci95: wilson95(pass_rate, effective_n),
        sessions_to_pass: quartiles(passed.iter().map(|r| r.sessions as f64).collect()),
        sessions_to_fail: quartiles(failed.iter().map(|r| r.sessions as f64).collect()),
        fail_reasons,
        runs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EngineConfig, StrategyConfig};
    use chrono::{Duration, TimeZone};

    fn mkbar(ts: DateTime<Utc>, o: f64, h: f64, l: f64, c: f64) -> Bar {
        Bar { ts, open: o, high: h, low: l, close: c, volume: 100.0 }
    }

    /// Synthetic 5-minute RTH sessions with a strong up-move after 10:00 on even days.
    fn synthetic(n_days: usize) -> Vec<Bar> {
        let mut out = vec![];
        let mut px = 20_000.0;
        let mut d = 0;
        let mut day = 0;
        let start = Utc.with_ymd_and_hms(2025, 6, 2, 13, 30, 0).unwrap();
        while day < n_days {
            let date = start + Duration::days(d);
            d += 1;
            if matches!(chrono::Datelike::weekday(&date), chrono::Weekday::Sat | chrono::Weekday::Sun) {
                continue;
            }
            for k in 0..78 {
                let ts = date + Duration::minutes(5 * k);
                let drift = if day % 2 == 0 && k > 6 { 4.0 } else { 0.0 };
                let wig = if k % 2 == 0 { 3.0 } else { -3.0 };
                let o = px;
                px += drift + wig;
                out.push(mkbar(ts, o, o.max(px) + 2.0, o.min(px) - 2.0, px));
            }
            day += 1;
        }
        out
    }

    #[test]
    fn stop_before_target_in_same_bar() {
        let mut cfg = EngineConfig::example_futures();
        cfg.strategies = vec![];
        let mut eng = Engine::new(cfg, false).unwrap();
        let mut sim = SimBroker::default();
        let t0 = Utc.with_ymd_and_hms(2025, 6, 2, 15, 0, 0).unwrap();
        sim.resting.insert("MNQ".into(), Resting { trade_id: 1, side: Side::Long, qty: 1.0, stop: 99.0, target: Some(101.0) });
        // fake an open position in the engine so the fill is accounted
        eng.instruments[0].position = Some(crate::strategy::OpenPosition { trade_id: 1, strategy: "x".into(), side: Side::Long, qty: 1.0, entry_price: 100.0, entry_time: t0, stop: 99.0, initial_stop: 99.0, target: Some(101.0), regime_at_entry: Regime::Neutral, bars_held: 0, mfe_price: 100.0, mae_price: 100.0 });
        sim.on_bar_open(&mut eng, "MNQ", &mkbar(t0, 100.0, 102.0, 98.0, 100.0));
        let t = eng.trades.last().expect("trade closed");
        assert_eq!(t.exit_reason, "stop loss");
        assert!((t.exit_price - 98.75).abs() < 1e-9, "stop minus one tick slippage, got {}", t.exit_price);
    }

    #[test]
    fn backtest_runs_end_to_end_and_respects_session() {
        let bars = synthetic(80);
        let mut cfg = EngineConfig::example_futures();
        cfg.account.rules = "personal".into();
        cfg.account.size = Some(50_000.0);
        cfg.guard.risk_per_trade_usd = Some(200.0);
        let mut p = serde_json::Map::new();
        p.insert("min_or_atr".into(), serde_json::json!(0.0));
        cfg.strategies = vec![StrategyConfig { id: "orb".into(), kind: "orb".into(), enabled: true, instruments: vec![], params: p }];
        let rep = run_backtest(cfg, vec![("MNQ".into(), bars)], BacktestOptions::default()).unwrap();
        assert!(rep.summary.trades > 5, "expected trades, got {}", rep.summary.trades);
        for t in &rep.trades {
            assert_eq!(t.entry_time.date_naive(), t.exit_time.date_naive(), "intraday strategy held overnight: {t:?}");
            let et = q22_core::session::et_minute_of_day(t.exit_time);
            assert!(et <= 15 * 60 + 58 + 5, "exit after flatten time: {et}");
        }
        assert!(rep.summary.trading_days >= 70);
    }

    #[test]
    fn wilson_interval_matches_reference() {
        // 8/20: the standard Wilson 95% interval is [0.219, 0.613]
        let (lo, hi) = wilson95(0.4, 20.0);
        assert!((lo - 0.2188).abs() < 1e-3 && (hi - 0.6134).abs() < 1e-3, "{lo} {hi}");
        assert_eq!(wilson95(0.5, 0.0), (0.0, 1.0));
    }

    #[test]
    fn pass_rate_study_starts_fresh_accounts() {
        let bars = synthetic(120);
        let last_day = trading_day(q22_core::MarketKind::CmeEquityIndex, bars.last().unwrap().ts);
        let mut cfg = EngineConfig::example_futures();
        cfg.guard.risk_per_trade_usd = Some(200.0);
        let mut p = serde_json::Map::new();
        p.insert("min_or_atr".into(), serde_json::json!(0.0));
        cfg.strategies = vec![StrategyConfig { id: "orb".into(), kind: "orb".into(), enabled: true, instruments: vec![], params: p }];
        let opts = PassRateOptions { every_days: 10, warmup_days: 20, max_days: 30, label: "t".into() };
        let rep = run_pass_rate(cfg, vec![("MNQ".into(), bars)], opts).unwrap();
        assert_eq!(rep.runs.len(), 10, "one run per start: 20, 30, … 110");
        assert_eq!(rep.passed + rep.failed + rep.open, rep.runs.len());
        for w in rep.runs.windows(2) {
            assert!(w[0].start < w[1].start);
        }
        for r in &rep.runs {
            assert!(r.sessions <= 30 && r.end >= r.start, "{r:?}");
            assert!(r.outcome != "open" || r.sessions == 30 || r.end == last_day, "open run neither capped nor at data end: {r:?}");
        }
        assert!(rep.runs.iter().any(|r| r.trades > 0), "the strategy should trade on the synthetic trend days");
    }
}
