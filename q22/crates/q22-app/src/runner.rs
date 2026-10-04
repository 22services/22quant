//! The live / paper / replay loop.
//!
//! ```text
//!   feed ──bars──► engine.on_bar ──commands──► executor (SimBroker | live Broker)
//!     ▲                 ▲    │                        │
//!     │        controls │    └── snapshot ──► dashboard (SSE)
//!     └─────────── reconcile (live): broker positions vs engine book
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::{json, Value};
use tokio::sync::{mpsc, RwLock};

use q22_broker::bybit::Bybit;
use q22_broker::projectx::ProjectX;
use q22_broker::Broker;
use q22_core::Bar;
use q22_engine::engine::{Command, Fill, FillKind};
use q22_engine::sim::SimBroker;
use q22_engine::snapshot::snapshot;
use q22_engine::Engine;

use crate::appcfg::{AppConfig, BrokerKind, FeedKind};
use crate::datafiles;

#[derive(Debug, Clone)]
pub enum Control {
    Pause,
    Resume,
    Flatten,
    Kill,
    Approve(u64),
    Reject(u64),
    Heartbeat,
}

pub struct Shared {
    pub snapshot: RwLock<Value>,
    pub meta: RwLock<Value>,
    pub ctrl: mpsc::UnboundedSender<Control>,
    pub token: String,
    pub reports_dir: PathBuf,
}

enum Feed {
    Replay { stream: Vec<(String, Bar)>, pos: usize, speed: f64, live_from: usize },
    Poll { src: Arc<dyn Broker>, last: HashMap<String, DateTime<Utc>>, tf: HashMap<String, i64>, bsym: HashMap<String, String> },
}

enum Exec {
    Sim(SimBroker),
    Live(Arc<dyn Broker>),
}

pub struct Runner {
    eng: Engine,
    feed: Feed,
    exec: Exec,
    shared: Arc<Shared>,
    ctrl_rx: mpsc::UnboundedReceiver<Control>,
    poll: StdDuration,
    state_dir: PathBuf,
    unmanaged: String,
    flatten_on_exit: bool,
    killed: bool,
    trades_logged: usize,
    /// Polled bars waiting for the other instruments' bar of the same timestamp.
    held: Vec<(String, Bar)>,
}

async fn make_broker(cfg: &AppConfig) -> Result<Arc<dyn Broker>> {
    Ok(match cfg.runtime.broker {
        BrokerKind::Projectx => {
            let mut b = ProjectX::from_env(cfg.runtime.projectx_account.clone(), cfg.runtime.projectx_api.clone())?;
            let a = b.connect().await.context("ProjectX connect")?;
            tracing::info!("connected to ProjectX account {} ({}), balance {:.2}, can_trade {}", a.name, a.id, a.balance, a.can_trade);
            Arc::new(b)
        }
        BrokerKind::Bybit => {
            let mut b = Bybit::from_env(&cfg.runtime.bybit_base)?;
            let a = b.connect().await.context("Bybit connect")?;
            tracing::info!("connected to Bybit ({}), equity {:.2}", cfg.runtime.bybit_base, a.balance);
            Arc::new(b)
        }
        BrokerKind::Rithmic => {
            #[cfg(feature = "rithmic")]
            {
                let mut b = q22_broker::rithmic::Rithmic::from_env()?;
                let a = b.connect().await.context("Rithmic connect")?;
                tracing::info!("connected to Rithmic account {}, balance {:.2}", a.name, a.balance);
                Arc::new(b)
            }
            #[cfg(not(feature = "rithmic"))]
            return Err(anyhow!("this build has no Rithmic support: rebuild with the default features"));
        }
        BrokerKind::Tradovate => {
            let mut b = q22_broker::tradovate::Tradovate::from_env()?;
            let a = b.connect().await.context("Tradovate connect")?;
            tracing::info!("connected to Tradovate account {} ({}), balance {:.2}", a.name, a.id, a.balance);
            Arc::new(b)
        }
        BrokerKind::Paper => return Err(anyhow!("paper has no remote broker")),
    })
}

