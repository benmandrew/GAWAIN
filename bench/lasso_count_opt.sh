#!/bin/sh
# Lasso-count optimisation on av3: before/ (whole relations interned) and
# after/ (interned rows) run weaken on ack.smv (seed 4) and abp4 p2false
# (seed 1), 30 x 40, five repetitions with the order of the arms
# alternated (rows `opt-final`). Every run is pinned to cores 0-7 by
# taskset. The `opt-arms` rows came from the same script with three arms:
# before/, threads/ (interned rows and one count thread per core) and
# rows/ (threads/ with the thread count fixed at 1). Layout under
# ~/aragon-model-counting as in bench/model_counting_ab.sh.
set -u
cd ~/aragon-model-counting
export PATH="$HOME/aragon-model-counting/spot/bin:$HOME/.local/bin:$HOME/.cargo/bin:/usr/local/bin:$PATH"
lim() { systemd-run --user --scope -q -p MemoryMax=16G -p MemorySwapMax=0 nice -n 10 timeout 1800 taskset -c 0-7 "$@"; }
ltlfilt --version | head -1
for arm in before after; do
  (cd $arm && CARGO_BUILD_JOBS=8 lim cargo build --release -q) || exit 1
done
mkdir -p out
ABP_PHI='G (s_get -> X (!s_get U (s_wait & ((!s_abp & r_ack0) | (s_abp & r_ack1)))))'
for rep in 1 2 3 4 5; do
  case $((rep % 2)) in
    1) order="before after" ;;
    *) order="after before" ;;
  esac
  for arm in $order; do
    lim ./$arm/target/release/weaken --model models/ack.smv --atoms req,slow,ack,late \
      --ltl 'G (req -> X ack)' --seed 4 --generations 30 --population 40 > out/ack-$arm-$rep.txt 2>&1
    lim ./$arm/target/release/weaken --model models/abp4-defines.smv --atoms s_get,s_wait,s_abp,r_ack0,r_ack1 \
      --ltl "$ABP_PHI" --seed 1 --generations 30 --population 40 > out/abp4-$arm-$rep.txt 2>&1
  done
done
echo done > out/DONE
