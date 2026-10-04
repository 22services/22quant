//! Bybit v5 REST adapter (linear USDT perpetuals) — HyroTrader funded accounts and personal use.
//!
//! * Public market data (klines) needs no key and is free.
//! * Private calls are signed: `X-BAPI-SIGN = hex(HMAC_SHA256(secret, ts + apiKey + recvWindow + payload))`
//!   where payload is the query string (GET) or the exact JSON body (POST).
//! * Entries carry `stopLoss`/`takeProfit` **in the same request** (tpslMode = Full), which
//!   satisfies HyroTrader's "stop-loss within 5 minutes" rule at the exchange level.
//! * Base URLs: mainnet `https://api.bybit.com`, demo `https://api-demo.bybit.com`,
//!   testnet `https://api-testnet.bybit.com`.

use std::time::Duration as StdDuration;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

use q22_core::{Bar, Side};

use crate::{AccountInfo, Broker, BrokerPosition, EntryAck};

pub const MAINNET: &str = "https://api.bybit.com";
pub const DEMO: &str = "https://api-demo.bybit.com";
pub const TESTNET: &str = "https://api-testnet.bybit.com";
const RECV_WINDOW: &str = "5000";

pub struct Bybit {
    base: String,
    key: Option<String>,
    secret: Option<String>,
    http: reqwest::Client,
    /// Quantity decimals per symbol (qtyStep).
    pub qty_decimals: u32,
    pub price_decimals: u32,
}

