#!/usr/bin/env python3
"""Model-check the RERS 2016/2017 sequential LTL problems by explicit search.

The sequential problems are C programs whose state is a fixed set of global
variables. `explore` compiles one problem with bench/rers_seq_explore.c,
which enumerates every reachable valuation of those globals breadth first,
and turns the result into a HOA automaton over the program's events: one
letter per input or output, read in the order the program performs them.
`check` then decides each formula with Spot: M |= phi exactly when the
model automaton and ltl2tgba's automaton for !phi have an empty product.

The search is sound and complete for these programs: they read only their
inputs and their own globals, have no heap and no I/O beyond printf, and
are deterministic, so a valuation of the globals fixes all future
behaviour. Two valuations are merged only when every byte is equal.

Events: input value v is atom i<chr(64+v)>, output value v is
o<chr(64+v)>, as in the RERS property files. An input that the program
rejects ("Invalid input", no output) ends the run, so it adds no edge;
only infinite runs count. `--invalid continue` keeps such inputs as an
input event with no output instead, to test that reading against the
official solutions.

Data (gitignored, under bench/data/rers/<year>/seq/), fetched by `fetch`:
  http://rers-challenge.org/2016/problems/LTL/LTLRERS2016.zip
  http://rers-challenge.org/2016/problems/LTL/LTLSolutions.zip
  http://rers-challenge.org/2017/problems/LTL/LTLRERS2017.zip
  http://rers-challenge.org/2017/problems/LTL/solutions_2017_sequential_ltl.zip

Stdlib only; needs gcc, objcopy, ltl2tgba and autfilt on PATH.
"""

import argparse
import concurrent.futures as cf
import os
import re
import subprocess
import sys
import tempfile
import time
import urllib.request
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
DATA = HERE / "data" / "rers"

DOWNLOADS = {
    2016: ["http://rers-challenge.org/2016/problems/LTL/LTLRERS2016.zip",
           "http://rers-challenge.org/2016/problems/LTL/LTLSolutions.zip"],
    2017: ["http://rers-challenge.org/2017/problems/LTL/LTLRERS2017.zip",
           "http://rers-challenge.org/2017/problems/LTL/solutions_2017_sequential_ltl.zip"],
}

WRAPPER = """\
#include <stdio.h>
#include <assert.h>
#include <math.h>
#include <stdlib.h>
void rers_out(int);
void rers_error(int);
#define printf(fmt, v) rers_out(v)
#define fprintf(...) ((void)0)
#define fflush(x) ((void)0)
#define scanf(...) 0
#define main rers_program_main
#define __VERIFIER_error rers_error
#include "{source}"
int rers_input_count(void) {{ return sizeof inputs / sizeof inputs[0]; }}
int rers_input(int k) {{ return inputs[k]; }}
"""


def seq_dir(year):
    return DATA / str(year) / "seq"


def problem_source(year, prob):
    return seq_dir(year) / f"LTLRERS{year}" / f"Problem{prob}" / f"Problem{prob}.c"


def fetch(years):
    for year in years:
        d = seq_dir(year)
        d.mkdir(parents=True, exist_ok=True)
        for url in DOWNLOADS[year]:
            z = d / url.rsplit("/", 1)[1]
            if not z.exists():
                print(f"fetching {url}", file=sys.stderr)
                urllib.request.urlretrieve(url, z)
            out = d / z.stem
            if not out.exists():
                with zipfile.ZipFile(z) as zf:
                    zf.extractall(out)


# --- Property and solution files --------------------------------------------

def load_properties(year, prob):
    """Return (atoms, [(index, gloss, formula)]) from the constraints file."""
    (path,) = (seq_dir(year) / f"LTLRERS{year}" / f"Problem{prob}").glob("constraints-*.txt")
    lines = path.read_text().splitlines()
    atoms = []
    for line in lines:
        m = re.match(r"#(inputs|outputs) \[+([^]]*)\]+", line)
        if m:
            atoms += [("i" if m.group(1) == "inputs" else "o") + a.strip()
                      for a in m.group(2).split(",")]
    props = []
    for k, line in enumerate(lines):
        m = re.match(r"#(\d+): (.*)", line)
        if m:
            props.append((int(m.group(1)), m.group(2).strip(), lines[k + 1].strip()))
    assert [p[0] for p in props] == list(range(len(props))), path
    return atoms, props


