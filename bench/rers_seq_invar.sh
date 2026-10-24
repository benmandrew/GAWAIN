#!/bin/sh
# Check `G ev != e_EVENT` for one RERS problem with nuXmv's IC3.
#
#   sh bench/rers_seq_invar.sh YEAR PROB MEM_GB SECONDS OUTDIR TAG ENGINE EVENT...
#
# Builds the SMV model of bench/rers_seq_smv.py, then runs one nuXmv
# session that checks each listed invariant with check_invar_ic3, after
# ENGINE (go_msat for SMT-based IC3, go_bmc for bit-level IC3):
# an EVENT such as iN stands for `ev != e_iN`, and `oob` for the bounds
# invariant `!oob`. Writes OUTDIR/invar-YEAR-PROB-TAG.log.
set -eu
year=$1 prob=$2 mem=$3 secs=$4 out=$5 tag=$6 engine=$7
shift 7
here=$(dirname "$0")
mkdir -p "$out"
model=$out/model-$year-$prob-$tag.smv
cmds=$out/invar-$year-$prob-$tag.cmd
systemd-run --user --scope -q -p MemoryMax="${mem}G" -p MemorySwapMax=0 nice -n 10 \
  timeout 3600 python3 "$here/rers_seq_smv.py" model --years "$year" --problems "$prob" > "$model"
{
  echo "set on_failure_script_quits 1"
  echo "$engine"
  echo "time"
  echo "echo @@build"
  for e in "$@"; do
    if [ "$e" = oob ]; then p="!oob"; else p="ev != e_$e"; fi
    echo "check_invar_ic3 -p \"$p\""
    echo "time"
    echo "echo @@$e"
  done
  echo "quit"
} > "$cmds"
systemd-run --user --scope -q -p MemoryMax="${mem}G" -p MemorySwapMax=0 nice -n 10 \
  timeout "$secs" nuXmv -source "$cmds" "$model" > "$out/invar-$year-$prob-$tag.log" 2>&1
