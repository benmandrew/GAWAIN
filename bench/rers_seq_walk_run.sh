#!/bin/sh
# Run bench/rers_seq_walk.py for several seeds in parallel, one scope each.
#
#   sh bench/rers_seq_walk_run.sh JOBS MEM_GB SECONDS OUTDIR WALKS STEPS CYCLE \
#      SEED... -- YEAR/PROB/PROP...
#
# Each seed checks every listed property and writes OUTDIR/walk-SEED.tsv and
# OUTDIR/walk-SEED.log.
set -eu
jobs=$1 mem=$2 secs=$3 out=$4 walks=$5 steps=$6 cycle=$7
shift 7
seeds=""
while [ "$1" != "--" ]; do seeds="$seeds $1"; shift; done
shift
mkdir -p "$out"
here=$(dirname "$0")
for s in $seeds; do echo "$s"; done | xargs -P "$jobs" -I{} sh -c "
  systemd-run --user --scope -q -p MemoryMax=${mem}G -p MemorySwapMax=0 nice -n 10 \
    timeout $secs python3 $here/rers_seq_walk.py $out/walk-{}.tsv {} $walks $steps $cycle $* \
    > $out/walk-{}.log 2>&1"
