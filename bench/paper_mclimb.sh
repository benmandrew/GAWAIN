#!/bin/sh
# The random-mutation baseline with the output climb, to pair with
# `tuned-climb` of paper_order.sh: same instances, budgets, caps, filter and
# nice, in blocks of five seeds, models then traces. Outputs go to runs-order.
# usage: paper_mclimb.sh JOBS [BLOCKS], BLOCKS as in paper_order.sh.
set -u
cd ~/aragon/order
export PATH=$HOME/aragon/spot/bin:$HOME/aragon/eval/bin:$HOME/.local/bin:$PATH
F='^(?!beem-anderson\.(3-2|3-4|5-2|5-4|7-4)$|beem-lamport\.8-4$|mcc-MedleyA-PT-09-)'
for block in ${2:-1-5 6-10 11-15 16-20 21-25 26-30}; do
  nice -n 19 python3 -I bench/graded_run.py bench/data/eval $1 --seeds $block \
    --arms mutation-climb --out runs-order --filter "$F" --memory-gb 8 --run-timeout 14400 \
    >> bench/data/eval/run-mclimb.log 2>&1
  nice -n 19 python3 -I bench/graded_run.py bench/data/eval-traces $1 --seeds $block \
    --arms mutation-climb --budget 30x30 --out runs-order --memory-gb 12 --run-timeout 14400 \
    >> bench/data/eval-traces/run-mclimb.log 2>&1
done
echo done > MCLIMB_DONE
