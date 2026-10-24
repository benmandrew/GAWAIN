#!/bin/sh
# Starts bench/graded_run.py on a remote host (av2 or av3), detached, with
# the tools that host lacks on PATH: verifypn in ./bin and Spot 2.15.1 in
# ./spot (the hosts' own Spot 2.14.5 lacks `ltlfilt --satisfiable`). Run it
# from the checkout on the remote host; the log goes to DIR/run-<tag>.log.
#
# usage: bench/graded_remote.sh DIR JOBS TAG [graded_run.py options...]
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
dir=$1 jobs=$2 tag=$3
shift 3
export PATH="$here/bin:$here/spot/bin:$HOME/.local/bin:$PATH"
export LD_LIBRARY_PATH="$here/spot/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
nohup setsid python3 "$here/bench/graded_run.py" "$dir" "$jobs" "$@" \
	> "$dir/run-$tag.log" 2>&1 < /dev/null &
echo "started $tag: pid $!"
