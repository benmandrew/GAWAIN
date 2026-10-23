#!/usr/bin/env python3
"""Run `trace_bench instances` on a seeded sample of trace sets per family.

Usage: python3 -I bench/traces_sample.py TRACE_BENCH K SEED DIR... > OUT.csv

Draws K `*.trace` files per family (source and directory under sets/) with
random.Random(SEED), then runs `TRACE_BENCH instances --seed SEED` on the
sample in one process, so the CSV has one header. Building an instance
costs one `ltlfilt` exec per file, up to 30 s under `--from-ltlf`, which
is too slow for all 16,822 sets.
"""

import collections
import os
import random
import sys


def main():
    trace_bench, k, seed, dirs = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4:]
    groups = collections.defaultdict(list)
    for top in dirs:
        for root, _, names in os.walk(top):
            for name in names:
                if name.endswith(".trace"):
                    path = os.path.join(root, name)
                    parts = path.split("/")
                    i = parts.index("sets")
                    groups[parts[i - 1] + "/" + parts[i + 1]].append(path)
    rng = random.Random(int(seed))
    sample = []
    for family in sorted(groups):
        members = sorted(groups[family])
        sample += rng.sample(members, min(k, len(members)))
    os.execv(trace_bench, [trace_bench, "instances", "--seed", seed] + sample)


if __name__ == "__main__":
    main()
