//! Performance statistics, including the Probabilistic and Deflated Sharpe Ratio
//! (Bailey & López de Prado 2012, 2014).

pub fn mean(x: &[f64]) -> f64 {
    if x.is_empty() { 0.0 } else { x.iter().sum::<f64>() / x.len() as f64 }
}

pub fn std(x: &[f64]) -> f64 {
    if x.len() < 2 {
        return 0.0;
    }
    let m = mean(x);
    (x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (x.len() as f64 - 1.0)).sqrt()
}

/// Annualised Sharpe of per-period returns (or P&L — Sharpe is scale-free).
pub fn sharpe(x: &[f64], periods_per_year: f64) -> f64 {
    let s = std(x);
    if s == 0.0 { 0.0 } else { mean(x) / s * periods_per_year.sqrt() }
}

/// (skewness, raw kurtosis) — normal is (0, 3).
pub fn skew_kurt(x: &[f64]) -> (f64, f64) {
    if x.len() < 4 {
        return (0.0, 3.0);
    }
    let m = mean(x);
    let n = x.len() as f64;
    let s = (x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / n).sqrt();
    if s == 0.0 {
        return (0.0, 3.0);
    }
    let sk = x.iter().map(|v| ((v - m) / s).powi(3)).sum::<f64>() / n;
    let ku = x.iter().map(|v| ((v - m) / s).powi(4)).sum::<f64>() / n;
    (sk, ku)
}

/// Max drawdown of a cumulative P&L (or equity) path, in the same units. Returns a non-positive number.
pub fn max_drawdown(path: &[f64]) -> f64 {
    let mut peak = f64::NEG_INFINITY;
    let mut mdd: f64 = 0.0;
    for &v in path {
        peak = peak.max(v);
        mdd = mdd.min(v - peak);
    }
    mdd
}

pub fn norm_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

/// Abramowitz–Stegun 7.1.26 with refinement (|err| < 1.5e-7) — plenty for test statistics.
pub fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * (-x * x).exp();
    sign * y
}

/// Inverse normal CDF (Acklam's rational approximation, rel. error < 1.2e-9).
pub fn norm_ppf(p: f64) -> f64 {
    assert!(p > 0.0 && p < 1.0, "p must be in (0,1)");
    const A: [f64; 6] = [-3.969683028665376e+01, 2.209460984245205e+02, -2.759285104469687e+02, 1.383_577_518_672_69e2, -3.066479806614716e+01, 2.506628277459239e+00];
    const B: [f64; 5] = [-5.447609879822406e+01, 1.615858368580409e+02, -1.556989798598866e+02, 6.680131188771972e+01, -1.328068155288572e+01];
    const C: [f64; 6] = [-7.784894002430293e-03, -3.223964580411365e-01, -2.400758277161838e+00, -2.549732539343734e+00, 4.374664141464968e+00, 2.938163982698783e+00];
    const D: [f64; 4] = [7.784695709041462e-03, 3.224671290700398e-01, 2.445134137142996e+00, 3.754408661907416e+00];
    let pl = 0.02425;
    if p < pl {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5]) / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - pl {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        -norm_ppf(1.0 - p)
    }
}

/// P[true per-period SR > sr_benchmark] given observed per-period `sr` over `n` observations.
pub fn probabilistic_sharpe(sr: f64, sr_benchmark: f64, n: usize, skew: f64, kurt: f64) -> f64 {
    if n < 2 {
        return 0.0;
    }
    let denom = (1.0 - skew * sr + (kurt - 1.0) / 4.0 * sr * sr).max(1e-12).sqrt();
    norm_cdf((sr - sr_benchmark) * ((n as f64) - 1.0).sqrt() / denom)
}

/// Expected maximum per-period Sharpe among `n_trials` zero-skill strategies whose Sharpe
/// estimates have variance `var_sr` (Bailey & López de Prado 2014, eq. 3).
pub fn expected_max_sharpe(n_trials: usize, var_sr: f64) -> f64 {
    if n_trials <= 1 {
        return 0.0;
    }
    let g = 0.5772156649015329;
    let n = n_trials as f64;
    var_sr.max(0.0).sqrt() * ((1.0 - g) * norm_ppf(1.0 - 1.0 / n) + g * norm_ppf(1.0 - 1.0 / (n * std::f64::consts::E)))
}

/// Deflated Sharpe: PSR of the selected strategy against the best-of-N-noise benchmark.
/// If `var_sr` is unknown, a standard choice is the variance of the SR estimator itself, 1/n.
pub fn deflated_sharpe(sr: f64, n: usize, n_trials: usize, var_sr: f64, skew: f64, kurt: f64) -> (f64, f64) {
    let bench = expected_max_sharpe(n_trials, var_sr);
    (probabilistic_sharpe(sr, bench, n, skew, kurt), bench)
}

/// Minimum number of observations for PSR(sr vs benchmark) to reach `confidence`.
pub fn min_track_record(sr: f64, sr_benchmark: f64, skew: f64, kurt: f64, confidence: f64) -> f64 {
    if sr <= sr_benchmark {
        return f64::INFINITY;
    }
    let z = norm_ppf(confidence);
    1.0 + (1.0 - skew * sr + (kurt - 1.0) / 4.0 * sr * sr) * (z / (sr - sr_benchmark)).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_functions() {
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-9);
        assert!((norm_cdf(1.96) - 0.975).abs() < 1e-4);
        assert!((norm_ppf(0.975) - 1.959964).abs() < 1e-5);
        assert!((norm_ppf(0.01) + 2.326348).abs() < 1e-5);
    }

    #[test]
    fn drawdown_and_sharpe() {
        assert_eq!(max_drawdown(&[0.0, 10.0, 4.0, 12.0, 2.0, 5.0]), -10.0);
        assert_eq!(sharpe(&[1.0, 1.0, 1.0], 252.0), 0.0);
        assert!(sharpe(&[1.0, 2.0, 1.5, 2.5], 252.0) > 0.0);
    }

    #[test]
    fn psr_dsr() {
        assert!((probabilistic_sharpe(0.0, 0.0, 500, 0.0, 3.0) - 0.5).abs() < 1e-9);
        assert!(probabilistic_sharpe(0.1, 0.0, 1000, 0.0, 3.0) > 0.99);
        assert!(expected_max_sharpe(100, 0.01) > expected_max_sharpe(10, 0.01));
        let (dsr, bench) = deflated_sharpe(0.05, 500, 200, 1.0 / 500.0, 0.0, 3.0);
        assert!(bench > 0.05 && dsr < 0.5);
        assert!(min_track_record(0.05, 0.0, 0.0, 3.0, 0.95) > min_track_record(0.2, 0.0, 0.0, 3.0, 0.95));
    }
}
