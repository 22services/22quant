//! Loading bar files for backtests and replays: `SYMBOL=path[@Timezone]`.

use std::path::Path;

use anyhow::{anyhow, bail, Result};
use chrono::NaiveDate;
use chrono_tz::Tz;

use q22_core::data::{detect_timeframe_minutes, load_bars, resample, slice_dates};
use q22_core::Bar;

pub struct DataSpec {
    pub symbol: String,
    pub path: String,
    pub tz: Option<Tz>,
}

pub fn parse_spec(s: &str) -> Result<DataSpec> {
    let (sym, rest) = s.split_once('=').ok_or_else(|| anyhow!("--data expects SYMBOL=path[@Timezone], got {s:?}"))?;
    let (path, tz) = match rest.rsplit_once('@') {
        Some((p, tz)) if tz.contains('/') => (p.to_string(), Some(tz.parse::<Tz>().map_err(|e| anyhow!("bad timezone {tz}: {e}"))?)),
        _ => (rest.to_string(), None),
    };
    Ok(DataSpec { symbol: sym.to_ascii_uppercase(), path, tz })
}

/// Load, slice and resample to `target_tf` minutes.
pub fn load(path: &Path, tz: Option<Tz>, target_tf: i64, from: Option<NaiveDate>, to: Option<NaiveDate>) -> Result<Vec<Bar>> {
    let bars = load_bars(path, tz)?;
    let bars = slice_dates(&bars, from, to);
    if bars.is_empty() {
        bail!("{}: no bars in the requested date range", path.display());
    }
    let tf = detect_timeframe_minutes(&bars);
    if tf > target_tf {
        bail!("{}: data timeframe {tf} min is coarser than the configured {target_tf} min", path.display());
    }
    Ok(if tf < target_tf {
        if target_tf % tf != 0 {
            bail!("{}: cannot resample {tf} min to {target_tf} min", path.display());
        }
        resample(&bars, target_tf)
    } else {
        bars
    })
}
