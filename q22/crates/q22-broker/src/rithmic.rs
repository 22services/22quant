//! Rithmic R | Protocol API (WebSocket + protobuf) — the execution and data path of **Lucid
//! Trading** and most other Rithmic-cleared prop firms. Built on the `rithmic-rs` crate.
//!
//! Before it can reach a production or paper system, an application must pass Rithmic's
//! **conformance test**: Rithmic then assigns the app-name prefix to put in
//! `Q22_RITHMIC_APP_NAME`, and the WebSocket URL / system name to use. Lucid's accounts sit on
//! a Lucid-specific Rithmic system: use the system name shown in R|Trader Pro's login screen.
//!
//! Credentials come from the environment only:
//! `Q22_RITHMIC_URL`, `Q22_RITHMIC_ALT_URL` (optional), `Q22_RITHMIC_SYSTEM`, `Q22_RITHMIC_USER`,
//! `Q22_RITHMIC_PASSWORD`, `Q22_RITHMIC_APP_NAME`, `Q22_RITHMIC_APP_VERSION` (default 1.0),
//! `Q22_RITHMIC_ACCOUNT` (optional: default = first account of the login).
//!
//! Orders: every entry is a **market bracket** (stop leg, optional target leg) so the
//! protection lives on Rithmic's servers from the first fill, with the CME automated-order flag
//! (`manual_or_auto = AUTO`). After the fill the stop leg is re-aimed at the exact price the
//! engine asked for; later trailing moves modify that stop order directly.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use tokio::sync::OnceCell;

use rithmic_rs::rti::messages::RithmicMessage;
use rithmic_rs::{
    ConnectStrategy, OrderSide, OrderStatus, OrderType, RithmicAccount, RithmicBracketLevelAdjustment, RithmicBracketOrder, RithmicCancelOrder, RithmicConfig, RithmicConfigBuilder, RithmicEnv,
    RithmicExitPosition, RithmicHistoryPlant, RithmicHistoryPlantHandle, RithmicModifyOrder, RithmicOrderPlant, RithmicOrderPlantHandle, RithmicPnlPlant, RithmicPnlPlantHandle, RithmicResponse,
    RithmicTickerPlant, RithmicTickerPlantHandle, TimeBarType, TimeInForce,
};

use q22_core::{Bar, InstrumentSpec, Side};

use crate::{AccountInfo, Broker, BrokerPosition, EntryAck};

/// Rithmic price-type codes as they appear in order notifications.
const PT_STOP_MARKET: i32 = 4;

#[derive(Clone, Debug)]
pub struct RithmicSettings {
    pub url: String,
    pub alt_url: String,
    pub system_name: String,
    pub user: String,
    pub password: String,
    pub app_name: String,
    pub app_version: String,
    pub account: Option<String>,
    pub exchange: String,
}

impl RithmicSettings {
    pub fn from_env() -> Result<Self> {
        let need = |k: &str| std::env::var(k).map_err(|_| anyhow!("{k} is not set (see the Lucid / Rithmic section of the README)"));
        let url = need("Q22_RITHMIC_URL")?;
        Ok(Self {
            alt_url: std::env::var("Q22_RITHMIC_ALT_URL").unwrap_or_else(|_| url.clone()),
            url,
            system_name: need("Q22_RITHMIC_SYSTEM")?,
            user: need("Q22_RITHMIC_USER")?,
            password: need("Q22_RITHMIC_PASSWORD")?,
            app_name: need("Q22_RITHMIC_APP_NAME")?,
            app_version: std::env::var("Q22_RITHMIC_APP_VERSION").unwrap_or_else(|_| "1.0".into()),
            account: std::env::var("Q22_RITHMIC_ACCOUNT").ok().filter(|s| !s.is_empty()),
            exchange: std::env::var("Q22_RITHMIC_EXCHANGE").unwrap_or_else(|_| "CME".into()),
        })
    }

    fn config(&self) -> Result<RithmicConfig> {
        RithmicConfigBuilder::new(RithmicEnv::Live)
            .url(self.url.clone())
            .beta_url(self.alt_url.clone())
            .system_name(self.system_name.clone())
            .user(self.user.clone())
            .password(self.password.clone())
            .app_name(self.app_name.clone())
            .app_version(self.app_version.clone())
            .retry_timeout(StdDuration::from_secs(30))
            .build()
            .map_err(|e| anyhow!("rithmic config: {e}"))
    }
}

