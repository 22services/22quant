//! Tradovate REST API (orders, positions, fills, balance) + market-data WebSocket (bars).
//!
//! **Eligibility (Tradovate policy, 2026):** API keys are issued only on a *live, funded
//! Tradovate brokerage account* (≥ $1,000) with the API Access add-on ($25/month), and **prop-firm
//! and evaluation accounts are not eligible** — a personal key cannot reach a Lucid, Apex or
//! Tradeify account hosted at Tradovate. Use this adapter for your own Tradovate account (or if
//! a firm grants API access in writing); use [`crate::rithmic`] for Lucid. Real-time CME data
//! over the API needs its own market-data entitlement.
//!
//! Credentials (environment only): `Q22_TRADOVATE_USER`, `Q22_TRADOVATE_PASSWORD`,
//! `Q22_TRADOVATE_CID`, `Q22_TRADOVATE_SECRET`, `Q22_TRADOVATE_APP_ID` (default "q22"),
//! `Q22_TRADOVATE_ACCOUNT` (optional), `Q22_TRADOVATE_ENV` = `demo` | `live` (default demo).
//!
//! Every order carries `isAutomated: true` (CME rule for automated order entry). Entries are
//! order-sends-order brackets (`placeOSO`): market entry, a stop and an optional limit target
//! that cancel each other.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration as StdDuration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

use q22_core::contracts::front_symbol;
use q22_core::{Bar, InstrumentSpec, Side};

use crate::projectx::RateWindow;
use crate::{AccountInfo, Broker, BrokerPosition, EntryAck};

#[derive(Clone, Debug)]
pub struct TradovateConfig {
    pub user: String,
    pub password: String,
    pub cid: String,
    pub secret: String,
    pub app_id: String,
    pub account: Option<String>,
    pub live: bool,
}

impl TradovateConfig {
    pub fn from_env() -> Result<Self> {
        let need = |k: &str| std::env::var(k).map_err(|_| anyhow!("{k} is not set"));
        Ok(Self {
            user: need("Q22_TRADOVATE_USER")?,
            password: need("Q22_TRADOVATE_PASSWORD")?,
            cid: need("Q22_TRADOVATE_CID")?,
            secret: need("Q22_TRADOVATE_SECRET")?,
            app_id: std::env::var("Q22_TRADOVATE_APP_ID").unwrap_or_else(|_| "q22".into()),
            account: std::env::var("Q22_TRADOVATE_ACCOUNT").ok().filter(|s| !s.is_empty()),
            live: std::env::var("Q22_TRADOVATE_ENV").map(|v| v.eq_ignore_ascii_case("live")).unwrap_or(false),
        })
    }
    fn rest(&self) -> &'static str {
        if self.live { "https://live.tradovateapi.com/v1" } else { "https://demo.tradovateapi.com/v1" }
    }
    fn md(&self) -> &'static str {
        if self.live { "wss://md.tradovateapi.com/v1/websocket" } else { "wss://md-demo.tradovateapi.com/v1/websocket" }
    }
}

#[derive(Default)]
struct Session {
    token: String,
    md_token: String,
    expires: Option<DateTime<Utc>>,
    account_id: i64,
    account_name: String,
}

pub struct Tradovate {
    cfg: TradovateConfig,
    http: reqwest::Client,
    session: Mutex<Session>,
    rate: Mutex<RateWindow>,
    /// our symbol → (contract name, contract id)
    contracts: Mutex<HashMap<String, (String, i64)>>,
    /// contract id → our symbol
    by_id: Mutex<HashMap<i64, String>>,
    /// our symbol → (stop order id, target order id)
    protective: Mutex<HashMap<String, (Option<i64>, Option<i64>)>>,
}

fn side_action(side: Side) -> &'static str {
    if side == Side::Long { "Buy" } else { "Sell" }
}

fn failure(v: &Value) -> Option<String> {
    let reason = v.get("failureReason").and_then(|x| x.as_str());
    let text = v.get("failureText").or_else(|| v.get("errorText")).and_then(|x| x.as_str());
    match (reason, text) {
        (None, None) => None,
        (r, t) => Some(format!("{} {}", r.unwrap_or(""), t.unwrap_or("")).trim().to_string()),
    }
}

