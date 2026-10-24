#!/bin/sh
# Run bench/rers_seq_abs.py once per listed RERS problem, each in its own
# memory-capped scope, at most JOBS at a time. Used on av2:
#   sh bench/rers_seq_abs_run.sh MODE JOBS MEM_GB SECONDS OUTDIR YEAR/PROB... [-- extra args]
# MODE is `check` or `crosscheck`. Each problem logs to OUTDIR/MODE-YEAR-PROB.log.
mode=$1 jobs=$2 mem=$3 secs=$4 out=$5
shift 5
probs=""
while [ $# -gt 0 ] && [ "$1" != "--" ]; do probs="$probs $1"; shift; done
[ "$1" = "--" ] && shift
mkdir -p "$out" "$out/tmp"
for yp in $probs; do echo "${yp%/*} ${yp#*/}"; done |
  xargs -P "$jobs" -L 1 sh -c '
    systemd-run --user --scope -q -p MemoryMax='"$mem"'G -p MemorySwapMax=0 \
      nice -n 10 timeout '"$secs"' python3 bench/rers_seq_abs.py '"$mode"' \
      --years "$0" --problems "$1" --out-dir '"$out"' --tmp '"$out"'/tmp '"$*"' \
      > '"$out"'/'"$mode"'-"$0-$1".log 2>&1
    echo "$0/$1 exit $?" >> '"$out"'/done-'"$mode"'.txt'