#[derive(Clone, Debug, Default)]
struct OrderInfo {
    contract: String,
    status: OrderStatus,
    price_type: i32,
    qty: i32,
    trigger: Option<f64>,
    avg_fill: Option<f64>,
}

#[derive(Default)]
struct State {
    /// our symbol (MNQ) → front-month contract (MNQZ6)
    contracts: HashMap<String, String>,
    last_price: HashMap<String, f64>,
    /// contract → (signed net qty, average open price)
    positions: HashMap<String, (i32, f64)>,
    balance: Option<f64>,
    orders: HashMap<String, OrderInfo>,
    /// contract → (entry bracket basket id, entry fill price)
    entries: HashMap<String, (String, f64)>,
    last_fill: HashMap<String, f64>,
    connection_error: Option<String>,
}

struct Inner {
    order: RithmicOrderPlantHandle,
    history: RithmicHistoryPlantHandle,
    ticker: RithmicTickerPlantHandle,
    _pnl: RithmicPnlPlantHandle,
    account: RithmicAccount,
    // the plants own the actor tasks: keep them alive for the life of the adapter
    _plants: (RithmicOrderPlant, RithmicHistoryPlant, RithmicTickerPlant, RithmicPnlPlant),
}

pub struct Rithmic {
    s: RithmicSettings,
    inner: OnceCell<Inner>,
    state: Arc<Mutex<State>>,
}

fn ok(resp: &[RithmicResponse], what: &str) -> Result<()> {
    if let Some(e) = resp.iter().find_map(|r| r.error.as_ref()) {
        bail!("rithmic refused {what}: {e}");
    }
    Ok(())
}

impl Rithmic {
    pub fn new(s: RithmicSettings) -> Self {
        Self { s, inner: OnceCell::new(), state: Arc::new(Mutex::new(State::default())) }
    }

    pub fn from_env() -> Result<Self> {
        Ok(Self::new(RithmicSettings::from_env()?))
    }

    fn inner(&self) -> Result<&Inner> {
        self.inner.get().ok_or_else(|| anyhow!("rithmic: not connected"))
    }

    fn healthy(&self) -> Result<()> {
        if let Some(e) = &self.state.lock().unwrap().connection_error {
            bail!("rithmic connection lost: {e}");
        }
        Ok(())
    }

    fn tick(symbol: &str) -> f64 {
        InstrumentSpec::builtin(symbol).map(|s| s.tick_size).unwrap_or(0.25)
    }

