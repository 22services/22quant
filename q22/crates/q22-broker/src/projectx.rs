//! ProjectX Gateway API (TopstepX) — REST, polling.
//!
//! Endpoints and field names follow the provider's Swagger contract as mirrored by the
//! MIT-0 `projectx-client` crate (v6): `POST /api/Auth/loginKey`, `/api/Account/search`,
//! `/api/Contract/search`, `/api/History/retrieveBars`, `/api/Order/place|modify|cancel|searchOpen`,
//! `/api/Position/searchOpen|closeContract`, `/api/Trade/search`. Enums: side 0 = buy (bid),
//! 1 = sell (ask); order type 1 limit, 2 market, 4 stop; position type 1 long, 2 short;
//! bar unit 2 = minute. Rate limits: 50 history requests / 30 s, 200 other requests / 60 s.
//!
//! Protective orders: after the market entry fills, a stop order (type 4, absolute
//! `stopPrice`) and an optional limit target (type 1) are placed for the opposite side; the
//! runner cancels the survivor once the position is flat. This works whether or not the
//! account has "Auto OCO Brackets" enabled, and every price is absolute (no tick-sign ambiguity).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration as StdDuration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use q22_core::{Bar, Side};

use crate::{AccountInfo, Broker, BrokerPosition, EntryAck};

pub const TOPSTEPX_API: &str = "https://api.topstepx.com";
pub const THEFUTURESDESK_API: &str = "https://api.thefuturesdesk.projectx.com";

#[derive(Clone, Debug)]
pub struct ProjectXConfig {
    pub api_base: String,
    pub user_name: String,
    pub api_key: String,
    /// Account name as shown on the platform (e.g. "50KTC-V2-…"); first tradable account if None.
    pub account_name: Option<String>,
    /// Futures contract search per configured symbol, e.g. MNQ → "MNQ".
    pub live_data: bool,
}

struct RateWindow {
    window: StdDuration,
    max: usize,
    hits: VecDeque<Instant>,
}

