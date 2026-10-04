//! `q22 backtest` and `q22 compare` (ablation study).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use chrono::NaiveDate;

use q22_engine::sim::{run_backtest, BacktestOptions, BacktestReport};
use q22_engine::EngineConfig;

use crate::datafiles::{load, parse_spec};

pub struct BacktestArgs {
    pub data: Vec<String>,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub replay: bool,
    pub trials: usize,
    pub label: String,
}

pub fn load_all(cfg: &EngineConfig, args: &BacktestArgs) -> Result<Vec<(String, Vec<q22_core::Bar>)>> {
    let mut out = vec![];
    for d in &args.data {
        let spec = parse_spec(d)?;
        let ic = cfg.instruments.iter().find(|i| i.symbol.eq_ignore_ascii_case(&spec.symbol)).ok_or_else(|| anyhow!("--data symbol {} is not in [[instrument]]", spec.symbol))?;
        let bars = load(Path::new(&spec.path), spec.tz, ic.timeframe_min, args.from, args.to)?;
        eprintln!("loaded {} {}-min bars for {} ({} → {})", bars.len(), ic.timeframe_min, spec.symbol, bars.first().unwrap().ts.date_naive(), bars.last().unwrap().ts.date_naive());
        out.push((spec.symbol, bars));
    }
    if out.is_empty() {
        return Err(anyhow!("give at least one --data SYMBOL=path"));
    }
    Ok(out)
}

pub fn run(cfg: EngineConfig, args: &BacktestArgs) -> Result<BacktestReport> {
    let data = load_all(&cfg, args)?;
    run_backtest(cfg, data, BacktestOptions { prop_replay: args.replay, n_trials: args.trials, label: args.label.clone() })
}

fn money(x: f64) -> String {
    let s = format!("{:.0}", x.abs());
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    let body: String = out.chars().rev().collect();
    if x < 0.0 { format!("-${body}") } else { format!("${body}") }
}

pub fn print_report(r: &BacktestReport) {
    let s = &r.summary;
    println!("\n=== {} — rules {} ===", r.label, r.prop.rules);
    for d in &r.data {
        println!("data: {} {} bars of {} min, {} → {}", d.symbol, d.bars, d.timeframe_min, d.from.date_naive(), d.to.date_naive());
    }
    println!(
        "net {} (gross {}, costs {}) | trades {} | win {:.1}% | PF {:.2} | avg {:+.3}R | expectancy {}/trade",
        money(s.net_pnl), money(s.gross_pnl), money(s.costs), s.trades, s.win_rate * 100.0, s.profit_factor, s.avg_r, money(s.expectancy_usd)
    );
    println!(
        "days {} (with trades {}) | daily Sharpe {:.2} ann | PSR(>0) {:.3} | DSR {:.3} ({} trials, benchmark SR {:.2}) | MinTRL {:.0} days",
        s.trading_days, s.days_with_trades, s.sharpe_daily_ann, s.psr_vs_zero, s.dsr, s.dsr_trials, s.dsr_benchmark_sr_ann, s.min_track_record_days
    );
    println!("max drawdown {} | best day {} | worst day {} | net / max-loss {:.2}", money(s.max_drawdown_usd), money(s.best_day), money(s.worst_day), s.return_on_max_loss);
    println!(
        "prop replay: {} attempts → {} passed, {} failed, {} open | pass rate {:.0}% | median active days (with a closed trade) to pass {} / to fail {} — see `q22 passrate` for calendar time",
        r.prop.attempts, r.prop.passed, r.prop.failed, r.prop.open, r.prop.pass_rate * 100.0, r.prop.median_days_to_pass.map_or("—".into(), |d| format!("{d:.0}")), r.prop.median_days_to_fail.map_or("—".into(), |d| format!("{d:.0}"))
    );
    println!("{:<18} {:>6} {:>6} {:>7} {:>8} {:>11}", "strategy", "trades", "win%", "PF", "avg R", "net");
    for g in &r.by_strategy {
        println!("{:<18} {:>6} {:>5.1}% {:>7.2} {:>+8.3} {:>11}", g.key, g.trades, g.win_rate * 100.0, g.profit_factor, g.avg_r, money(g.net));
    }
    println!("{:<18} {:>6} {:>6} {:>7} {:>8} {:>11}", "regime", "trades", "win%", "PF", "avg R", "net");
    for g in &r.by_regime {
        println!("{:<18} {:>6} {:>5.1}% {:>7.2} {:>+8.3} {:>11}", g.key, g.trades, g.win_rate * 100.0, g.profit_factor, g.avg_r, money(g.net));
    }
    let rd: Vec<String> = r.regime_days.iter().map(|(k, v)| format!("{k}: {v}")).collect();
    println!("regime days: {}", rd.join(", "));
    let mut bc: Vec<(&String, &usize)> = r.block_counts.iter().collect();
    bc.sort_by(|a, b| b.1.cmp(a.1));
    println!("entries not taken (top reasons):");
    for (k, v) in bc.iter().take(12) {
        println!("  {v:>6}  {k}");
    }
}

pub fn save(r: &BacktestReport, path: &Path) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, serde_json::to_vec(r)?)?;
    eprintln!("report written to {}", path.display());
    Ok(())
}

