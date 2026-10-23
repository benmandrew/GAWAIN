#!/bin/sh
# Fetches the MCC 2026 data used by the GAWAIN MCC subset into
# bench/data/mcc/ (gitignored): the consensus oracle archive, which also
# holds Tedd's StateSpace counts (*-SS.out), and one archive per instance.
#
# usage: bench/mcc_fetch.sh                 # oracle only
#        bench/mcc_fetch.sh INSTANCE...     # oracle and these instances
#        bench/mcc_fetch.sh -f LIST         # oracle and every instance in LIST
#                                           # (first column of each line)
#
# Source: https://yanntm.github.io/pnmcc-models-2026/ (Yann Thierry-Mieg,
# extracted from the MCC 2026 archives at https://mcc.lip6.fr/archives/).
set -eu
base=https://yanntm.github.io/pnmcc-models-2026
data=$(dirname "$0")/data/mcc
mkdir -p "$data/inputs"
if [ ! -d "$data/oracle" ]; then
	curl -sSfL -o "$data/oracle.tar.gz" "$base/oracle.tar.gz"
	mkdir -p "$data/oracle"
	tar xzf "$data/oracle.tar.gz" -C "$data/oracle" --strip-components=1
	rm "$data/oracle.tar.gz"
fi
if [ "${1:-}" = -f ]; then
	set -- $(awk -F, '!/^#/ && NF && $1 != "instance" { print $1 }' "$2" | sort -u)
fi
for instance in "$@"; do
	[ -d "$data/inputs/$instance" ] && continue
	curl -sSfL -o "$data/inputs/$instance.tgz" "$base/INPUTS/$instance.tgz"
	tar xzf "$data/inputs/$instance.tgz" -C "$data/inputs"
	rm "$data/inputs/$instance.tgz"
done
