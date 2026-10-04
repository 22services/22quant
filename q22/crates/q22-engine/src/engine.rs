//! The engine: one decision loop for backtests, paper trading and live accounts.

use std::collections::VecDeque;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Serialize;

use q22_core::instrument::MarketKind;
use q22_core::session::trading_day;
use q22_core::{Bar, InstrumentSpec, Side};

use crate::allocator::{Allocator, Decision};
use crate::config::{EngineConfig, Mode};
use crate::guard::{size_position, ComplianceGuard, GuardAction};
use crate::market::MarketState;
use crate::prop::{AccountStatus, PropTracker};
use crate::regime::Regime;
use crate::strategies;
use crate::strategy::{EntrySignal, Manage, OpenPosition, PeerRef, Strategy, StrategyCtx};

/// Orders the engine wants executed. Prices are absolute.
#[derive(Clone, Debug, Serialize)]
pub enum Command {
    Enter { trade_id: u64, symbol: String, side: Side, qty: f64, stop: f64, target: Option<f64>, strategy: String, reason: String },
    ModifyStop { trade_id: u64, symbol: String, stop: f64 },
    /// `immediate`: fill now at the current price (forced flatten); otherwise next bar's open.
    Exit { trade_id: u64, symbol: String, reason: String, immediate: bool },
}