/// `q22 broker-check`: connect, print the account, positions and the last bars of each
/// configured instrument. Read-only — never places an order.
pub async fn broker_check(cfg: &AppConfig) -> Result<()> {
    if cfg.runtime.broker == BrokerKind::Paper {
        return Err(anyhow!("[runtime] broker is \"paper\": nothing to check"));
    }
    let b = make_broker(cfg).await?;
    let a = b.account().await?;
    println!("✓ {} account {} ({}) balance {:.2} can_trade {}", b.name(), a.name, a.id, a.balance, a.can_trade);
    for p in b.positions().await? {
        println!("  open position {} {:+} @ {:.2}", p.symbol, p.qty, p.avg_price);
    }
    for ic in &cfg.engine.instruments {
        let sym = ic.broker_symbol.clone().unwrap_or_else(|| ic.symbol.clone());
        match b.bars(&sym, ic.timeframe_min, Utc::now() - Duration::days(3)).await {
            Ok(bars) => match bars.last() {
                Some(l) => println!("✓ {sym}: {} bars of {} min, last {} close {:.2}", bars.len(), ic.timeframe_min, l.ts, l.close),
                None => println!("! {sym}: no bars returned (market closed or no data entitlement?)"),
            },
            Err(e) => println!("✗ {sym}: bars failed: {e:#}"),
        }
    }
    println!("read-only check complete — no order was sent");
    Ok(())
}

impl Runner {
    pub async fn new(cfg: AppConfig, shared: Arc<Shared>, ctrl_rx: mpsc::UnboundedReceiver<Control>) -> Result<Self> {
        let live = cfg.runtime.broker != BrokerKind::Paper;
        let mut eng = Engine::new(cfg.engine.clone(), live)?;
        let mut warnings = vec![];
        let remote = if live || cfg.runtime.feed == FeedKind::Broker { Some(make_broker(&cfg).await?) } else { None };
        let exec = match cfg.runtime.broker {
            BrokerKind::Paper => Exec::Sim(SimBroker::default()),
            _ => Exec::Live(remote.clone().unwrap()),
        };
        // ---- feed + warm-up
        let warmup_since = Utc::now() - Duration::days(cfg.runtime.warmup_days);
        let feed = match cfg.runtime.feed {
            FeedKind::Replay => {
                let from = cfg.runtime.replay_from.as_deref().map(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d")).transpose()?;
                let mut stream = vec![];
                // with replay_from, only load what the warm-up needs (long histories stay fast)
                let load_from = from.map(|d| d - Duration::days(cfg.runtime.warmup_days + 10));
                for ic in &cfg.engine.instruments {
                    let path = cfg.runtime.replay_files.get(&ic.symbol).ok_or_else(|| anyhow!("[runtime.replay_files] has no file for {}", ic.symbol))?;
                    let bars = datafiles::load(&cfg.rel(path), None, ic.timeframe_min, load_from, None)?;
                    stream.extend(bars.into_iter().map(|b| (ic.symbol.clone(), b)));
                }
                stream.sort_by_key(|(s, b)| (b.ts, s.clone()));
                // replay starts `replay_from` (or after warmup_days of history) — earlier bars warm up instantly
                let first = stream.first().map(|x| x.1.ts).ok_or_else(|| anyhow!("empty replay data"))?;
                let start = from.map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc()).unwrap_or(first + Duration::days(cfg.runtime.warmup_days));
                let live_from = stream.iter().position(|(_, b)| b.ts >= start).unwrap_or(stream.len());
                warnings.push(format!("REPLAY of historical data — {} bars, live-speed from {}", stream.len(), start.date_naive()));
                Feed::Replay { stream, pos: 0, speed: cfg.runtime.replay_speed, live_from }
            }
            FeedKind::Broker | FeedKind::BybitPublic => {
                let src: Arc<dyn Broker> = match cfg.runtime.feed {
                    FeedKind::BybitPublic => Arc::new(Bybit::new(&cfg.runtime.bybit_base, None, None)?),
                    _ => remote.clone().unwrap(),
                };
                let mut last = HashMap::new();
                let mut tf = HashMap::new();
                let mut bsym = HashMap::new();
                eng.paused = true; // warm-up: indicators only, no orders
                let mut merged: Vec<(String, Bar)> = vec![];
                for ic in &cfg.engine.instruments {
                    let b = ic.broker_symbol.clone().unwrap_or_else(|| ic.symbol.clone());
                    let hist = src.bars(&b, ic.timeframe_min, warmup_since).await.with_context(|| format!("warm-up bars for {}", ic.symbol))?;
                    tracing::info!("warm-up {}: {} bars", ic.symbol, hist.len());
                    last.insert(ic.symbol.clone(), hist.last().map(|b| b.ts).unwrap_or(warmup_since));
                    tf.insert(ic.symbol.clone(), ic.timeframe_min);
                    bsym.insert(ic.symbol.clone(), b);
                    merged.extend(hist.into_iter().map(|bar| (ic.symbol.clone(), bar)));
                }
                // time-ordered, same-timestamp bars together (sessions roll once, peers stay in step)
                merged.sort_by_key(|(s, b)| (b.ts, s.clone()));
                for group in merged.chunk_by(|a, b| a.1.ts == b.1.ts) {
                    let g: Vec<(&str, Bar)> = group.iter().map(|(s, b)| (s.as_str(), *b)).collect();
                    let _ = eng.on_bars(&g);
                }
                eng.paused = false;
                Feed::Poll { src, last, tf, bsym }
            }
        };
        if live {
            if eng.guard.cfg.operator_heartbeat_s.is_none() {
                eng.guard.cfg.operator_heartbeat_s = Some(180);
                warnings.push("operator heartbeat enabled (180 s): keep the dashboard open while the bot trades".into());
            }
            if eng.acct.rules.vps_vpn_prohibited {
                warnings.push(format!("{}: VPS / VPN / remote servers are prohibited — run q22 on your own computer", eng.acct.rules.firm));
            }
        }
        let state_dir = cfg.rel(&cfg.runtime.state_dir);
        std::fs::create_dir_all(&state_dir).ok();
        *shared.meta.write().await = json!({
            "broker": format!("{:?}", cfg.runtime.broker).to_lowercase(),
            "feed": format!("{:?}", cfg.runtime.feed).to_lowercase(),
            "config": cfg.path.display().to_string(),
            "warnings": warnings,
            "started": Utc::now(),
        });
        Ok(Self {
            eng,
            feed,
            exec,
            shared,
            ctrl_rx,
            poll: StdDuration::from_secs(cfg.runtime.poll_secs.max(1)),
            state_dir,
            unmanaged: cfg.runtime.on_unmanaged_position.clone(),
            flatten_on_exit: cfg.runtime.flatten_on_exit,
            killed: false,
            trades_logged: 0,
            held: vec![],
        })
    }