def solution_file(year, prob):
    if year == 2016:
        return seq_dir(year) / "LTLSolutions" / f"Problem{prob}-solutions.txt"
    return (seq_dir(year) / "solutions_2017_sequential_ltl" / "solutions_2017_sequential_ltl"
            / f"Problem{prob}-solutions.txt")


def load_solutions(year, prob):
    """Return [(formula, holds, lasso)] in file order.

    2016 writes `Formula: F`; 2017 writes `#n: gloss` and then F alone. A
    false formula is followed by a lasso `[u1, u2] ([v1, v2])*`, returned
    as (u, v); a true one has lasso None.
    """
    out = []
    formula = None
    lines = solution_file(year, prob).read_text().splitlines()
    for k, line in enumerate(lines):
        if line.startswith("Formula: "):
            formula = line[len("Formula: "):].strip()
        elif line.startswith("("):
            formula = line.strip()
        elif line.startswith("Formula is satisfied"):
            out.append((formula, True, None))
        elif line.startswith("Formula is not satisfied"):
            m = re.match(r"^\[(.*)\] \(\[(.*)\]\)\*$", lines[k + 1].strip())
            lasso = tuple([e.strip() for e in g.split(",") if e.strip()] for g in m.groups())
            out.append((formula, False, lasso))
    return out


def to_spot(f):
    """RERS LTL to Spot syntax: only the weak until is spelled differently."""
    return re.sub(r"\bWU\b", "W", f)




AFFECTED = re.compile(r"^\(false R \(([io][A-Z]) & (.*)\)\)$")


def corrected(formula):
    """The intended formula for a G(Q & ...) mistranslation, else None."""
    m = AFFECTED.match(formula)
    return f"(false R (! {m.group(1)} | {m.group(2)}))" if m else None


def one_event(atoms):
    """G(exactly one atom holds): true of every run of a RERS program."""
    some = " | ".join(atoms)
    pairs = " & ".join(f"!({a} & {b})" for k, a in enumerate(atoms) for b in atoms[k + 1:])
    return f"G(({some}) & {pairs})"


def unsatisfiable(formulas):
    """Which of the formulas are unsatisfiable, one ltlfilt call each."""
    out = []
    for f in formulas:
        r = subprocess.run(["ltlfilt", "--equivalent-to=false", "-f", f], capture_output=True,
                           text=True, timeout=3600, check=False)
        if r.returncode not in (0, 1):
            raise RuntimeError(f"ltlfilt: {r.stderr[-500:]}")
        out.append(bool(r.stdout.strip()))
    return out


# --- Exploration ------------------------------------------------------------

def run(cmd, timeout, **kw):
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, **kw)
    if r.returncode != 0:
        raise RuntimeError(f"{cmd[0]} exit {r.returncode}: {r.stderr[-500:]}")
    return r


def compile_problem(year, prob, workdir, opt, harness="rers_seq_explore.c"):
    """Build the explorer (or another harness) for one problem. Return the
    executable's path."""
    wrap = workdir / "wrap.c"
    wrap.write_text(WRAPPER.format(source=problem_source(year, prob).resolve()))
    obj = workdir / "problem.o"
    # Position-dependent code keeps pointer initialisers in .data, and
    # -fno-zero-initialized-in-bss keeps zeroed globals there too, so
    # .data is the whole program state.
    run(["gcc", opt, "-fwrapv", "-fno-pie", "-fno-common",
         "-fno-zero-initialized-in-bss", "-w", "-c", str(wrap), "-o", str(obj)], 7200)
    sections = run(["objdump", "-h", str(obj)], 60).stdout
    writable = [m.group(1) for m in re.finditer(r"^\s*\d+\s+(\S+)\s+([0-9a-f]+)", sections, re.M)
                if int(m.group(2), 16) > 0 and re.match(r"\.(data|bss)", m.group(1))]
    if writable != [".data"]:
        raise RuntimeError(f"unexpected writable sections {writable}")
    run(["objcopy", "--rename-section", ".data=rers_state", str(obj)], 600)
    exe = workdir / "explore"
    run(["gcc", "-O2", "-no-pie", str(HERE / harness), str(obj), "-o", str(exe)],
        600)
    obj.unlink()
    return exe