impl RateWindow {
    fn new(max: usize, secs: u64) -> Self {
        Self { window: StdDuration::from_secs(secs), max, hits: VecDeque::new() }
    }
    /// Seconds to wait before the next request is allowed.
    fn admit(&mut self) -> Option<StdDuration> {
        let now = Instant::now();
        while self.hits.front().is_some_and(|t| now.duration_since(*t) > self.window) {
            self.hits.pop_front();
        }
        if self.hits.len() >= self.max {
            let oldest = *self.hits.front().unwrap();
            return Some(self.window.saturating_sub(now.duration_since(oldest)) + StdDuration::from_millis(50));
        }
        self.hits.push_back(now);
        None
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContractDto {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    tick_size: f64,
    tick_value: f64,
    active_contract: bool,
}

pub struct ProjectX {
    cfg: ProjectXConfig,
    http: reqwest::Client,
    token: Mutex<Option<(String, Instant)>>,
    account_id: Mutex<Option<i64>>,
    contracts: Mutex<std::collections::HashMap<String, ContractDto>>,
    general: Mutex<RateWindow>,
    history: Mutex<RateWindow>,
    /// symbol -> (stop order id, target order id)
    protective: Mutex<std::collections::HashMap<String, (Option<i64>, Option<i64>)>>,
}

fn side_code(s: Side) -> i32 {
    match s {
        Side::Long => 0,
        Side::Short => 1,
    }
}

impl ProjectX {
    pub fn new(cfg: ProjectXConfig) -> Result<Self> {
        let http = reqwest::Client::builder().timeout(StdDuration::from_secs(20)).user_agent("q22/0.1").build()?;
        Ok(Self {
            cfg,
            http,
            token: Mutex::new(None),
            account_id: Mutex::new(None),
            contracts: Mutex::new(Default::default()),
            general: Mutex::new(RateWindow::new(180, 60)),
            history: Mutex::new(RateWindow::new(45, 30)),
            protective: Mutex::new(Default::default()),
        })
    }

    pub fn from_env(account_name: Option<String>, api_base: Option<String>) -> Result<Self> {
        let user_name = std::env::var("Q22_PROJECTX_USER").context("set Q22_PROJECTX_USER (your TopstepX user name)")?;
        let api_key = std::env::var("Q22_PROJECTX_KEY").context("set Q22_PROJECTX_KEY (API key from TopstepX settings)")?;
        Self::new(ProjectXConfig { api_base: api_base.unwrap_or_else(|| TOPSTEPX_API.into()), user_name, api_key, account_name, live_data: false })
    }

    async fn throttle(&self, history: bool) {
        loop {
            let wait = if history { self.history.lock().unwrap().admit() } else { self.general.lock().unwrap().admit() };
            match wait {
                None => return,
                Some(d) => tokio::time::sleep(d).await,
            }
        }
    }

    async fn login(&self) -> Result<String> {
        let url = format!("{}/api/Auth/loginKey", self.cfg.api_base);
        let body = json!({"userName": self.cfg.user_name, "apiKey": self.cfg.api_key});
        self.throttle(false).await;
        let v: Value = self.http.post(&url).json(&body).send().await?.error_for_status()?.json().await?;
        check_envelope(&v, "login")?;
        let token = v.get("token").and_then(|t| t.as_str()).ok_or_else(|| anyhow!("login: no token in response"))?.to_string();
        *self.token.lock().unwrap() = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    async fn bearer(&self) -> Result<String> {
        let cached = self.token.lock().unwrap().clone();
        match cached {
            // tokens live 24h; refresh after 20h
            Some((t, at)) if at.elapsed() < StdDuration::from_secs(20 * 3600) => Ok(t),
            _ => self.login().await,
        }
    }

    /// POST with bearer auth and the provider's {success, errorCode, errorMessage} envelope.
    async fn call(&self, path: &str, body: Value, history: bool, mutation: bool) -> Result<Value> {
        let url = format!("{}/{}", self.cfg.api_base, path);
        for attempt in 0..2 {
            let token = self.bearer().await?;
            self.throttle(history).await;
            let resp = self.http.post(&url).bearer_auth(&token).json(&body).send().await.with_context(|| format!("POST {path}"))?;
            if resp.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                *self.token.lock().unwrap() = None;
                continue;
            }
            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                if mutation {
                    bail!("{path}: provider rate limit (429) — mutation not retried");
                }
                tokio::time::sleep(StdDuration::from_secs(5)).await;
                continue;
            }
            let v: Value = resp.error_for_status().with_context(|| format!("POST {path}"))?.json().await?;
            check_envelope(&v, path)?;
            return Ok(v);
        }
        bail!("{path}: failed after re-authentication")
    }

    fn account(&self) -> Result<i64> {
        self.account_id.lock().unwrap().ok_or_else(|| anyhow!("not connected"))
    }

    async fn contract(&self, symbol: &str) -> Result<ContractDto> {
        if let Some(c) = self.contracts.lock().unwrap().get(symbol) {
            return Ok(c.clone());
        }
        let v = self.call("api/Contract/search", json!({"live": self.cfg.live_data, "searchText": symbol}), false, false).await?;
        let list: Vec<ContractDto> = serde_json::from_value(v.get("contracts").cloned().unwrap_or(Value::Array(vec![])))?;
        let sym = symbol.to_ascii_uppercase();
        let pick = list
            .iter()
            .find(|c| c.id.eq_ignore_ascii_case(symbol))
            .or_else(|| list.iter().find(|c| c.active_contract && c.name.to_ascii_uppercase().starts_with(&sym) && c.id.contains(&format!(".{sym}."))))
            .or_else(|| list.iter().find(|c| c.active_contract && c.name.to_ascii_uppercase().starts_with(&sym)))
            .cloned()
            .ok_or_else(|| anyhow!("no active ProjectX contract for {symbol:?} (got {:?})", list.iter().map(|c| &c.name).collect::<Vec<_>>()))?;
        tracing::info!("contract {symbol} → {} ({}, tick {} = ${})", pick.id, pick.description, pick.tick_size, pick.tick_value);
        self.contracts.lock().unwrap().insert(symbol.to_string(), pick.clone());
        Ok(pick)
    }

    async fn place(&self, body: Value) -> Result<i64> {
        let v = self.call("api/Order/place", body, false, true).await?;
        v.get("orderId").and_then(|x| x.as_i64()).ok_or_else(|| anyhow!("order/place: no orderId"))
    }

    async fn open_position(&self, contract_id: &str) -> Result<Option<(f64, f64)>> {
        let v = self.call("api/Position/searchOpen", json!({"accountId": self.account()?}), false, false).await?;
        for p in v.get("positions").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
            if p.get("contractId").and_then(|x| x.as_str()) == Some(contract_id) {
                let size = p.get("size").and_then(|x| x.as_f64()).unwrap_or(0.0);
                let sign = if p.get("type").and_then(|x| x.as_i64()) == Some(2) { -1.0 } else { 1.0 };
                let avg = p.get("averagePrice").and_then(|x| x.as_f64()).unwrap_or(f64::NAN);
                return Ok(Some((sign * size, avg)));
            }
        }
        Ok(None)
    }