    /// Our symbol → current front-month contract (cached; subscribes to its trades for prices).
    async fn contract(&self, symbol: &str) -> Result<String> {
        if let Some(c) = self.state.lock().unwrap().contracts.get(symbol).cloned() {
            return Ok(c);
        }
        let inner = self.inner()?;
        let r = inner.ticker.get_front_month_contract(symbol, &self.s.exchange, false).await.map_err(|e| anyhow!("front month of {symbol}: {e}"))?;
        if let Some(e) = &r.error {
            bail!("front month of {symbol}: {e}");
        }
        let c = match &r.message {
            RithmicMessage::ResponseFrontMonthContract(m) => m.trading_symbol.clone().or_else(|| m.symbol.clone()),
            _ => None,
        }
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("no front-month contract for {symbol} (maintenance window?)"))?;
        let sub = inner.ticker.subscribe(&c, &self.s.exchange).await;
        if let Ok(r) = &sub {
            if let Some(e) = &r.error {
                tracing::warn!("rithmic: market data for {c} refused: {e}");
            }
        }
        self.state.lock().unwrap().contracts.insert(symbol.to_string(), c.clone());
        tracing::info!("rithmic: {symbol} → {c}");
        Ok(c)
    }

    fn symbol_of(&self, contract: &str) -> String {
        let st = self.state.lock().unwrap();
        st.contracts.iter().find(|(_, c)| *c == contract).map(|(s, _)| s.clone()).unwrap_or_else(|| contract.to_string())
    }

    async fn reference_price(&self, symbol: &str, contract: &str) -> Result<f64> {
        if let Some(p) = self.state.lock().unwrap().last_price.get(contract).copied() {
            return Ok(p);
        }
        let bars = self.bars(symbol, 1, Utc::now() - chrono::Duration::minutes(30)).await?;
        bars.last().map(|b| b.close).ok_or_else(|| anyhow!("no recent price for {contract}"))
    }

    fn working_stop(&self, contract: &str) -> Option<(String, OrderInfo)> {
        let st = self.state.lock().unwrap();
        st.orders.iter().find(|(_, o)| o.contract == contract && o.price_type == PT_STOP_MARKET && o.status.is_active()).map(|(k, o)| (k.clone(), o.clone()))
    }

    fn spawn_listeners(&self, mut order_rx: RithmicOrderPlantHandle, mut pnl_rx: RithmicPnlPlantHandle, mut ticker_rx: RithmicTickerPlantHandle) {
        let st = self.state.clone();
        tokio::spawn(async move {
            loop {
                let Ok(u) = order_rx.subscription_receiver.recv().await else { break };
                let mut s = st.lock().unwrap();
                if let Some(e) = &u.error {
                    if e.is_connection_issue() {
                        s.connection_error = Some(e.to_string());
                    }
                }
                match &u.message {
                    RithmicMessage::RithmicOrderNotification(n) => {
                        let Some(b) = n.basket_id.clone() else { continue };
                        let o = s.orders.entry(b).or_default();
                        if let Some(c) = &n.symbol {
                            o.contract = c.clone();
                        }
                        if let Some(x) = &n.status {
                            o.status = OrderStatus::from_str(x).unwrap_or_default();
                        }
                        if let Some(x) = n.price_type {
                            o.price_type = x;
                        }
                        if let Some(x) = n.quantity {
                            o.qty = x;
                        }
                        if n.trigger_price.is_some() {
                            o.trigger = n.trigger_price;
                        }
                        if n.avg_fill_price.is_some() {
                            o.avg_fill = n.avg_fill_price;
                        }
                    }
                    RithmicMessage::ExchangeOrderNotification(n) => {
                        if n.notify_type == Some(5) {
                            if let (Some(c), Some(p)) = (&n.symbol, n.fill_price) {
                                s.last_fill.insert(c.clone(), p);
                            }
                            if let Some(b) = &n.basket_id {
                                if let Some(o) = s.orders.get_mut(b) {
                                    o.avg_fill = n.avg_fill_price.or(n.fill_price).or(o.avg_fill);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
        let st = self.state.clone();
        tokio::spawn(async move {
            loop {
                let Ok(u) = pnl_rx.subscription_receiver.recv().await else { break };
                let mut s = st.lock().unwrap();
                match &u.message {
                    RithmicMessage::InstrumentPnLPositionUpdate(p) => {
                        if let Some(c) = &p.symbol {
                            s.positions.insert(c.clone(), (p.net_quantity.unwrap_or(0), p.avg_open_fill_price.unwrap_or(0.0)));
                        }
                    }
                    RithmicMessage::AccountPnLPositionUpdate(a) => {
                        if let Some(b) = a.account_balance.as_deref().and_then(|x| x.parse::<f64>().ok()) {
                            s.balance = Some(b);
                        }
                    }
                    _ => {}
                }
            }
        });
        let st = self.state.clone();
        tokio::spawn(async move {
            loop {
                let Ok(u) = ticker_rx.subscription_receiver.recv().await else { break };
                if let RithmicMessage::LastTrade(t) = &u.message {
                    if let (Some(c), Some(p)) = (&t.symbol, t.trade_price) {
                        st.lock().unwrap().last_price.insert(c.clone(), p);
                    }
                }
            }
        });
    }
}

#[async_trait]
impl Broker for Rithmic {
    fn name(&self) -> &str {
        "rithmic"
    }

    async fn connect(&mut self) -> Result<AccountInfo> {
        let cfg = self.s.config()?;
        let strat = ConnectStrategy::AlternateWithRetry;
        let order_plant = RithmicOrderPlant::connect(&cfg, strat).await.context("rithmic order plant")?;
        let history_plant = RithmicHistoryPlant::connect(&cfg, strat).await.context("rithmic history plant")?;
        let ticker_plant = RithmicTickerPlant::connect(&cfg, strat).await.context("rithmic ticker plant")?;
        let pnl_plant = RithmicPnlPlant::connect(&cfg, strat).await.context("rithmic pnl plant")?;

        // login is user-scoped: a placeholder account is enough to log in and list accounts
        let probe = order_plant.get_handle(&RithmicAccount::new("", "", ""));
        probe.login().await.map_err(|e| anyhow!("rithmic login (order plant): {e} — check user/password/system name and that the app passed conformance"))?;
        let list = probe.get_account_list().await.map_err(|e| anyhow!("account list: {e}"))?;
        ok(&list, "account list")?;
        let accounts: Vec<RithmicAccount> = list
            .iter()
            .filter_map(|r| match &r.message {
                RithmicMessage::ResponseAccountList(a) => a.account_id.clone().filter(|x| !x.is_empty()).map(|id| RithmicAccount::new(a.fcm_id.clone().unwrap_or_default(), a.ib_id.clone().unwrap_or_default(), id)),
                _ => None,
            })
            .collect();
        let account = match &self.s.account {
            Some(want) => accounts.iter().find(|a| &a.account_id == want).cloned().ok_or_else(|| anyhow!("account {want} not in this login: {:?}", accounts.iter().map(|a| &a.account_id).collect::<Vec<_>>()))?,
            None => accounts.first().cloned().ok_or_else(|| anyhow!("this login has no trading account"))?,
        };

        let order = order_plant.get_handle(&account);
        let history = history_plant.get_handle();
        let ticker = ticker_plant.get_handle();
        let pnl = pnl_plant.get_handle(&account);
        history.login().await.map_err(|e| anyhow!("rithmic login (history plant): {e}"))?;
        ticker.login().await.map_err(|e| anyhow!("rithmic login (ticker plant): {e}"))?;
        pnl.login().await.map_err(|e| anyhow!("rithmic login (pnl plant): {e}"))?;

        // listeners first, then subscriptions, so no notification is missed
        self.spawn_listeners(order_plant.get_handle(&account), pnl_plant.get_handle(&account), ticker_plant.get_handle());
        for (what, r) in [("order updates", order.subscribe_order_updates().await), ("bracket updates", order.subscribe_bracket_updates().await)] {
            match r {
                Ok(r) if r.error.is_some() => tracing::warn!("rithmic: {what} refused: {:?}", r.error),
                Err(e) => bail!("rithmic {what}: {e}"),
                _ => {}
            }
        }
        pnl.subscribe_pnl_updates().await.map_err(|e| anyhow!("pnl updates: {e}"))?;
        pnl.get_pnl_position_snapshot().await.map_err(|e| anyhow!("pnl snapshot: {e}"))?;
        // the trade route for the exchange must exist or every order is refused
        order.trade_route_for(&self.s.exchange).await.map_err(|e| anyhow!("no trade route for {} on this account: {e}", self.s.exchange))?;

        let id = account.account_id.clone();
        self.inner.set(Inner { order, history, ticker, _pnl: pnl, account, _plants: (order_plant, history_plant, ticker_plant, pnl_plant) }).map_err(|_| anyhow!("rithmic: already connected"))?;
        tokio::time::sleep(StdDuration::from_millis(800)).await; // let the PnL snapshot arrive
        let balance = self.state.lock().unwrap().balance.unwrap_or(0.0);
        Ok(AccountInfo { id: id.clone(), name: format!("{id} ({})", self.s.system_name), balance, can_trade: true, simulated: None })
    }

    async fn account(&self) -> Result<AccountInfo> {
        self.healthy()?;
        let inner = self.inner()?;
        let balance = self.state.lock().unwrap().balance.unwrap_or(0.0);
        let id = inner.account.account_id.clone();
        Ok(AccountInfo { id: id.clone(), name: id, balance, can_trade: true, simulated: None })
    }

    async fn bars(&self, symbol: &str, tf_minutes: i64, since: DateTime<Utc>) -> Result<Vec<Bar>> {
        let c = self.contract(symbol).await?;
        let now = Utc::now();
        let r = tokio::time::timeout(
            StdDuration::from_secs(120),
            self.inner()?.history.load_time_bars_all(c.clone(), self.s.exchange.clone(), TimeBarType::MinuteBar, tf_minutes as i32, since.timestamp() as i32, now.timestamp() as i32),
        )
        .await
        .map_err(|_| anyhow!("bar replay for {c} timed out"))?
        .map_err(|e| anyhow!("bar replay for {c}: {e}"))?;
        ok(&r, "bar replay")?;
        let mut out: Vec<Bar> = r
            .iter()
            .filter_map(|x| match &x.message {
                // `marker` is the bar's end (seconds since epoch); q22 bars are stamped at the open
                RithmicMessage::ResponseTimeBarReplay(b) => {
                    let end = Utc.timestamp_opt(b.marker? as i64, 0).single()?;
                    let bar = Bar { ts: end - chrono::Duration::minutes(tf_minutes), open: b.open_price?, high: b.high_price?, low: b.low_price?, close: b.close_price?, volume: b.volume.unwrap_or(0) as f64 };
                    (end <= now && bar.ts >= since && bar.is_valid()).then_some(bar)
                }
                _ => None,
            })
            .collect();
        out.sort_by_key(|b| b.ts);
        out.dedup_by_key(|b| b.ts);
        Ok(out)
    }

    async fn enter(&self, symbol: &str, side: Side, qty: f64, stop: f64, target: Option<f64>, tag: &str) -> Result<EntryAck> {
        self.healthy()?;
        let inner = self.inner()?;
        let c = self.contract(symbol).await?;
        let tick = Self::tick(symbol);
        let reference = self.reference_price(symbol, &c).await?;
        let ticks = |a: f64, b: f64| (((a - b).abs() / tick).round() as i32).max(1);
        let mut b = RithmicBracketOrder::new()
            .symbol(&c)
            .exchange(&self.s.exchange)
            .quantity(qty as i32)
            .action(if side == Side::Long { OrderSide::Buy } else { OrderSide::Sell })
            .price_type(OrderType::Market)
            .duration(TimeInForce::Day)
            .localid(tag)
            .stop(ticks(reference, stop));
        if let Some(t) = target {
            b = b.target(ticks(t, reference));
        }
        let order = b.build().map_err(|e| anyhow!("bracket: {e}"))?;
        let resp = inner.order.place_bracket_order(order).await.map_err(|e| anyhow!("place bracket: {e}"))?;
        ok(&resp, "bracket order")?;
        let basket = resp
            .iter()
            .find_map(|r| match &r.message {
                RithmicMessage::ResponseBracketOrder(b) => b.basket_id.clone(),
                _ => None,
            })
            .ok_or_else(|| anyhow!("bracket accepted without a basket id"))?;
        // wait for the entry fill (market order: normally milliseconds)
        let mut fill = None;
        for _ in 0..80 {
            if let Some(o) = self.state.lock().unwrap().orders.get(&basket) {
                if o.status == OrderStatus::Complete || o.avg_fill.is_some() {
                    fill = o.avg_fill;
                    if fill.is_some() {
                        break;
                    }
                }
                if o.status == OrderStatus::Rejected {
                    bail!("entry {basket} rejected");
                }
            }
            tokio::time::sleep(StdDuration::from_millis(100)).await;
        }
        if let Some(f) = fill {
            self.state.lock().unwrap().entries.insert(c.clone(), (basket.clone(), f));
            // re-aim the stop leg at the engine's absolute price, measured from the real fill
            let want = ticks(f, stop);
            if want != ticks(reference, stop) {
                let adj = RithmicBracketLevelAdjustment::new().id(basket.clone()).ticks(want).build().map_err(|e| anyhow!("{e}"))?;
                if let Err(e) = inner.order.adjust_stop(adj).await {
                    tracing::warn!("rithmic: stop re-aim after fill failed ({e}); stop stays {} ticks from the fill", ticks(reference, stop));
                }
            }
        }
        Ok(EntryAck { order_id: basket, fill_price: fill, filled_qty: if fill.is_some() { qty } else { 0.0 }, stop_order_id: None, target_order_id: None })
    }

    async fn modify_stop(&self, symbol: &str, _side: Side, qty: f64, new_stop: f64) -> Result<()> {
        self.healthy()?;
        let inner = self.inner()?;
        let c = self.contract(symbol).await?;
        let tick = Self::tick(symbol);
        let px = (new_stop / tick).round() * tick;
        if let Some((basket, o)) = self.working_stop(&c) {
            let q = if o.qty > 0 { o.qty } else { qty as i32 };
            let m = RithmicModifyOrder::new().id(basket).symbol(&c).exchange(&self.s.exchange).quantity(q).price_type(OrderType::StopMarket).trigger_price(px).build().map_err(|e| anyhow!("{e}"))?;
            let r = inner.order.modify_order(m).await.map_err(|e| anyhow!("modify stop: {e}"))?;
            return ok(&r, "stop modification");
        }
        // fall back to the bracket's stop level, in ticks from the entry fill
        let (basket, fill) = self.state.lock().unwrap().entries.get(&c).cloned().ok_or_else(|| anyhow!("no working stop on record for {c}"))?;
        let t = (((fill - px).abs() / tick).round() as i32).max(1);
        let adj = RithmicBracketLevelAdjustment::new().id(basket).ticks(t).build().map_err(|e| anyhow!("{e}"))?;
        let r = inner.order.adjust_stop(adj).await.map_err(|e| anyhow!("adjust stop: {e}"))?;
        ok(&[r], "stop adjustment")
    }

    async fn close(&self, symbol: &str) -> Result<Option<f64>> {
        let inner = self.inner()?;
        let c = self.contract(symbol).await?;
        self.cancel_all(symbol).await?;
        let flat = |s: &Self| s.state.lock().unwrap().positions.get(&c).is_none_or(|p| p.0 == 0);
        if !flat(self) {
            let x = RithmicExitPosition::new().symbol(&c).exchange(&self.s.exchange).build().map_err(|e| anyhow!("{e}"))?;
            let r = inner.order.exit_position(x).await.map_err(|e| anyhow!("exit position: {e}"))?;
            ok(&r, "exit position")?;
            for _ in 0..50 {
                if flat(self) {
                    break;
                }
                tokio::time::sleep(StdDuration::from_millis(100)).await;
            }
        }
        self.state.lock().unwrap().entries.remove(&c);
        Ok(self.state.lock().unwrap().last_fill.get(&c).copied())
    }

    async fn positions(&self) -> Result<Vec<BrokerPosition>> {
        self.healthy()?;
        let snapshot: Vec<(String, (i32, f64))> = self.state.lock().unwrap().positions.iter().map(|(k, v)| (k.clone(), *v)).collect();
        Ok(snapshot.into_iter().filter(|(_, (q, _))| *q != 0).map(|(c, (q, p))| BrokerPosition { symbol: self.symbol_of(&c), qty: q as f64, avg_price: p }).collect())
    }

    async fn cancel_all(&self, symbol: &str) -> Result<()> {
        let inner = self.inner()?;
        let c = self.contract(symbol).await?;
        let working: Vec<String> = self.state.lock().unwrap().orders.iter().filter(|(_, o)| o.contract == c && o.status.is_active()).map(|(k, _)| k.clone()).collect();
        for b in working {
            let x = RithmicCancelOrder::new().id(b.clone()).build().map_err(|e| anyhow!("{e}"))?;
            if let Err(e) = inner.order.cancel_order(x).await {
                tracing::warn!("rithmic: cancel {b}: {e}");
            }
        }
        Ok(())
    }

    async fn last_exit_price(&self, symbol: &str) -> Result<Option<f64>> {
        let c = self.contract(symbol).await?;
        Ok(self.state.lock().unwrap().last_fill.get(&c).copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_require_the_environment() {
        // SAFETY: test-only, single-threaded access to these variables
        unsafe { std::env::remove_var("Q22_RITHMIC_URL") };
        assert!(RithmicSettings::from_env().unwrap_err().to_string().contains("Q22_RITHMIC_URL"));
    }

    #[test]
    fn bracket_is_market_with_auto_flag_and_static_legs() {
        let o = RithmicBracketOrder::new().symbol("MNQZ6").exchange("CME").quantity(2).action(OrderSide::Buy).price_type(OrderType::Market).stop(40).target(80).build().unwrap();
        assert_eq!(o.bracket_type, Some(rithmic_rs::BracketType::TargetAndStopStatic));
        assert_eq!(o.manual_or_auto, rithmic_rs::ManualOrAutoEntry::Auto, "CME requires automated orders to be flagged");
    }
}
