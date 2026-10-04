//! JSON snapshot of the engine for the dashboard. Everything a human needs to supervise the
//! automation: account vs firm rules, compliance state, regime per instrument, each strategy's
//! fit and health, the open position, pending approvals, the decision log and recent trades.

use chrono::Utc;
use serde_json::{json, Value};

use q22_core::session::{ct_minute_of_day, et_minute_of_day, fmt_hhmm};

use crate::engine::Engine;

pub fn snapshot(e: &Engine, max_bars: usize) -> Value {
    let a = &e.acct;
    let r = &a.rules;
    let now = Utc::now();
    let instruments: Vec<Value> = e
        .instruments
        .iter()
        .map(|rt| {
            let reg = rt.market.regime;
            let s = rt.market.session.as_ref();
            let pos = rt.position.as_ref().map(|p| {
                let unreal = p.unrealized(&rt.spec, rt.last_price);
                let risk = (p.entry_price - p.initial_stop).abs() * p.qty * rt.spec.point_value();
                json!({
                    "trade_id": p.trade_id, "strategy": p.strategy, "side": p.side, "qty": p.qty,
                    "entry_price": p.entry_price, "entry_time": p.entry_time, "stop": p.stop, "initial_stop": p.initial_stop,
                    "target": p.target, "unrealized": unreal, "r_now": if risk > 0.0 { unreal / risk } else { 0.0 },
                    "risk_usd": risk, "bars_held": p.bars_held, "regime_at_entry": p.regime_at_entry,
                })
            });
            let strategies: Vec<Value> = rt
                .strategies
                .iter()
                .map(|st| {
                    let h = e.allocator.health.get(st.id());
                    json!({
                        "id": st.id(), "family": st.family(), "affinity": st.affinity(reg),
                        "gated_out": e.allocator.cfg.regime_gating && st.affinity(reg) * e.allocator.health_of(st.id()) < e.allocator.cfg.min_score,
                        "health": e.allocator.health_of(st.id()),
                        "trades": h.map_or(0, |h| h.trades), "sum_r": h.map_or(0.0, |h| h.sum_r),
                        "win_rate": h.map_or(0.0, |h| if h.trades > 0 { h.wins as f64 / h.trades as f64 } else { 0.0 }),
                        "status": st.status(),
                    })
                })
                .collect();
            let bars: Vec<Value> = rt.market.recent.iter().rev().take(max_bars).rev().map(|b| json!([b.ts.timestamp(), b.open, b.high, b.low, b.close, b.volume])).collect();
            // Noise area (Zarattini et al.) at the latest bar: where price sits relative to the
            // band drives the NQ/ES confirmation filter shown in the dashboard.
            let noise = match (s, rt.market.recent.back()) {
                (Some(s), Some(last)) if rt.market.in_session(last.ts) => {
                    let offset = rt.market.offset_end(last);
                    rt.market.noise_sigma(offset).map(|sigma| {
                        let pc = s.prev_close.unwrap_or(s.open);
                        let (ub, lb) = (s.open.max(pc) * (1.0 + sigma), s.open.min(pc) * (1.0 - sigma));
                        let px = rt.last_price;
                        let state = if px > ub { "above" } else if px < lb { "below" } else { "inside" };
                        json!({"offset": offset, "sigma": sigma, "upper": ub, "lower": lb, "state": state})
                    })
                }
                _ => None,
            };
            json!({
                "symbol": rt.spec.symbol, "description": rt.spec.description, "broker_symbol": rt.broker_symbol,
                "timeframe_min": rt.tf_minutes, "last_price": if rt.last_price.is_finite() { json!(rt.last_price) } else { Value::Null },
                "last_bar_end": rt.last_bar_end, "point_value": rt.spec.point_value(), "tick_size": rt.spec.tick_size,
                "regime": reg, "regime_label": reg.label(), "features": rt.market.features,
                "session": s.map(|s| json!({"date": s.date, "open": s.open, "high": s.high, "low": s.low, "vwap": s.vwap(), "vwap_std": s.vwap_std(), "prev_close": s.prev_close, "bars": s.bars})),
                "position": pos, "strategies": strategies, "bars": bars, "noise": noise,
            })
        })
        .collect();
    let trades: Vec<&crate::engine::ClosedTrade> = e.trades.iter().rev().take(300).collect();
    let n = e.trades.len();
    let wins = e.trades.iter().filter(|t| t.pnl_net > 0.0).count();
    let net: f64 = e.trades.iter().map(|t| t.pnl_net).sum();
    let g = &e.guard;
    json!({
        "ts": now,
        "et_time": fmt_hhmm(et_minute_of_day(now)),
        "ct_time": fmt_hhmm(ct_minute_of_day(now)),
        "mode": e.cfg.mode, "paused": e.paused, "live": e.live,
        "account": {
            "rules": r.name, "firm": r.firm, "status": a.status, "automation": r.automation,
            "start_balance": a.start_balance, "balance": a.balance, "equity": a.equity, "threshold": a.threshold,
            "buffer": a.buffer(), "max_loss": r.max_loss, "buffer_frac": a.buffer() / r.max_loss,
            "day": a.day, "day_pnl": a.day_pnl(), "daily_loss": a.daily_loss(),
            "daily_loss_stop": g.daily_loss_stop(a), "firm_daily_loss_limit": r.daily_loss_limit, "day_halted": a.day_halted,
            "profit_target": r.profit_target, "total_profit": a.total_profit(),
            "target_progress": r.profit_target.map(|t| a.total_profit() / t),
            "best_day": a.best_day(), "consistency_share": r.consistency_share, "consistency_mode": r.consistency_mode,
            "consistency_ok": a.consistency_ok(), "profit_lock": g.profit_lock(a),
            "trading_days": a.trading_days(), "min_trading_days": r.min_trading_days,
            "flat_by_ct": r.flat_by_ct, "overnight_allowed": r.overnight_allowed,
            "stop_required_within_s": r.stop_required_within_s, "max_risk_per_trade_pct": r.max_risk_per_trade_pct,
            "max_contracts_mini": r.max_contracts_mini, "vps_vpn_prohibited": r.vps_vpn_prohibited, "hft_prohibited": r.hft_prohibited,
            "notes": r.notes, "sources": r.sources,
        },
        "guard": {
            "trades_today": g.day.trades, "max_trades_per_day": g.cfg.max_trades_per_day,
            "consecutive_losses": g.day.consecutive_losses, "max_consecutive_losses": g.cfg.max_consecutive_losses,
            "day_halted": g.day.halted, "profit_locked": g.day.profit_locked, "account_halted": g.account_halted,
            "entry_window_et": [g.cfg.entry_start_et, g.cfg.entry_end_et], "flatten_at_et": fmt_hhmm(g.flatten_minute_et()),
            "max_orders_per_minute": g.cfg.max_orders_per_minute, "min_hold_seconds": g.cfg.min_hold_seconds,
            "heartbeat_required_s": g.cfg.operator_heartbeat_s,
            "heartbeat_age_s": g.last_heartbeat.map(|t| (now - t).num_seconds()),
            "next_news": g.cfg.news_events.iter().filter(|t| **t >= now).min(),
            "risk_per_trade_usd": g.cfg.risk_per_trade_usd, "risk_per_trade_frac": g.cfg.risk_per_trade_frac,
        },
        "allocator": e.allocator.cfg,
        "instruments": instruments,
        "approvals": e.approvals,
        "block_counts": e.block_counts,
        "events": e.events.iter().rev().take(300).collect::<Vec<_>>(),
        "trades": trades,
        "stats": { "trades": n, "win_rate": if n > 0 { wins as f64 / n as f64 } else { 0.0 }, "net": net },
        "equity": e.intraday_equity.iter().rev().take(3000).rev().map(|(t, v)| json!([t.timestamp(), v])).collect::<Vec<_>>(),
        "daily_equity": e.daily_equity.iter().rev().take(400).rev().collect::<Vec<_>>(),
    })
}