pub fn sign(secret: &str, ts_ms: &str, key: &str, recv_window: &str, payload: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac accepts any key length");
    mac.update(format!("{ts_ms}{key}{recv_window}{payload}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Bybit kline interval string for a timeframe in minutes.
pub fn interval(tf_minutes: i64) -> Result<&'static str> {
    Ok(match tf_minutes {
        1 => "1",
        3 => "3",
        5 => "5",
        15 => "15",
        30 => "30",
        60 => "60",
        120 => "120",
        240 => "240",
        360 => "360",
        720 => "720",
        1440 => "D",
        other => bail!("unsupported Bybit interval {other} min"),
    })
}

/// Parse `result.list` of /v5/market/kline into completed bars, oldest first.
pub fn parse_klines(v: &Value, tf_minutes: i64, now: DateTime<Utc>) -> Result<Vec<Bar>> {
    let list = v.pointer("/result/list").and_then(|x| x.as_array()).ok_or_else(|| anyhow!("kline: no result.list"))?;
    let mut out = vec![];
    for row in list {
        let r = row.as_array().ok_or_else(|| anyhow!("kline row not an array"))?;
        let s = |k: usize| r.get(k).and_then(|x| x.as_str()).unwrap_or("NaN");
        let ms: i64 = s(0).parse()?;
        let ts = Utc.timestamp_millis_opt(ms).single().ok_or_else(|| anyhow!("bad kline ts"))?;
        let p = |k: usize| s(k).parse::<f64>().unwrap_or(f64::NAN);
        let bar = Bar { ts, open: p(1), high: p(2), low: p(3), close: p(4), volume: p(5) };
        if bar.is_valid() && ts + Duration::minutes(tf_minutes) <= now {
            out.push(bar);
        }
    }
    out.sort_by_key(|b| b.ts);
    out.dedup_by_key(|b| b.ts);
    Ok(out)
}

impl Bybit {
    pub fn new(base: &str, key: Option<String>, secret: Option<String>) -> Result<Self> {
        Ok(Self { base: base.trim_end_matches('/').to_string(), key, secret, http: reqwest::Client::builder().timeout(StdDuration::from_secs(15)).user_agent("q22/0.1").build()?, qty_decimals: 3, price_decimals: 1 })
    }

    /// Credentials from `Q22_BYBIT_KEY` / `Q22_BYBIT_SECRET` (public data works without them).
    pub fn from_env(base: &str) -> Result<Self> {
        Self::new(base, std::env::var("Q22_BYBIT_KEY").ok(), std::env::var("Q22_BYBIT_SECRET").ok())
    }

    fn creds(&self) -> Result<(&str, &str)> {
        match (&self.key, &self.secret) {
            (Some(k), Some(s)) => Ok((k, s)),
            _ => bail!("Bybit API key/secret missing: set Q22_BYBIT_KEY and Q22_BYBIT_SECRET"),
        }
    }

    async fn public_get(&self, path: &str, query: &str) -> Result<Value> {
        let url = format!("{}{}?{}", self.base, path, query);
        let v: Value = self.http.get(&url).send().await?.error_for_status()?.json().await?;
        check(&v, path)?;
        Ok(v)
    }

    async fn private_get(&self, path: &str, query: &str) -> Result<Value> {
        let (k, s) = self.creds()?;
        let ts = Utc::now().timestamp_millis().to_string();
        let sig = sign(s, &ts, k, RECV_WINDOW, query);
        let url = format!("{}{}?{}", self.base, path, query);
        let v: Value = self
            .http
            .get(&url)
            .header("X-BAPI-API-KEY", k)
            .header("X-BAPI-TIMESTAMP", &ts)
            .header("X-BAPI-RECV-WINDOW", RECV_WINDOW)
            .header("X-BAPI-SIGN", sig)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        check(&v, path)?;
        Ok(v)
    }

    async fn private_post(&self, path: &str, body: &Value) -> Result<Value> {
        let (k, s) = self.creds()?;
        let ts = Utc::now().timestamp_millis().to_string();
        let payload = serde_json::to_string(body)?;
        let sig = sign(s, &ts, k, RECV_WINDOW, &payload);
        let v: Value = self
            .http
            .post(format!("{}{}", self.base, path))
            .header("X-BAPI-API-KEY", k)
            .header("X-BAPI-TIMESTAMP", &ts)
            .header("X-BAPI-RECV-WINDOW", RECV_WINDOW)
            .header("X-BAPI-SIGN", sig)
            .header("Content-Type", "application/json")
            .body(payload)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        check(&v, path)?;
        Ok(v)
    }

    fn fmt_qty(&self, q: f64) -> String {
        format!("{:.*}", self.qty_decimals as usize, q)
    }
    fn fmt_px(&self, p: f64) -> String {
        format!("{:.*}", self.price_decimals as usize, p)
    }

    /// Public klines (free). Pages backwards with `end` until `since` is covered (max ~10 pages).
    pub async fn klines(&self, symbol: &str, tf_minutes: i64, since: DateTime<Utc>) -> Result<Vec<Bar>> {
        let iv = interval(tf_minutes)?;
        let now = Utc::now();
        let mut end = now.timestamp_millis();
        let mut out: Vec<Bar> = vec![];
        for _ in 0..10 {
            let q = format!("category=linear&symbol={symbol}&interval={iv}&start={}&end={end}&limit=1000", since.timestamp_millis());
            let v = self.public_get("/v5/market/kline", &q).await?;
            let page = parse_klines(&v, tf_minutes, now)?;
            let Some(first) = page.first().map(|b| b.ts) else { break };
            out.extend(page);
            if first <= since {
                break;
            }
            end = first.timestamp_millis() - 1;
        }
        out.sort_by_key(|b| b.ts);
        out.dedup_by_key(|b| b.ts);
        Ok(out.into_iter().filter(|b| b.ts >= since).collect())
    }
}

fn check(v: &Value, what: &str) -> Result<()> {
    let code = v.get("retCode").and_then(|x| x.as_i64()).unwrap_or(-1);
    if code != 0 {
        bail!("{what}: Bybit retCode {code} {}", v.get("retMsg").and_then(|x| x.as_str()).unwrap_or(""));
    }
    Ok(())
}

#[async_trait]
impl Broker for Bybit {
    fn name(&self) -> &str {
        "bybit"
    }

    async fn connect(&mut self) -> Result<AccountInfo> {
        self.account().await
    }

    async fn account(&self) -> Result<AccountInfo> {
        let v = self.private_get("/v5/account/wallet-balance", "accountType=UNIFIED").await?;
        let a = v.pointer("/result/list/0").ok_or_else(|| anyhow!("wallet-balance: empty"))?;
        let eq: f64 = a.get("totalEquity").and_then(|x| x.as_str()).unwrap_or("NaN").parse().unwrap_or(f64::NAN);
        Ok(AccountInfo { id: "bybit-unified".into(), name: "Bybit UNIFIED".into(), balance: eq, can_trade: true, simulated: Some(self.base != MAINNET) })
    }

    async fn bars(&self, symbol: &str, tf_minutes: i64, since: DateTime<Utc>) -> Result<Vec<Bar>> {
        self.klines(symbol, tf_minutes, since).await
    }

    async fn enter(&self, symbol: &str, side: Side, qty: f64, stop: f64, target: Option<f64>, tag: &str) -> Result<EntryAck> {
        let mut body = json!({
            "category": "linear", "symbol": symbol, "side": if side == Side::Long { "Buy" } else { "Sell" },
            "orderType": "Market", "qty": self.fmt_qty(qty), "positionIdx": 0,
            "stopLoss": self.fmt_px(stop), "slTriggerBy": "LastPrice", "tpslMode": "Full",
            "orderLinkId": tag
        });
        if let Some(t) = target {
            body["takeProfit"] = json!(self.fmt_px(t));
            body["tpTriggerBy"] = json!("LastPrice");
        }
        let v = self.private_post("/v5/order/create", &body).await.context("order/create")?;
        let order_id = v.pointer("/result/orderId").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let mut fill = None;
        for _ in 0..20 {
            tokio::time::sleep(StdDuration::from_millis(250)).await;
            if let Some(p) = self.positions().await?.into_iter().find(|p| p.symbol == symbol && p.qty != 0.0) {
                fill = Some(p);
                break;
            }
        }
        let p = fill.ok_or_else(|| anyhow!("entry {order_id} sent but no position after 5 s"))?;
        Ok(EntryAck { order_id, fill_price: Some(p.avg_price), filled_qty: p.qty.abs(), stop_order_id: None, target_order_id: None })
    }

    async fn modify_stop(&self, symbol: &str, _side: Side, _qty: f64, new_stop: f64) -> Result<()> {
        let body = json!({"category": "linear", "symbol": symbol, "stopLoss": self.fmt_px(new_stop), "tpslMode": "Full", "slTriggerBy": "LastPrice", "positionIdx": 0});
        self.private_post("/v5/position/trading-stop", &body).await?;
        Ok(())
    }

    async fn close(&self, symbol: &str) -> Result<Option<f64>> {
        self.cancel_all(symbol).await?;
        if let Some(p) = self.positions().await?.into_iter().find(|p| p.symbol == symbol && p.qty != 0.0) {
            let body = json!({"category": "linear", "symbol": symbol, "side": if p.qty > 0.0 { "Sell" } else { "Buy" }, "orderType": "Market", "qty": self.fmt_qty(p.qty.abs()), "reduceOnly": true, "positionIdx": 0});
            self.private_post("/v5/order/create", &body).await?;
            tokio::time::sleep(StdDuration::from_millis(500)).await;
        }
        self.last_exit_price(symbol).await
    }

    async fn positions(&self) -> Result<Vec<BrokerPosition>> {
        let v = self.private_get("/v5/position/list", "category=linear&settleCoin=USDT").await?;
        Ok(v.pointer("/result/list")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|p| {
                let size: f64 = p.get("size")?.as_str()?.parse().ok()?;
                if size == 0.0 {
                    return None;
                }
                let sign = if p.get("side")?.as_str()? == "Sell" { -1.0 } else { 1.0 };
                Some(BrokerPosition { symbol: p.get("symbol")?.as_str()?.to_string(), qty: sign * size, avg_price: p.get("avgPrice")?.as_str()?.parse().ok()? })
            })
            .collect())
    }

    async fn cancel_all(&self, symbol: &str) -> Result<()> {
        self.private_post("/v5/order/cancel-all", &json!({"category": "linear", "symbol": symbol})).await?;
        Ok(())
    }

    async fn last_exit_price(&self, symbol: &str) -> Result<Option<f64>> {
        let v = self.private_get("/v5/position/closed-pnl", &format!("category=linear&symbol={symbol}&limit=1")).await?;
        Ok(v.pointer("/result/list/0/avgExitPrice").and_then(|x| x.as_str()).and_then(|s| s.parse().ok()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_matches_openssl_reference() {
        // printf '%s' '<ts><key><recv><payload>' | openssl dgst -sha256 -hmac 'testsecret'
        assert_eq!(sign("testsecret", "1700000000000", "testkey", "5000", r#"{"category":"linear","symbol":"BTCUSDT"}"#), "f42da2d9c4b8c2d6d28353a1599894049533ee04291205fdda32dd85ed4de04b");
        assert_eq!(sign("testsecret", "1700000000000", "testkey", "5000", "category=linear&symbol=BTCUSDT"), "f2f79889fd1201752936b389c890e9d393e01e0311a6d8785fb80753aa26c69b");
    }

    #[test]
    fn klines_drop_the_forming_candle_and_sort() {
        let v: Value = serde_json::from_str(r#"{"retCode":0,"retMsg":"OK","result":{"list":[["1700003600000","101","102","100","101.5","10","1000"],["1700000000000","100","101","99","101","12","1200"]]}}"#).unwrap();
        let now = Utc.timestamp_millis_opt(1_700_003_600_000 + 30 * 60 * 1000).unwrap();
        let b = parse_klines(&v, 60, now).unwrap();
        assert_eq!(b.len(), 1, "the 1h candle opened 30 min ago is not complete");
        assert_eq!(b[0].close, 101.0);
        assert_eq!(interval(1440).unwrap(), "D");
        assert!(interval(7).is_err());
    }
}
