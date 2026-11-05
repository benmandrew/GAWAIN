#!/bin/sh
# Phase 3 of the headline rerun: the frozen GA against random mutation at
# the 120x80 budget on the evaluation model instances less
# SieveSingleMsgMbox, nice 19, 4 h cap per run.
# usage: phase3.sh SEEDS JOBS TAG
set -u
cd ~/aragon/next
export PATH=$HOME/aragon/spot/bin:$HOME/aragon/eval/bin:$HOME/.local/bin:$PATH
F='^(?!beem-anderson\.(3-2|3-4|5-2|5-4|7-4)$|beem-lamport\.8-4$|mcc-MedleyA-PT-09-|mcc-SieveSingleMsgMbox-)'
nice -n 19 python3 -I bench/graded_run.py bench/data/eval $2 --seeds $1 \
  --arms tuned,mutation-tuned --budget 120x80 \
  --out runs-120 --filter "$F" --memory-gb 8 --run-timeout 14400 \
  > bench/data/eval/run-p3-$3.log 2>&1
echo done > ~/aragon/next/P3_DONE_$3
