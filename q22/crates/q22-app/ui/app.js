// q22 dashboard — vanilla JS, no build step. All untrusted strings go through textContent.
'use strict';
(() => {
  const LWC = window.LightweightCharts;
  const $ = (id) => document.getElementById(id);

  // ---------- token (from ?token=…, kept in localStorage, removed from the address bar)
  const qs = new URLSearchParams(location.search);
  if (qs.get('token')) {
    try { localStorage.setItem('q22.token', qs.get('token')); } catch (_) {}
    history.replaceState(null, '', location.pathname);
  }
  let TOKEN = '';
  try { TOKEN = localStorage.getItem('q22.token') || ''; } catch (_) {}
  const api = (path, opts = {}) => fetch(path, { ...opts, headers: { 'Content-Type': 'application/json', Authorization: 'Bearer ' + TOKEN, ...(opts.headers || {}) } });
  const control = (action, id) => api('/api/control', { method: 'POST', body: JSON.stringify({ action, id }) });

  // ---------- helpers
  function el(tag, attrs = {}, ...kids) {
    const n = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (v == null) continue;
      if (k === 'class') n.className = v;
      else if (k === 'style') n.setAttribute('style', v);
      else if (k.startsWith('on')) n.addEventListener(k.slice(2), v);
      else n.setAttribute(k, v);
    }
    for (const c of kids.flat()) if (c != null) n.append(c instanceof Node ? c : document.createTextNode(String(c)));
    return n;
  }
  // Intl with the browser locale, falling back to en-US when the platform locale is unusual.
  const NF = new Map();
  function nf(d) {
    if (!NF.has(d)) {
      let f;
      try { f = new Intl.NumberFormat(undefined, { minimumFractionDigits: d, maximumFractionDigits: d }); f.format(1); } catch (_) { f = new Intl.NumberFormat('en-US', { minimumFractionDigits: d, maximumFractionDigits: d }); }
      NF.set(d, f);
    }
    return NF.get(d);
  }
  const fmt = (x, d = 2) => (x == null || !isFinite(x)) ? '—' : nf(d).format(Number(x));
  const money = (x, d = 0) => (x == null || !isFinite(x)) ? '—' : (x < 0 ? '−$' : '$') + fmt(Math.abs(x), d);
  const signed = (x, d = 0) => (x == null || !isFinite(x)) ? '—' : (x > 0 ? '+' : x < 0 ? '−' : '') + '$' + fmt(Math.abs(x), d);
  const pct = (x, d = 0) => (x == null || !isFinite(x)) ? '—' : fmt(x * 100, d) + '%';
  const cls = (x) => x > 0 ? 'pos' : x < 0 ? 'neg' : '';
  const css = (v) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();
  // "YYYY-MM-DD HH:MM:SS" in New York time (decision log, trades) — same clock as the charts
  const ET_PARTS = new Intl.DateTimeFormat('en-US', { timeZone: 'America/New_York', year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false });
  const tsLabel = (t) => {
    const d = new Date(t);
    if (isNaN(d)) return String(t);
    const p = Object.fromEntries(ET_PARTS.formatToParts(d).map((x) => [x.type, x.value]));
    return `${p.year}-${p.month}-${p.day} ${p.hour === '24' ? '00' : p.hour}:${p.minute}:${p.second}`;
  };
  const STRAT_COLORS = ['--series-1', '--series-2', '--series-3', '--series-4', '--series-5'];
  const stratColor = new Map();
  function colorFor(id) { if (!stratColor.has(id)) stratColor.set(id, STRAT_COLORS[stratColor.size % STRAT_COLORS.length]); return `var(${stratColor.get(id)})`; }
  function setPill(id, level, text) { const p = $(id); p.className = 'pill ' + (level || ''); p.lastElementChild.textContent = text; }
  function meter(frac, level) { const m = el('div', { class: 'meter ' + (level || '') }); m.append(el('span', { style: `width:${Math.max(0, Math.min(1, frac || 0)) * 100}%` })); return m; }

  // ---------- theme
  $('btn-theme').addEventListener('click', () => {
    const cur = document.documentElement.getAttribute('data-theme') || (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
    const next = cur === 'dark' ? 'light' : 'dark';
    document.documentElement.setAttribute('data-theme', next);
    try { localStorage.setItem('q22.theme', next); } catch (_) {}
    applyChartTheme();
  });
  try { const t = localStorage.getItem('q22.theme'); if (t) document.documentElement.setAttribute('data-theme', t); } catch (_) {}

  // ---------- tabs
  document.querySelectorAll('nav.tabs button').forEach((b) => b.addEventListener('click', () => {
    document.querySelectorAll('nav.tabs button').forEach((x) => x.setAttribute('aria-selected', String(x === b)));
    for (const t of ['live', 'backtests', 'rules']) $('tab-' + t).classList.toggle('hidden', b.dataset.tab !== t);
    if (b.dataset.tab === 'backtests') loadReports();
    if (b.dataset.tab === 'live') { priceChart?.timeScale().fitContent(); }
  }));

  // ---------- charts
  let priceChart, candles, vwapSeries, markersApi, equityChart, equitySeries, priceLines = [];
  const LOCALE = (() => { try { new Intl.NumberFormat(navigator.language).format(1); new Date().toLocaleString(navigator.language); return navigator.language; } catch (_) { return 'en-US'; } })();
  // Intraday times are shown in New York time — the clock CME sessions and prop rules use.
  const etFmt = (opts) => { try { return new Intl.DateTimeFormat(LOCALE, { timeZone: 'America/New_York', ...opts }); } catch (_) { return new Intl.DateTimeFormat('en-US', { timeZone: 'America/New_York', ...opts }); } };
  const ET_HM = etFmt({ hour: '2-digit', minute: '2-digit', hour12: false });
  const ET_DAY = etFmt({ month: 'short', day: 'numeric' });
  const ET_FULL = etFmt({ year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false });
  function chartOptions() {
    return {
      autoSize: true,
      localization: { locale: LOCALE, timeFormatter: (t) => (typeof t === 'number' ? ET_FULL.format(new Date(t * 1000)) + ' ET' : String(typeof t === 'object' ? `${t.year}-${t.month}-${t.day}` : t)) },
      layout: { background: { color: css('--surface-1') }, textColor: css('--text-secondary'), fontSize: 11, attributionLogo: true },
      grid: { vertLines: { color: css('--grid') }, horzLines: { color: css('--grid') } },
      rightPriceScale: { borderColor: css('--border') },
      timeScale: {
        borderColor: css('--border'), timeVisible: true, secondsVisible: false,
        tickMarkFormatter: (t, type) => (typeof t === 'number' ? (type <= 2 ? ET_DAY : ET_HM).format(new Date(t * 1000)) : null),
      },
      crosshair: { mode: LWC.CrosshairMode.Normal },
    };
  }
  function initCharts() {
    priceChart = LWC.createChart($('price-chart'), chartOptions());
    candles = priceChart.addSeries(LWC.CandlestickSeries, {});
    vwapSeries = priceChart.addSeries(LWC.LineSeries, { lineWidth: 2, priceLineVisible: false, lastValueVisible: false, crosshairMarkerVisible: false });
    markersApi = LWC.createSeriesMarkers(candles, []);
    priceChart.subscribeCrosshairMove((p) => {
      const lg = $('price-legend'); lg.textContent = '';
      const d = p && p.seriesData ? p.seriesData.get(candles) : null;
      if (!d) return;
      const v = p.seriesData.get(vwapSeries);
      lg.append('O ', el('b', {}, fmt(d.open)), '  H ', el('b', {}, fmt(d.high)), '  L ', el('b', {}, fmt(d.low)), '  C ', el('b', {}, fmt(d.close)));
      if (v) lg.append('   VWAP ', el('b', {}, fmt(v.value)));
    });
    equityChart = LWC.createChart($('equity-chart'), chartOptions());
    equitySeries = equityChart.addSeries(LWC.LineSeries, { lineWidth: 2, priceLineVisible: false });
    equityChart.subscribeCrosshairMove((p) => {
      const lg = $('equity-legend'); lg.textContent = '';
      const d = p && p.seriesData ? p.seriesData.get(equitySeries) : null;
      if (d) lg.append('equity ', el('b', {}, money(d.value, 2)));
    });
    applyChartTheme();
  }
  function applyChartTheme() {
    if (!priceChart) return;
    const o = chartOptions();
    priceChart.applyOptions(o); equityChart.applyOptions(o);
    const up = css('--series-1'), down = css('--down');
    candles.applyOptions({ upColor: up, downColor: down, borderUpColor: up, borderDownColor: down, wickUpColor: up, wickDownColor: down });
    vwapSeries.applyOptions({ color: css('--series-2') });
    equitySeries.applyOptions({ color: css('--series-1') });
    for (const c of bt.charts) c.applyOptions(o);
  }

  // ---------- state rendering
  let state = null, selectedSymbol = null, lastBarsKey = '';
  function statusOf(s) {
    const st = s.account.status;
    if (typeof st === 'object' && st.Failed) return ['critical', 'FAILED — ' + st.Failed.reason];
    if (typeof st === 'object' && st.Passed) return ['good', 'PASSED ' + st.Passed.date];
    if (s.guard.account_halted) return ['critical', 'HALTED'];
    if (s.guard.day_halted || s.account.day_halted) return ['serious', 'DAY HALTED'];
    if (s.paused) return ['warning', 'PAUSED'];
    return ['good', 'ACTIVE'];
  }

  function renderHeader(s) {
    const mode = String(s.mode || '—').toUpperCase();
    setPill('mode', mode === 'AUTO' ? 'good' : 'warning', mode + (s.live ? ' · LIVE' : ' · PAPER'));
    const [lvl, txt] = statusOf(s); setPill('status', lvl, txt);
    $('clock').textContent = `ET ${s.et_time} · CT ${s.ct_time}`;
    $('btn-pause').textContent = s.paused ? 'Resume' : 'Pause';
    const m = s.meta || {};
    const warnings = [...(m.warnings || [])];
    const hb = s.guard.heartbeat_required_s;
    if (s.live && hb && (s.guard.heartbeat_age_s == null || s.guard.heartbeat_age_s > hb)) warnings.unshift('No operator heartbeat — new entries are blocked until this page is visible.');
    const b = $('banner'); b.textContent = '';
    if (warnings.length) { b.append(el('strong', {}, 'Heads-up'), el('ul', {}, warnings.map((w) => el('li', {}, w)))); }
    b.classList.toggle('show', warnings.length > 0);
  }

  function kpi(label, value, sub, extra, valueClass) {
    return el('div', { class: 'kpi' }, el('div', { class: 'label' }, label), el('div', { class: 'value ' + (valueClass || '') }, value), sub ? el('div', { class: 'sub' }, sub) : null, extra || null);
  }

  function renderKpis(s) {
    const a = s.account, g = s.guard, k = $('kpis'); k.textContent = '';
    const bufFrac = a.buffer_frac;
    k.append(kpi('Balance', money(a.balance, 2), `${a.firm}`));
    k.append(kpi('Day P&L', signed(a.day_pnl, 2), `equity ${money(a.equity, 2)}`, null, cls(a.day_pnl)));
    k.append(kpi('Buffer to max-loss', money(a.buffer), `threshold ${money(a.threshold)} · ${pct(bufFrac)} of $${fmt(a.max_loss, 0)}`, meter(bufFrac, bufFrac < 0.25 ? 'critical' : bufFrac < 0.5 ? 'warning' : 'good')));
    const dlFrac = a.daily_loss_stop ? a.daily_loss / a.daily_loss_stop : 0;
    k.append(kpi('Daily loss used', money(a.daily_loss), `your stop ${money(a.daily_loss_stop)}${a.firm_daily_loss_limit ? ' · firm ' + money(a.firm_daily_loss_limit) : ''}`, meter(dlFrac, dlFrac > 0.75 ? 'critical' : dlFrac > 0.5 ? 'warning' : 'good')));
    if (a.profit_target) {
      k.append(kpi('Target progress', pct(a.target_progress), `${signed(a.total_profit)} of ${money(a.profit_target)} · ${a.trading_days} days`, meter(a.target_progress, 'good')));
    } else {
      k.append(kpi('Total profit', signed(a.total_profit), `${a.trading_days} trading days`));
    }
    const capTxt = a.profit_lock ? `today locks at ${money(a.profit_lock)}` : (a.consistency_share ? `best day ≤ ${pct(a.consistency_share)} of total` : 'no consistency rule');
    k.append(kpi('Consistency', a.consistency_ok ? 'OK' : 'BREACHED', `best day ${money(a.best_day)} · ${capTxt}`, null, a.consistency_ok ? 'pos' : 'neg'));
    k.append(kpi('Trades today', `${g.trades_today} / ${g.max_trades_per_day}`, `losses in a row ${g.consecutive_losses}/${g.max_consecutive_losses} · flatten ${g.flatten_at_et} ET`));
  }

  function currentInstrument(s) {
    const sel = $('symbol-select');
    const syms = s.instruments.map((i) => i.symbol);
    if (sel.options.length !== syms.length) { sel.textContent = ''; syms.forEach((x) => sel.append(el('option', { value: x }, x))); }
    if (!selectedSymbol || !syms.includes(selectedSymbol)) selectedSymbol = syms[0];
    sel.value = selectedSymbol;
    return s.instruments.find((i) => i.symbol === selectedSymbol);
  }
  $('symbol-select').addEventListener('change', (e) => { selectedSymbol = e.target.value; lastBarsKey = ''; if (state) render(state); });

  function renderChart(s, inst) {
    $('chart-title').textContent = `${inst.symbol} · ${inst.timeframe_min}-min`;
    $('chart-sub').textContent = inst.last_price != null ? `last ${fmt(inst.last_price)}` : '';
    const bars = inst.bars || [];
    const key = inst.symbol + ':' + bars.length + ':' + (bars.length ? bars[bars.length - 1][0] + ':' + bars[bars.length - 1][4] : '');
    if (key !== lastBarsKey) {
      const first = lastBarsKey === '' || !lastBarsKey.startsWith(inst.symbol + ':');
      lastBarsKey = key;
      candles.setData(bars.map((b) => ({ time: b[0], open: b[1], high: b[2], low: b[3], close: b[4] })));
      // session VWAP recomputed client-side from the visible session bars (display only)
      const vw = []; let pv = 0, v = 0, day = null;
      for (const b of bars) {
        const d = new Date(b[0] * 1000).toISOString().slice(0, 10);
        if (d !== day) { day = d; pv = 0; v = 0; }
        const w = b[5] > 0 ? b[5] : 1; pv += ((b[2] + b[3] + b[4]) / 3) * w; v += w; vw.push({ time: b[0], value: pv / v });
      }
      vwapSeries.setData(vw);
      const t0 = bars.length ? bars[0][0] : 0;
      const marks = [];
      for (const t of (s.trades || []).filter((x) => x.symbol === inst.symbol)) {
        const et = Math.floor(new Date(t.entry_time).getTime() / 1000), xt = Math.floor(new Date(t.exit_time).getTime() / 1000);
        if (et >= t0) marks.push({ time: snap(bars, et), position: t.side === 'Long' ? 'belowBar' : 'aboveBar', shape: t.side === 'Long' ? 'arrowUp' : 'arrowDown', color: css('--text-secondary'), text: t.strategy });
        if (xt >= t0) marks.push({ time: snap(bars, xt), position: t.side === 'Long' ? 'aboveBar' : 'belowBar', shape: 'circle', color: t.pnl_net >= 0 ? css('--good') : css('--critical'), text: signed(t.pnl_net) });
      }
      marks.sort((a, b) => a.time - b.time);
      markersApi.setMarkers(marks);
      if (first) priceChart.timeScale().fitContent();
    }
    for (const pl of priceLines) candles.removePriceLine(pl);
    priceLines = [];
    const p = inst.position;
    if (p) {
      priceLines.push(candles.createPriceLine({ price: p.entry_price, color: css('--text-secondary'), lineWidth: 1, lineStyle: 2, title: `entry ${p.side}` }));
      priceLines.push(candles.createPriceLine({ price: p.stop, color: css('--critical'), lineWidth: 2, lineStyle: 0, title: 'stop' }));
      if (p.target != null) priceLines.push(candles.createPriceLine({ price: p.target, color: css('--good'), lineWidth: 2, lineStyle: 0, title: 'target' }));
    }
  }
  function snap(bars, t) { let best = bars.length ? bars[0][0] : t; for (const b of bars) { if (b[0] <= t) best = b[0]; else break; } return best; }

  function renderPosition(inst) {
    const box = $('position'); box.textContent = '';
    const p = inst.position;
    if (!p) { box.append(el('div', { class: 'muted' }, 'Flat.')); return; }
    box.append(
      el('div', { class: 'regime' }, el('span', { class: 'tag' }, p.side.toUpperCase()), `${fmt(p.qty, p.qty % 1 ? 3 : 0)} ${inst.symbol}`, el('span', { class: cls(p.unrealized) }, signed(p.unrealized, 2))),
      el('dl', { class: 'kv' },
        el('dt', {}, 'strategy'), el('dd', {}, el('span', { class: 'chip' }, el('i', { style: `background:${colorFor(p.strategy)}` }), p.strategy)),
        el('dt', {}, 'entry'), el('dd', {}, fmt(p.entry_price)),
        el('dt', {}, 'stop (initial)'), el('dd', {}, `${fmt(p.stop)} (${fmt(p.initial_stop)})`),
        el('dt', {}, 'target'), el('dd', {}, p.target != null ? fmt(p.target) : 'session exit'),
        el('dt', {}, 'risk / R now'), el('dd', {}, `${money(p.risk_usd)} / ${fmt(p.r_now)}R`),
        el('dt', {}, 'regime at entry'), el('dd', {}, String(p.regime_at_entry)),
        el('dt', {}, 'bars held'), el('dd', {}, String(p.bars_held)),
      ));
  }

  const REGIME_LEVEL = { Shock: 'critical', Volatile: 'warning', TrendingUp: 'good', TrendingDown: 'good', Ranging: '', Neutral: '', Unknown: '' };
  function renderRegime(inst) {
    const box = $('regime'); box.textContent = '';
    const f = inst.features || {};
    box.append(el('div', { class: 'regime' }, el('span', { class: 'pill ' + (REGIME_LEVEL[inst.regime] || '') }, el('span', { class: 'dot' }), el('span', {}, inst.regime_label))));
    const s = inst.session || {};
    box.append(el('dl', { class: 'kv', style: 'margin-top:10px' },
      el('dt', {}, 'ADX(14) daily'), el('dd', {}, fmt(f.adx_d, 1)),
      el('dt', {}, 'efficiency ratio (10d)'), el('dd', {}, fmt(f.er_d, 2)),
      el('dt', {}, 'ATR(14) daily'), el('dd', {}, `${fmt(f.atr_d)} (${pct(f.atr_pct_of_price, 2)})`),
      el('dt', {}, 'volatility percentile (1y)'), el('dd', {}, pct(f.vol_percentile)),
      el('dt', {}, 'opening gap'), el('dd', {}, f.gap_atr != null ? fmt(f.gap_atr, 2) + ' ATR' : '—'),
      el('dt', {}, 'session open / VWAP'), el('dd', {}, `${fmt(s.open)} / ${fmt(s.vwap)}`),
      el('dt', {}, 'prev close'), el('dd', {}, fmt(s.prev_close)),
      el('dt', {}, 'days of history'), el('dd', {}, String(f.days ?? 0)),
    ));
  }

  function renderStrategies(inst) {
    const box = $('strategies'); box.textContent = '';
    const t = el('table');
    t.append(el('thead', {}, el('tr', {}, el('th', {}, 'strategy'), el('th', { class: 'num' }, 'fit'), el('th', { class: 'num' }, 'health'), el('th', { class: 'num' }, 'trades'), el('th', { class: 'num' }, 'win'), el('th', { class: 'num' }, 'ΣR'), el('th', {}, 'state'))));
    const tb = el('tbody');
    for (const s of inst.strategies) {
      const st = s.status && typeof s.status === 'object' ? Object.entries(s.status).filter(([, v]) => v != null).map(([k, v]) => `${k}=${typeof v === 'number' ? fmt(v, 2) : v}`).join(' ') : '';
      tb.append(el('tr', {},
        el('td', {}, el('span', { class: 'chip', title: s.family }, el('i', { style: `background:${colorFor(s.id)}` }), s.id), s.gated_out ? el('span', { class: 'tag', style: 'margin-left:6px' }, 'gated out') : null),
        el('td', { class: 'num' }, fmt(s.affinity, 2)), el('td', { class: 'num' }, fmt(s.health, 2)), el('td', { class: 'num' }, String(s.trades)),
        el('td', { class: 'num' }, s.trades ? pct(s.win_rate) : '—'), el('td', { class: 'num ' + cls(s.sum_r) }, fmt(s.sum_r, 2)),
        el('td', { class: 'state', title: st }, st)));
    }
    t.append(tb); box.append(el('div', { class: 'tablewrap' }, t));
  }

  function renderApprovals(s) {
    const card = $('approvals-card'), box = $('approvals'); box.textContent = '';
    const list = s.approvals || [];
    card.classList.toggle('hidden', s.mode !== 'assist' && list.length === 0);
    if (!list.length) { box.append(el('div', { class: 'muted' }, 'Nothing pending.')); return; }
    for (const a of list) {
      const left = Math.max(0, Math.round((new Date(a.expires) - Date.now()) / 1000));
      box.append(el('div', { class: 'approval' },
        el('div', {}, el('strong', {}, `#${a.id}`), ' ', a.explanation),
        el('div', { class: 'row' },
          el('button', { class: 'btn primary', onclick: () => control('approve', a.id) }, `Approve (risk ${money(a.risk_usd)})`),
          el('button', { class: 'btn', onclick: () => control('reject', a.id) }, 'Reject'),
          el('span', { class: 'muted' }, `expires in ${left}s`))));
    }
  }

  const LOG_GROUPS = { trading: ['entry', 'fill', 'win', 'loss', 'exit', 'manage', 'signal', 'approve'], blocks: ['block', 'skip'], risk: ['halt', 'fail', 'flatten', 'warn'], regime: ['regime'] };
  $('log-filter').addEventListener('change', () => state && renderLog(state));
  $('log-search').addEventListener('input', () => state && renderLog(state));
  function renderLog(s) {
    const box = $('log'); box.textContent = '';
    const f = $('log-filter').value, q = $('log-search').value.toLowerCase();
    let n = 0;
    for (const e of s.events || []) {
      if (f !== 'all' && !(LOG_GROUPS[f] || []).includes(e.level)) continue;
      if (q && !(e.msg.toLowerCase().includes(q) || (e.symbol || '').toLowerCase().includes(q))) continue;
      box.append(el('div', {}, el('span', { class: 'muted' }, tsLabel(e.ts)), el('span', { class: 'lvl ' + e.level }, e.level), el('span', {}, (e.symbol ? e.symbol + ' · ' : '') + e.msg)));
      if (++n > 250) break;
    }
    if (!n) box.append(el('div', { class: 'muted' }, 'No events yet.'));
  }

  function tradesTable(trades) {
    const t = el('table');
    t.append(el('thead', {}, el('tr', {}, ['#', 'symbol', 'strategy', 'regime', 'side', 'qty', 'entry', 'exit', 'reason', 'net', 'R', 'held'].map((h, i) => el('th', { class: i >= 5 && i !== 8 ? 'num' : '' }, h)))));
    const tb = el('tbody');
    for (const x of trades) {
      tb.append(el('tr', {},
        el('td', {}, String(x.trade_id)), el('td', {}, x.symbol),
        el('td', {}, el('span', { class: 'chip' }, el('i', { style: `background:${colorFor(x.strategy)}` }), x.strategy)),
        el('td', { class: 'muted' }, String(x.regime)), el('td', {}, x.side),
        el('td', { class: 'num' }, fmt(x.qty, x.qty % 1 ? 3 : 0)),
        el('td', { class: 'num' }, `${tsLabel(x.entry_time).slice(5, 16)} @ ${fmt(x.entry_price)}`),
        el('td', { class: 'num' }, `${tsLabel(x.exit_time).slice(11, 16)} @ ${fmt(x.exit_price)}`),
        el('td', { class: 'muted' }, x.exit_reason),
        el('td', { class: 'num ' + cls(x.pnl_net) }, signed(x.pnl_net, 2)),
        el('td', { class: 'num ' + cls(x.r_multiple) }, fmt(x.r_multiple, 2)),
        el('td', { class: 'num' }, String(x.bars_held))));
    }
    t.append(tb);
    return t;
  }

  function renderTrades(s) {
    const box = $('trades'); box.replaceWith(Object.assign(tradesTable(s.trades || []), { id: 'trades' }));
    $('trades-sub').textContent = `${s.stats.trades} trades · win ${pct(s.stats.win_rate)} · net ${signed(s.stats.net, 2)}`;
  }

  let lastEqKey = '';
  // One point per trading day (end-of-day balance + open P&L), plus today's live equity.
  function renderEquity(s) {
    const days = s.daily_equity || [];
    const today = s.account.day;
    const key = days.length + ':' + today + ':' + s.account.equity;
    if (key === lastEqKey) return;
    lastEqKey = key;
    const pts = days.map(([d, v]) => ({ time: d, value: v }));
    if (today && (!pts.length || pts[pts.length - 1].time < today)) pts.push({ time: today, value: s.account.equity });
    equitySeries.setData(pts);
    if (pts.length < 3) equityChart.timeScale().fitContent();
  }

  function renderRules(s) {
    const a = s.account, g = s.guard;
    const ck = $('checklist'); ck.textContent = '';
    const item = (ok, text) => el('div', { style: 'display:flex;gap:8px;padding:4px 0' }, el('span', { class: 'pill ' + (ok === true ? 'good' : ok === false ? 'critical' : 'warning') }, el('span', { class: 'dot' }), el('span', {}, ok === true ? 'ok' : ok === false ? 'no' : 'check')), el('span', {}, text));
    ck.append(
      item(a.automation === 'Full' || s.mode !== 'auto' || !s.live, `automation allowed by the firm in this mode (${a.automation}, mode ${s.mode})`),
      item(null, a.vps_vpn_prohibited ? 'running on your own computer — VPS/VPN/remote servers are prohibited by this firm' : 'hosting: no VPS restriction listed for this firm'),
      item(!s.live || !g.heartbeat_required_s || (g.heartbeat_age_s != null && g.heartbeat_age_s <= g.heartbeat_required_s), !s.live ? 'actively monitored: not required in paper / replay mode' : `actively monitored: operator heartbeat ${g.heartbeat_age_s == null ? 'not received' : g.heartbeat_age_s + 's ago'}`),
      item(true, `every entry carries a resting stop${a.stop_required_within_s ? ` (firm requires within ${a.stop_required_within_s}s)` : ''}`),
      item(true, `anti-HFT: ≤ ${g.max_orders_per_minute} order actions per minute; min hold ${g.min_hold_seconds}s before discretionary exits`),
      item(true, `flat by ${g.flatten_at_et} ET${a.flat_by_ct ? ` (firm: ${a.flat_by_ct} CT)` : ''}; overnight ${a.overnight_allowed ? 'allowed' : 'never'}`),
      item(a.consistency_ok, `consistency rule ${a.consistency_share ? pct(a.consistency_share) : 'n/a'} — best day ${money(a.best_day)}`),
      item(true, 'one net direction per correlated group (no hedging NQ vs ES)'),
      item(g.next_news == null ? null : true, g.next_news ? `next news blackout ${tsLabel(g.next_news)}` : 'no news calendar loaded (add [guard] news_events)'),
    );
    const kv = (o) => el('dl', { class: 'kv' }, Object.entries(o).flatMap(([k, v]) => [el('dt', {}, k), el('dd', {}, v == null ? '—' : typeof v === 'object' ? JSON.stringify(v) : String(v))]));
    $('rules').textContent = '';
    $('rules').append(kv({ preset: a.rules, firm: a.firm, 'account size': money(a.start_balance), 'profit target': money(a.profit_target), 'max loss': money(a.max_loss), 'current threshold': money(a.threshold, 2), 'daily loss limit': money(a.firm_daily_loss_limit), consistency: a.consistency_share ? `${pct(a.consistency_share)} (${a.consistency_mode})` : 'none', 'min trading days': a.min_trading_days, 'max contracts (minis)': a.max_contracts_mini, 'flat by (CT)': a.flat_by_ct, 'max risk / trade': a.max_risk_per_trade_pct ? pct(a.max_risk_per_trade_pct, 1) : '—' }));
    $('guard').textContent = '';
    $('guard').append(kv({ 'entry window ET': g.entry_window_et.join('–'), 'flatten at ET': g.flatten_at_et, 'your daily loss stop': money(a.daily_loss_stop), 'profit lock (consistency)': money(a.profit_lock), 'max trades / day': g.max_trades_per_day, 'max consecutive losses': g.max_consecutive_losses, 'risk per trade': g.risk_per_trade_usd ? money(g.risk_per_trade_usd) : pct(g.risk_per_trade_frac, 2) + ' equity', 'regime gating': s.allocator.regime_gating, 'adaptive health': s.allocator.adaptive_health, 'min allocation score': s.allocator.min_score }));
    const n = $('notes'); n.textContent = '';
    n.append(el('ul', {}, (a.notes || []).map((x) => el('li', {}, x))));
    n.append(el('div', { class: 'muted' }, 'Sources:'), el('ul', {}, (a.sources || []).map((u) => el('li', {}, el('a', { href: u, target: '_blank', rel: 'noopener' }, u)))));
  }

  // ---------- NQ ↔ ES confirmation monitor (needs ≥ 2 index instruments)
  const div = { chart: null, series: new Map(), key: '' };
  const SERIES_VARS = ['--series-1', '--series-2', '--series-3', '--series-4'];
  const NOISE_WORD = { above: 'above the noise area', below: 'below the noise area', inside: 'inside the noise area' };
  function renderDivergence(s) {
    const insts = (s.instruments || []).filter((x) => x.session && x.session.open);
    const card = $('divergence-card');
    card.classList.toggle('hidden', insts.length < 2);
    if (insts.length < 2) return;
    // summary: both outside on the same side = confirmed, one outside alone = divergence
    const st = insts.map((x) => (x.noise ? x.noise.state : null));
    const [a, b] = [insts[0].symbol, insts[1].symbol];
    let level = '', text;
    if (st.some((x) => x == null)) text = 'Noise area warming up (needs 10 sessions of history at this time of day).';
    else if (st[0] === st[1] && st[0] !== 'inside') { level = 'good'; text = `Confirmed ${st[0] === 'above' ? 'up' : 'down'}-breakout: ${a} and ${b} are both ${NOISE_WORD[st[0]]} — breakouts in this direction can be taken.`; }
    else if (st[0] === 'inside' && st[1] === 'inside') text = `${a} and ${b} are both inside their noise area — no signal.`;
    else if (st[0] !== 'inside' && st[1] !== 'inside') { level = 'warning'; text = `Opposite breakouts (${a} ${st[0]}, ${b} ${st[1]}) — no trade.`; }
    else { level = 'warning'; const lead = st[0] !== 'inside' ? 0 : 1; text = `Divergence: ${insts[lead].symbol} is ${NOISE_WORD[st[lead]]} but ${insts[1 - lead].symbol} is not — ${insts[lead].symbol}'s breakout is skipped (unconfirmed breakouts lost money in the 2010–2026 tests).`; }
    const sum = $('div-summary'); sum.textContent = '';
    sum.append(el('span', { class: 'pill ' + level }, el('span', { class: 'dot' }), el('span', {}, level === 'good' ? 'confirmed' : level === 'warning' ? 'divergent' : 'no signal')), ' ', text);
    // table
    const t = el('table');
    t.append(el('thead', {}, el('tr', {}, ['index', 'since open', 'vs noise area', 'band (low – high)', 'vs VWAP'].map((h, i) => el('th', { class: i === 1 ? 'num' : '' }, h)))));
    const tb = el('tbody');
    insts.forEach((x, i) => {
      const px = x.last_price, o = x.session.open;
      const ch = px != null && o ? (px / o - 1) : null;
      const vw = x.session.vwap;
      tb.append(el('tr', {},
        el('td', {}, el('span', { class: 'chip' }, el('i', { style: `background:var(${SERIES_VARS[i % SERIES_VARS.length]})` }), x.symbol)),
        el('td', { class: 'num ' + cls(ch) }, ch == null ? '—' : (ch >= 0 ? '+' : '') + fmt(ch * 100, 2) + '%'),
        el('td', {}, x.noise ? NOISE_WORD[x.noise.state] : '—'),
        el('td', {}, x.noise ? `${fmt(x.noise.lower, 2)} – ${fmt(x.noise.upper, 2)}` : '—'),
        el('td', {}, vw == null || px == null ? '—' : px >= vw ? 'above' : 'below')));
    });
    t.append(tb);
    const box = $('div-table'); box.textContent = ''; box.append(el('div', { class: 'tablewrap' }, t));
    // chart: % move since the session open, one line per index (legend + direct colours)
    if (!div.chart) {
      div.chart = LWC.createChart($('div-chart'), chartOptions());
      div.chart.subscribeCrosshairMove((p) => {
        const lg = $('div-legend'); lg.textContent = '';
        if (!p || !p.seriesData) return;
        for (const [sym, ser] of div.series) { const d = p.seriesData.get(ser); if (d) lg.append(sym + ' ', el('b', {}, (d.value >= 0 ? '+' : '') + fmt(d.value, 2) + '%'), '   '); }
      });
    }
    const keys = $('div-keys');
    if (keys.childElementCount !== insts.length) {
      keys.textContent = '';
      insts.forEach((x, i) => keys.append(el('span', { class: 'key' }, el('i', { style: `background:var(${SERIES_VARS[i % SERIES_VARS.length]})` }), x.symbol)));
    }
    insts.forEach((x, i) => {
      let ser = div.series.get(x.symbol);
      if (!ser) { ser = div.chart.addSeries(LWC.LineSeries, { color: css(SERIES_VARS[i % SERIES_VARS.length]), lineWidth: 2, priceLineVisible: false, priceFormat: { type: 'custom', formatter: (v) => fmt(v, 2) + '%' } }); div.series.set(x.symbol, ser); }
      const n = Math.max(0, x.session.bars || 0);
      const bars = (x.bars || []).slice(-n);
      ser.setData(bars.map((r) => ({ time: r[0], value: (r[4] / x.session.open - 1) * 100 })));
    });
    const key = insts.map((x) => `${x.symbol}:${x.session.date}:${x.session.bars}`).join('|');
    if (key !== div.key) { div.key = key; div.chart.timeScale().fitContent(); }
  }

  function render(s) {
    state = s;
    if (!s.account) return;
    renderHeader(s);
    renderKpis(s);
    renderDivergence(s);
    const inst = currentInstrument(s);
    if (inst) { renderChart(s, inst); renderPosition(inst); renderRegime(inst); renderStrategies(inst); }
    renderApprovals(s);
    renderLog(s);
    renderTrades(s);
    renderEquity(s);
    renderRules(s);
  }

  // ---------- controls
  async function confirmThen(text, action) {
    const d = $('confirm'); $('confirm-text').textContent = text; d.showModal();
    d.addEventListener('close', function once() { d.removeEventListener('close', once); if (d.returnValue === 'ok') control(action); });
  }
  $('btn-pause').addEventListener('click', () => control(state && state.paused ? 'resume' : 'pause'));
  $('btn-flatten').addEventListener('click', () => confirmThen('Close every open position at market and pause new entries?', 'flatten'));
  $('btn-kill').addEventListener('click', () => confirmThen('Flatten everything and STOP the engine process?', 'kill'));

  // heartbeat: only while this page is actually visible ("actively monitored")
  setInterval(() => { if (document.visibilityState === 'visible') control('heartbeat'); }, 15000);
  document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible') control('heartbeat'); });

  // ---------- live stream
  function connect() {
    if (!TOKEN) { setPill('conn', 'critical', 'no token'); $('banner').classList.add('show'); $('banner').textContent = 'Open the URL printed in the terminal (it contains ?token=…).'; return; }
    const es = new EventSource('/api/stream?token=' + encodeURIComponent(TOKEN));
    es.onopen = () => setPill('conn', 'good', 'connected');
    es.onmessage = (m) => { try { const s = JSON.parse(m.data); if (s.account) render(s); else renderIdle(s); } catch (e) { console.error(e); } };
    es.onerror = () => { setPill('conn', 'critical', 'disconnected'); };
  }
  function renderIdle(s) {
    const m = s.meta || {};
    setPill('status', 'warning', (m.warnings && m.warnings[0]) || 'starting…');
  }

  // ---------- backtest reports
  const bt = { charts: [] };
  async function loadReports() {
    const r = await api('/api/reports'); if (!r.ok) return;
    const list = await r.json(); const sel = $('report-select'); const cur = sel.value;
    sel.textContent = ''; sel.append(el('option', { value: '' }, '— choose a report —'));
    for (const x of list) sel.append(el('option', { value: x.name }, x.name.replace(/\.json$/, '')));
    if (cur) sel.value = cur;
  }
  $('report-select').addEventListener('change', async (e) => {
    const name = e.target.value; if (!name) return;
    const body = $('report-body'); body.style.opacity = 0.5;
    const r = await api('/api/report/' + encodeURIComponent(name)); body.style.opacity = 1;
    if (!r.ok) { body.textContent = 'cannot load report'; return; }
    const rep = await r.json();
    if (Array.isArray(rep.runs)) renderPassRate(rep); else renderReport(rep);
  });
  // ---------- rolling-start pass-rate study (`q22 passrate`)
  const OUTCOME = { passed: ['--good', 'passed'], failed: ['--critical', 'failed'], open: ['--text-muted', 'open at data end'] };
  const iqr = (q) => q ? `${fmt(q.median, 0)} sessions` : '—';
  const iqrSub = (q) => q ? `IQR ${fmt(q.p25, 0)}–${fmt(q.p75, 0)} · ≈${fmt(q.median / 21, 1)} months` : 'none';
  function renderPassRate(rep) {
    for (const c of bt.charts) c.remove(); bt.charts = [];
    const body = $('report-body'); body.textContent = '';
    const o = rep.options;
    $('report-meta').textContent = `${rep.label} · ${rep.data.map((d) => `${d.symbol} ${d.timeframe_min}m ${d.from.slice(0, 10)}→${d.to.slice(0, 10)}`).join(', ')} · rules ${rep.rules} · a fresh evaluation every ${o.every_days} sessions`;
    const k = el('div', { class: 'kpis' });
    k.append(kpi('Pass rate', pct(rep.pass_rate), `95% CI ${pct(rep.pass_rate_ci95[0])}–${pct(rep.pass_rate_ci95[1])} · ≈${fmt(rep.effective_n, 0)} independent attempts`));
    k.append(kpi('Evaluations', String(rep.runs.length), `${rep.passed} passed · ${rep.failed} failed · ${rep.open} open`));
    k.append(kpi('Time to pass (median)', iqr(rep.sessions_to_pass), iqrSub(rep.sessions_to_pass)));
    k.append(kpi('Time to fail (median)', iqr(rep.sessions_to_fail), iqrSub(rep.sessions_to_fail)));
    body.append(k);
    const card = el('div', { class: 'card' }, el('h2', {}, 'Outcome by start date ', el('span', { class: 'muted' }, 'bar height = sessions until the evaluation ended')));
    card.append(el('div', {}, ...Object.values(OUTCOME).map(([v, label]) => el('span', { class: 'key' }, el('i', { class: 'box', style: `background:var(${v})` }), label))));
    const holder = el('div', { class: 'chart small', style: 'height:260px' }); const lg = el('div', { class: 'legend' }); holder.append(lg); card.append(holder); body.append(card);
    const ch = LWC.createChart(holder, chartOptions()); bt.charts.push(ch);
    const hs = ch.addSeries(LWC.HistogramSeries, { priceLineVisible: false, lastValueVisible: false, priceFormat: { type: 'price', precision: 0, minMove: 1 } });
    const byTime = new Map(rep.runs.map((x) => [x.start, x]));
    hs.setData(rep.runs.map((x) => ({ time: x.start, value: x.sessions, color: css((OUTCOME[x.outcome] || OUTCOME.open)[0]) })));
    ch.subscribeCrosshairMove((p) => {
      lg.textContent = ''; const x = p && p.time ? byTime.get(typeof p.time === 'string' ? p.time : `${p.time.year}-${String(p.time.month).padStart(2, '0')}-${String(p.time.day).padStart(2, '0')}`) : null;
      if (x) lg.append(`start ${x.start} · `, el('b', {}, (OUTCOME[x.outcome] || OUTCOME.open)[1]), ` after ${x.sessions} sessions · ${x.trades} trades · P&L `, el('b', {}, signed(x.profit)));
    });
    ch.timeScale().fitContent();
    const split = el('div', { class: 'split', style: 'margin-top:16px' });
    const ft = el('table'); ft.append(el('thead', {}, el('tr', {}, el('th', {}, 'why evaluations failed'), el('th', { class: 'num' }, 'count'))));
    const ftb = el('tbody'); for (const [why, n] of Object.entries(rep.fail_reasons)) ftb.append(el('tr', {}, el('td', { style: 'white-space:normal' }, why), el('td', { class: 'num' }, String(n)))); ft.append(ftb);
    split.append(el('div', { class: 'card' }, el('h2', {}, 'Failures'), ft, el('p', { class: 'muted' }, '“guard stop” = q22 stopped trading with less than its minimum buffer left — counted as a failure, although the firm would not have closed the account yet.')));
    split.append(el('div', { class: 'card' }, el('h2', {}, 'How to read this'), el('p', { class: 'secondary', style: 'margin:0' }, `Each run is a new evaluation started on a different date and traded by the same engine until it passed, failed or the data ended. Neighbouring runs share most of their days, so the confidence interval uses ≈${fmt(rep.effective_n, 0)} independent attempts rather than ${rep.runs.length}. Sessions are exchange trading days (≈21 a month) — the unit monthly fees are paid in.`)));
    body.append(split);
    const rt = el('table'); rt.append(el('thead', {}, el('tr', {}, ['start', 'end', 'outcome', 'sessions', 'active days', 'trades', 'P&L', 'best day', 'reason'].map((h, i) => el('th', { class: i >= 3 && i <= 7 ? 'num' : '' }, h)))));
    const rtb = el('tbody');
    for (const x of rep.runs) rtb.append(el('tr', {}, el('td', {}, x.start), el('td', {}, x.end), el('td', {}, el('span', { class: 'pill ' + (x.outcome === 'passed' ? 'good' : x.outcome === 'failed' ? 'critical' : 'warning') }, el('span', { class: 'dot' }), el('span', {}, x.outcome))), el('td', { class: 'num' }, String(x.sessions)), el('td', { class: 'num' }, String(x.trading_days)), el('td', { class: 'num' }, String(x.trades)), el('td', { class: 'num ' + cls(x.profit) }, signed(x.profit)), el('td', { class: 'num' }, money(x.best_day)), el('td', { class: 'muted', style: 'white-space:normal' }, x.reason)));
    rt.append(rtb);
    body.append(el('div', { class: 'card', style: 'margin-top:16px' }, el('h2', {}, 'Every evaluation'), el('div', { class: 'scroll' }, rt)));
  }
  function statTable(rows, keyLabel) {
    const t = el('table');
    t.append(el('thead', {}, el('tr', {}, [keyLabel, 'trades', 'win', 'PF', 'avg R', 'ΣR', 'net', 'avg bars'].map((h, i) => el('th', { class: i ? 'num' : '' }, h)))));
    const tb = el('tbody');
    for (const g of rows) tb.append(el('tr', {}, el('td', {}, keyLabel === 'strategy' ? el('span', { class: 'chip' }, el('i', { style: `background:${colorFor(g.key)}` }), g.key) : g.key), el('td', { class: 'num' }, String(g.trades)), el('td', { class: 'num' }, pct(g.win_rate)), el('td', { class: 'num' }, fmt(g.profit_factor)), el('td', { class: 'num ' + cls(g.avg_r) }, fmt(g.avg_r, 3)), el('td', { class: 'num ' + cls(g.sum_r) }, fmt(g.sum_r, 1)), el('td', { class: 'num ' + cls(g.net) }, signed(g.net)), el('td', { class: 'num' }, fmt(g.avg_bars, 1))));
    t.append(tb); return el('div', { class: 'tablewrap' }, t);
  }
  function renderReport(rep) {
    for (const c of bt.charts) c.remove(); bt.charts = [];
    const s = rep.summary, body = $('report-body'); body.textContent = '';
    $('report-meta').textContent = `${rep.label} · ${rep.data.map((d) => `${d.symbol} ${d.timeframe_min}m ${d.from.slice(0, 10)}→${d.to.slice(0, 10)}`).join(', ')} · rules ${rep.prop.rules}`;
    const k = el('div', { class: 'kpis' });
    k.append(kpi('Net P&L', signed(s.net_pnl), `gross ${signed(s.gross_pnl)} · costs ${money(s.costs)}`, null, cls(s.net_pnl)));
    k.append(kpi('Trades', String(s.trades), `win ${pct(s.win_rate)} · PF ${fmt(s.profit_factor)} · ${fmt(s.avg_r, 3)} R avg`));
    k.append(kpi('Daily Sharpe (ann.)', fmt(s.sharpe_daily_ann), `PSR ${fmt(s.psr_vs_zero, 3)} · DSR ${fmt(s.dsr, 3)} (${s.dsr_trials} trials)`));
    k.append(kpi('Max drawdown', money(s.max_drawdown_usd), `worst day ${signed(s.worst_day)} · best ${signed(s.best_day)}`));
    k.append(kpi('Prop pass rate', pct(rep.prop.pass_rate), `${rep.prop.passed} passed / ${rep.prop.failed} failed / ${rep.prop.open} open`));
    k.append(kpi('Track record needed', isFinite(s.min_track_record_days) ? fmt(s.min_track_record_days, 0) + ' days' : '∞', 'for 95% confidence that Sharpe > 0'));
    body.append(k);
    const eqCard = el('div', { class: 'card' }, el('h2', {}, 'Cumulative P&L ', el('span', { class: 'muted' }, 'all evaluations stitched; strategy P&L independent of account resets')));
    const holder = el('div', { class: 'chart small', style: 'height:260px' }); const lg = el('div', { class: 'legend' }); holder.append(lg); eqCard.append(holder); body.append(eqCard);
    const ch = LWC.createChart(holder, chartOptions()); bt.charts.push(ch);
    const ls = ch.addSeries(LWC.LineSeries, { color: css('--series-1'), lineWidth: 2, priceLineVisible: false });
    ls.setData(rep.equity.map(([d, v]) => ({ time: d, value: v - s.start_balance })));
    ch.subscribeCrosshairMove((p) => { lg.textContent = ''; const d = p && p.seriesData ? p.seriesData.get(ls) : null; if (d) lg.append(String(p.time), '  P&L ', el('b', {}, signed(d.value))); });
    ch.timeScale().fitContent();
    const split = el('div', { class: 'split', style: 'margin-top:16px' });
    split.append(el('div', { class: 'card' }, el('h2', {}, 'By strategy'), statTable(rep.by_strategy, 'strategy')));
    split.append(el('div', { class: 'card' }, el('h2', {}, 'By regime at entry'), statTable(rep.by_regime, 'regime'), el('div', { class: 'muted', style: 'margin-top:8px' }, 'Regime days: ' + Object.entries(rep.regime_days).map(([a, b]) => `${a} ${b}`).join(' · '))));
    split.append(el('div', { class: 'card' }, el('h2', {}, 'By exit'), statTable(rep.by_exit, 'exit')));
    const at = el('table'); at.append(el('thead', {}, el('tr', {}, ['#', 'start', 'end', 'outcome', 'active days', 'profit', 'best day', 'reason'].map((h) => el('th', {}, h)))));
    const atb = el('tbody'); for (const a of rep.attempts) atb.append(el('tr', {}, el('td', {}, String(a.n)), el('td', {}, a.start), el('td', {}, a.end), el('td', {}, el('span', { class: 'pill ' + (a.outcome === 'passed' ? 'good' : a.outcome === 'failed' ? 'critical' : 'warning') }, el('span', { class: 'dot' }), el('span', {}, a.outcome))), el('td', {}, String(a.trading_days)), el('td', { class: cls(a.profit) }, signed(a.profit)), el('td', {}, money(a.best_day)), el('td', { class: 'muted', style: 'white-space:normal' }, a.reason)));
    at.append(atb);
    const attemptsCard = el('div', { class: 'card', style: 'margin-top:16px' }, el('h2', {}, 'Prop evaluation replay ', el('span', { class: 'muted' }, 'each attempt is a fresh account under the firm rules')), el('div', { class: 'scroll' }, at));
    body.append(split, attemptsCard);
    body.append(el('div', { class: 'card', style: 'margin-top:16px' }, el('h2', {}, 'Trades ', el('span', { class: 'muted' }, `(first 1000 of ${rep.trades.length})`)), el('div', { class: 'scroll' }, tradesTable(rep.trades.slice(0, 1000)))));
  }

  initCharts();
  connect();
  fetch('/api/state', { headers: { Authorization: 'Bearer ' + TOKEN } }).then((r) => r.ok ? r.json() : null).then((s) => { if (s && s.account) render(s); });
})();
