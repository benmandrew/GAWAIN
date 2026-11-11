#!/bin/sh
# The order arms against the frozen `tuned` runs of phases 1 and 2: a
# reproduction check of `tuned` at seed 1, then the five order arms on the
# 100 evaluation model instances (30x40) and the 126 trace instances
# (30x30), in blocks of five seeds, models then traces, so a partial run
# stays paired across arms and sets. Same caps, filter and nice as
# paper_phase2.sh. Outputs go to runs-order and runs-repro.
# usage: paper_order.sh JOBS [BLOCKS], BLOCKS a space-separated list of
# seed ranges (default all six), so two hosts can split the seeds. The
# reproduction check runs once per host, on the first call.
set -u
cd ~/aragon/order
export PATH=$HOME/aragon/spot/bin:$HOME/aragon/eval/bin:$HOME/.local/bin:$PATH
F='^(?!beem-anderson\.(3-2|3-4|5-2|5-4|7-4)$|beem-lamport\.8-4$|mcc-MedleyA-PT-09-)'
ARMS=tuned-ordered,tuned-gated,tuned-demotion,tuned-climb,tuned-order
[ -e REPRO_DONE ] || {
nice -n 19 python3 -I bench/graded_run.py bench/data/eval $1 --seeds 1 --arms tuned \
  --out runs-repro --filter "$F" --memory-gb 8 --run-timeout 14400 \
  > bench/data/eval/run-repro.log 2>&1
nice -n 19 python3 -I bench/graded_run.py bench/data/eval-traces $1 --seeds 1 --arms tuned \
  --budget 30x30 --out runs-repro --memory-gb 12 --run-timeout 14400 \
  > bench/data/eval-traces/run-repro.log 2>&1
echo done > REPRO_DONE
}
for block in ${2:-1-5 6-10 11-15 16-20 21-25 26-30}; do
  nice -n 19 python3 -I bench/graded_run.py bench/data/eval $1 --seeds $block --arms $ARMS \
    --out runs-order --filter "$F" --memory-gb 8 --run-timeout 14400 \
    >> bench/data/eval/run-order.log 2>&1
  nice -n 19 python3 -I bench/graded_run.py bench/data/eval-traces $1 --seeds $block \
    --arms $ARMS --budget 30x30 --out runs-order --memory-gb 12 --run-timeout 14400 \
    >> bench/data/eval-traces/run-order.log 2>&1
  echo "$block" >> ORDER_BLOCKS_DONE
done
echo done > ORDER_DONE
