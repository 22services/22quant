#!/usr/bin/env bash
# In-sample two-bot portfolios (PREREGISTRATION.md D-5). IS only: never pass dates after 2018-12-31.
set -euo pipefail
cd "$(dirname "$0")/.."
Q=./target/release/q22
OUT=results/is_portfolios.txt
TMP=$(mktemp -d)
render() { # name mnq_exit mnq_peer risk cost
  local f="$TMP/$1_$4_$5.toml" slip mnq mes
  if [ "$5" = contract ]; then slip=1.0; mnq='commission_per_side = 0.75'; mes='commission_per_side = 0.75'
  else slip=0.0; mnq=$'commission_per_side = 0.0\nfee_rate = 0.000025'; mes=$'commission_per_side = 0.0\nfee_rate = 0.0000615'; fi
  sed -e "s/@@RISK@@/$4/" -e "s/@@SLIP@@/$slip/g" -e "s/@@MNQ_EXIT@@/$2/" -e "s/@@MNQ_PEER@@/$3/" config/research/_portfolio.toml \
    | awk -v a="$mnq" -v b="$mes" '{ if ($0=="@@MNQ_COST@@") print a; else if ($0=="@@MES_COST@@") print b; else print }' > "$f"
  echo "$f"
}
run() { # name mnq_exit mnq_peer risk cost
  local f; f=$(render "$@")
  local data="--data MNQ=data/databento/NQ_c1_1m_ratio.csv --data MES=data/databento/ES_c1_1m_ratio.csv"
  [ "$5" = contract ] && data="--data MNQ=data/databento/NQ_c1_1m.csv --data MES=data/databento/ES_c1_1m.csv"
  local label="IS_$1_r$4_$5"
  local bt; bt=$($Q backtest -c "$f" $data --from 2010-06-07 --to 2018-12-31 --label "$label" --out "$TMP/$label.json" 2>/dev/null)
  local pr; pr=$($Q passrate -c "$f" $data --from 2010-06-07 --to 2018-12-31 --label "$label" --out "$TMP/${label}_pr.json" 2>/dev/null)
  echo "$label"
  echo "$bt" | grep -E "^net|^days|^max drawdown" | sed 's/^/   /'
  echo "$bt" | awk '/^strategy/{f=1} /^regime/{f=0} f' | sed 's/^/   /'
  echo "$pr" | grep -E "evaluations|pass rate|sessions to" | sed 's/^/   /'
}
export -f render run; export Q TMP
: > "$OUT"
jobs=()
for p in "P1 checks none" "P2 resting none" "P3 resting confirm"; do for risk in 150.0 250.0; do for cost in bps contract; do
  jobs+=("$p $risk $cost"); done; done; done
printf '%s\n' "${jobs[@]}" | xargs -P 4 -I{} bash -c 'run {} > '"$TMP"'/$(echo {} | tr " " _).out'
cat "$TMP"/*.out >> "$OUT"
cat "$OUT"
