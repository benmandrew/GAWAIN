#!/bin/sh
# Run oracle_bench on each job of a jobs.tsv from bench/oracle_prep.py, PARALLEL at once,
# each in its own memory-capped scope. One CSV (and its stderr) per job in OUT_DIR.
#
# usage: oracle_run.sh JOBS_TSV OUT_DIR PARALLEL MEMORY_GB WALL_S [oracle_bench flags...]
#
# MEMORY_GB caps the job's scope; WALL_S bounds the whole job. The remaining flags
# (--one, --two, --seed, --timeout, --memory-mb, ...) go to every job.
set -eu
jobs=$1 out=$2 parallel=$3 mem=$4 wall=$5
shift 5
mkdir -p "$out"
export OUT="$out" MEM="$mem" WALL="$wall" FLAGS="$*"
# shellcheck disable=SC2016
tr '\t' ' ' < "$jobs" | xargs -P "$parallel" -L 1 sh -c '
  name="$OUT/$0__$1"
  systemd-run --user --scope -q -p MemoryMax="${MEM}G" -p MemorySwapMax=0 \
    nice -n 10 timeout "$WALL" target/release/oracle_bench --backend "$2" --model "$3" \
      --atoms "$4/atoms" --properties "$4/properties" --bench "$0" $FLAGS \
      > "$name.csv" 2> "$name.err" || echo "exit $? $0 $1" >> "$name.err"
'
