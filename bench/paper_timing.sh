#!/bin/sh
# Wall time of the output climb on an unloaded host: tuned and tuned-climb at
# seeds 1-3 on the 100 evaluation model instances (30x40) and the 126 trace
# instances (30x30), 8 jobs on 32 cores so the runs do not contend. Same caps,
# filter and nice as paper_order.sh. Outputs go to runs-timing.
# usage: paper_timing.sh [JOBS]
set -u
cd ~/aragon/order
export PATH=$HOME/aragon/spot/bin:$HOME/aragon/eval/bin:$HOME/.local/bin:$PATH
F='^(?!beem-anderson\.(3-2|3-4|5-2|5-4|7-4)$|beem-lamport\.8-4$|mcc-MedleyA-PT-09-)'
J=${1:-8}
nice -n 19 python3 -I bench/graded_run.py bench/data/eval $J --seeds 1-3 --arms tuned,tuned-climb \
  --out runs-timing --filter "$F" --memory-gb 8 --run-timeout 14400 \
  >> bench/data/eval/run-timing.log 2>&1
nice -n 19 python3 -I bench/graded_run.py bench/data/eval-traces $J --seeds 1-3 --arms tuned,tuned-climb \
  --budget 30x30 --out runs-timing --memory-gb 12 --run-timeout 14400 \
  >> bench/data/eval-traces/run-timing.log 2>&1
echo done > TIMING_DONE
