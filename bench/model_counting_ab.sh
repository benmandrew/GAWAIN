#!/bin/sh
# Builds each arm in release mode and runs weaken on ack.smv (seed 4) and
# abp4 p2false (seed 1), 30 x 40, rotating the order of the arms on each
# repetition. Layout under ~/aragon-model-counting: base/ (no counting
# objective), path/ (PEREDUR's path count), prefix/ (the prefix count) and lasso/ (the lasso count)
# are trees of the four versions (`git archive`, or `git ls-files | tar` for
# an uncommitted one), models/ holds ack.smv and
# abp4-defines.smv (as tests/search.rs writes them), and spot/ a Spot
# 2.15.1 install; the remote Spot 2.14.5 lacks `ltlfilt --satisfiable`.
set -u
cd ~/aragon-model-counting
export PATH="$HOME/aragon-model-counting/spot/bin:$HOME/.local/bin:$HOME/.cargo/bin:/usr/local/bin:$PATH"
lim() { systemd-run --user --scope -q -p MemoryMax=16G -p MemorySwapMax=0 nice -n 10 timeout 1800 "$@"; }
ltlfilt --version | head -1
for arm in base path prefix lasso; do
  (cd $arm && CARGO_BUILD_JOBS=8 lim cargo build --release -q) || exit 1
done
mkdir -p out
ABP_PHI='G (s_get -> X (!s_get U (s_wait & ((!s_abp & r_ack0) | (s_abp & r_ack1)))))'
for rep in 1 2 3 4 5; do
  case $((rep % 4)) in
    1) order="base path prefix lasso" ;;
    2) order="path prefix lasso base" ;;
    3) order="prefix lasso base path" ;;
    *) order="lasso base path prefix" ;;
  esac
  for arm in $order; do
    lim ./$arm/target/release/weaken --model models/ack.smv --atoms req,slow,ack,late \
      --ltl 'G (req -> X ack)' --seed 4 --generations 30 --population 40 > out/ack-$arm-$rep.txt 2>&1
    lim ./$arm/target/release/weaken --model models/abp4-defines.smv --atoms s_get,s_wait,s_abp,r_ack0,r_ack1 \
      --ltl "$ABP_PHI" --seed 1 --generations 30 --population 40 > out/abp4-$arm-$rep.txt 2>&1
  done
done
echo done > out/DONE