def explore(exe, workdir, max_states, count=False):
    """Run the explorer. Return (edge list path, complete?, program states)."""
    edges = workdir / "edges.txt"
    with open(edges, "w") as fh:
        r = subprocess.run([str(exe), str(max_states)] + (["count"] if count else []),
                           stdout=fh, stderr=subprocess.PIPE, text=True)
    if r.returncode not in (0, 3):
        raise RuntimeError(f"explore exit {r.returncode}: {r.stderr[-500:]}")
    with open(edges, "rb") as fh:
        fh.seek(max(0, edges.stat().st_size - 100))
        last = fh.read().decode().split("\n")[-2]
    return edges, last.startswith("END"), int(last.split("=")[1])


def event(kind, value):
    return kind + chr(64 + value)


class Hoa:
    """Writes an automaton over one-event letters, every state accepting.

    The body goes to a temporary file, since the state count is known only
    at the end.
    """

    def __init__(self, path, atoms, nstates):
        self.path, self.atoms, self.n = path, atoms, nstates
        self.index = {a: k for k, a in enumerate(atoms)}
        n = len(atoms)
        self.labels = ["&".join(str(j) if j == k else f"!{j}" for j in range(n))
                       for k in range(n)]
        self.body = open(f"{path}.body", "w")
        self.starts = []
        self.defined = set()

    def fresh(self):
        self.n += 1
        return self.n - 1

    def state(self, s, edges):
        """All edges of state s, as [(event, target)]."""
        self.body.write(f"State: {s}\n")
        self.defined.add(s)
        for e, t in edges:
            self.body.write(f"[{self.labels[self.index[e]]}] {t}\n")

    def close(self):
        self.body.close()
        with open(self.path, "w") as fh:
            fh.write(f"HOA: v1\nStates: {self.n}\n")
            for s in self.starts:
                fh.write(f"Start: {s}\n")
            fh.write(f"AP: {len(self.atoms)} " + " ".join(f'"{a}"' for a in self.atoms) + "\n")
            fh.write("Acceptance: 0 t\n--BODY--\n")
            with open(f"{self.path}.body") as body:
                for line in body:
                    fh.write(line)
            # Spot wants every state defined; found-but-unexpanded ones
            # have no edges.
            for s in range(self.n):
                if s not in self.defined:
                    fh.write(f"State: {s}\n")
            fh.write("--END--\n")
        os.unlink(f"{self.path}.body")


def add_program(hoa, edges, invalid):
    """Add the explored program graph; its state k is automaton state k.

    Program states that were found but not expanded (when the search hit
    its limit) get no edges, so every infinite run of the automaton is a
    run of the program.
    """
    hoa.starts.append(0)
    with open(edges) as fh:
        cur, out = None, []
        for line in fh:
            if line.startswith(("END", "LIMIT")):
                break
            nums = [int(x) for x in line.split()]
            src, inp, dst, outs = nums[0], nums[1], nums[2], nums[3:]
            if src != cur:
                if cur is not None:
                    hoa.state(cur, out)
                cur, out = src, []
            if not outs and invalid == "end":
                continue
            evs = [event("i", inp)] + [event("o", o) for o in outs]
            at = None
            for k, e in enumerate(evs):
                target = dst if k == len(evs) - 1 else hoa.fresh()
                if at is None:
                    out.append((e, target))
                else:
                    hoa.state(at, [(e, target)])
                at = target
        if cur is not None:
            hoa.state(cur, out)


def add_lasso(hoa, u, v):
    """Add the single word u v^omega as a separate component."""
    word = u + v
    first = hoa.n
    for _ in word:
        hoa.fresh()
    hoa.starts.append(first)
    for k, e in enumerate(word):
        t = first + k + 1 if k + 1 < len(word) else first + len(u)
        hoa.state(first + k, [(e, t)])


