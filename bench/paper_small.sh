#!/bin/sh
# The precedes set and the CEGIW case studies at the frozen configuration,
# 30 seeds, with the forced arms (both sets hold degenerate cases).
# usage: small.sh JOBS
set -u
cd ~/aragon/next
export PATH=$HOME/aragon/spot/bin:$HOME/aragon/eval/bin:$HOME/.local/bin:$PATH
for d in precedes cegiw; do
  nice -n 19 python3 -I bench/graded_run.py bench/data/$d $1 --seeds 1-30 \
    --arms forced-tuned,forced-mutation-tuned \
    --out runs-frozen --memory-gb 8 --run-timeout 14400 \
    > bench/data/$d/run-frozen.log 2>&1
done
echo done > ~/aragon/next/SMALL_DONE