/// Ablation: all strategies with/without regime gating and adaptive health, then each alone.
pub fn compare(cfg: EngineConfig, args: &BacktestArgs, out_dir: &Path) -> Result<Vec<(String, BacktestReport)>> {
    let data = load_all(&cfg, args)?;
    let mut variants: Vec<(String, EngineConfig)> = vec![];
    variants.push(("ALL · regime gating ON · adaptive ON".into(), cfg.clone()));
    let mut c = cfg.clone();
    c.allocator.adaptive_health = false;
    variants.push(("ALL · regime gating ON · adaptive OFF".into(), c));
    let mut c = cfg.clone();
    c.allocator.regime_gating = false;
    c.allocator.adaptive_health = false;
    variants.push(("ALL · no gating · no adaptive".into(), c));
    for s in cfg.strategies.iter().filter(|s| s.enabled) {
        let mut c = cfg.clone();
        c.strategies = vec![s.clone()];
        c.allocator.regime_gating = false;
        c.allocator.adaptive_health = false;
        variants.push((format!("{} alone (ungated)", s.id), c.clone()));
        let mut c2 = c;
        c2.allocator.regime_gating = true;
        variants.push((format!("{} alone (gated)", s.id), c2));
    }
    let n = variants.len();
    let mut out = vec![];
    for (label, c) in variants {
        let rep = run_backtest(c, data.clone(), BacktestOptions { prop_replay: args.replay, n_trials: n, label: label.clone() })?;
        let file = out_dir.join(format!("{}__{}.json", args.label, slug(&label)));
        save(&rep, &file)?;
        out.push((label, rep));
    }
    println!("\n=== ablation: {} variants (DSR deflated for {} trials) ===", n, n);
    println!("{:<40} {:>6} {:>6} {:>6} {:>8} {:>10} {:>7} {:>9} {:>6} {:>6} {:>9}", "variant", "trades", "win%", "PF", "avgR", "net", "Sharpe", "maxDD", "PSR", "DSR", "pass");
    for (label, r) in &out {
        let s = &r.summary;
        println!(
            "{:<40} {:>6} {:>5.1}% {:>6.2} {:>+8.3} {:>10} {:>7.2} {:>9} {:>6.3} {:>6.3} {:>3}/{:<3}",
            label, s.trades, s.win_rate * 100.0, s.profit_factor, s.avg_r, money(s.net_pnl), s.sharpe_daily_ann, money(s.max_drawdown_usd), s.psr_vs_zero, s.dsr, r.prop.passed, r.prop.passed + r.prop.failed
        );
    }
    let summary: Vec<serde_json::Value> = out
        .iter()
        .map(|(l, r)| serde_json::json!({"variant": l, "summary": r.summary, "prop": r.prop, "by_strategy": r.by_strategy}))
        .collect();
    std::fs::write(out_dir.join(format!("{}__compare.json", args.label)), serde_json::to_vec_pretty(&summary)?)?;
    Ok(out)
}

pub fn slug(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect::<String>().split('_').filter(|x| !x.is_empty()).collect::<Vec<_>>().join("_")
}

pub fn default_report_path(dir: &str, label: &str) -> PathBuf {
    PathBuf::from(dir).join(format!("{}.json", slug(label)))
}

/// `q22 passrate`: a fresh evaluation every N sessions — the probability of passing and how long it takes.
pub fn pass_rate(cfg: EngineConfig, args: &BacktestArgs, opts: q22_engine::sim::PassRateOptions) -> Result<q22_engine::sim::PassRateReport> {
    let data = load_all(&cfg, args)?;
    q22_engine::sim::run_pass_rate(cfg, data, opts)
}

pub fn print_pass_rate(r: &q22_engine::sim::PassRateReport) {
    println!("\n=== {} — rolling-start evaluations under {} ===", r.label, r.rules);
    for d in &r.data {
        println!("data: {} {} bars of {} min, {} → {}", d.symbol, d.bars, d.timeframe_min, d.from.date_naive(), d.to.date_naive());
    }
    let cap = if r.options.max_days > 0 { format!(", capped at {} sessions", r.options.max_days) } else { String::new() };
    println!("{} evaluations (one every {} sessions{cap}) → {} passed, {} failed, {} open", r.runs.len(), r.options.every_days, r.passed, r.failed, r.open);
    println!(
        "pass rate {:.0}%  (95% CI {:.0}–{:.0}% on ≈{:.0} independent attempts — runs overlap)",
        r.pass_rate * 100.0, r.pass_rate_ci95.0 * 100.0, r.pass_rate_ci95.1 * 100.0, r.effective_n
    );
    let q = |x: &Option<q22_engine::sim::Quartiles>| x.as_ref().map(|q| format!("median {:.0} (IQR {:.0}–{:.0})", q.median, q.p25, q.p75)).unwrap_or_else(|| "—".into());
    println!("sessions to pass: {} | sessions to fail: {}", q(&r.sessions_to_pass), q(&r.sessions_to_fail));
    for (reason, n) in &r.fail_reasons {
        println!("  failed {n:>4}× {reason}");
    }
}

pub fn save_json<T: serde::Serialize>(r: &T, path: &Path) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, serde_json::to_vec(r)?)?;
    eprintln!("report written to {}", path.display());
    Ok(())
}
