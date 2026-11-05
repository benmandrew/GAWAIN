#!/bin/sh
# Phase 2 of the headline rerun: the frozen arms on the 100 evaluation model
# instances (30x40) and on the 126 trace instances (30x30), all with
# the weakening filter on, nice 19, 4 h cap per run.
# usage: phase2.sh SEEDS JOBS TAG
set -u
cd ~/aragon/next
export PATH=$HOME/aragon/spot/bin:$HOME/aragon/eval/bin:$HOME/.local/bin:$PATH
F='^(?!beem-anderson\.(3-2|3-4|5-2|5-4|7-4)$|beem-lamport\.8-4$|mcc-MedleyA-PT-09-)'
nice -n 19 python3 -I bench/graded_run.py bench/data/eval $2 --seeds $1 \
  --arms tuned,mutation-tuned,weaken-walk \
  --out runs-next --filter "$F" --memory-gb 8 --run-timeout 14400 \
  > bench/data/eval/run-p2-$3.log 2>&1
echo done > ~/aragon/next/P2_MODELS_DONE_$3
nice -n 19 python3 -I bench/graded_run.py bench/data/eval-traces $2 --seeds $1 \
  --arms tuned,mutation-tuned,weaken-walk --budget 30x30 \
  --out runs-next --memory-gb 12 --run-timeout 14400 \
  > bench/data/eval-traces/run-p2-$3.log 2>&1
echo done > ~/aragon/next/P2_DONE_$3