/// Parse Tradovate chart bars (`timestamp` = bar open, UTC) into q22 bars.
pub fn parse_chart_bars(bars: &[Value]) -> Vec<Bar> {
    bars.iter()
        .filter_map(|b| {
            let ts = DateTime::parse_from_rfc3339(b.get("timestamp")?.as_str()?).ok()?.with_timezone(&Utc);
            let f = |k: &str| b.get(k).and_then(|x| x.as_f64());
            let vol = f("upVolume").unwrap_or(0.0) + f("downVolume").unwrap_or(0.0);
            let bar = Bar { ts, open: f("open")?, high: f("high")?, low: f("low")?, close: f("close")?, volume: vol };
            bar.is_valid().then_some(bar)
        })
        .collect()
}

impl Tradovate {
    pub fn new(cfg: TradovateConfig) -> Result<Self> {
        let http = reqwest::Client::builder().timeout(StdDuration::from_secs(20)).user_agent("q22/0.1").build()?;
        Ok(Self {
            cfg,
            http,
            session: Mutex::new(Session::default()),
            rate: Mutex::new(RateWindow::new(60, 60)),
            contracts: Mutex::new(HashMap::new()),
            by_id: Mutex::new(HashMap::new()),
            protective: Mutex::new(HashMap::new()),
        })
    }

    pub fn from_env() -> Result<Self> {
        Self::new(TradovateConfig::from_env()?)
    }

    async fn throttle(&self) {
        loop {
            let wait = self.rate.lock().unwrap().admit();
            match wait {
                Some(w) => tokio::time::sleep(w).await,
                None => return,
            }
        }
    }

