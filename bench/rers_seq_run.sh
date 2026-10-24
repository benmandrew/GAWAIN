#!/bin/sh
# Run bench/rers_seq.py once per RERS sequential problem, each in its own
# memory-capped scope, at most JOBS at a time. Used on av2/av3:
#   sh bench/rers_seq_run.sh MODE JOBS MEM_GB SECONDS OUTDIR [extra args...]
# MODE is `count` or `check`. Each problem logs to OUTDIR/YEAR-PROB.log.
mode=$1 jobs=$2 mem=$3 secs=$4 out=$5
shift 5
mkdir -p "$out" "$out/tmp"
for y in 2016 2017; do for p in 1 2 3 4 5 6 7 8 9; do echo "$y $p"; done; done |
  xargs -P "$jobs" -L 1 sh -c '
    systemd-run --user --scope -q -p MemoryMax='"$mem"'G -p MemorySwapMax=0 \
      nice -n 10 timeout '"$secs"' python3 bench/rers_seq.py '"$mode"' \
      --years "$0" --problems "$1" --jobs 1 --out-dir '"$out"' --tmp '"$out"'/tmp '"$*"' \
      > '"$out"'/"$0-$1".log 2>&1
    echo "$0/$1 exit $?" >> '"$out"'/done.txt'
