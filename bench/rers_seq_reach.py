#!/usr/bin/env python3
"""Decide whether a RERS sequential program can print one output, by
explicit search over its mutable globals only.

The program is compiled as for bench/rers_seq.py and linked with
bench/rers_seq_reach.c. Its globals of at most 8 bytes (ints and
pointers) form the stored state; every larger global is an array, and the
harness checks after each step that no array byte changed, so the search
is the exact explicit search of rers_seq.py with smaller states. It needs
gcc, objcopy and nm on PATH.

  rers_seq_reach.py 2017 6 oW [--max-states N] [--tmp DIR]

prints the harness's last line: `END states=N` (the output never occurs),
`FOUND depth=D inputs=...` or `LIMIT states=N`.
"""

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

import rers_seq


def mask(exe):
    """The (offset, length) of each global of at most 8 bytes, relative to
    the start of the state section, in address order."""
    out = subprocess.run(["nm", "-S", str(exe)], capture_output=True, text=True,
                         check=True).stdout
    start = stop = None
    syms = []
    for line in out.splitlines():
        f = line.split()
        if len(f) == 3 and f[2] == "__start_rers_state":
            start = int(f[0], 16)
        elif len(f) == 3 and f[2] == "__stop_rers_state":
            stop = int(f[0], 16)
        elif len(f) == 4 and f[2] in "dD":
            syms.append((int(f[0], 16), int(f[1], 16), f[3]))
    segs = sorted((a - start, n) for a, n, _ in syms
                  if start <= a < stop and n <= 8)
    return segs


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("year", type=int)
    ap.add_argument("problem", type=int)
    ap.add_argument("event", help="an output atom such as oW")
    ap.add_argument("--max-states", type=int, default=4_000_000_000)
    ap.add_argument("--tmp", type=Path, default=None)
    args = ap.parse_args()
    if not re.fullmatch(r"o[A-Z]", args.event):
        sys.exit("the event must be an output atom")
    target = ord(args.event[1]) - 64
    with tempfile.TemporaryDirectory(dir=args.tmp) as wd:
        wd = Path(wd)
        exe = rers_seq.compile_problem(args.year, args.problem, wd, "-O0",
                                       "rers_seq_reach.c")
        segs = mask(exe)
        (wd / "mask").write_text("".join(f"{o} {n}\n" for o, n in segs))
        r = subprocess.run([str(exe), str(wd / "mask"), str(target), str(args.max_states)],
                           stdout=subprocess.PIPE, text=True)
        last = (r.stdout.strip().splitlines() or [f"exit {r.returncode}"])[-1]
        if r.returncode not in (0, 3):
            sys.exit(f"reach exit {r.returncode}")
        print(f"RESULT {args.year}/{args.problem} {args.event} {last}")


if __name__ == "__main__":
    main()
