#!/bin/sh
# Compares the "new" arm (batched ltl2tgba, counts beside the oracle) with
# "seq" (one exec per formula, counts after the oracle), interleaved.
set -u
cd ~/aragon-model-counting
export PATH="$HOME/aragon-model-counting/spot/bin:$HOME/.local/bin:$HOME/.cargo/bin:/usr/local/bin:$PATH"
lim() { systemd-run --user --scope -q -p MemoryMax=16G -p MemorySwapMax=0 nice -n 10 timeout 1800 "$@"; }
rm -rf seq && cp -r new seq && rm -rf seq/target && python3 model_counting_seq.py seq
(cd seq && CARGO_BUILD_JOBS=8 lim cargo build --release -q) || exit 1
mkdir -p out2
ABP_PHI='G (s_get -> X (!s_get U (s_wait & ((!s_abp & r_ack0) | (s_abp & r_ack1)))))'
for rep in 1 2 3 4 5; do
  if [ $((rep % 2)) -eq 1 ]; then order="new seq"; else order="seq new"; fi
  for arm in $order; do
    lim ./$arm/target/release/weaken --model models/ack.smv --atoms req,slow,ack,late \
      --ltl 'G (req -> X ack)' --seed 4 --generations 30 --population 40 > out2/ack-$arm-$rep.txt 2>&1
    lim ./$arm/target/release/weaken --model models/abp4-defines.smv --atoms s_get,s_wait,s_abp,r_ack0,r_ack1 \
      --ltl "$ABP_PHI" --seed 1 --generations 30 --population 40 > out2/abp4-$arm-$rep.txt 2>&1
  done
done
uptime > out2/DONE
