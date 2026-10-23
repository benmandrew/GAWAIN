#!/usr/bin/env python3
"""Summarise `trace_bench check`, `trace_bench instances` or
`bench/traces_run.py` CSV by family.

Usage: python3 -I bench/traces_summary.py check|instances|runs FILE.csv

A family is the source and the directory under its sets/ folder, read from
the file path `bench/data/<source>/sets/<family>/<name>.trace`.
"""

import collections
import csv
import statistics
import sys


def family(path):
    parts = path.split("/")
    i = parts.index("sets")
    return parts[i - 1] + "/" + parts[i + 1]


def check(rows):
    groups = collections.defaultdict(list)
    for row in rows:
        groups[family(row["file"])].append(row)
    print("family,files,errors,separating,accept_fail,reject_fail,"
          "median_us,p90_us,max_us,median_us_per_position")
    for name, rs in sorted(groups.items()):
        ok = [r for r in rs if not r["error"]]
        sep = [r for r in ok if int(r["accepted"]) == int(r["positives"])
               and int(r["rejected"]) == int(r["negatives"])]
        acc = [r for r in ok if int(r["accepted"]) != int(r["positives"])]
        rej = [r for r in ok if int(r["rejected"]) != int(r["negatives"])]
        us = sorted(float(r["check_us"]) for r in ok)
        per = sorted(float(r["check_us"]) / (int(r["positives"]) * int(r["max_length"]))
                     for r in ok)
        q = lambda v, p: v[min(len(v) - 1, int(p * len(v)))] if v else float("nan")
        print(f"{name},{len(rs)},{len(rs) - len(ok)},{len(sep)},{len(acc)},"
              f"{len(rej)},{q(us, .5):.1f},{q(us, .9):.1f},"
              f"{us[-1] if us else float('nan'):.1f},{q(per, .5):.3f}")
    errors = collections.Counter(r["error"] for r in rows if r["error"])
    for e, n in errors.most_common():
        print(f"error,{n},{e}")


def instances(rows):
    groups = collections.defaultdict(list)
    for row in rows:
        groups[family(row["file"])].append(row)
    print("family,files,instances,median_phi_accepted_share")
    for name, rs in sorted(groups.items()):
        ok = [r for r in rs if not r["error"]]
        share = [int(r["phi_accepted"]) / int(r["positives"]) for r in ok]
        med = statistics.median(share) if share else float("nan")
        print(f"{name},{len(rs)},{len(ok)},{med:.2f}")
    errors = collections.Counter(r["error"] for r in rows if r["error"])
    for e, n in errors.most_common():
        print(f"error,{n},{e}")


def runs(rows):
    groups = collections.defaultdict(list)
    for row in rows:
        groups[row["family"]].append(row)
    groups["all"] = rows
    keys = ["memo_hits", "lasso_hits", "implication_hits", "backend_calls",
            "spot_calls"]
    print("family,runs,timeouts,failed,with_repairs,stronger,equivalent,weaker,"
          "incomparable,median_wall_s,max_wall_s,queries," + ",".join(keys))
    for name, rs in sorted(groups.items(), key=lambda kv: (kv[0] == "all", kv[0])):
        done = [r for r in rs if r["exit"] == "0"]
        vs = collections.Counter(r["vs_psi"] for r in done)
        walls = sorted(float(r["wall_s"]) for r in done)
        total = lambda k: sum(int(r[k]) for r in done)
        med = statistics.median(walls) if walls else float("nan")
        timeouts = sum(r["exit"] == "timeout" for r in rs)
        print(f"{name},{len(rs)},{timeouts},{len(rs) - len(done) - timeouts},"
              f"{sum(int(r['repairs']) > 0 for r in done)},{vs['stronger']},"
              f"{vs['equivalent']},{vs['weaker']},{vs['incomparable']},"
              f"{med:.2f},{walls[-1] if walls else float('nan'):.2f},"
              f"{total('queries')}," + ",".join(str(total(k)) for k in keys))


def main():
    mode, path = sys.argv[1], sys.argv[2]
    with open(path) as f:
        rows = list(csv.DictReader(f))
    {"check": check, "instances": instances, "runs": runs}[mode](rows)


if __name__ == "__main__":
    main()
