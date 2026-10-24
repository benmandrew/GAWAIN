#!/bin/bash
# Replays dumped lasso counts under several builds of count_bench ("Lasso
# count time" in docs/dev/search.md). Each ARM is a checkout with
# target/release/count_bench built; each DUMP a file written by a search
# with bench/notes/count-dump.patch applied. Every (dump, arm) pair runs
# once, JOBS at a time, the arms of one dump in the order given and then
# rotated for the next dump, each in its own systemd scope capped at 8 GB
# and 3600 s, with GAWAIN_COUNT_PROFILE set. OUT/<dump>.<arm>.out gets the
# counts and OUT/<dump>.<arm>.err the summary line and the profile lines.
#
# Run it from the directory holding the arms, with Spot 2.15.1 on PATH.
#
# usage: bench/count_replay.sh OUT JOBS "ARM..." DUMP...
set -u
if [ "$1" = --one ]; then
	out=$2 arm=$3 dump=$4
	name=$(basename "$dump" .txt)
	systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 \
		nice -n 10 timeout 3600 env GAWAIN_COUNT_PROFILE=1 \
		"$arm/target/release/count_bench" "$dump" \
		> "$out/$name.$arm.out" 2> "$out/$name.$arm.err"
	exit 0
fi
out=$1 jobs=$2 arms=($3)
shift 3
mkdir -p "$out"
i=0
for dump in "$@"; do
	n=${#arms[@]}
	for k in $(seq 0 $((n - 1))); do
		echo "--one $out ${arms[$(((i + k) % n))]} $dump"
	done
	i=$((i + 1))
done | xargs -P "$jobs" -L 1 "$0"
