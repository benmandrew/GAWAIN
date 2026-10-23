#!/usr/bin/env python3
"""Run `weaken --traces` on a sample of trace-set weakening instances.

Usage:
  python3 -I bench/traces_run.py INSTANCES.csv WEAKEN OUT.csv
      [--per-family K] [--seed S] [--jobs J] [--generations G] [--population P]
      [--timeout SECONDS]

INSTANCES.csv is the output of `trace_bench instances`. K instances are drawn
per family (source and directory) with Python's random.Random(S). Each run
writes one row to OUT.csv: the repair count, the wall and gate times, the
oracle counters, the most negatives any repair rejects, and how the
repairs compare with ψ, the source formula and known weakening of φ:
`stronger` when some repair is strictly stronger than ψ, `equivalent`
when one is equivalent to it and none is stronger, else `weaker` when
every repair is implied by ψ, else `incomparable`. The comparison is one
`ltlfilt` exec per run, with `--from-ltlf` for finite traces. A run still
going after SECONDS (default 600) has its process group killed and is
recorded with exit `timeout`.
"""

import argparse
import collections
import concurrent.futures
import csv
import os
import random
import re
import signal
import subprocess
import sys

ALIVE = "gawain_alive"


def family(path):
    parts = path.split("/")
    i = parts.index("sets")
    return parts[i - 1] + "/" + parts[i + 1]


def implied(pairs, finite):
    """For each (a, b), whether Spot proves a -> b."""
    if not pairs:
        return []
    lines = "".join(f"({a}) & !({b})\n" for a, b in pairs)
    command = ["ltlfilt", "-F", "-", "-v", "--satisfiable", "--format=%L"]
    if finite:
        command.insert(1, f"--from-ltlf={ALIVE}")
    out = subprocess.run(command, input=lines, capture_output=True, text=True,
                         timeout=300)
    if out.returncode not in (0, 1):
        raise RuntimeError(out.stderr)
    proved = {int(l) for l in out.stdout.split()}
    return [i + 1 in proved for i in range(len(pairs))]


def compare(repairs, psi, finite):
    if not repairs:
        return "none"
    pairs = []
    for r in repairs:
        pairs += [(r, psi), (psi, r)]
    proved = implied(pairs, finite)
    kinds = []
    for k in range(len(repairs)):
        to_psi, from_psi = proved[2 * k], proved[2 * k + 1]
        kinds.append("equivalent" if to_psi and from_psi else
                     "stronger" if to_psi else
                     "weaker" if from_psi else "incomparable")
    for kind in ("stronger", "equivalent"):
        if kind in kinds:
            return kind
    return "weaker" if all(k == "weaker" for k in kinds) else "incomparable"


def run(row, args):
    command = [args.weaken, "--traces", row["file"], "--ltl", row["phi"],
               "--seed", str(args.seed), "--generations", str(args.generations),
               "--population", str(args.population)]
    result = {"file": row["file"], "family": family(row["file"]),
              "semantics": row["semantics"], "psi": row["psi"],
              "phi": row["phi"], "negatives": row["negatives"]}
    child = subprocess.Popen(command, stdout=subprocess.PIPE,
                             stderr=subprocess.DEVNULL, text=True,
                             start_new_session=True)
    try:
        stdout, _ = child.communicate(timeout=args.timeout)
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGKILL)
        child.communicate()
        result.update(exit="timeout", wall_s=args.timeout)
        return result
    result["exit"] = child.returncode
    repairs = []
    rejected = []
    for line in stdout.splitlines():
        if line.startswith("repair\t"):
            repairs.append(line.rsplit("\t", 1)[1])
        elif line.startswith("negatives\t"):
            rejected.append(int(line.split("\t")[1].split("/")[0]))
        elif line.startswith("oracle: "):
            for key, value in re.findall(r"(\w+)=(\d+)", line):
                result[key] = int(value)
        elif line.startswith("wall: "):
            wall, gate = re.findall(r"([\d.]+)s", line)
            result["wall_s"], result["gate_s"] = float(wall), float(gate)
    result["repairs"] = len(repairs)
    result["max_negatives_rejected"] = max(rejected, default="")
    result["vs_psi"] = compare(repairs, row["psi"], row["semantics"] == "finite")
    return result


FIELDS = ["file", "family", "semantics", "psi", "phi", "exit", "repairs",
          "vs_psi", "max_negatives_rejected", "negatives", "wall_s", "gate_s",
          "queries", "memo_hits", "lasso_hits", "implication_hits",
          "spot_calls", "spot_errors", "backend_calls", "backend_unknowns"]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("instances")
    parser.add_argument("weaken")
    parser.add_argument("out")
    parser.add_argument("--per-family", type=int, default=5)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--generations", type=int, default=30)
    parser.add_argument("--population", type=int, default=30)
    parser.add_argument("--timeout", type=float, default=600)
    args = parser.parse_args()
    with open(args.instances) as f:
        rows = [r for r in csv.DictReader(f) if not r["error"]]
    groups = collections.defaultdict(list)
    for r in rows:
        groups[family(r["file"])].append(r)
    rng = random.Random(args.seed)
    sample = []
    for name in sorted(groups):
        members = groups[name]
        sample += rng.sample(members, min(args.per_family, len(members)))
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        results = list(pool.map(lambda r: run(r, args), sample))
    with open(args.out, "w", newline="") as f:
        writer = csv.DictWriter(f, FIELDS, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(results)
    print(f"{len(results)} runs", file=sys.stderr)


if __name__ == "__main__":
    main()