    async fn authenticate(&self) -> Result<()> {
        let body = json!({
            "name": self.cfg.user, "password": self.cfg.password, "appId": self.cfg.app_id, "appVersion": "1.0",
            "cid": self.cfg.cid, "sec": self.cfg.secret, "deviceId": format!("q22-{}", self.cfg.user),
        });
        self.throttle().await;
        let v: Value = self.http.post(format!("{}/auth/accesstokenrequest", self.cfg.rest())).json(&body).send().await?.json().await?;
        if v.get("p-ticket").is_some() {
            bail!("Tradovate returned a captcha/penalty ticket (too many logins): log in once in the Tradovate web app, wait {}s, retry", v.get("p-time").and_then(|x| x.as_i64()).unwrap_or(60));
        }
        if let Some(e) = failure(&v) {
            bail!("Tradovate login refused: {e} (prop/evaluation accounts are not eligible for API access)");
        }
        let token = v.get("accessToken").and_then(|x| x.as_str()).ok_or_else(|| anyhow!("no accessToken in {v}"))?;
        let mut s = self.session.lock().unwrap();
        s.token = token.to_string();
        s.md_token = v.get("mdAccessToken").and_then(|x| x.as_str()).unwrap_or(token).to_string();
        s.expires = v.get("expirationTime").and_then(|x| x.as_str()).and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Utc));
        Ok(())
    }

    async fn ensure_token(&self) -> Result<()> {
        let stale = {
            let s = self.session.lock().unwrap();
            s.token.is_empty() || s.expires.is_some_and(|e| e - Utc::now() < Duration::minutes(10))
        };
        if stale {
            self.authenticate().await?;
        }
        Ok(())
    }

    async fn get(&self, path: &str) -> Result<Value> {
        self.ensure_token().await?;
        self.throttle().await;
        let token = self.session.lock().unwrap().token.clone();
        let r = self.http.get(format!("{}/{path}", self.cfg.rest())).bearer_auth(token).send().await?;
        let status = r.status();
        let v: Value = r.json().await.with_context(|| format!("GET {path}"))?;
        if !status.is_success() {
            bail!("GET {path}: HTTP {status} {v}");
        }
        Ok(v)
    }

    /// POST — never retried (orders move money).
    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.ensure_token().await?;
        self.throttle().await;
        let token = self.session.lock().unwrap().token.clone();
        let r = self.http.post(format!("{}/{path}", self.cfg.rest())).bearer_auth(token).json(&body).send().await?;
        let status = r.status();
        let v: Value = r.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            bail!("POST {path}: HTTP {status} {v}");
        }
        if let Some(e) = failure(&v) {
            bail!("POST {path} refused: {e}");
        }
        Ok(v)
    }

    fn account_id(&self) -> Result<(i64, String)> {
        let s = self.session.lock().unwrap();
        if s.account_id == 0 {
            bail!("tradovate: not connected");
        }
        Ok((s.account_id, s.account_name.clone()))
    }

    /// Our symbol (MNQ) → front contract (MNQZ6) and its id.
    async fn contract(&self, symbol: &str) -> Result<(String, i64)> {
        if let Some(c) = self.contracts.lock().unwrap().get(symbol).cloned() {
            return Ok(c);
        }
        let name = front_symbol(symbol, Utc::now().date_naive());
        let v = self.get(&format!("contract/find?name={name}")).await?;
        let id = v.get("id").and_then(|x| x.as_i64()).ok_or_else(|| anyhow!("contract {name} not found: {v}"))?;
        self.contracts.lock().unwrap().insert(symbol.to_string(), (name.clone(), id));
        self.by_id.lock().unwrap().insert(id, symbol.to_string());
        Ok((name, id))
    }

    async fn fill_price_of(&self, order_id: i64) -> Result<Option<f64>> {
        let v = self.get(&format!("fill/deps?masterid={order_id}")).await?;
        let fills = v.as_array().cloned().unwrap_or_default();
        let (mut q, mut pq) = (0.0, 0.0);
        for f in fills {
            let (qty, px) = (f.get("qty").and_then(|x| x.as_f64()).unwrap_or(0.0), f.get("price").and_then(|x| x.as_f64()).unwrap_or(0.0));
            q += qty;
            pq += qty * px;
        }
        Ok((q > 0.0).then(|| pq / q))
    }

    async fn ws_chart(&self, contract: &str, tf_minutes: i64, since: DateTime<Utc>) -> Result<Vec<Bar>> {
        self.ensure_token().await?;
        let md_token = self.session.lock().unwrap().md_token.clone();
        let (mut ws, _) = tokio_tungstenite::connect_async(self.cfg.md()).await.context("tradovate md websocket")?;
        ws.send(Message::Text(format!("authorize\n0\n\n{md_token}").into())).await?;
        let req = json!({
            "symbol": contract,
            "chartDescription": {"underlyingType": "MinuteBar", "elementSize": tf_minutes, "elementSizeUnit": "UnderlyingUnits", "withHistogram": false},
            "timeRange": {"asFarAsTimestamp": since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)},
        });
        ws.send(Message::Text(format!("md/getChart\n1\n\n{req}").into())).await?;
        let mut bars = vec![];
        let deadline = Instant::now() + StdDuration::from_secs(30);
        let mut last_hb = Instant::now();
        let mut sub_id: Option<i64> = None;
        loop {
            if Instant::now() > deadline {
                bail!("tradovate chart for {contract}: timed out before end of history");
            }
            if last_hb.elapsed() > StdDuration::from_millis(2500) {
                ws.send(Message::Text("[]".into())).await?;
                last_hb = Instant::now();
            }
            let Some(msg) = tokio::time::timeout(StdDuration::from_secs(3), ws.next()).await.ok().flatten() else { continue };
            let Message::Text(t) = msg? else { continue };
            let Some(body) = t.strip_prefix('a') else { continue };
            let frames: Vec<Value> = serde_json::from_str(body).unwrap_or_default();
            let mut done = false;
            for f in frames {
                if f.get("i").and_then(|x| x.as_i64()) == Some(1) {
                    if f.get("s").and_then(|x| x.as_i64()) != Some(200) {
                        bail!("tradovate md/getChart refused: {f}");
                    }
                    sub_id = f.pointer("/d/historicalId").and_then(|x| x.as_i64());
                }
                if f.get("i").and_then(|x| x.as_i64()) == Some(0) && f.get("s").and_then(|x| x.as_i64()) != Some(200) {
                    bail!("tradovate md authorization refused: {f} (does the account have CME data for the API?)");
                }
                if f.get("e").and_then(|x| x.as_str()) == Some("chart") {
                    for c in f.pointer("/d/charts").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
                        if let Some(b) = c.get("bars").and_then(|x| x.as_array()) {
                            bars.extend(parse_chart_bars(b));
                        }
                        if c.get("eoh").and_then(|x| x.as_bool()) == Some(true) {
                            done = true;
                        }
                    }
                }
            }
            if done {
                break;
            }
        }
        if let Some(id) = sub_id {
            let _ = ws.send(Message::Text(format!("md/cancelChart\n2\n\n{}", json!({"subscriptionId": id})).into())).await;
        }
        let _ = ws.close(None).await;
        let now = Utc::now();
        bars.retain(|b| b.ts >= since && b.ts + Duration::minutes(tf_minutes) <= now);
        bars.sort_by_key(|b| b.ts);
        bars.dedup_by_key(|b| b.ts);
        Ok(bars)
    }
}