def replay_lasso(exe, u, v, rounds=60):
    """Is u v^omega a run of the program?

    Runs u v^rounds and checks every output. The word is a run when the
    program state repeats at two step boundaries that lie in the loop and
    in the same phase of it: from there on the program sees the same state
    and the same inputs, so it repeats forever.
    """
    word = u + v * rounds
    steps = []  # [input value, [output values], position after the step]
    for k, e in enumerate(word):
        if e[0] == "i":
            steps.append([ord(e[1]) - 64, [], k + 1])
        elif steps:
            steps[-1][1].append(ord(e[1]) - 64)
            steps[-1][2] = k + 1
        else:
            return False
    steps.pop()  # its outputs may run into the next round
    r = run([str(exe), "replay"], 600, input="".join(f"{s[0]}\n" for s in steps))
    seen = set()
    for (_, outs, pos), line in zip(steps, r.stdout.splitlines()):
        got, state = line.split("|")
        if [int(x) for x in got.split()] != outs:
            return False
        if pos >= len(u):
            key = ((pos - len(u)) % len(v), state)
            if key in seen:
                return True
            seen.add(key)
    return False


def violated(model, negation_hoa):
    """Does some infinite run of the model satisfy the negated formula?"""
    r = subprocess.run(["autfilt", "-q", f"--intersect={negation_hoa}", str(model)],
                       capture_output=True, text=True, timeout=7200)
    if r.returncode not in (0, 1) or r.stderr:
        raise RuntimeError(f"autfilt exit {r.returncode}: {r.stderr[:500]}")
    return r.returncode == 0


def translate(formula, path):
    """ltl2tgba's automaton for !formula, written to path."""
    with open(path, "w") as fh:
        subprocess.run(["ltl2tgba", "-H", f"!({formula})"], stdout=fh, check=True,
                       timeout=3600)
    return path


def check_problem(year, prob):
    """Decide the properties of one problem; write a TSV of verdicts.

    The known-runs automaton K holds the explored program graph and every
    official lasso that replays as a real run, so each infinite run of K is
    a run of M. A formula fails on M when K has a run violating it, and
    holds when it does not and the exploration was complete. For each
    mistranslated property, phi and the fix are also asked whether some run
    satisfies them (the degeneracy check), and the fix is compared with the
    official verdicts: a true property psi with psi & one_event => fix
    shows the fix holds, and psi & one_event => !fix shows no run
    satisfies it.
    """
    t0 = time.monotonic()
    wd = Path(tempfile.mkdtemp(prefix=f"rers-seq-{year}-{prob}-", dir=ARGS.tmp))
    atoms, props = load_properties(year, prob)
    sols = load_solutions(year, prob)
    assert len(sols) == len(props) and all(s[0] == p[2] for s, p in zip(sols, props)), prob
    exe = compile_problem(year, prob, wd, ARGS.opt)
    edges, complete, nprog = explore(exe, wd, ARGS.max_states)
    t_explore = time.monotonic() - t0
    lassos = [s[2] for s in sols if s[2]]
    # Arithmetic problems can take hundreds of loop rounds to repeat a state.
    real = [lz for lz in lassos if any(replay_lasso(exe, *lz, rounds=n) for n in (60, 600, 3000))]
    model = wd / "known.hoa"
    hoa = Hoa(model, atoms, nprog)
    add_program(hoa, edges, ARGS.invalid)
    for u, v in real:
        add_lasso(hoa, u, v)
    hoa.close()
    edges.unlink()

    def verdict(formula, tag):
        if violated(model, translate(formula, wd / f"{tag}.hoa")):
            return "fails"
        return "holds" if complete else "unknown"

    def satisfiable_on_m(formula, tag):
        return {"fails": "yes", "holds": "no"}.get(verdict(f"!({formula})", tag), "unknown")

    mutex = one_event(atoms)
    ins = " | ".join(a for a in atoms if a[0] == "i")
    outs = " | ".join(a for a in atoms if a[0] == "o")
    truths = [to_spot(f) for (_, _, f), s in zip(props, sols) if s[1]]
    rows = []
    for (n, _, f), (_, official, _) in zip(props, sols):
        fixed = corrected(f)
        if ARGS.only_affected and fixed is None:
            continue
        phi = to_spot(f)
        row = {"prop": n, "official": "holds" if official else "fails",
               "phi": verdict(phi, f"p{n}")}
        if fixed:
            fix = to_spot(fixed)
            row["phi_sat"] = satisfiable_on_m(phi, f"np{n}")
            # Every run of M has infinitely many inputs and outputs, one
            # event at a time, and phi needs one atom at every position.
            (no_run,) = unsatisfiable([f"({mutex}) & GF({ins}) & GF({outs}) & ({phi})"])
            row["phi_sat_spot"] = "no" if no_run else "unknown"
            row["fix"] = verdict(fix, f"f{n}")
            row["fix_sat"] = satisfiable_on_m(fix, f"nf{n}")
            imp = unsatisfiable([f"({mutex}) & ({t}) & !({fix})" for t in truths])
            neg = unsatisfiable([f"({mutex}) & ({t}) & ({fix})" for t in truths])
            row["fix_official"] = "holds" if any(imp) else "fails" if any(neg) else "unknown"
            row["fix_sat_official"] = "no" if any(neg) else "unknown"
        rows.append(row)
    keys = ["prop", "official", "phi", "phi_sat", "phi_sat_spot", "fix", "fix_sat",
            "fix_official", "fix_sat_official"]
    out = Path(ARGS.out_dir) / f"{year}-{prob}.tsv"
    with open(out, "w") as fh:
        fh.write(f"# year={year} problem={prob} complete={complete} program_states={nprog} "
                 f"automaton_states={hoa.n} lassos={len(lassos)} real_lassos={len(real)} "
                 f"explore_s={t_explore:.1f} total_s={time.monotonic() - t0:.1f} "
                 f"invalid={ARGS.invalid}\n")
        fh.write("\t".join(keys) + "\n")
        for r in rows:
            fh.write("\t".join(str(r.get(k, "")) for k in keys) + "\n")
    if not ARGS.keep:
        for p in wd.iterdir():
            p.unlink()
        wd.rmdir()
    return year, prob, f"{nprog} states, complete={complete}", time.monotonic() - t0