    async fn place_protective(&self, symbol: &str, contract: &ContractDto, side: Side, qty: f64, stop: f64, target: Option<f64>, tag: &str) -> Result<(Option<i64>, Option<i64>)> {
        let acct = self.account()?;
        let exit_side = side_code(side.opposite());
        let stop_px = (stop / contract.tick_size).round() * contract.tick_size;
        let sl = self.place(json!({"accountId": acct, "contractId": contract.id, "type": 4, "side": exit_side, "size": qty as i64, "stopPrice": stop_px, "customTag": format!("{tag}-sl")})).await?;
        let tp = match target {
            Some(t) => {
                let px = (t / contract.tick_size).round() * contract.tick_size;
                Some(self.place(json!({"accountId": acct, "contractId": contract.id, "type": 1, "side": exit_side, "size": qty as i64, "limitPrice": px, "customTag": format!("{tag}-tp")})).await?)
            }
            None => None,
        };
        self.protective.lock().unwrap().insert(symbol.to_string(), (Some(sl), tp));
        Ok((Some(sl), tp))
    }
}

fn check_envelope(v: &Value, what: &str) -> Result<()> {
    let ok = v.get("success").and_then(|s| s.as_bool()).unwrap_or(true);
    if !ok {
        let code = v.get("errorCode").and_then(|c| c.as_i64()).unwrap_or(-1);
        let msg = v.get("errorMessage").and_then(|m| m.as_str()).unwrap_or("");
        bail!("{what}: provider error {code} {msg}");
    }
    Ok(())
}

/// Parse a retrieveBars response into completed bars (oldest first).
pub fn parse_bars(v: &Value) -> Result<Vec<Bar>> {
    let mut out = vec![];
    for b in v.get("bars").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
        let ts = b.get("t").and_then(|x| x.as_str()).ok_or_else(|| anyhow!("bar without t"))?;
        let ts = DateTime::parse_from_rfc3339(ts)?.with_timezone(&Utc);
        let f = |k: &str| b.get(k).and_then(|x| x.as_f64()).unwrap_or(f64::NAN);
        let bar = Bar { ts, open: f("o"), high: f("h"), low: f("l"), close: f("c"), volume: f("v") };
        if bar.is_valid() {
            out.push(bar);
        }
    }
    out.sort_by_key(|b| b.ts);
    out.dedup_by_key(|b| b.ts);
    Ok(out)
}

#[async_trait]
impl Broker for ProjectX {
    fn name(&self) -> &str {
        "projectx"
    }