#[async_trait]
impl Broker for Tradovate {
    fn name(&self) -> &str {
        "tradovate"
    }

    async fn connect(&mut self) -> Result<AccountInfo> {
        self.authenticate().await?;
        let v = self.get("account/list").await?;
        let accts = v.as_array().cloned().unwrap_or_default();
        let pick = match &self.cfg.account {
            Some(n) => accts.iter().find(|a| a.get("name").and_then(|x| x.as_str()) == Some(n.as_str())),
            None => accts.iter().find(|a| a.get("active").and_then(|x| x.as_bool()).unwrap_or(true)),
        }
        .cloned()
        .ok_or_else(|| anyhow!("no matching Tradovate account in {}", accts.iter().filter_map(|a| a.get("name").and_then(|x| x.as_str())).collect::<Vec<_>>().join(", ")))?;
        {
            let mut s = self.session.lock().unwrap();
            s.account_id = pick.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
            s.account_name = pick.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
        }
        self.account().await
    }

    async fn account(&self) -> Result<AccountInfo> {
        let (id, name) = self.account_id()?;
        let v = self.post("cashBalance/getcashbalancesnapshot", json!({"accountId": id})).await?;
        let bal = v.get("netLiq").or_else(|| v.get("totalCashValue")).and_then(|x| x.as_f64()).unwrap_or(0.0);
        Ok(AccountInfo { id: id.to_string(), name, balance: bal, can_trade: true, simulated: Some(!self.cfg.live) })
    }

    async fn bars(&self, symbol: &str, tf_minutes: i64, since: DateTime<Utc>) -> Result<Vec<Bar>> {
        let (name, _) = self.contract(symbol).await?;
        self.ws_chart(&name, tf_minutes, since).await
    }

    async fn enter(&self, symbol: &str, side: Side, qty: f64, stop: f64, target: Option<f64>, tag: &str) -> Result<EntryAck> {
        let (id, name) = self.account_id()?;
        let (contract, _) = self.contract(symbol).await?;
        let tick = InstrumentSpec::builtin(symbol).map(|s| s.tick_size).unwrap_or(0.25);
        let round = |p: f64| (p / tick).round() * tick;
        let exit = side_action(side.opposite());
        let mut body = json!({
            "accountSpec": name, "accountId": id, "action": side_action(side), "symbol": contract, "orderQty": qty as i64,
            "orderType": "Market", "isAutomated": true, "text": tag,
            "bracket1": {"action": exit, "orderType": "Stop", "stopPrice": round(stop)},
        });
        if let Some(t) = target {
            body["bracket2"] = json!({"action": exit, "orderType": "Limit", "price": round(t)});
        }
        let v = self.post("order/placeOSO", body).await?;
        let oid = v.get("orderId").and_then(|x| x.as_i64()).ok_or_else(|| anyhow!("placeOSO without orderId: {v}"))?;
        let sl = v.get("oso1Id").and_then(|x| x.as_i64());
        let tp = v.get("oso2Id").and_then(|x| x.as_i64());
        self.protective.lock().unwrap().insert(symbol.to_string(), (sl, tp));
        let mut fill = None;
        for _ in 0..20 {
            fill = self.fill_price_of(oid).await.unwrap_or(None);
            if fill.is_some() {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(250)).await;
        }
        Ok(EntryAck { order_id: oid.to_string(), fill_price: fill, filled_qty: if fill.is_some() { qty } else { 0.0 }, stop_order_id: sl.map(|x| x.to_string()), target_order_id: tp.map(|x| x.to_string()) })
    }

    async fn modify_stop(&self, symbol: &str, _side: Side, qty: f64, new_stop: f64) -> Result<()> {
        let sl = self.protective.lock().unwrap().get(symbol).and_then(|p| p.0).ok_or_else(|| anyhow!("no stop order on record for {symbol}"))?;
        let tick = InstrumentSpec::builtin(symbol).map(|s| s.tick_size).unwrap_or(0.25);
        self.post("order/modifyorder", json!({"orderId": sl, "orderQty": qty as i64, "orderType": "Stop", "stopPrice": (new_stop / tick).round() * tick, "isAutomated": true})).await?;
        Ok(())
    }

