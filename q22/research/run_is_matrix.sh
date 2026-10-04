#!/usr/bin/env bash
# In-sample engine matrix (see PREREGISTRATION.md D-3/D-4). Usage: research/run_is_matrix.sh [risk_usd]
# Never pass dates after 2018-12-31 here — the OOS period is reserved.
set -euo pipefail
cd "$(dirname "$0")/.."
Q=./target/release/q22
RISK=${1:-250.0}
OUT=results/is_engine_matrix.txt
TMP=$(mktemp -d)
render() { # sym exit peer cost
  local sym=$1 exit=$2 peer=$3 cost=$4 f="$TMP/${1}_${2}_${3}_${4}.toml"
  local slip mnq mes
  if [ "$cost" = contract ]; then slip=1.0; mnq='commission_per_side = 0.75'; mes='commission_per_side = 0.75'
  else slip=0.0; mnq=$'commission_per_side = 0.0\nfee_rate = 0.000025'; mes=$'commission_per_side = 0.0\nfee_rate = 0.0000615'; fi
  sed -e "s/@@RISK@@/$RISK/" -e "s/@@SLIP@@/$slip/g" -e "s/@@SYM@@/$sym/g" -e "s/@@SYM_LC@@/${sym,,}/" -e "s/@@EXIT@@/$exit/" -e "s/@@PEER@@/$peer/" config/research/_template.toml \
    | awk -v a="$mnq" -v b="$mes" '{ if ($0=="@@MNQ_COST@@") print a; else if ($0=="@@MES_COST@@") print b; else print }' > "$f"
  echo "$f"
}
run() { # sym exit peer cost
  local f; f=$(render "$@")
  local data
  if [ "$4" = contract ]; then data="--data MNQ=data/databento/NQ_c1_1m.csv --data MES=data/databento/ES_c1_1m.csv"
  else data="--data MNQ=data/databento/NQ_c1_1m_ratio.csv --data MES=data/databento/ES_c1_1m_ratio.csv"; fi
  local label="IS_${1}_${2}_${3}_${4}"
  $Q backtest -c "$f" $data --from 2010-06-07 --to 2018-12-31 --label "$label" --out "$TMP/$label.json" 2>/dev/null \
    | awk -v l="$label" '/^net/{n=$0} /^days/{d=$0} /^max drawdown/{m=$0} /^prop replay/{p=$0} END{
        split(n,a," "); split(d,b,"|"); split(m,c,"|"); print l " | " n " | " b[2] " | " c[1] " | " p}'
}
export -f render run; export Q RISK TMP
: > "$OUT"
jobs=()
for sym in MNQ MES; do for exit in resting checks; do for peer in none confirm diverge; do for cost in contract bps; do
  jobs+=("$sym $exit $peer $cost"); done; done; done; done
printf '%s\n' "${jobs[@]}" | xargs -P 4 -I{} bash -c 'run {}' | sort >> "$OUT"
cat "$OUT"
