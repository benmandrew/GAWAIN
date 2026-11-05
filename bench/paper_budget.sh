#!/bin/sh
# Budget curve at the frozen configuration: the GA against random mutation
# at 15x20 and 60x80 on the 99 phase 3 model instances, nice 19, 4 h cap.
# 30x40 and 120x80 come from phases 2 and 3.
# usage: budget.sh SEEDS JOBS TAG
set -u
cd ~/aragon/next
export PATH=$HOME/aragon/spot/bin:$HOME/aragon/eval/bin:$HOME/.local/bin:$PATH
F='^(?!beem-anderson\.(3-2|3-4|5-2|5-4|7-4)$|beem-lamport\.8-4$|mcc-MedleyA-PT-09-|mcc-SieveSingleMsgMbox-)'
for b in 15x20 60x80; do
  nice -n 19 python3 -I bench/graded_run.py bench/data/eval $2 --seeds $1 \
    --arms tuned,mutation-tuned --budget $b \
    --out runs-b$b --filter "$F" --memory-gb 8 --run-timeout 14400 \
    > bench/data/eval/run-b$b-$3.log 2>&1
done
echo done > ~/aragon/next/BUDGET_DONE_$3