    async fn close(&self, symbol: &str) -> Result<Option<f64>> {
        let (id, _) = self.account_id()?;
        let (_, cid) = self.contract(symbol).await?;
        self.post("order/liquidateposition", json!({"accountId": id, "contractId": cid, "admin": false})).await?;
        self.protective.lock().unwrap().remove(symbol);
        tokio::time::sleep(StdDuration::from_millis(500)).await;
        self.last_exit_price(symbol).await
    }

    async fn positions(&self) -> Result<Vec<BrokerPosition>> {
        let (id, _) = self.account_id()?;
        let v = self.get("position/list").await?;
        let mut out = vec![];
        for p in v.as_array().cloned().unwrap_or_default() {
            if p.get("accountId").and_then(|x| x.as_i64()) != Some(id) {
                continue;
            }
            let q = p.get("netPos").and_then(|x| x.as_f64()).unwrap_or(0.0);
            if q == 0.0 {
                continue;
            }
            let cid = p.get("contractId").and_then(|x| x.as_i64()).unwrap_or(0);
            let known = self.by_id.lock().unwrap().get(&cid).cloned();
            let symbol = match known {
                Some(s) => s,
                None => self.get(&format!("contract/item?id={cid}")).await.ok().and_then(|c| c.get("name").and_then(|x| x.as_str()).map(String::from)).unwrap_or_else(|| cid.to_string()),
            };
            out.push(BrokerPosition { symbol, qty: q, avg_price: p.get("netPrice").and_then(|x| x.as_f64()).unwrap_or(0.0) });
        }
        Ok(out)
    }

    async fn cancel_all(&self, symbol: &str) -> Result<()> {
        let (id, _) = self.account_id()?;
        let (_, cid) = self.contract(symbol).await?;
        let v = self.get("order/list").await?;
        for o in v.as_array().cloned().unwrap_or_default() {
            let working = matches!(o.get("ordStatus").and_then(|x| x.as_str()), Some("Working" | "PendingNew" | "Suspended"));
            if working && o.get("accountId").and_then(|x| x.as_i64()) == Some(id) && o.get("contractId").and_then(|x| x.as_i64()) == Some(cid) {
                if let Some(oid) = o.get("id").and_then(|x| x.as_i64()) {
                    if let Err(e) = self.post("order/cancelorder", json!({"orderId": oid, "isAutomated": true})).await {
                        tracing::warn!("tradovate: cancel {oid}: {e:#}");
                    }
                }
            }
        }
        self.protective.lock().unwrap().remove(symbol);
        Ok(())
    }

    async fn last_exit_price(&self, symbol: &str) -> Result<Option<f64>> {
        let (_, cid) = self.contract(symbol).await?;
        let v = self.get("fill/list").await?;
        let last = v
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|f| f.get("contractId").and_then(|x| x.as_i64()) == Some(cid))
            .max_by_key(|f| f.get("timestamp").and_then(|x| x.as_str()).unwrap_or("").to_string());
        Ok(last.and_then(|f| f.get("price").and_then(|x| x.as_f64())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chart_bars_parse_and_skip_bad_rows() {
        let raw = json!([
            {"timestamp": "2026-10-02T13:30:00Z", "open": 20000.0, "high": 20010.0, "low": 19990.0, "close": 20005.0, "upVolume": 120, "downVolume": 80},
            {"timestamp": "not a time", "open": 1.0, "high": 1.0, "low": 1.0, "close": 1.0},
            {"timestamp": "2026-10-02T13:35:00Z", "open": 20005.0, "high": 20000.0, "low": 19990.0, "close": 19995.0}
        ]);
        let bars = parse_chart_bars(raw.as_array().unwrap());
        assert_eq!(bars.len(), 1, "a malformed row and an inconsistent bar (high < open) are dropped");
        assert_eq!(bars[0].volume, 200.0);
    }

    #[test]
    fn failures_are_detected() {
        assert!(failure(&json!({"orderId": 1})).is_none());
        assert_eq!(failure(&json!({"failureReason": "RiskCheck", "failureText": "max position"})).as_deref(), Some("RiskCheck max position"));
    }
}
