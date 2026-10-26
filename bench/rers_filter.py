#!/usr/bin/env python3
"""Degeneracy filter for the RERS parallel LTL track (2016-2019).

For every property phi of every Promela model M, check both phi and !(phi)
with SPIN. A false property survives the filter when M |/= phi and M |/= !phi,
i.e. some runs satisfy phi and some do not. M |/= phi comes from SPIN or, when
SPIN times out, from the official solution.

Per claim (phi or !(phi)): strip the shipped `ltl` blocks, append one
`ltl c {...}`, run `spin -a`, gcc and `pan -a`, each memory- and time-capped
in a systemd scope. Claims run in parallel across all models.

Formulas that use X are first rewritten to read only the states where
Listener@step holds (one per action; see `project`), and their never claims
come from Spot's ltl2tgba, which must be on PATH.

Verdicts per claim:
  holds    - no acceptance cycle and the search was complete
  violated - pan found a counterexample (valid even if the search was cut)
  unknown  - no counterexample, but the search was incomplete (depth limit,
             memory limit, vector size) or timed out

Stdlib only. Data under bench/data/rers is read-only; all work happens in a
temporary directory.
"""

import argparse
import concurrent.futures as cf
import csv
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
DATA = HERE / "data" / "rers"
OUT = HERE / "results" / "rers-parallel-filter.csv"

MODELS = {
    2016: ("2016/ParallelRERS2016_version2/ParallelRERS2016_version2", range(101, 121)),
    2017: ("2017/ParallelRERS2017_version2/ParallelRERS2017", range(101, 116)),
    2018: ("2018/ParallelLtlRers2018WithPml/ParallelLtlRers2018WithPml", range(101, 104)),
    2019: ("2019/ParallelRers2019Models/ParallelRers2019Models", range(101, 110)),
}

SOLUTIONS = {
    2016: "2016/participantsolutions/Parallel-solution.csv",
    2017: "2017/RERS_2017_results_and_solutions/RERS_2017_results_and_solutions/"
          "solutions/Solutions_Parallel_LTL_Graphs_Petri_RERS_2017.csv",
    2018: "2018/RERS_18_submissions_and_solutions/RERS_18_submissions_and_solutions/"
          "solutions/RERS_18_solutions_dot_petri.csv",
    2019: "2019/solution-and-submissions-RERS-2019/solution/Par_LTL_solution.csv",
}

# 2017 uses the dot/Petri answers. The Promela answers differ on 23 properties
# of problems 111-115, all of them with X: they read X as SPIN's next state,
# stutter steps included, rather than the next action.
# 2018 has 23 answers per problem against 20 pml properties; answers 1-20
# line up with the pml numbering (checked against SPIN verdicts).

# Value in the solution file -> does the property hold?
TRUTH = {"true": True, "false": False, "yes": True, "no": False}

LTL_RE = re.compile(r"^\s*ltl\s+(\w+)\s*\{(.*)\}\s*$")

# Most specific first: a MEMLIM stop also prints "Search not completed".
INCOMPLETE_MARKERS = (
    "max search depth too small",
    "reached -DMEMLIM bound",
    "out of memory",
    "Search not completed",
    "VECTORSZ is too small",
    "aborted",
)


def load_solutions(year):
    """Return {(problem, prop): bool}."""
    text = (DATA / SOLUTIONS[year]).read_bytes().decode()
    sol = {}
    for line in re.split(r"\r\n|\r|\n", text):
        parts = [p.strip() for p in line.split(",")]
        if len(parts) != 3 or not parts[0].isdigit():
            continue
        sol[(int(parts[0]), int(parts[1]))] = TRUTH[parts[2].lower()]
    return sol


def parse_model(pml):
    """Split a Promela file into (body without ltl lines, [(n, formula)])."""
    body, props = [], []
    for line in pml.read_text().splitlines():
        m = LTL_RE.match(line)
        if m:
            name = m.group(1)
            assert name.startswith("p"), name
            props.append((int(name[1:]), m.group(2).strip()))
        else:
            body.append(line)
    return "\n".join(body), props