    async fn publish(&mut self) {
        let snap = snapshot(&self.eng, 400);
        *self.shared.snapshot.write().await = snap;
        // append new closed trades to the journal
        if self.eng.trades.len() > self.trades_logged {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.state_dir.join("trades.jsonl")) {
                for t in &self.eng.trades[self.trades_logged..] {
                    let _ = writeln!(f, "{}", serde_json::to_string(t).unwrap_or_default());
                }
            }
            self.trades_logged = self.eng.trades.len();
        }
    }

    async fn execute(&mut self, bar: Option<&Bar>, cmds: Vec<Command>) {
        if cmds.is_empty() {
            return;
        }
        match &mut self.exec {
            Exec::Sim(sim) => {
                if let Some(b) = bar {
                    sim.on_commands(&mut self.eng, b, cmds);
                } else {
                    // timer-driven commands in paper mode: execute at the last known price
                    let i = self.eng.instrument_index(cmds[0].symbol()).unwrap_or(0);
                    let px = self.eng.instruments[i].last_price;
                    let ts = Utc::now();
                    let fake = Bar { ts, open: px, high: px, low: px, close: px, volume: 0.0 };
                    sim.on_commands(&mut self.eng, &fake, cmds);
                }
            }
            Exec::Live(b) => {
                let b = b.clone();
                for c in cmds {
                    self.execute_live(&b, c).await;
                }
            }
        }
    }

    async fn execute_live(&mut self, b: &Arc<dyn Broker>, c: Command) {
        let now = Utc::now();
        let bsym = |eng: &Engine, s: &str| eng.instrument_index(s).map(|i| eng.instruments[i].broker_symbol.clone()).unwrap_or_else(|| s.to_string());
        match c {
            Command::Enter { trade_id, symbol, side, qty, stop, target, .. } => {
                let bs = bsym(&self.eng, &symbol);
                match b.enter(&bs, side, qty, stop, target, &format!("q22-{trade_id}")).await {
                    Ok(ack) => {
                        let px = ack.fill_price.unwrap_or(f64::NAN);
                        self.eng.on_fill(&Fill { trade_id, symbol: symbol.clone(), kind: FillKind::Entry, qty: ack.filled_qty, price: px, ts: now, note: format!("order {}", ack.order_id) });
                    }
                    Err(e) => {
                        let msg = format!("{e:#}");
                        self.eng.cancel_pending(&symbol, &msg, now);
                        if msg.contains("CRITICAL") {
                            self.eng.log(now, "halt", &symbol, format!("protective orders failed — flattening and pausing: {msg}"));
                            let _ = b.close(&bs).await;
                            self.eng.paused = true;
                        }
                    }
                }
            }
            Command::ModifyStop { symbol, stop, .. } => {
                let bs = bsym(&self.eng, &symbol);
                let (side, qty) = self.eng.instrument_index(&symbol).and_then(|i| self.eng.instruments[i].position.as_ref().map(|p| (p.side, p.qty))).unwrap_or((q22_core::Side::Long, 0.0));
                if let Err(e) = b.modify_stop(&bs, side, qty, stop).await {
                    self.eng.log(now, "warn", &symbol, format!("stop modify failed (old stop still active): {e:#}"));
                }
            }
            Command::Exit { trade_id, symbol, reason, .. } => {
                let bs = bsym(&self.eng, &symbol);
                match b.close(&bs).await {
                    Ok(px) => {
                        let i = self.eng.instrument_index(&symbol).unwrap_or(0);
                        let price = px.unwrap_or(self.eng.instruments[i].last_price);
                        let qty = self.eng.instruments[i].position.as_ref().map(|p| p.qty).unwrap_or(0.0);
                        self.eng.on_fill(&Fill { trade_id, symbol, kind: FillKind::Exit, qty, price, ts: now, note: reason });
                    }
                    Err(e) => self.eng.log(now, "halt", &symbol, format!("EXIT FAILED — check the platform now: {e:#}")),
                }
            }
        }
    }

    /// Live: compare broker positions with the engine's book.
    async fn reconcile(&mut self) {
        let Exec::Live(b) = &self.exec else { return };
        let b = b.clone();
        let now = Utc::now();
        let positions = match b.positions().await {
            Ok(p) => p,
            Err(e) => {
                self.eng.log(now, "warn", "", format!("reconcile failed: {e:#}"));
                return;
            }
        };
        for i in 0..self.eng.instruments.len() {
            let (sym, bs) = (self.eng.instruments[i].spec.symbol.clone(), self.eng.instruments[i].broker_symbol.clone());
            let broker_pos = positions.iter().find(|p| p.symbol.eq_ignore_ascii_case(&bs) || p.symbol.eq_ignore_ascii_case(&sym));
            let engine_has = self.eng.instruments[i].position.is_some();
            match (engine_has, broker_pos) {
                (true, None) => {
                    // stop or target filled at the broker: book it and cancel the surviving order (OCO)
                    let px = b.last_exit_price(&bs).await.ok().flatten().unwrap_or(self.eng.instruments[i].last_price);
                    self.eng.on_external_flat(&sym, px, now, "closed at broker (stop/target)");
                    if let Err(e) = b.cancel_all(&bs).await {
                        self.eng.log(now, "warn", &sym, format!("cancel of remaining protective order failed: {e:#}"));
                    }
                }
                (false, Some(p)) => {
                    let msg = format!("UNMANAGED position at broker: {} {} @ {:.2}", p.qty, p.symbol, p.avg_price);
                    if self.unmanaged == "flatten" {
                        self.eng.log(now, "halt", &sym, format!("{msg} — flattening"));
                        let _ = b.close(&bs).await;
                    } else if !self.eng.paused {
                        self.eng.log(now, "halt", &sym, format!("{msg} — engine paused (set on_unmanaged_position = \"flatten\" to auto-close)"));
                        self.eng.paused = true;
                    }
                }
                _ => {}
            }
        }
    }

    async fn handle_controls(&mut self) {
        while let Ok(c) = self.ctrl_rx.try_recv() {
            let now = Utc::now();
            match c {
                Control::Heartbeat => self.eng.guard.last_heartbeat = Some(now),
                Control::Pause => {
                    self.eng.paused = true;
                    self.eng.log(now, "info", "", "paused by operator (open positions keep their stops)");
                }
                Control::Resume => {
                    self.eng.paused = false;
                    self.eng.log(now, "info", "", "resumed by operator");
                }
                Control::Flatten => {
                    self.eng.paused = true;
                    let cmds = self.eng.flatten_all("operator flatten", now);
                    self.execute(None, cmds).await;
                }
                Control::Kill => {
                    self.eng.paused = true;
                    let cmds = self.eng.flatten_all("operator kill switch", now);
                    self.execute(None, cmds).await;
                    self.killed = true;
                }
                Control::Approve(id) => match self.eng.approve(id, now) {
                    Ok(cmd) => self.execute(None, vec![cmd]).await,
                    Err(e) => self.eng.log(now, "warn", "", format!("approval #{id} refused: {e:#}")),
                },
                Control::Reject(id) => self.eng.reject(id, now),
            }
        }
    }

    pub async fn run(mut self) -> Result<()> {
        let mut shutdown = Box::pin(tokio::signal::ctrl_c());
        loop {
            self.handle_controls().await;
            if self.killed {
                break;
            }
            // ---- bars
            let mut new_bars: Vec<(String, Bar)> = vec![];
            let mut sleep = self.poll;
            match &mut self.feed {
                Feed::Replay { stream, pos, speed, live_from } => {
                    if *pos >= stream.len() {
                        self.eng.log(Utc::now(), "info", "", "replay finished");
                        self.publish().await;
                        tokio::select! { _ = tokio::time::sleep(StdDuration::from_secs(3600)) => {}, _ = &mut shutdown => break }
                        continue;
                    }
                    let batch = if *pos < *live_from { (*live_from - *pos).min(5_000) } else { 1 };
                    new_bars.extend(stream[*pos..(*pos + batch).min(stream.len())].iter().cloned());
                    *pos += batch;
                    sleep = if *pos <= *live_from || *speed <= 0.0 { StdDuration::from_millis(1) } else { StdDuration::from_secs_f64(1.0 / *speed) };
                }
                Feed::Poll { src, last, tf, bsym } => {
                    for (sym, since) in last.iter_mut() {
                        let t = tf[sym];
                        match src.bars(&bsym[sym], t, *since - Duration::minutes(t)).await {
                            Ok(bars) => {
                                let cutoff = *since;
                                for b in bars.into_iter().filter(|b| b.ts > cutoff) {
                                    *since = b.ts;
                                    new_bars.push((sym.clone(), b));
                                }
                            }
                            Err(e) => tracing::warn!("bars {sym}: {e:#}"),
                        }
                    }
                    // Cross-asset strategies must see every instrument at the same timestamp:
                    // hold a timestamp until all instruments delivered it (or 20 s after the bar
                    // closed, so one stalled feed cannot freeze the other).
                    self.held.append(&mut new_bars);
                    self.held.sort_by_key(|(s, b)| (b.ts, s.clone()));
                    let n_inst = last.len();
                    let now = Utc::now();
                    let mut release = 0;
                    for group in self.held.chunk_by(|a, b| a.1.ts == b.1.ts) {
                        let ts = group[0].1.ts;
                        let closed_for = now - (ts + Duration::minutes(tf.get(&group[0].0).copied().unwrap_or(1)));
                        if group.len() >= n_inst || closed_for > Duration::seconds(20) {
                            release += group.len();
                        } else {
                            break;
                        }
                    }
                    new_bars = self.held.drain(..release).collect();
                }
            }
            for group in new_bars.chunk_by(|a, b| a.1.ts == b.1.ts) {
                if let Exec::Sim(sim) = &mut self.exec {
                    for (sym, bar) in group {
                        sim.on_bar_open(&mut self.eng, sym, bar);
                    }
                }
                for (sym, bar) in group {
                    self.eng.ingest(sym, bar);
                }
                for (sym, bar) in group {
                    let cmds = self.eng.decide(sym, bar);
                    self.execute(Some(bar), cmds).await;
                }
            }
            // ---- live housekeeping
            if matches!(self.exec, Exec::Live(_)) {
                self.reconcile().await;
                let cmds = self.eng.on_time(Utc::now());
                self.execute(None, cmds).await;
            }
            self.publish().await;
            tokio::select! {
                _ = tokio::time::sleep(sleep) => {}
                _ = &mut shutdown => { tracing::info!("Ctrl-C received"); break; }
            }
        }
        if self.flatten_on_exit && !self.killed {
            let cmds = self.eng.flatten_all("shutdown (flatten_on_exit)", Utc::now());
            self.execute(None, cmds).await;
        }
        self.publish().await;
        std::fs::write(self.state_dir.join("last_snapshot.json"), serde_json::to_vec_pretty(&*self.shared.snapshot.read().await)?)?;
        tracing::info!("stopped; journal in {}", self.state_dir.display());
        Ok(())
    }
}
