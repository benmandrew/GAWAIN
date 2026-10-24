#!/usr/bin/env python3
"""Random-walk counterexamples for the fixes of bench/sets/rers-precedes.csv.

For each named (year, problem, property), the script reads the pattern
from the English gloss, builds bench/rers_seq_walk.c against the program,
and runs random walks steered by a monitor for that pattern. A lasso the
walk reports counts only when two independent checks pass: the program
replays it (rers_seq.replay_lasso), and Spot accepts the event word
u v^omega in the automaton of !fix (`autfilt --accept-word`). Such a
lasso is a run of the program that violates the fix, so the fix fails.
A walk that finds nothing decides nothing.

Usage: rers_seq_walk.py OUT.tsv SEED WALKS STEPS CYCLE_STATES YEAR/PROB/PROP...
Writes one row per property: year, problem, prop, verdict (fails or
unknown), seconds, and the lasso.
"""

import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import rers_seq

HERE = Path(__file__).resolve().parent

PATTERNS = [
    # "S P precedes S2 Q after Q2 R until Q3 R2" with event names.
    ("prec", re.compile(r"(input|output) (\w) precedes (input|output) (\w) after "
                        r"(input|output) (\w) until (input|output) (\w)$")),
    ("chain", re.compile(r"(input|output) (\w) always responds to (input|output) (\w), "
                         r"(input|output) (\w)$")),
    ("resp", re.compile(r"(input|output) (\w) responds to (input|output) (\w) after "
                        r"(input|output) (\w) until (input|output) (\w)$")),
]


def code(kind, letter):
    v = ord(letter) - 64
    return v if kind == "input" else 100 + v


def pattern(gloss):
    """The walk's KIND and ARGS for an English gloss."""
    for kind, rx in PATTERNS:
        m = rx.match(gloss.strip())
        if not m:
            continue
        e = [code(m.group(2 * k + 1), m.group(2 * k + 2)) for k in range(len(m.groups()) // 2)]
        if kind == "prec":  # R precedes S after P until Q -> P Q S R
            return kind, [e[2], e[3], e[1], e[0]]
        if kind == "chain":  # T responds to N, M -> N M T
            return kind, [e[1], e[2], e[0]]
        return kind, [e[2], e[3], e[1], e[0]]  # X responds to O after R until M
    raise ValueError(f"no pattern for {gloss!r}")


def build(year, prob, wd):
    """Compile the program once; link the replay explorer and the walk."""
    wrap = wd / "wrap.c"
    wrap.write_text(rers_seq.WRAPPER.format(source=rers_seq.problem_source(year, prob).resolve()))
    obj = wd / "problem.o"
    rers_seq.run(["gcc", "-O1", "-fwrapv", "-fno-pie", "-fno-common",
                  "-fno-zero-initialized-in-bss", "-w", "-c", str(wrap), "-o", str(obj)], 7200)
    rers_seq.run(["objcopy", "--rename-section", ".data=rers_state", str(obj)], 600)
    exe, walk = wd / "explore", wd / "walk"
    for main, out in ((HERE / "rers_seq_explore.c", exe), (HERE / "rers_seq_walk.c", walk)):
        rers_seq.run(["gcc", "-O2", "-no-pie", str(main), str(obj), "-o", str(out)], 600)
    obj.unlink()
    return exe, walk


def word(exe, stem, loop):
    """The event lasso (u, v) of the run that feeds the inputs `stem` and
    then `loop` forever, with the outputs the program prints."""
    r = rers_seq.run([str(exe), "replay"], 600,
                     input="".join(f"{i}\n" for i in stem + loop))
    events = []
    for i, line in zip(stem + loop, r.stdout.splitlines()):
        outs = line.split("|")[0].split()
        events.append([rers_seq.event("i", i)] + [rers_seq.event("o", int(o)) for o in outs])
    flat = [[e for step in part for e in step] for part in (events[:len(stem)], events[len(stem):])]
    return flat[0], flat[1]


def spot_accepts(atoms, neg_hoa, u, v):
    """Does the automaton neg_hoa accept the word u v^omega? The word is
    written as a lasso-shaped automaton with one letter per edge, and Spot
    checks that its product with neg_hoa is non-empty."""
    letter = {a: "&".join(str(j) if b == a else f"!{j}" for j, b in enumerate(atoms))
              for a in atoms}
    word = u + v
    lasso = Path(neg_hoa).with_name("lasso.hoa")
    with open(lasso, "w") as fh:
        fh.write(f"HOA: v1\nStates: {len(word)}\nStart: 0\n")
        fh.write(f"AP: {len(atoms)} " + " ".join(f'"{a}"' for a in atoms) + "\n")
        fh.write("Acceptance: 0 t\n--BODY--\n")
        for k, a in enumerate(word):
            nxt = k + 1 if k + 1 < len(word) else len(u)
            fh.write(f"State: {k}\n[{letter[a]}] {nxt}\n")
        fh.write("--END--\n")
    r = subprocess.run(["autfilt", f"--intersect={neg_hoa}", "--is-empty", "-c", str(lasso)],
                       capture_output=True, text=True, timeout=600)
    lasso.unlink()
    return r.stdout.strip() == "0"


def main():
    out, seed, walks, steps, cycle = sys.argv[1:6]
    rows = []
    built = {}
    for spec in sys.argv[6:]:
        year, prob, prop = map(int, spec.split("/"))
        t0 = time.monotonic()
        atoms, props = rers_seq.load_properties(year, prob)
        (_, gloss, f), = [p for p in props if p[0] == prop]
        fix = rers_seq.to_spot(rers_seq.corrected(f))
        kind, args = pattern(gloss)
        if (year, prob) not in built:
            wd = Path(tempfile.mkdtemp(prefix=f"rers-walk-{year}-{prob}-"))
            built[(year, prob)] = (wd, *build(year, prob, wd))
        wd, exe, walk = built[(year, prob)]
        neg = rers_seq.translate(fix, wd / "neg.hoa")  # automaton of !fix
        r = subprocess.run([str(walk), seed, walks, steps, cycle, kind] + list(map(str, args)),
                           capture_output=True, text=True)
        verdict, lasso = "unknown", ""
        for line in r.stdout.splitlines():
            if not line.startswith("LASSO"):
                continue
            pre, loop = line[5:].split("|")
            u, v = word(exe, [int(x) for x in pre.split()], [int(x) for x in loop.split()])
            if rers_seq.replay_lasso(exe, u, v) and spot_accepts(atoms, neg, u, v):
                verdict, lasso = "fails", " ".join(u) + " | " + " ".join(v)
                break
        prefixes = r.stdout.splitlines()[-1].split()[1].split("=")[1] if r.stdout else "?"
        secs = time.monotonic() - t0
        rows.append((year, prob, prop, kind, verdict, f"{secs:.1f}", prefixes, lasso))
        print(f"{year}/{prob}/{prop} {kind}: {verdict} in {secs:.1f} s "
              f"({r.stdout.splitlines()[-1] if r.stdout else r.stderr[-200:]})", flush=True)
        neg.unlink()
    for wd, _, _ in built.values():
        for p in wd.iterdir():
            p.unlink()
        wd.rmdir()
    with open(out, "w") as fh:
        fh.write(f"# seed={seed} walks={walks} steps={steps} cycle_states={cycle}\n")
        fh.write("year\tproblem\tprop\tpattern\tverdict\tseconds\tbad_points\tlasso\n")
        for row in rows:
            fh.write("\t".join(map(str, row)) + "\n")


if __name__ == "__main__":
    main()