# --- Action-step projection for formulas that use X -------------------------
#
# In the shipped models, lastAction changes only when the Listener receives an
# action, and the never claim also steps through the intermediate states
# (environment hand-offs, internal channel syncs). Stutter-invariant formulas
# do not notice, but X does: `G(a => X !a)` fails on the stutter state that
# repeats `a`. RERS means X as "next action". So formulas that contain X are
# evaluated on the subsequence of states where `Listener@step` holds, which is
# exactly one state per action.

STEP = "Listener@step"
TOKEN_RE = re.compile(r"\s*(<->|->|&&|\|\||\[\]|<>|[()!]|\w+)")


def tokenize(f):
    pos, out = 0, []
    f = f.strip()
    while pos < len(f):
        m = TOKEN_RE.match(f, pos)
        if not m:
            raise ValueError(f"cannot tokenise {f[pos:]!r}")
        out.append(m.group(1))
        pos = m.end()
    return out


def parse_ltl(f):
    """Parse SPIN LTL into nested tuples. Precedence, loosest first:
    -> <->, ||, &&, U V W, unary (! [] <> X)."""
    toks = tokenize(f)
    i = 0

    def peek():
        return toks[i] if i < len(toks) else None

    def take():
        nonlocal i
        i += 1
        return toks[i - 1]

    def binary(ops, sub, right=False):
        def rule():
            lhs = sub()
            while peek() in ops:
                op = take()
                rhs = rule() if right else sub()
                lhs = (op, lhs, rhs)
                if right:
                    break
            return lhs
        return rule

    def unary():
        t = peek()
        if t in ("!", "[]", "<>", "X"):
            take()
            return (t, unary())
        if t == "(":
            take()
            e = implies()
            assert take() == ")", f
            return e
        take()
        return t

    until = binary(("U", "V", "W"), unary)
    conj = binary(("&&",), until)
    disj = binary(("||",), conj)
    implies = binary(("->", "<->"), disj, right=True)
    e = implies()
    assert i == len(toks), f
    return e


def project(e):
    """Rewrite e so that it reads only the Listener@step states."""
    if isinstance(e, str):
        return e
    op, *args = e
    a = [project(x) for x in args]
    S = STEP
    if op == "!":
        return f"!({a[0]})"
    if op in ("&&", "||", "->", "<->"):
        return f"(({a[0]}) {op} ({a[1]}))"
    if op == "X":
        return f"X((!{S}) U ({S} && ({a[0]})))"
    if op == "U":
        return f"(({S} -> ({a[0]})) U ({S} && ({a[1]})))"
    if op == "W":
        return f"(({S} -> ({a[0]})) W ({S} && ({a[1]})))"
    if op == "V":
        return f"(({S} && ({a[0]})) V ({S} -> ({a[1]})))"
    if op == "[]":
        return f"[]({S} -> ({a[0]}))"
    if op == "<>":
        return f"<>({S} && ({a[0]}))"
    raise ValueError(op)


def uses_next(f):
    return re.search(r"\bX\b", f) is not None


def claim_text(f):
    """The formula to give SPIN: as shipped, or step-projected if it uses X."""
    if not uses_next(f):
        return f
    return f"((!{STEP}) U ({STEP} && ({project(parse_ltl(f))}))) || [](!{STEP})"


def limited(cmd, seconds):
    """Wrap a command so it runs in a memory-capped, niced, time-limited scope."""
    return ["systemd-run", "--user", "--scope", "-q",
            "-p", f"MemoryMax={ARGS.memmax}M", "-p", "MemorySwapMax=0",
            "nice", "-n", "10", "timeout", str(int(seconds)), *cmd]


