//! Streaming technical indicators. Each `update` consumes one completed bar/value and
//! returns the indicator value *including* that bar. No indicator can see the future.

use std::collections::VecDeque;

use crate::types::Bar;

#[derive(Clone, Debug)]
pub struct Ema {
    alpha: f64,
    value: Option<f64>,
    n: usize,
    warmup: usize,
}

impl Ema {
    pub fn new(span: usize) -> Self {
        Self { alpha: 2.0 / (span as f64 + 1.0), value: None, n: 0, warmup: span }
    }
    pub fn update(&mut self, x: f64) -> f64 {
        self.n += 1;
        let v = match self.value {
            None => x,
            Some(p) => p + self.alpha * (x - p),
        };
        self.value = Some(v);
        v
    }
    pub fn value(&self) -> Option<f64> {
        if self.n >= self.warmup { self.value } else { None }
    }
}

/// Wilder-smoothed average true range.
#[derive(Clone, Debug)]
pub struct Atr {
    n: usize,
    count: usize,
    prev_close: Option<f64>,
    value: f64,
}

impl Atr {
    pub fn new(n: usize) -> Self {
        Self { n, count: 0, prev_close: None, value: 0.0 }
    }
    pub fn true_range(b: &Bar, prev_close: Option<f64>) -> f64 {
        match prev_close {
            Some(pc) => b.high.max(pc) - b.low.min(pc),
            None => b.high - b.low,
        }
    }
    pub fn update(&mut self, b: &Bar) -> f64 {
        let tr = Self::true_range(b, self.prev_close);
        self.prev_close = Some(b.close);
        self.count += 1;
        if self.count <= self.n {
            self.value += (tr - self.value) / self.count as f64; // simple mean during warm-up
        } else {
            self.value += (tr - self.value) / self.n as f64;
        }
        self.value
    }
    pub fn value(&self) -> Option<f64> {
        (self.count >= self.n).then_some(self.value)
    }
}

/// Wilder's ADX (trend strength, direction-agnostic).
#[derive(Clone, Debug)]
pub struct Adx {
    n: usize,
    count: usize,
    prev: Option<Bar>,
    tr_s: f64,
    pdm_s: f64,
    ndm_s: f64,
    adx: f64,
    dx_count: usize,
}

impl Adx {
    pub fn new(n: usize) -> Self {
        Self { n, count: 0, prev: None, tr_s: 0.0, pdm_s: 0.0, ndm_s: 0.0, adx: 0.0, dx_count: 0 }
    }
    pub fn update(&mut self, b: &Bar) -> Option<f64> {
        let Some(p) = self.prev.replace(*b) else { return None };
        let up = b.high - p.high;
        let down = p.low - b.low;
        let pdm = if up > down && up > 0.0 { up } else { 0.0 };
        let ndm = if down > up && down > 0.0 { down } else { 0.0 };
        let tr = Atr::true_range(b, Some(p.close));
        let n = self.n as f64;
        self.count += 1;
        if self.count <= self.n {
            self.tr_s += tr;
            self.pdm_s += pdm;
            self.ndm_s += ndm;
            if self.count < self.n {
                return None;
            }
        } else {
            self.tr_s = self.tr_s - self.tr_s / n + tr;
            self.pdm_s = self.pdm_s - self.pdm_s / n + pdm;
            self.ndm_s = self.ndm_s - self.ndm_s / n + ndm;
        }
        if self.tr_s <= 0.0 {
            return None;
        }
        let pdi = 100.0 * self.pdm_s / self.tr_s;
        let ndi = 100.0 * self.ndm_s / self.tr_s;
        let dx = if pdi + ndi > 0.0 { 100.0 * (pdi - ndi).abs() / (pdi + ndi) } else { 0.0 };
        self.dx_count += 1;
        if self.dx_count <= self.n {
            self.adx += (dx - self.adx) / self.dx_count as f64;
        } else {
            self.adx = (self.adx * (n - 1.0) + dx) / n;
        }
        (self.dx_count >= self.n).then_some(self.adx)
    }
}

/// Kaufman efficiency ratio: |net move| / sum |steps| over `n` steps. 1 = straight line, 0 = noise.
#[derive(Clone, Debug)]
pub struct EfficiencyRatio {
    n: usize,
    window: VecDeque<f64>,
}

impl EfficiencyRatio {
    pub fn new(n: usize) -> Self {
        Self { n, window: VecDeque::with_capacity(n + 1) }
    }
    pub fn update(&mut self, x: f64) -> Option<f64> {
        self.window.push_back(x);
        if self.window.len() > self.n + 1 {
            self.window.pop_front();
        }
        if self.window.len() < self.n + 1 {
            return None;
        }
        let net = (self.window.back().unwrap() - self.window.front().unwrap()).abs();
        let path: f64 = self.window.iter().zip(self.window.iter().skip(1)).map(|(a, b)| (b - a).abs()).sum();
        Some(if path > 0.0 { net / path } else { 0.0 })
    }
}

/// Rolling mean / std / min / max / percentile rank over a fixed window.
#[derive(Clone, Debug)]
pub struct Rolling {
    n: usize,
    w: VecDeque<f64>,
    sum: f64,
    sum2: f64,
}