impl Command {
    pub fn symbol(&self) -> &str {
        match self {
            Command::Enter { symbol, .. } | Command::ModifyStop { symbol, .. } | Command::Exit { symbol, .. } => symbol,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub enum FillKind {
    Entry,
    Stop,
    Target,
    Exit,
}

#[derive(Clone, Debug, Serialize)]
pub struct Fill {
    pub trade_id: u64,
    pub symbol: String,
    pub kind: FillKind,
    pub qty: f64,
    pub price: f64,
    pub ts: DateTime<Utc>,
    pub note: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClosedTrade {
    pub trade_id: u64,
    pub symbol: String,
    pub strategy: String,
    pub regime: Regime,
    pub side: Side,
    pub qty: f64,
    pub entry_time: DateTime<Utc>,
    pub entry_price: f64,
    pub exit_time: DateTime<Utc>,
    pub exit_price: f64,
    pub exit_reason: String,
    pub pnl_gross: f64,
    pub costs: f64,
    pub pnl_net: f64,
    pub risk_usd: f64,
    pub r_multiple: f64,
    pub mfe_r: f64,
    pub mae_r: f64,
    pub bars_held: usize,
    pub day: NaiveDate,
}

#[derive(Clone, Debug, Serialize)]
pub struct EngineEvent {
    pub ts: DateTime<Utc>,
    pub level: &'static str,
    pub symbol: String,
    pub msg: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PendingApproval {
    pub id: u64,
    pub created: DateTime<Utc>,
    pub expires: DateTime<Utc>,
    pub command: Command,
    pub entry_ref: f64,
    pub risk_usd: f64,
    pub explanation: String,
}

struct PendingEntry {
    trade_id: u64,
    side: Side,
    qty: f64,
    stop: f64,
    target: Option<f64>,
    strategy: String,
    regime: Regime,
    created: DateTime<Utc>,
    day: NaiveDate,
}

pub struct InstrumentRt {
    pub spec: InstrumentSpec,
    pub tf_minutes: i64,
    pub slippage_ticks: f64,
    pub broker_symbol: String,
    pub market: MarketState,
    pub strategies: Vec<Box<dyn Strategy>>,
    pub position: Option<OpenPosition>,
    pending: Option<PendingEntry>,
    entry_costs: f64,
    pub last_price: f64,
    pub last_bar_end: Option<DateTime<Utc>>,
    exit_requested: bool,
}

impl InstrumentRt {
    pub fn open_pnl(&self) -> f64 {
        self.position.as_ref().map_or(0.0, |p| p.unrealized(&self.spec, self.last_price))
    }
}

pub struct Engine {
    pub cfg: EngineConfig,
    pub acct: PropTracker,
    pub guard: ComplianceGuard,
    pub allocator: Allocator,
    pub instruments: Vec<InstrumentRt>,
    pub events: VecDeque<EngineEvent>,
    pub trades: Vec<ClosedTrade>,
    pub daily_equity: Vec<(NaiveDate, f64)>,
    pub intraday_equity: VecDeque<(DateTime<Utc>, f64)>,
    pub approvals: Vec<PendingApproval>,
    pub paused: bool,
    pub live: bool,
    pub current_day: Option<NaiveDate>,
    pub regime_days: Vec<(NaiveDate, String, Regime)>,
    next_id: u64,
    pub max_events: usize,
    /// Why entries did not happen, by category (shown on the dashboard and in reports).
    pub block_counts: std::collections::BTreeMap<String, usize>,
}

fn ev(ts: DateTime<Utc>, level: &'static str, symbol: &str, msg: impl Into<String>) -> EngineEvent {
    EngineEvent { ts, level, symbol: symbol.to_string(), msg: msg.into() }
}

impl Engine {
    pub fn new(cfg: EngineConfig, live: bool) -> Result<Self> {
        let rules = cfg.account.resolve_rules()?;
        let acct = match cfg.account.resume_balance {
            Some(b) => PropTracker::resume(rules.clone(), b, cfg.account.resume_threshold, cfg.account.resume_high_eod_balance),
            None => PropTracker::new(rules.clone()),
        };
        if live && cfg.mode == Mode::Auto && rules.automation != crate::prop::Automation::Full {
            return Err(anyhow!(
                "{} does not allow fully automated trading on funded accounts ({:?}). Use mode = \"assist\" or \"signals\".",
                rules.firm,
                rules.automation
            ));
        }
        let guard = ComplianceGuard::new(cfg.guard.clone(), rules.flat_by_ct.as_deref());
        let mut instruments = vec![];
        for ic in &cfg.instruments {
            let spec = ic.spec()?;
            let mut strats: Vec<Box<dyn Strategy>> = vec![];
            for sc in cfg.strategies.iter().filter(|s| s.enabled) {
                if sc.instruments.is_empty() || sc.instruments.iter().any(|x| x.eq_ignore_ascii_case(&ic.symbol)) {
                    let s = strategies::build(sc)?;
                    if s.holds_overnight() && !rules.overnight_allowed {
                        // keep it out entirely: the guard would block every entry anyway
                        continue;
                    }
                    strats.push(s);
                }
            }
            instruments.push(InstrumentRt {
                market: MarketState::new(spec.kind, ic.timeframe_min, cfg.regime.clone()),
                broker_symbol: ic.broker_symbol.clone().unwrap_or_else(|| ic.symbol.clone()),
                spec,
                tf_minutes: ic.timeframe_min,
                slippage_ticks: ic.slippage_ticks,
                strategies: strats,
                position: None,
                pending: None,
                entry_costs: 0.0,
                last_price: f64::NAN,
                last_bar_end: None,
                exit_requested: false,
            });
        }
        if instruments.is_empty() {
            return Err(anyhow!("no instruments configured"));
        }
        let allocator = Allocator::new(cfg.allocator.clone());
        Ok(Self {
            cfg,
            acct,
            guard,
            allocator,
            instruments,
            events: VecDeque::new(),
            trades: vec![],
            daily_equity: vec![],
            intraday_equity: VecDeque::new(),
            approvals: vec![],
            paused: false,
            live,
            current_day: None,
            regime_days: vec![],
            next_id: 1,
            max_events: 2_000,
            block_counts: Default::default(),
        })
    }

    pub fn instrument_index(&self, symbol: &str) -> Option<usize> {
        self.instruments.iter().position(|r| r.spec.symbol.eq_ignore_ascii_case(symbol) || r.broker_symbol.eq_ignore_ascii_case(symbol))
    }

    fn push_events(&mut self, evs: Vec<EngineEvent>) {
        for e in evs {
            self.events.push_back(e);
        }
        while self.events.len() > self.max_events {
            self.events.pop_front();
        }
    }

    pub fn log(&mut self, ts: DateTime<Utc>, level: &'static str, symbol: &str, msg: impl Into<String>) {
        self.push_events(vec![ev(ts, level, symbol, msg)]);
    }

    fn open_pnl_total(&self) -> f64 {
        self.instruments.iter().map(|r| r.open_pnl()).sum()
    }

    fn has_any_position(&self) -> bool {
        self.instruments.iter().any(|r| r.position.is_some() || r.pending.is_some())
    }

    fn roll_day(&mut self, day: NaiveDate, ts: DateTime<Utc>) {
        if self.current_day == Some(day) {
            return;
        }
        if let Some(prev) = self.current_day {
            self.daily_equity.push((prev, self.acct.balance + self.open_pnl_total()));
        }
        self.current_day = Some(day);
        self.acct.begin_day(day);
        self.guard.new_day();
        // expire stale approvals from previous day
        self.approvals.clear();
        self.log(ts, "info", "", format!("new trading day {day} — balance {:.2}, threshold {:.2}, buffer {:.2}", self.acct.balance, self.acct.threshold, self.acct.buffer()));
    }

    /// Feed one **completed** bar. Returns the commands to execute.
    ///
    /// With several instruments, prefer [`Engine::on_bars`] with every bar of the same
    /// timestamp, so cross-asset strategies see their peers at that same timestamp.
    pub fn on_bar(&mut self, symbol: &str, bar: &Bar) -> Vec<Command> {
        self.ingest(symbol, bar);
        self.decide(symbol, bar)
    }

    /// Feed all bars that closed at the same time: every market is updated first, then each
    /// instrument decides (in the given order).
    pub fn on_bars(&mut self, bars: &[(&str, Bar)]) -> Vec<Command> {
        for (s, b) in bars {
            self.ingest(s, b);
        }
        let mut cmds = vec![];
        for (s, b) in bars {
            cmds.extend(self.decide(s, b));
        }
        cmds
    }

    /// Phase 1: market state, session hooks, position excursion bookkeeping.
    pub fn ingest(&mut self, symbol: &str, bar: &Bar) {
        let Some(i) = self.instrument_index(symbol) else { return };
        let mut evs = vec![];
        let (kind, tf) = (self.instruments[i].spec.kind, self.instruments[i].tf_minutes);
        let bar_end = bar.ts + Duration::minutes(tf);
        let day = trading_day(kind, bar.ts);
        self.roll_day(day, bar.ts);
        {
            let InstrumentRt { market, strategies, spec, position, last_price, last_bar_end, .. } = &mut self.instruments[i];
            *last_price = bar.close;
            *last_bar_end = Some(bar_end);
            market.update(bar);
            if market.new_session {
                for s in strategies.iter_mut() {
                    s.on_session_start(market);
                }
                let reg = market.regime;
                self.regime_days.push((day, spec.symbol.clone(), reg));
                evs.push(ev(bar.ts, "regime", &spec.symbol, format!("session regime: {} (ADX {:.1}, ER {:.2}, vol pct {:.0}%, gap {:+.2} ATR)", reg.label(), market.features.adx_d.unwrap_or(f64::NAN), market.features.er_d.unwrap_or(f64::NAN), market.features.vol_percentile.unwrap_or(f64::NAN) * 100.0, market.features.gap_atr.unwrap_or(0.0))));
            }
            if let Some(p) = position.as_mut() {
                p.bars_held += 1;
                if p.side == Side::Long {
                    p.mfe_price = p.mfe_price.max(bar.high);
                    p.mae_price = p.mae_price.min(bar.low);
                } else {
                    p.mfe_price = p.mfe_price.min(bar.low);
                    p.mae_price = p.mae_price.max(bar.high);
                }
            }
        }
        self.push_events(evs);
    }

    /// Phase 2: account mark, guard actions, position management and new entries.
    pub fn decide(&mut self, symbol: &str, bar: &Bar) -> Vec<Command> {
        let Some(i) = self.instrument_index(symbol) else { return vec![] };
        let mut cmds = vec![];
        let mut evs = vec![];
        let (kind, tf) = (self.instruments[i].spec.kind, self.instruments[i].tf_minutes);
        let bar_end = bar.ts + Duration::minutes(tf);
        let day = trading_day(kind, bar.ts);

        // 2. account mark & firm rules
        let open = self.open_pnl_total();
        if let Some(reason) = self.acct.mark(open) {
            evs.push(ev(bar.ts, "fail", symbol, format!("ACCOUNT FAILED — {reason}")));
        }
        self.intraday_equity.push_back((bar_end, self.acct.equity));
        if self.intraday_equity.len() > 5_000 {
            self.intraday_equity.pop_front();
        }

        // 3. guard continuous actions
        let has_pos = self.has_any_position();
        let actions = self.guard.continuous(bar_end, kind, &self.acct, has_pos);
        let mut flatten_reason = None;
        for a in actions {
            match a {
                GuardAction::FlattenAll(r) => flatten_reason = Some(r),
                GuardAction::HaltDay(r) => evs.push(ev(bar.ts, "halt", symbol, format!("day halted: {r}"))),
                GuardAction::HaltAccount(r) => {
                    self.guard.account_halted = Some(r.clone());
                    evs.push(ev(bar.ts, "halt", symbol, format!("account halted: {r}")));
                }
            }
        }
        if let Some(r) = flatten_reason {
            cmds.extend(self.flatten_all_cmds(&r, bar.ts, &mut evs));
            self.push_events(evs);
            return cmds;
        }

        // 4. manage the open position
        let in_session = self.instruments[i].market.in_session(bar.ts);
        let now = bar_end;
        let regime = self.instruments[i].market.regime;
        if self.instruments[i].position.is_some() && !self.instruments[i].exit_requested {
            let (before, rest) = self.instruments.split_at_mut(i);
            let (cur, after) = rest.split_first_mut().expect("instrument index in range");
            let peers: Vec<PeerRef> = before.iter().chain(after.iter()).map(|r| PeerRef { symbol: &r.spec.symbol, m: &r.market, last_close: r.last_price }).collect();
            let InstrumentRt { market, strategies, spec, position, exit_requested, .. } = cur;
            let pos = position.clone().unwrap();
            let offset = market.offset_end(bar);
            if let Some(s) = strategies.iter_mut().find(|s| s.id() == pos.strategy) {
                let ctx = StrategyCtx { symbol: &spec.symbol, spec, bar, bar_end, m: market, regime, offset, peers: &peers };
                match s.manage(&ctx, &pos) {
                    Manage::Hold => {}
                    Manage::MoveStop(px) => {
                        let px = spec.round_price(px);
                        let valid = match pos.side {
                            Side::Long => px > pos.stop && px < bar.close,
                            Side::Short => px < pos.stop && px > bar.close,
                        };
                        if valid && self.guard.order_budget_ok(now) {
                            self.guard.note_order(now);
                            position.as_mut().unwrap().stop = px;
                            cmds.push(Command::ModifyStop { trade_id: pos.trade_id, symbol: spec.symbol.clone(), stop: px });
                            evs.push(ev(bar.ts, "manage", &spec.symbol, format!("{} trail stop → {px:.2}", pos.strategy)));
                        }
                    }
                    Manage::Exit(reason) => {
                        if self.guard.may_exit_discretionary(pos.entry_time, now) {
                            *exit_requested = true;
                            cmds.push(Command::Exit { trade_id: pos.trade_id, symbol: spec.symbol.clone(), reason: reason.clone(), immediate: false });
                            evs.push(ev(bar.ts, "exit", &spec.symbol, format!("{} exit: {reason}", pos.strategy)));
                        }
                    }
                }
            }
        }

        // 5. look for a new entry
        let flat = self.instruments[i].position.is_none() && self.instruments[i].pending.is_none();
        if flat && in_session && !self.paused && self.acct.is_active() {
            if let Some(c) = self.try_entry(i, bar, bar_end, regime, day, &mut evs) {
                cmds.push(c);
            }
        }
        self.push_events(evs);
        cmds
    }

    fn try_entry(&mut self, i: usize, bar: &Bar, bar_end: DateTime<Utc>, regime: Regime, day: NaiveDate, evs: &mut Vec<EngineEvent>) -> Option<Command> {
        let (before, rest) = self.instruments.split_at_mut(i);
        let (cur, after) = rest.split_first_mut().expect("instrument index in range");
        let peers: Vec<PeerRef> = before.iter().chain(after.iter()).map(|r| PeerRef { symbol: &r.spec.symbol, m: &r.market, last_close: r.last_price }).collect();
        let InstrumentRt { market, strategies, spec, pending, .. } = cur;
        let offset = market.offset_end(bar);
        let mut signals: Vec<(String, f64, EntrySignal, bool)> = vec![];
        for s in strategies.iter_mut() {
            let ctx = StrategyCtx { symbol: &spec.symbol, spec, bar, bar_end, m: market, regime, offset, peers: &peers };
            if let Some(sig) = s.entry(&ctx) {
                let valid = match sig.side {
                    Side::Long => sig.stop < bar.close,
                    Side::Short => sig.stop > bar.close,
                } && sig.target.is_none_or(|t| (t - bar.close) * sig.side.sign() > 0.0);
                if valid {
                    signals.push((s.id().to_string(), s.affinity(regime), sig, s.holds_overnight()));
                } else {
                    evs.push(ev(bar.ts, "block", &spec.symbol, format!("{} signal rejected: stop/target on wrong side", s.id())));
                    *self.block_counts.entry(format!("{}: invalid stop/target", s.id())).or_default() += 1;
                }
            }
        }
        if signals.is_empty() {
            return None;
        }
        let overnight: std::collections::HashMap<String, bool> = signals.iter().map(|(id, _, _, o)| (id.clone(), *o)).collect();
        let decision = self.allocator.decide(regime, signals.into_iter().map(|(a, b, c, _)| (a, b, c)).collect());
        let (cand, size_mult) = match decision {
            Decision::None { reason, candidates } => {
                if !candidates.is_empty() {
                    evs.push(ev(bar.ts, "skip", &spec.symbol, format!("allocator: {reason} [{}]", candidates.iter().map(|c| format!("{} {:.2}", c.strategy, c.score)).collect::<Vec<_>>().join(", "))));
                    let cat = if reason.starts_with("conflict") { "conflict".to_string() } else { format!("score below min ({})", candidates[0].strategy) };
                    *self.block_counts.entry(format!("allocator: {cat}")).or_default() += 1;
                }
                return None;
            }
            Decision::Take { candidate, size_mult } => (candidate, size_mult),
        };
        let sig = &cand.signal;
        let holds = overnight.get(&cand.strategy).copied().unwrap_or(false);
        if let Err(why) = self.guard.check_entry(bar_end, spec.kind, spec, sig.side, holds, &self.acct, self.live) {
            evs.push(ev(bar.ts, "block", &spec.symbol, format!("{} {} blocked by guard: {why}", cand.strategy, sig.side)));
            let short: String = why.split(|c: char| c == ':' || c.is_ascii_digit()).next().unwrap_or("").trim().to_string();
            *self.block_counts.entry(format!("guard: {short} ({})", cand.strategy)).or_default() += 1;
            return None;
        }
        let dl_room = (self.guard.daily_loss_stop(&self.acct) + self.acct.day_pnl().min(0.0)).max(0.0);
        let stop = spec.round_price(sig.stop);
        let (qty, why) = size_position(spec, bar.close, stop, &self.acct, &self.guard.cfg, size_mult, sig.max_notional_frac, dl_room);
        if qty <= 0.0 {
            evs.push(ev(bar.ts, "block", &spec.symbol, format!("{} {} not tradable at this size: {why}", cand.strategy, sig.side)));
            *self.block_counts.entry(format!("size: stop too wide for budget ({})", cand.strategy)).or_default() += 1;
            return None;
        }
        let trade_id = self.next_id;
        self.next_id += 1;
        let risk_usd = qty * (bar.close - stop).abs() * spec.point_value();
        let explanation = format!(
            "{} {} {qty} {} @≈{:.2} stop {stop:.2}{} | regime {} | score {:.2} (aff {:.2} × health {:.2} × conf {:.2}) | {} | {}",
            cand.strategy,
            sig.side,
            spec.symbol,
            bar.close,
            sig.target.map(|t| format!(" target {t:.2}")).unwrap_or_default(),
            regime.label(),
            cand.score,
            cand.affinity,
            cand.health,
            sig.confidence,
            sig.reason,
            why
        );
        let cmd = Command::Enter { trade_id, symbol: spec.symbol.clone(), side: sig.side, qty, stop, target: sig.target.map(|t| spec.round_price(t)), strategy: cand.strategy.clone(), reason: sig.reason.clone() };
        match self.cfg.mode {
            Mode::Signals => {
                evs.push(ev(bar.ts, "signal", &spec.symbol, format!("SIGNAL (not sent) {explanation}")));
                None
            }
            Mode::Assist if self.live => {
                evs.push(ev(bar.ts, "approve", &spec.symbol, format!("AWAITING APPROVAL #{trade_id}: {explanation}")));
                self.approvals.push(PendingApproval { id: trade_id, created: bar_end, expires: bar_end + Duration::minutes(self.cfg.approval_timeout_min), command: cmd, entry_ref: bar.close, risk_usd, explanation });
                None
            }
            _ => {
                *pending = Some(PendingEntry { trade_id, side: sig.side, qty, stop, target: sig.target.map(|t| spec.round_price(t)), strategy: cand.strategy.clone(), regime, created: bar_end, day });
                self.guard.note_entry(bar_end, &spec.group, sig.side, &spec.symbol);
                self.guard.note_order(bar_end);
                evs.push(ev(bar.ts, "entry", &spec.symbol, format!("ENTRY #{trade_id} {explanation} | risk ${risk_usd:.0}")));
                Some(cmd)
            }
        }
    }

    /// Approve an assist-mode intent (from the dashboard).
    pub fn approve(&mut self, id: u64, now: DateTime<Utc>) -> Result<Command> {
        let k = self.approvals.iter().position(|a| a.id == id).ok_or_else(|| anyhow!("no pending approval #{id}"))?;
        let a = self.approvals.remove(k);
        if now > a.expires {
            return Err(anyhow!("approval #{id} expired"));
        }
        let Command::Enter { trade_id, ref symbol, side, qty, stop, target, ref strategy, .. } = a.command else { return Err(anyhow!("not an entry")) };
        let i = self.instrument_index(symbol).ok_or_else(|| anyhow!("unknown symbol"))?;
        if self.instruments[i].position.is_some() || self.instruments[i].pending.is_some() {
            return Err(anyhow!("{symbol} is no longer flat"));
        }
        let spec = self.instruments[i].spec.clone();
        self.guard.check_entry(now, spec.kind, &spec, side, false, &self.acct, self.live).map_err(|e| anyhow!("guard: {e}"))?;
        let regime = self.instruments[i].market.regime;
        let day = self.current_day.unwrap_or_else(|| now.date_naive());
        self.instruments[i].pending = Some(PendingEntry { trade_id, side, qty, stop, target, strategy: strategy.clone(), regime, created: now, day });
        self.guard.note_entry(now, &spec.group, side, &spec.symbol);
        self.guard.note_order(now);
        self.log(now, "entry", &spec.symbol, format!("APPROVED #{trade_id} by operator"));
        Ok(a.command)
    }

    pub fn reject(&mut self, id: u64, now: DateTime<Utc>) {
        self.approvals.retain(|a| a.id != id);
        self.log(now, "info", "", format!("operator rejected #{id}"));
    }

    fn flatten_all_cmds(&mut self, reason: &str, ts: DateTime<Utc>, evs: &mut Vec<EngineEvent>) -> Vec<Command> {
        let mut out = vec![];
        for rt in self.instruments.iter_mut() {
            if let Some(p) = &rt.position {
                if !rt.exit_requested {
                    rt.exit_requested = true;
                    out.push(Command::Exit { trade_id: p.trade_id, symbol: rt.spec.symbol.clone(), reason: reason.to_string(), immediate: true });
                    evs.push(ev(ts, "flatten", &rt.spec.symbol, format!("FLATTEN #{}: {reason}", p.trade_id)));
                }
            }
            if let Some(pe) = rt.pending.take() {
                evs.push(ev(ts, "flatten", &rt.spec.symbol, format!("cancel pending entry #{}: {reason}", pe.trade_id)));
                self.guard.note_flat(&rt.spec.group);
            }
        }
        out
    }

    /// Operator / kill-switch flatten (also used on shutdown).
    pub fn flatten_all(&mut self, reason: &str, now: DateTime<Utc>) -> Vec<Command> {
        let mut evs = vec![];
        let c = self.flatten_all_cmds(reason, now, &mut evs);
        self.push_events(evs);
        c
    }

    /// Cancel an entry that could not be filled (e.g. rejected by the broker).
    pub fn cancel_pending(&mut self, symbol: &str, why: &str, now: DateTime<Utc>) {
        if let Some(i) = self.instrument_index(symbol) {
            if let Some(pe) = self.instruments[i].pending.take() {
                let g = self.instruments[i].spec.group.clone();
                self.guard.note_flat(&g);
                self.log(now, "warn", symbol, format!("entry #{} cancelled: {why}", pe.trade_id));
            }
        }
    }

    /// Timer for live mode between bars: forced flatten time, approval expiry, guard checks.
    pub fn on_time(&mut self, now: DateTime<Utc>) -> Vec<Command> {
        let before = self.approvals.len();
        self.approvals.retain(|a| a.expires >= now);
        if self.approvals.len() < before {
            self.log(now, "info", "", "assist intent expired without approval");
        }
        let kind = self.instruments[0].spec.kind;
        let open = self.open_pnl_total();
        if let Some(reason) = self.acct.mark(open) {
            self.log(now, "fail", "", format!("ACCOUNT FAILED — {reason}"));
        }
        let has_pos = self.has_any_position();
        let mut evs = vec![];
        let mut out = vec![];
        for a in self.guard.continuous(now, kind, &self.acct, has_pos) {
            if let GuardAction::FlattenAll(r) = a {
                out.extend(self.flatten_all_cmds(&r, now, &mut evs));
            }
        }
        self.push_events(evs);
        out
    }

    /// Broker/sim reports a fill.
    pub fn on_fill(&mut self, f: &Fill) {
        let Some(i) = self.instrument_index(&f.symbol) else { return };
        let mut evs = vec![];
        match f.kind {
            FillKind::Entry => {
                let rt = &mut self.instruments[i];
                let Some(pe) = rt.pending.take() else {
                    evs.push(ev(f.ts, "warn", &f.symbol, format!("unexpected entry fill #{}", f.trade_id)));
                    self.push_events(evs);
                    return;
                };
                let costs = rt.spec.cost_one_side(f.qty, f.price);
                rt.entry_costs = costs;
                rt.exit_requested = false;
                rt.position = Some(OpenPosition {
                    trade_id: pe.trade_id,
                    strategy: pe.strategy.clone(),
                    side: pe.side,
                    qty: f.qty,
                    entry_price: f.price,
                    entry_time: f.ts,
                    stop: pe.stop,
                    initial_stop: pe.stop,
                    target: pe.target,
                    regime_at_entry: pe.regime,
                    bars_held: 0,
                    mfe_price: f.price,
                    mae_price: f.price,
                });
                let _ = (pe.created, pe.day);
                if (f.qty - pe.qty).abs() > 1e-9 {
                    evs.push(ev(f.ts, "warn", &f.symbol, format!("partial fill #{}: {} of {}", pe.trade_id, f.qty, pe.qty)));
                }
                self.acct.on_realized(-costs);
                evs.push(ev(f.ts, "fill", &f.symbol, format!("filled #{} {} {} @ {:.2} (stop {:.2}{})", pe.trade_id, pe.side, f.qty, f.price, pe.stop, pe.target.map(|t| format!(", target {t:.2}")).unwrap_or_default())));
            }
            _ => {
                let day = self.current_day.unwrap_or_else(|| f.ts.date_naive());
                let rt = &mut self.instruments[i];
                let Some(p) = rt.position.take() else { return };
                rt.exit_requested = false;
                let spec = rt.spec.clone();
                let gross = spec.pnl(p.qty * p.side.sign(), p.entry_price, f.price);
                let exit_costs = spec.cost_one_side(p.qty, f.price);
                let costs = rt.entry_costs + exit_costs;
                rt.entry_costs = 0.0;
                let risk_usd = (p.entry_price - p.initial_stop).abs() * p.qty * spec.point_value();
                let net = gross - costs;
                let r = if risk_usd > 0.0 { net / risk_usd } else { 0.0 };
                let pts = |x: f64| (x - p.entry_price) * p.side.sign() * p.qty * spec.point_value() / risk_usd.max(1e-9);
                let reason = match f.kind {
                    FillKind::Stop => if (p.stop - p.initial_stop).abs() > 1e-9 { "trailing stop".to_string() } else { "stop loss".to_string() },
                    FillKind::Target => "target".to_string(),
                    _ => f.note.clone(),
                };
                self.acct.on_realized(gross - exit_costs);
                self.allocator.record(&p.strategy, r);
                self.guard.record_trade_closed(net);
                self.guard.note_flat(&spec.group);
                if let Some(s) = rt.strategies.iter_mut().find(|s| s.id() == p.strategy) {
                    s.on_trade_closed(r);
                }
                evs.push(ev(f.ts, if net >= 0.0 { "win" } else { "loss" }, &f.symbol, format!("closed #{} {} {} {:.2}→{:.2} ({reason}) net ${net:.2} ({r:+.2}R)", p.trade_id, p.strategy, p.side, p.entry_price, f.price)));
                self.trades.push(ClosedTrade {
                    trade_id: p.trade_id,
                    symbol: f.symbol.clone(),
                    strategy: p.strategy.clone(),
                    regime: p.regime_at_entry,
                    side: p.side,
                    qty: p.qty,
                    entry_time: p.entry_time,
                    entry_price: p.entry_price,
                    exit_time: f.ts,
                    exit_price: f.price,
                    exit_reason: reason,
                    pnl_gross: gross,
                    costs,
                    pnl_net: net,
                    risk_usd,
                    r_multiple: r,
                    mfe_r: pts(p.mfe_price),
                    mae_r: pts(p.mae_price),
                    bars_held: p.bars_held,
                    day,
                });
                let open = self.open_pnl_total();
                if let Some(reason) = self.acct.mark(open) {
                    evs.push(ev(f.ts, "fail", &f.symbol, format!("ACCOUNT FAILED — {reason}")));
                }
            }
        }
        self.push_events(evs);
    }

    /// Live reconciliation: the broker reports the position is gone (stop/target hit server-side).
    pub fn on_external_flat(&mut self, symbol: &str, price: f64, ts: DateTime<Utc>, note: &str) {
        let Some(i) = self.instrument_index(symbol) else { return };
        let Some(p) = &self.instruments[i].position else { return };
        let kind = if p.target.is_some_and(|t| (price - t).abs() <= self.instruments[i].spec.tick_size * 4.0) {
            FillKind::Target
        } else if (price - p.stop).abs() <= self.instruments[i].spec.tick_size * 8.0 {
            FillKind::Stop
        } else {
            FillKind::Exit
        };
        let f = Fill { trade_id: p.trade_id, symbol: symbol.to_string(), kind, qty: p.qty, price, ts, note: note.to_string() };
        self.on_fill(&f);
    }

    /// Start a fresh evaluation (prop replay in backtests).
    pub fn reset_account(&mut self) {
        let rules = self.acct.rules.clone();
        self.acct = PropTracker::new(rules);
        self.guard.account_halted = None;
        self.guard.new_day();
        if let Some(d) = self.current_day {
            self.acct.begin_day(d);
        }
    }

    pub fn account_status(&self) -> &AccountStatus {
        &self.acct.status
    }

    pub fn is_crypto(&self) -> bool {
        self.instruments.iter().all(|r| r.spec.kind == MarketKind::Crypto)
    }

    pub fn finish(&mut self) {
        if let Some(d) = self.current_day {
            self.daily_equity.push((d, self.acct.balance + self.open_pnl_total()));
        }
        self.acct.close_books();
    }
}
