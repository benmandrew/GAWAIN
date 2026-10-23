#!/bin/sh
# Runs weaken on abp4 p2false (seed 1, 30 x 40) with and without the
# degeneracy filter, five repetitions, alternating which arm goes first.
# Usage, inside `nix develop`: degeneracy_ab.sh BASE_BIN NEW_BIN MODEL OUT
# BASE_BIN is weaken built from the commit before the filter, NEW_BIN from
# the commit that adds it (both `cargo build --release`), MODEL is abp4.smv
# with the DEFINEs of tests/search.rs appended, and OUT a directory for the
# logs. Prints one CSV row per run.
set -u
base=$1 new=$2 model=$3 out=$4
mkdir -p "$out"
lim() { systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 nice -n 10 timeout 1800 "$@"; }
PHI='G (s_get -> X (!s_get U (s_wait & ((!s_abp & r_ack0) | (s_abp & r_ack1)))))'
echo "arm,rep,check_s,wall_s,backend_calls,repairs"
for rep in 1 2 3 4 5; do
  if [ $((rep % 2)) = 1 ]; then order="base new"; else order="new base"; fi
  for arm in $order; do
    eval bin=\$$arm
    log="$out/abp4-$arm-$rep.txt"
    lim "$bin" --model "$model" --atoms s_get,s_wait,s_abp,r_ack0,r_ack1 \
      --ltl "$PHI" --seed 1 --generations 30 --population 40 > "$log" 2>&1
    check=$(sed -n 's/^instance: proper time=\([0-9.]*\)s$/\1/p' "$log")
    wall=$(sed -n 's/^wall: \([0-9.]*\)s.*/\1/p' "$log")
    calls=$(sed -n 's/^oracle: .* backend_calls=\([0-9]*\) .*/\1/p' "$log")
    repairs=$(sed -n 's/^repairs: //p' "$log")
    echo "$arm,$rep,${check:-},$wall,$calls,$repairs"
  done
done