    async fn connect(&mut self) -> Result<AccountInfo> {
        self.login().await?;
        let v = self.call("api/Account/search", json!({"onlyActiveAccounts": true}), false, false).await?;
        let accts = v.get("accounts").and_then(|a| a.as_array()).cloned().unwrap_or_default();
        let pick = accts
            .iter()
            .find(|a| match &self.cfg.account_name {
                Some(n) => a.get("name").and_then(|x| x.as_str()) == Some(n.as_str()),
                None => a.get("canTrade").and_then(|x| x.as_bool()).unwrap_or(false),
            })
            .ok_or_else(|| anyhow!("account {:?} not found among {:?}", self.cfg.account_name, accts.iter().filter_map(|a| a.get("name")).collect::<Vec<_>>()))?;
        let id = pick.get("id").and_then(|x| x.as_i64()).ok_or_else(|| anyhow!("account without id"))?;
        *self.account_id.lock().unwrap() = Some(id);
        Ok(AccountInfo {
            id: id.to_string(),
            name: pick.get("name").and_then(|x| x.as_str()).unwrap_or("").into(),
            balance: pick.get("balance").and_then(|x| x.as_f64()).unwrap_or(f64::NAN),
            can_trade: pick.get("canTrade").and_then(|x| x.as_bool()).unwrap_or(false),
            simulated: pick.get("simulated").and_then(|x| x.as_bool()),
        })
    }

    async fn account(&self) -> Result<AccountInfo> {
        let v = self.call("api/Account/search", json!({"onlyActiveAccounts": true}), false, false).await?;
        let id = self.account()?;
        let a = v.get("accounts").and_then(|a| a.as_array()).and_then(|l| l.iter().find(|a| a.get("id").and_then(|x| x.as_i64()) == Some(id)).cloned()).ok_or_else(|| anyhow!("account {id} disappeared"))?;
        Ok(AccountInfo {
            id: id.to_string(),
            name: a.get("name").and_then(|x| x.as_str()).unwrap_or("").into(),
            balance: a.get("balance").and_then(|x| x.as_f64()).unwrap_or(f64::NAN),
            can_trade: a.get("canTrade").and_then(|x| x.as_bool()).unwrap_or(false),
            simulated: a.get("simulated").and_then(|x| x.as_bool()),
        })
    }

    async fn bars(&self, symbol: &str, tf_minutes: i64, since: DateTime<Utc>) -> Result<Vec<Bar>> {
        let c = self.contract(symbol).await?;
        let now = Utc::now();
        let body = json!({
            "contractId": c.id, "live": self.cfg.live_data,
            "startTime": since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "endTime": now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "unit": 2, "unitNumber": tf_minutes, "limit": 20000, "includePartialBar": false
        });
        let v = self.call("api/History/retrieveBars", body, true, false).await?;
        let bars = parse_bars(&v)?;
        // belt and braces: drop anything not yet complete
        Ok(bars.into_iter().filter(|b| b.ts + Duration::minutes(tf_minutes) <= now).collect())
    }

    async fn enter(&self, symbol: &str, side: Side, qty: f64, stop: f64, target: Option<f64>, tag: &str) -> Result<EntryAck> {
        let c = self.contract(symbol).await?;
        let acct = self.account()?;
        let body = json!({"accountId": acct, "contractId": c.id, "type": 2, "side": side_code(side), "size": qty as i64, "customTag": tag});
        let order_id = self.place(body).await?;
        // wait for the fill to show up as a position (market orders fill in milliseconds)
        let mut fill = None;
        for _ in 0..20 {
            tokio::time::sleep(StdDuration::from_millis(250)).await;
            if let Some((q, avg)) = self.open_position(&c.id).await? {
                if q.abs() >= qty - 1e-9 {
                    fill = Some((q.abs(), avg));
                    break;
                }
            }
        }
        let (filled, avg) = fill.ok_or_else(|| anyhow!("entry order {order_id} placed but no position after 5 s — check the platform"))?;
        let (sl, tp) = self.place_protective(symbol, &c, side, filled, stop, target, tag).await.context("CRITICAL: position open but protective orders failed")?;
        Ok(EntryAck { order_id: order_id.to_string(), fill_price: Some(avg), filled_qty: filled, stop_order_id: sl.map(|x| x.to_string()), target_order_id: tp.map(|x| x.to_string()) })
    }

    async fn modify_stop(&self, symbol: &str, _side: Side, _qty: f64, new_stop: f64) -> Result<()> {
        let c = self.contract(symbol).await?;
        let sl = self.protective.lock().unwrap().get(symbol).and_then(|p| p.0).ok_or_else(|| anyhow!("no stop order on record for {symbol}"))?;
        let px = (new_stop / c.tick_size).round() * c.tick_size;
        self.call("api/Order/modify", json!({"accountId": self.account()?, "orderId": sl, "stopPrice": px}), false, true).await?;
        Ok(())
    }

