//! q22 — multi-strategy, regime-aware, prop-firm-compliant trading system.
//!
//!   q22 rules                         list prop-firm presets and their compliance notes
//!   q22 backtest -c cfg --data S=path backtest one config (prop replay on by default)
//!   q22 compare  -c cfg --data S=path ablation: regime gating on/off, each strategy alone
//!   q22 run      -c cfg               paper / replay / live trading + dashboard
//!   q22 serve    --reports dir        dashboard for backtest reports only

mod appcfg;
mod backtest;
mod dashboard;
mod datafiles;
mod runner;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use rand::Rng;
use serde_json::json;
use tokio::sync::{mpsc, RwLock};

use q22_engine::prop::PropRules;

#[derive(Parser)]
#[command(name = "q22", version, about = "Regime-aware multi-strategy trading engine with prop-firm compliance and a local dashboard")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List prop-firm presets (or show one in full).
    Rules { name: Option<String> },
    /// Backtest a config on CSV data.
    Backtest {
        #[arg(short, long)]
        config: PathBuf,
        /// SYMBOL=path[@Timezone] (repeatable). Naive timestamps are UTC unless a timezone is given.
        #[arg(long, required = true)]
        data: Vec<String>,
        #[arg(long)]
        from: Option<NaiveDate>,
        #[arg(long)]
        to: Option<NaiveDate>,
        /// Disable prop-evaluation replay (one continuous account).
        #[arg(long)]
        no_replay: bool,
        /// Number of configurations tried so far (deflates the Sharpe ratio).
        #[arg(long, default_value_t = 1)]
        trials: usize,
        #[arg(long, default_value = "backtest")]
        label: String,
        /// Write the JSON report here (default: reports/<label>.json).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Ablation study: regime gating on/off and each strategy alone, with deflated Sharpe.
    Compare {
        #[arg(short, long)]
        config: PathBuf,
        #[arg(long, required = true)]
        data: Vec<String>,
        #[arg(long)]
        from: Option<NaiveDate>,
        #[arg(long)]
        to: Option<NaiveDate>,
        #[arg(long)]
        no_replay: bool,
        #[arg(long, default_value = "compare")]
        label: String,
        #[arg(long, default_value = "reports")]
        out_dir: PathBuf,
    },
    /// Rolling-start study: start a fresh prop evaluation every N sessions; pass probability and time to pass.
    Passrate {
        #[arg(short, long)]
        config: PathBuf,
        #[arg(long, required = true)]
        data: Vec<String>,
        #[arg(long)]
        from: Option<NaiveDate>,
        #[arg(long)]
        to: Option<NaiveDate>,
        /// Start a new evaluation every N sessions.
        #[arg(long, default_value_t = 5)]
        every: usize,
        /// Sessions of indicator-only warm-up before each start.
        #[arg(long, default_value_t = 45)]
        warmup: usize,
        /// Book an evaluation as "open" after N sessions (0 = run until the data ends).
        #[arg(long, default_value_t = 0)]
        max_days: usize,
        #[arg(long, default_value = "passrate")]
        label: String,
        /// Write the JSON report here (default: reports/<label>.json).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Run the engine (paper, replay or live, per [runtime]) with the dashboard.
    Run {
        #[arg(short, long)]
        config: PathBuf,
    },
    /// Dashboard for backtest reports only (no trading).
    Serve {
        #[arg(long, default_value = "reports")]
        reports: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8722")]
        bind: String,
    },
    /// Validate a config and print the compliance checklist.
    Check {
        #[arg(short, long)]
        config: PathBuf,
    },
    /// Convert a Databento all-contract OHLCV-1m CSV into a continuous, back-adjusted front-month series.
    Databento {
        /// Databento CSV (GLBX.MDP3 ohlcv-1m, symbols like ESM0 and spreads like ESM0-ESU0).
        #[arg(long)]
        input: PathBuf,
        /// Product root to extract (ES, NQ, …).
        #[arg(long)]
        root: String,
        /// Output CSV (unix_timestamp,open,high,low,close,volume,contract).
        #[arg(long)]
        out: PathBuf,
    },
    /// Download the free research datasets (index CFD 5-min 2020-23, BTC hourly) into a folder.
    FetchData {
        #[arg(long, default_value = "data")]
        out: PathBuf,
    },
}

const DATASETS: &[(&str, &str)] = &[
    ("USATECHIDXUSD_M5.csv", "https://raw.githubusercontent.com/TheSnowGuru/Stocks-Futures-Financial-Time-series-Tick-Bar-Data/main/indices/nasdaq100/USATECHIDXUSD_M5.csv"),
    ("USA500IDXUSD_M5.csv", "https://raw.githubusercontent.com/TheSnowGuru/Stocks-Futures-Financial-Time-series-Tick-Bar-Data/main/indices/s%26p500/USA500IDXUSD_M5.csv"),
    ("NQ_1m_sample_2026.csv", "https://raw.githubusercontent.com/getdata-finance/nq-1m-ohlcv-stocks-historical-data/main/NQ_1m.csv"),
    ("btc_hourly.csv", "https://media.githubusercontent.com/media/mouadja02/bitcoin-technical-indicators-dataset/main/bitcoin-hourly-ohlcv.csv"),
];

async fn fetch_data(out: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let http = reqwest::Client::builder().user_agent("q22/0.1").build()?;
    for (name, url) in DATASETS {
        let dest = out.join(name);
        if dest.exists() {
            println!("✓ {} (exists)", dest.display());
            continue;
        }
        print!("↓ {name} … ");
        let bytes = http.get(*url).send().await?.error_for_status()?.bytes().await?;
        if bytes.starts_with(b"version https://git-lfs") {
            anyhow::bail!("{name}: got a Git-LFS pointer instead of data");
        }
        std::fs::write(&dest, &bytes)?;
        println!("{:.1} MB", bytes.len() as f64 / 1e6);
    }
    Ok(())
}

fn databento(input: &std::path::Path, root: &str, out: &std::path::Path) -> Result<()> {
    use q22_core::databento::{build_continuous, read_contracts, write_continuous};
    use std::io::Write;
    let t0 = std::time::Instant::now();
    let f = std::fs::File::open(input).map_err(|e| anyhow::anyhow!("opening {}: {e}", input.display()))?;
    let (contracts, st) = read_contracts(std::io::BufReader::with_capacity(1 << 20, f), root)?;
    println!(
        "{}: {} rows, {} spread rows dropped, {} other-root rows, {} {root} contracts ({:.1}s)",
        input.display(), st.rows, st.spread_rows, st.other_root_rows, st.contracts, t0.elapsed().as_secs_f64()
    );
    let (bars, rolls, sessions) = build_continuous(&contracts)?;
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut w = std::io::BufWriter::new(std::fs::File::create(out)?);
    write_continuous(&mut w, &bars)?;
    let log = out.with_extension("rolls.csv");
    let mut lw = std::io::BufWriter::new(std::fs::File::create(&log)?);
    writeln!(lw, "first_session,from,to,gap_points,measured_at_utc")?;
    for r in &rolls {
        writeln!(lw, "{},{},{},{},{}", r.day, r.from, r.to, r.gap, r.at.to_rfc3339())?;
    }
    let cum: f64 = rolls.iter().map(|r| r.gap).sum();
    println!(
        "continuous {root}: {} bars, {sessions} sessions, {} rolls (cumulative back-adjustment {cum:+.2} pts), {} → {}",
        bars.len(), rolls.len(), bars.first().map(|b| b.bar.ts.to_rfc3339()).unwrap_or_default(), bars.last().map(|b| b.bar.ts.to_rfc3339()).unwrap_or_default()
    );
    println!("written {} and {} ({:.1}s)", out.display(), log.display(), t0.elapsed().as_secs_f64());
    Ok(())
}

fn token() -> String {
    if let Ok(t) = std::env::var("Q22_DASH_TOKEN") {
        if t.len() >= 12 {
            return t;
        }
    }
    let mut rng = rand::thread_rng();
    (0..24).map(|_| {
        const A: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
        A[rng.gen_range(0..A.len())] as char
    }).collect()
}

fn checklist(cfg: &appcfg::AppConfig) -> Result<()> {
    let rules = cfg.engine.account.resolve_rules()?;
    println!("rules: {} — {}", rules.name, rules.firm);
    println!("  automation: {:?} | mode: {:?} | broker: {:?} | feed: {:?}", rules.automation, cfg.engine.mode, cfg.runtime.broker, cfg.runtime.feed);
    println!("  max loss ${:.0} ({:?}) | target {:?} | DLL {:?} | consistency {:?} {:?}", rules.max_loss, rules.drawdown_mode, rules.profit_target, rules.daily_loss_limit, rules.consistency_share, rules.consistency_mode);
    let opt = |o: Option<String>| o.unwrap_or_else(|| "—".into());
    println!(
        "  flat by {} CT | overnight {} | stop required within {} | max risk/trade {}",
        opt(rules.flat_by_ct.clone()), rules.overnight_allowed, opt(rules.stop_required_within_s.map(|s| format!("{s}s"))), opt(rules.max_risk_per_trade_pct.map(|p| format!("{:.1}%", p * 100.0)))
    );
    for n in &rules.notes {
        println!("  • {n}");
    }
    let live = cfg.runtime.broker != appcfg::BrokerKind::Paper;
    if live && cfg.engine.mode == q22_engine::config::Mode::Auto && rules.automation != q22_engine::prop::Automation::Full {
        println!("  ✗ this firm does not allow fully automated trading on funded accounts → set mode = \"assist\"");
    }
    if rules.vps_vpn_prohibited {
        println!("  ! run on your own computer: VPS/VPN/remote servers are prohibited by this firm");
    }
    let e = q22_engine::Engine::new(cfg.engine.clone(), false)?;
    for rt in &e.instruments {
        println!("  instrument {} ({} min): strategies {:?}", rt.spec.symbol, rt.tf_minutes, rt.strategies.iter().map(|s| s.id().to_string()).collect::<Vec<_>>());
    }
    println!("  CME futures only: flatten at {} ET; entries {}–{} ET (crypto trades 24/7)", q22_core::session::fmt_hhmm(e.guard.flatten_minute_et()), cfg.engine.guard.entry_start_et, cfg.engine.guard.entry_end_et);
    println!("config OK");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,hyper=warn,reqwest=warn".into())).with_target(false).init();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Rules { name } => match name {
            Some(n) => {
                let r = PropRules::preset(&n, None).ok_or_else(|| anyhow::anyhow!("unknown preset {n}"))?;
                println!("{}", serde_json::to_string_pretty(&r)?);
            }
            None => {
                for n in PropRules::preset_names() {
                    let r = PropRules::preset(n, None).unwrap();
                    println!("{:<26} {:<44} automation {:?}", n, r.firm, r.automation);
                }
            }
        },
        Cmd::Backtest { config, data, from, to, no_replay, trials, label, out } => {
            let cfg = appcfg::AppConfig::load(&config)?;
            let args = backtest::BacktestArgs { data, from, to, replay: !no_replay, trials, label: label.clone() };
            let rep = tokio::task::spawn_blocking(move || backtest::run(cfg.engine, &args)).await??;
            backtest::print_report(&rep);
            let path = out.unwrap_or_else(|| backtest::default_report_path("reports", &label));
            backtest::save(&rep, &path)?;
        }
        Cmd::Compare { config, data, from, to, no_replay, label, out_dir } => {
            let cfg = appcfg::AppConfig::load(&config)?;
            let args = backtest::BacktestArgs { data, from, to, replay: !no_replay, trials: 1, label };
            tokio::task::spawn_blocking(move || backtest::compare(cfg.engine, &args, &out_dir)).await??;
        }
        Cmd::Passrate { config, data, from, to, every, warmup, max_days, label, out } => {
            let cfg = appcfg::AppConfig::load(&config)?;
            let args = backtest::BacktestArgs { data, from, to, replay: true, trials: 1, label: label.clone() };
            let opts = q22_engine::sim::PassRateOptions { every_days: every, warmup_days: warmup, max_days, label: label.clone() };
            let rep = tokio::task::spawn_blocking(move || backtest::pass_rate(cfg.engine, &args, opts)).await??;
            backtest::print_pass_rate(&rep);
            backtest::save_json(&rep, &out.unwrap_or_else(|| backtest::default_report_path("reports", &label)))?;
        }
        Cmd::FetchData { out } => fetch_data(&out).await?,
        Cmd::Databento { input, root, out } => tokio::task::spawn_blocking(move || databento(&input, &root, &out)).await??,
        Cmd::Check { config } => {
            let cfg = appcfg::AppConfig::load(&config)?;
            checklist(&cfg)?;
        }
        Cmd::Serve { reports, bind } => {
            let (tx, _rx) = mpsc::unbounded_channel();
            let shared = Arc::new(runner::Shared { snapshot: RwLock::new(json!({})), meta: RwLock::new(json!({"feed": "none", "broker": "none", "warnings": ["reports-only mode: no engine running"]})), ctrl: tx, token: token(), reports_dir: reports });
            dashboard::serve(shared, &bind).await?;
        }
        Cmd::Run { config } => {
            let cfg = appcfg::AppConfig::load(&config)?;
            checklist(&cfg)?;
            let (tx, rx) = mpsc::unbounded_channel();
            let shared = Arc::new(runner::Shared { snapshot: RwLock::new(json!({})), meta: RwLock::new(json!({})), ctrl: tx, token: token(), reports_dir: cfg.rel(&cfg.dashboard.reports_dir) });
            let bind = cfg.dashboard.bind.clone();
            let sh = shared.clone();
            tokio::spawn(async move {
                if let Err(e) = dashboard::serve(sh, &bind).await {
                    tracing::error!("dashboard: {e:#}");
                }
            });
            let r = runner::Runner::new(cfg, shared, rx).await?;
            r.run().await?;
        }
    }
    Ok(())
}