def check_claim(workdir, body, formula):
    """Build and run one claim in its own directory.

    Each claim gets its own model with a single `ltl` block, so a slow LTL
    translation or compile affects only that claim. Returns
    (verdict, reason, pan seconds).
    """
    workdir.mkdir()
    if uses_next(formula):
        # SPIN's own LTL translation takes many minutes on the projected
        # formulas, so build the never claim (for the negation) with Spot.
        text = claim_text(formula).replace(STEP, "stp")
        try:
            r = subprocess.run(limited(["ltl2tgba", "--spin", f"!({text})"],
                                       ARGS.build_timeout),
                               capture_output=True, text=True,
                               timeout=ARGS.build_timeout + 60)
        except subprocess.TimeoutExpired:
            return "unknown", "ltl2tgba timeout", 0.0
        if r.returncode != 0:
            return "unknown", f"ltl2tgba exit {r.returncode}", 0.0
        claim = f"#define stp ({STEP})\n{r.stdout}"
    else:
        claim = f"ltl c {{ {formula} }}"
    (workdir / "model.pml").write_text(f"{body}\n\n{claim}\n")
    flags = ["-DCOLLAPSE", "-DVECTORSZ=8192", f"-DMEMLIM={ARGS.memmax * 7 // 8}"]
    # Partial-order reduction is unsound for formulas that use next-time.
    if uses_next(formula):
        flags.append("-DNOREDUCE")
    for step, cmd, secs in (("spin -a", ["spin", "-a", "model.pml"], ARGS.build_timeout),
                            ("gcc", ["gcc", "-O2", *flags, "-w", "-o", "pan", "pan.c"],
                             ARGS.build_timeout)):
        # Retry: under load, systemd-run itself sometimes fails to start the scope.
        for attempt in range(3):
            if step == "gcc" and attempt > 0:
                # gcc -O2 needs over 8 GB on the largest 2017 pan.c files.
                cmd = [("-O0" if c == "-O2" else c) for c in cmd]
            try:
                r = subprocess.run(limited(cmd, secs), cwd=workdir, capture_output=True,
                                   text=True, timeout=secs + 60)
            except subprocess.TimeoutExpired:
                return "unknown", f"{step} timeout", 0.0
            if r.returncode in (0, 124):
                break
            time.sleep(5 * (attempt + 1))
        if r.returncode == 124:
            return "unknown", f"{step} timeout", 0.0
        if r.returncode != 0:
            tail = (r.stderr or r.stdout).strip().splitlines()[-1:] or [""]
            return "unknown", f"{step} exit {r.returncode}: {tail[0][:60]}", 0.0
    t0 = time.monotonic()
    try:
        r = subprocess.run(
            limited(["./pan", "-a", f"-m{ARGS.depth}"], ARGS.timeout),
            cwd=workdir, capture_output=True, text=True, timeout=ARGS.timeout + 60)
    except subprocess.TimeoutExpired:
        return "unknown", "timeout", time.monotonic() - t0
    dt = time.monotonic() - t0
    (workdir / "pan.out").write_text(r.stdout + r.stderr)
    if r.returncode == 124:
        return "unknown", "timeout", dt
    out = r.stdout + r.stderr
    m = re.search(r"errors:\s*(\d+)", out)
    if m and int(m.group(1)) > 0:
        return "violated", "", dt
    for mark in INCOMPLETE_MARKERS:
        if mark in out:
            return "unknown", mark, dt
    if m:
        return "holds", "", dt
    if r.returncode != 0:
        return "unknown", f"exit {r.returncode} (killed or out of memory?)", dt
    return "unknown", "no result line", dt


def run_task(workdir, body, formula):
    try:
        return check_claim(workdir, body, formula)
    finally:
        if not ARGS.keep:
            shutil.rmtree(workdir, ignore_errors=True)