    async fn close(&self, symbol: &str) -> Result<Option<f64>> {
        let c = self.contract(symbol).await?;
        self.cancel_all(symbol).await?;
        if self.open_position(&c.id).await?.is_some() {
            self.call("api/Position/closeContract", json!({"accountId": self.account()?, "contractId": c.id}), false, true).await?;
        }
        tokio::time::sleep(StdDuration::from_millis(500)).await;
        self.last_exit_price(symbol).await
    }

    async fn positions(&self) -> Result<Vec<BrokerPosition>> {
        let v = self.call("api/Position/searchOpen", json!({"accountId": self.account()?}), false, false).await?;
        let contracts = self.contracts.lock().unwrap().clone();
        Ok(v.get("positions")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|p| {
                let cid = p.get("contractId")?.as_str()?;
                let sym = contracts.iter().find(|(_, c)| c.id == cid).map(|(s, _)| s.clone()).unwrap_or_else(|| cid.to_string());
                let size = p.get("size")?.as_f64()?;
                let sign = if p.get("type").and_then(|x| x.as_i64()) == Some(2) { -1.0 } else { 1.0 };
                Some(BrokerPosition { symbol: sym, qty: sign * size, avg_price: p.get("averagePrice").and_then(|x| x.as_f64()).unwrap_or(f64::NAN) })
            })
            .collect())
    }

    async fn cancel_all(&self, symbol: &str) -> Result<()> {
        let c = self.contract(symbol).await?;
        let acct = self.account()?;
        let v = self.call("api/Order/searchOpen", json!({"accountId": acct}), false, false).await?;
        for o in v.get("orders").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
            if o.get("contractId").and_then(|x| x.as_str()) == Some(c.id.as_str()) {
                if let Some(id) = o.get("id").and_then(|x| x.as_i64()) {
                    self.call("api/Order/cancel", json!({"accountId": acct, "orderId": id}), false, true).await?;
                }
            }
        }
        self.protective.lock().unwrap().remove(symbol);
        Ok(())
    }

    async fn last_exit_price(&self, symbol: &str) -> Result<Option<f64>> {
        let c = self.contract(symbol).await?;
        let start = (Utc::now() - Duration::hours(12)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let v = self.call("api/Trade/search", json!({"accountId": self.account()?, "startTimestamp": start}), false, false).await?;
        let mut trades: Vec<Value> = v.get("trades").and_then(|x| x.as_array()).cloned().unwrap_or_default().into_iter().filter(|t| t.get("contractId").and_then(|x| x.as_str()) == Some(c.id.as_str()) && !t.get("voided").and_then(|x| x.as_bool()).unwrap_or(false)).collect();
        trades.sort_by_key(|t| t.get("creationTimestamp").and_then(|x| x.as_str()).unwrap_or("").to_string());
        Ok(trades.last().and_then(|t| t.get("price").and_then(|x| x.as_f64())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bars_envelope() {
        let v: Value = serde_json::from_str(r#"{"bars":[{"t":"2026-07-01T14:05:00+00:00","o":20000.25,"h":20010,"l":19995.5,"c":20005,"v":1234},{"t":"2026-07-01T14:00:00+00:00","o":19990,"h":20001,"l":19989,"c":20000.25,"v":999}],"success":true,"errorCode":0,"errorMessage":null}"#).unwrap();
        let b = parse_bars(&v).unwrap();
        assert_eq!(b.len(), 2);
        assert!(b[0].ts < b[1].ts, "must sort oldest first");
        assert_eq!(b[1].close, 20005.0);
        assert!(check_envelope(&v, "x").is_ok());
        let bad: Value = serde_json::from_str(r#"{"success":false,"errorCode":2,"errorMessage":"Invalid"}"#).unwrap();
        assert!(check_envelope(&bad, "x").is_err());
    }

    #[test]
    fn rate_window() {
        let mut w = RateWindow::new(2, 60);
        assert!(w.admit().is_none());
        assert!(w.admit().is_none());
        assert!(w.admit().is_some());
    }
}
