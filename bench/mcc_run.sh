#!/bin/sh
# Runs mcc_check over a candidate list, one memory-capped job per instance,
# JOBS at a time, and writes one CSV per instance into OUT.
#
# usage: bench/mcc_run.sh LIST OUT JOBS TIMEOUT [MCC_CHECK]
#   LIST     CSV from bench/mcc_select.py: instance first, formula fourth
#            (such as LTLCardinality-03)
#   OUT      output directory; OUT/<instance>.csv per instance
#   JOBS     parallel jobs
#   TIMEOUT  seconds per verifypn call (mcc_check --timeout)
# verifypn must be on PATH. Each job is capped at 16 GB with no swap, and
# an instance whose CSV exists already is skipped, so a run can resume.
set -eu
list=$1 out=$2 jobs=$3 timeout=$4
check=${5:-$(dirname "$0")/../target/release/mcc_check}
data=$(dirname "$0")/data/mcc/inputs
mkdir -p "$out"
awk -F, 'NR > 1 { ids[$1] = ids[$1] (ids[$1] ? "," : "") $1 "-" $4 }
	END { for (i in ids) print i, ids[i] }' "$list" |
	xargs -P "$jobs" -L 1 sh -c '
		out=$0 check=$1 data=$2 timeout=$3 instance=$4 ids=$5
		[ -s "$out/$instance.csv" ] && exit 0
		systemd-run --user --scope -q -p MemoryMax=16G -p MemorySwapMax=0 \
			nice -n 10 "$check" --instance "$data/$instance" --formulas "$ids" \
			--timeout "$timeout" --memory-mb 12288 \
			> "$out/$instance.part" 2> "$out/$instance.err" || true
		mv "$out/$instance.part" "$out/$instance.csv"' "$out" "$check" "$data" "$timeout"