def count_problem(year, prob):
    """Explore without recording edges; report the state count."""
    t0 = time.monotonic()
    wd = Path(tempfile.mkdtemp(prefix=f"rers-seq-{year}-{prob}-", dir=ARGS.tmp))
    exe = compile_problem(year, prob, wd, ARGS.opt)
    _, complete, n = explore(exe, wd, ARGS.max_states, count=True)
    for p in wd.iterdir():
        p.unlink()
    wd.rmdir()
    return year, prob, f"{n} states, complete={complete}", time.monotonic() - t0


def main():
    if ARGS.command == "fetch":
        fetch(ARGS.years)
        return
    Path(ARGS.out_dir).mkdir(parents=True, exist_ok=True)
    task = count_problem if ARGS.command == "count" else check_problem
    jobs = [(y, p) for y in ARGS.years for p in ARGS.problems]
    with cf.ThreadPoolExecutor(ARGS.jobs) as ex:
        futs = {ex.submit(task, y, p): (y, p) for y, p in jobs}
        for fut in cf.as_completed(futs):
            try:
                print("%s/%s: %s, %.1f s" % fut.result(), flush=True)
            except Exception as e:  # report and carry on with the other problems
                print(f"{futs[fut]}: FAILED {e!r}", flush=True)


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("command", choices=["fetch", "count", "check"])
    ap.add_argument("--years", type=int, nargs="*", default=[2016, 2017])
    ap.add_argument("--problems", type=int, nargs="*", default=list(range(1, 10)))
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--opt", default="-O1", help="gcc optimisation for the problem")
    ap.add_argument("--max-states", type=int, default=300_000,
                    help="stop the search after this many program states")
    ap.add_argument("--invalid", choices=["end", "continue"], default="end")
    ap.add_argument("--only-affected", action="store_true")
    ap.add_argument("--out-dir", default=os.environ.get("TMPDIR", "/tmp"))
    ap.add_argument("--tmp", default=os.environ.get("TMPDIR"))
    ap.add_argument("--keep", action="store_true")
    ARGS = ap.parse_args()
    main()