def main():
    tmp = Path(tempfile.mkdtemp(prefix="rers-par-", dir=ARGS.tmp))
    years = ARGS.years
    tasks = []  # (year, problem, claim, workdir, body, formula)
    formulas = {}
    for year in years:
        sub, problems = MODELS[year]
        for prob in problems:
            if ARGS.problems and prob not in ARGS.problems:
                continue
            pml = DATA / sub / f"problem{prob}" / f"problem{prob}.pml"
            body, props = parse_model(pml)
            assert len(props) == 20, (pml, len(props))
            for n, f in props:
                formulas[(year, prob, n)] = f
                for claim, g in ((f"p{n}", f), (f"n{n}", f"!({f})")):
                    tasks.append((year, prob, claim, tmp / f"{year}-{prob}-{claim}", body, g))
    print(f"{len(tasks)} claims, workdir {tmp}", file=sys.stderr, flush=True)

    res = {}
    if ARGS.reuse:
        # Keep decided verdicts and plain timeouts; redo build failures and
        # depth-limited searches.
        for r in csv.DictReader(open(ARGS.reuse)):
            for kind, col in (("p", "spin_phi"), ("n", "spin_negphi")):
                v = r[col]
                f = formulas.get((int(r["year"]), int(r["problem"]), int(r["prop"])))
                if f is None or (ARGS.redo_next and uses_next(f)):
                    continue
                if ARGS.only_killed:
                    # Keep every verdict except claims that an outside signal killed.
                    keep = "exit -15" not in v
                elif ARGS.only_unknown:
                    # Rerun only the !phi claims of rows the filter left
                    # unknown; their phi is already decided by the solution.
                    keep = kind == "p" or r["survives"] != "unknown"
                else:
                    keep = v in ("holds", "violated") or v == "unknown (timeout)"
                if keep:
                    m = re.fullmatch(r"(\w+)(?: \((.*)\))?", v)
                    res[(int(r["year"]), int(r["problem"]), f"{kind}{r['prop']}")] = (
                        m.group(1), m.group(2) or "", float(r["seconds"]) / 2)
        tasks = [t for t in tasks if (t[0], t[1], t[2]) not in res]
        print(f"reusing {len(res)}, rerunning {len(tasks)}", file=sys.stderr, flush=True)
    with cf.ThreadPoolExecutor(ARGS.jobs) as ex:
        futs = {ex.submit(run_task, *t[3:]): t for t in tasks}
        for i, fut in enumerate(cf.as_completed(futs), 1):
            year, prob, claim = futs[fut][:3]
            res[(year, prob, claim)] = fut.result()
            if ARGS.only_unknown:
                v, why, dt = res[(year, prob, claim)]
                print(f"  {year} {prob} {claim}: {v} {why} {dt:.1f}s",
                      file=sys.stderr, flush=True)
            if i % 50 == 0:
                print(f"  {i}/{len(tasks)}", file=sys.stderr, flush=True)

    rows = []
    for year in years:
        sol = load_solutions(year)
        for (y, prob, n) in sorted(k for k in formulas if k[0] == year):
            vp, why_p, tp = res[(y, prob, f"p{n}")]
            vn, why_n, tn = res[(y, prob, f"n{n}")]
            truth = sol.get((prob, n))
            # The official answer decides phi when SPIN cannot; on the claims
            # SPIN decided, it never contradicted one.
            phi_fails = vp == "violated" or truth is False
            phi_holds = vp == "holds" or truth is True
            if phi_fails and vn == "violated":
                survives = "yes"
            elif phi_holds or vn == "holds":
                survives = "no"
            else:
                survives = "unknown"
            rows.append({
                "year": y, "problem": prob, "prop": n,
                "solution_verdict": {True: "true", False: "false", None: ""}[truth],
                "spin_phi": vp + (f" ({why_p})" if why_p else ""),
                "spin_negphi": vn + (f" ({why_n})" if why_n else ""),
                "survives": survives,
                "seconds": f"{tp + tn:.1f}",
            })
    OUT.parent.mkdir(parents=True, exist_ok=True)
    with open(ARGS.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    if not ARGS.keep:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--years", type=int, nargs="*", default=sorted(MODELS))
    ap.add_argument("--problems", type=int, nargs="*")
    ap.add_argument("--jobs", type=int, default=6,
                    help="parallel pan runs (6 locally; up to ~28 on av2/av3)")
    ap.add_argument("--timeout", type=float, default=120)
    ap.add_argument("--build-timeout", type=float, default=900,
                    help="seconds each for spin -a and gcc")
    ap.add_argument("--depth", type=int, default=10_000_000)
    ap.add_argument("--memmax", type=int, default=4096,
                    help="MB cap per launch (systemd scope); pan MEMLIM is 7/8 of it")
    ap.add_argument("--tmp", default=os.environ.get("TMPDIR"))
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--reuse", help="previous results CSV; rerun only undecided claims")
    ap.add_argument("--redo-next", action="store_true",
                    help="with --reuse, also rerun every formula that uses X")
    ap.add_argument("--only-killed", action="store_true",
                    help="with --reuse, rerun only claims that ended with exit -15 "
                         "(killed by a signal); keep every other verdict")
    ap.add_argument("--only-unknown", action="store_true",
                    help="with --reuse, rerun only the !phi claims of rows whose "
                         "survives is unknown; keep every other verdict")
    ap.add_argument("--keep", action="store_true", help="keep the work directory")
    ARGS = ap.parse_args()
    main()
