#!/bin/bash
# Measures where the memory of the MCC searches goes ("Bounded monoid" in
# docs/dev/search.md). Every MCC instance of bench/data/graded/instances.tsv
# whose name matches FILTER and not EXCLUDE runs once per seed, at the
# ablation's budget (30 x 40, 120 s and 4 GB per back-end call), JOBS at a
# time, each in its own systemd scope capped at GB. Once a second the
# script records the scope's memory.current and memory.peak (OUT/<run>/cg.log)
# and the VmHWM and VmRSS in kB of every process in it (procs.log). The
# JSON line goes to stdout, stderr to stderr, the exit status to exit.
# bench/mcc_memory_stats.py summarises OUT.
#
# Run it from the checkout, with verifypn and Spot 2.15.1 on PATH as in
# bench/graded_remote.sh.
#
# usage: bench/mcc_memory.sh BIN OUT JOBS GB SEEDS [FILTER] [EXCLUDE]
#   e.g. bench/mcc_memory.sh target/release/search_bench mem/after 3 8 "1 2 3"
set -u
if [ "$1" = --one ]; then
	# One run: --one DIR GB -- command...
	out=$2 gb=$3
	shift 4
	mkdir -p "$out"
	unit=mcc-memory-$(basename "$out")-$$
	systemd-run --user --scope -q --unit="$unit" -p MemoryMax="${gb}G" -p MemorySwapMax=0 \
		nice -n 10 timeout 3600 "$@" > "$out/stdout" 2> "$out/stderr" &
	pid=$!
	cg=/sys/fs/cgroup/user.slice/user-$(id -u).slice/user@$(id -u).service/app.slice/$unit.scope
	start=$(date +%s)
	while kill -0 $pid 2> /dev/null; do
		if [ -r "$cg/memory.current" ]; then
			t=$(($(date +%s) - start))
			echo "$t $(cat "$cg/memory.current") $(cat "$cg/memory.peak")" >> "$out/cg.log"
			for p in $(cat "$cg/cgroup.procs" 2> /dev/null); do
				[ -r /proc/$p/status ] || continue
				echo "$t $p $(awk '/VmHWM|VmRSS/ {printf "%s ", $2}' /proc/$p/status)" \
					"$(tr '\0' ' ' < /proc/$p/cmdline | cut -c1-150)"
			done >> "$out/procs.log"
		fi
		sleep 1
	done
	wait $pid
	echo "exit $?" > "$out/exit"
	exit 0
fi
bin=$1 out=$2 jobs=$3 gb=$4 seeds=$5 filter=${6:-.} exclude=${7:-^$}
d=bench/data/graded
mkdir -p "$out"
grep -P '\tmcc\t' $d/instances.tsv | grep -E "$filter" | grep -vE "$exclude" | cut -f1,4,7 |
	while read -r name model prop; do
		for s in $seeds; do
			echo "--one $out/$name-$s $gb -- $bin --backend tapaal --model $d/$model" \
				"--property $prop --seed $s --generations 30 --population 40" \
				"--timeout 120 --memory-mb 4096 --status binary"
		done
	done | xargs -P "$jobs" -L 1 "$0"