impl Rolling {
    pub fn new(n: usize) -> Self {
        Self { n, w: VecDeque::with_capacity(n + 1), sum: 0.0, sum2: 0.0 }
    }
    pub fn push(&mut self, x: f64) {
        self.w.push_back(x);
        self.sum += x;
        self.sum2 += x * x;
        if self.w.len() > self.n {
            let y = self.w.pop_front().unwrap();
            self.sum -= y;
            self.sum2 -= y * y;
        }
    }
    pub fn len(&self) -> usize {
        self.w.len()
    }
    pub fn is_empty(&self) -> bool {
        self.w.is_empty()
    }
    pub fn full(&self) -> bool {
        self.w.len() == self.n
    }
    pub fn mean(&self) -> Option<f64> {
        (!self.w.is_empty()).then(|| self.sum / self.w.len() as f64)
    }
    pub fn std(&self) -> Option<f64> {
        let k = self.w.len() as f64;
        (k >= 2.0).then(|| ((self.sum2 - self.sum * self.sum / k) / (k - 1.0)).max(0.0).sqrt())
    }
    pub fn max(&self) -> Option<f64> {
        self.w.iter().copied().fold(None, |m, x| Some(m.map_or(x, |m: f64| m.max(x))))
    }
    pub fn min(&self) -> Option<f64> {
        self.w.iter().copied().fold(None, |m, x| Some(m.map_or(x, |m: f64| m.min(x))))
    }
    /// Fraction of window values strictly below `x` (0..1).
    pub fn percentile_rank(&self, x: f64) -> Option<f64> {
        (!self.w.is_empty()).then(|| self.w.iter().filter(|&&y| y < x).count() as f64 / self.w.len() as f64)
    }
    pub fn values(&self) -> impl Iterator<Item = &f64> {
        self.w.iter()
    }
}

/// Session VWAP with volume-weighted standard deviation of price around it.
/// Bars with zero volume get weight 1 (CFD/tick-volume data).
#[derive(Clone, Debug, Default)]
pub struct SessionVwap {
    pv: f64,
    v: f64,
    p2v: f64,
}

impl SessionVwap {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn update(&mut self, b: &Bar) {
        let w = if b.volume > 0.0 { b.volume } else { 1.0 };
        let p = b.typical();
        self.pv += p * w;
        self.v += w;
        self.p2v += p * p * w;
    }
    pub fn vwap(&self) -> Option<f64> {
        (self.v > 0.0).then(|| self.pv / self.v)
    }
    pub fn std(&self) -> Option<f64> {
        let m = self.vwap()?;
        Some((self.p2v / self.v - m * m).max(0.0).sqrt())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn bar(i: i64, o: f64, h: f64, l: f64, c: f64) -> Bar {
        Bar { ts: Utc.timestamp_opt(i * 60, 0).unwrap(), open: o, high: h, low: l, close: c, volume: 1.0 }
    }

    #[test]
    fn ema_converges() {
        let mut e = Ema::new(10);
        for _ in 0..200 {
            e.update(5.0);
        }
        assert!((e.value().unwrap() - 5.0).abs() < 1e-12);
    }

    #[test]
    fn efficiency_ratio_extremes() {
        let mut er = EfficiencyRatio::new(4);
        let mut last = None;
        for x in [1.0, 2.0, 3.0, 4.0, 5.0] {
            last = er.update(x);
        }
        assert!((last.unwrap() - 1.0).abs() < 1e-12);
        let mut er = EfficiencyRatio::new(4);
        for x in [1.0, 2.0, 1.0, 2.0, 1.0] {
            last = er.update(x);
        }
        assert!(last.unwrap().abs() < 1e-12);
    }

    #[test]
    fn atr_and_adx_trend() {
        let mut atr = Atr::new(14);
        let mut adx = Adx::new(14);
        let mut a = None;
        for i in 0..100 {
            let base = 100.0 + i as f64;
            let b = bar(i, base, base + 1.5, base - 0.5, base + 1.0);
            atr.update(&b);
            a = adx.update(&b).or(a);
        }
        assert!(atr.value().unwrap() > 1.5);
        assert!(a.unwrap() > 60.0, "steady up-trend must give high ADX, got {a:?}");
    }

    #[test]
    fn rolling_stats() {
        let mut r = Rolling::new(3);
        for x in [1.0, 2.0, 3.0, 4.0] {
            r.push(x);
        }
        assert_eq!(r.mean(), Some(3.0));
        assert!((r.std().unwrap() - 1.0).abs() < 1e-12);
        assert_eq!(r.max(), Some(4.0));
        assert_eq!(r.percentile_rank(3.5), Some(2.0 / 3.0));
    }

    #[test]
    fn vwap_weighted() {
        let mut v = SessionVwap::default();
        v.update(&Bar { volume: 1.0, ..bar(0, 10.0, 10.0, 10.0, 10.0) });
        v.update(&Bar { volume: 3.0, ..bar(1, 20.0, 20.0, 20.0, 20.0) });
        assert!((v.vwap().unwrap() - 17.5).abs() < 1e-12);
    }
}
