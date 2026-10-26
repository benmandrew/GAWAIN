#!/bin/sh
# Builds the instance directories of the evaluation (docs/dev/evaluation.md)
# under bench/data, leaving bench/data/graded, which earlier results read,
# as it is:
#
# - eval: bench/graded_prep.py, the ablation set with RERS 2018 problem 101;
# - eval-traces: the 126 trace-set instances of "Search on a sample" in
#   docs/dev/trace-sets.md (5 per family, seed 1).
#
# CEGIW's case studies run from bench/data/cegiw as they are.
#
# usage: bench/eval_prep.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
data=$here/data
[ -d "$data/cegiw" ] || { echo "eval_prep: no $data/cegiw" >&2; exit 1; }
python3 -I "$here/graded_prep.py" "$data/eval"
python3 -I "$here/traces_run.py" "$here/results/trace-sets-instances.csv" - - --graded "$data/eval-traces"
for d in eval eval-traces cegiw; do
	echo "$d: $(($(wc -l < "$data/$d/instances.tsv") - 1)) instances"
done
