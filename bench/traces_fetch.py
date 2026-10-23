#!/usr/bin/env python3
"""Fetch the three trace-set sources and convert them to GAWAIN trace sets.

Each source is downloaded into bench/data/<name>/ (gitignored) and every
instance is written as one trace-set file (the format in
src/oracle/traces.rs) under bench/data/<name>/sets/. Run from the
repository root with `python3 -I bench/traces_fetch.py`. Prints, per
source, how many instances were converted and why the others were skipped.

- ltlf-learning: SynthesisLab LTLf Learning Benchmarks, finite traces,
  formulas in Spot's LTLf syntax (X weak, X[!] strong). OrderedSequence
  formulas carry one stray `)` at the end, which is dropped when the
  closing parentheses outnumber the opening ones by exactly one. Hamming
  instances store no formula and are skipped.
- flie: samples2LTL (Neider and Gavran), lasso traces `p;p;...::k` where k
  is the loop start (the whole word loops when `::k` is absent), formulas
  in prefix syntax. Only traces/generated/ is converted; the PCTCP use
  cases have no formula.
- scarlet: Scarlet (Raha et al., MIT licence), finite traces, prefix
  formulas looked up in the directory's formulas.txt by the file's
  `f-NN` index. X is strong next (Scarlet's evaluator is false past the
  end), so it is written X[!]. flie_benchmarks/ has no formula file and is
  skipped.
"""

import collections
import io
import json
import os
import sys
import tarfile
import urllib.request
import zipfile

LTLF_URL = ("https://github.com/SynthesisLab/LTLf_Learning_Benchmarks/"
            "releases/download/benchmark/benchmarks.zip")
LTLF_SHA256 = "af9c6e2e5ec24516ca1bcfb20402484297dad6fbc501bc81ed2c7cfbba9c571a"
FLIE_URL = ("https://github.com/gergia/samples2LTL/archive/"
            "49b72a8eb14371e4d21b3c70bd9aff860b49c251.tar.gz")
SCARLET_URL = ("https://github.com/rajarshi008/Scarlet/archive/"
               "902aaa6e0bde34a6a138e530ecb23a2bffd53ec1.tar.gz")

DATA = os.path.join("bench", "data")


def download(url, path):
    if os.path.exists(path):
        return
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with urllib.request.urlopen(url, timeout=600) as response:
        body = response.read()
    with open(path + ".part", "wb") as out:
        out.write(body)
    os.replace(path + ".part", path)


def untar(path, into):
    if os.path.isdir(into):
        return
    with tarfile.open(path) as archive:
        archive.extractall(into + ".part", filter="data")
    os.replace(into + ".part", into)


def write_set(path, semantics, atoms, formula, positives, negatives):
    """positives and negatives are lists of trace strings."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    lines = [f"semantics {semantics}", "atoms " + " ".join(atoms)]
    if formula is not None:
        lines.append(f"formula {formula}")
    lines += [f"+ {t}" for t in positives]
    lines += [f"- {t}" for t in negatives]
    with open(path, "w") as out:
        out.write("\n".join(lines) + "\n")


# --- prefix formulas (Flie and Scarlet) ---------------------------------

UNARY = {"G": "G", "F": "F", "!": "!", "X": "X"}
BINARY = {"&": "&", "|": "|", "->": "->", "U": "U"}


def prefix_to_spot(text, strong_next):
    """`|(G(!(p)),F(q))` to `(G (!p)) | (F q)`, fully parenthesised."""
    text = text.replace(" ", "")
    pos = 0

    def parse():
        nonlocal pos
        for op in ("->", "&", "|", "U"):
            if text.startswith(op + "(", pos):
                pos += len(op) + 1
                left = parse()
                expect(",")
                right = parse()
                expect(")")
                return f"({left}) {BINARY[op]} ({right})"
        for op in ("G", "F", "!", "X"):
            if text.startswith(op + "(", pos):
                pos += len(op) + 1
                inner = parse()
                expect(")")
                spelt = "X[!]" if op == "X" and strong_next else UNARY[op]
                return f"{spelt} ({inner})"
        start = pos
        while pos < len(text) and (text[pos].isalnum() or text[pos] == "_"):
            pos += 1
        if start == pos:
            raise ValueError(f"bad prefix formula {text!r} at {pos}")
        return text[start:pos]

    def expect(c):
        nonlocal pos
        if not text.startswith(c, pos):
            raise ValueError(f"expected {c!r} in {text!r} at {pos}")
        pos += 1

    out = parse()
    if pos != len(text):
        raise ValueError(f"trailing text in {text!r}")
    return out


def bits(position):
    return "".join(position.split(","))


# --- LTLf Learning Benchmarks ---------------------------------------------

def fix_parentheses(formula):
    """Drop one trailing `)` when it is the only excess one, else None."""
    excess = formula.count(")") - formula.count("(")
    if excess == 0:
        return formula
    if excess == 1 and formula.rstrip().endswith(")"):
        return formula.rstrip()[:-1]
    return None


def ltlf_learning(counts):
    root = os.path.join(DATA, "ltlf-learning")
    archive = os.path.join(root, "benchmarks.zip")
    download(LTLF_URL, archive)
    import hashlib
    with open(archive, "rb") as f:
        digest = hashlib.sha256(f.read()).hexdigest()
    if digest != LTLF_SHA256:
        sys.exit(f"{archive}: sha256 {digest}, expected {LTLF_SHA256}")
    with zipfile.ZipFile(archive) as z:
        for name in sorted(z.namelist()):
            if not name.endswith(".json"):
                continue
            family = name.split("/")[-2]
            data = json.load(io.TextIOWrapper(z.open(name)))
            if not isinstance(data, dict):
                print(f"ltlf-learning: {name} is not an instance")
                counts["ltlf-learning", "skipped: not an instance"] += 1
                continue
            formula = data.get("generating_formula") or None
            if formula is None:
                counts["ltlf-learning", "skipped: no formula"] += 1
                continue
            fixed = fix_parentheses(formula)
            if fixed is None:
                counts["ltlf-learning", "skipped: unbalanced"] += 1
                continue
            if fixed != formula:
                counts["ltlf-learning", "parenthesis fixed"] += 1
            if data.get("trace_type") != "finite":
                counts["ltlf-learning", "skipped: not finite"] += 1
                continue
            atoms = data["atomic_propositions"]

            def trace(t):
                length = len(t[atoms[0]])
                return " ".join("".join(str(t[a][i]) for a in atoms)
                                for i in range(length))

            stem = os.path.splitext(os.path.basename(name))[0]
            out = os.path.join(root, "sets", family, stem + ".trace")
            write_set(out, "finite", atoms, fixed,
                      [trace(t) for t in data["positive_traces"]],
                      [trace(t) for t in data["negative_traces"]])
            counts["ltlf-learning", "converted"] += 1


# --- Flie -----------------------------------------------------------------

def flie_trace(line):
    if "::" in line:
        word, start = line.split("::")
        start = int(start)
    else:
        word, start = line, 0
    positions = [bits(p) for p in word.split(";")]
    return " ".join(positions[:start]) + " | " + " ".join(positions[start:])


def flie(counts):
    root = os.path.join(DATA, "flie")
    archive = os.path.join(root, "samples2LTL.tar.gz")
    download(FLIE_URL, archive)
    untar(archive, os.path.join(root, "src"))
    (top,) = os.listdir(os.path.join(root, "src"))
    generated = os.path.join(root, "src", top, "traces", "generated")
    for directory, _, files in sorted(os.walk(generated)):
        for name in sorted(files):
            if not name.endswith(".trace"):
                continue
            path = os.path.join(directory, name)
            parts = open(path).read().split("---")
            positives = [l.strip() for l in parts[0].split("\n") if l.strip()]
            negatives = [l.strip() for l in parts[1].split("\n") if l.strip()]
            formula = parts[4].strip() if len(parts) > 4 else ""
            if not formula or formula == "none":
                counts["flie", "skipped: no formula"] += 1
                continue
            width = len(positives[0].split("::")[0].split(";")[0].split(","))
            atoms = [f"x{i}" for i in range(width)]
            family = os.path.relpath(directory, generated).replace(os.sep, "_")
            out = os.path.join(root, "sets", family,
                               os.path.splitext(name)[0] + ".trace")
            write_set(out, "infinite", atoms,
                      prefix_to_spot(formula, strong_next=False),
                      [flie_trace(t) for t in positives],
                      [flie_trace(t) for t in negatives])
            counts["flie", "converted"] += 1


# --- Scarlet --------------------------------------------------------------

def scarlet(counts):
    root = os.path.join(DATA, "scarlet")
    archive = os.path.join(root, "Scarlet.tar.gz")
    download(SCARLET_URL, archive)
    untar(archive, os.path.join(root, "src"))
    (top,) = os.listdir(os.path.join(root, "src"))
    benchmarks = os.path.join(root, "src", top, "All_benchmarks")
    for directory, _, files in sorted(os.walk(benchmarks)):
        traces = sorted(f for f in files if f.endswith(".trace"))
        if not traces:
            continue
        table = os.path.join(os.path.dirname(directory), "formulas.txt")
        if not os.path.exists(table):
            counts["scarlet", "skipped: no formulas.txt"] += len(traces)
            continue
        formulas = [l.strip().split(";")[0]
                    for l in open(table).read().split("\n") if l.strip()]
        family = os.path.relpath(os.path.dirname(directory), benchmarks)
        family = family.replace(os.sep, "_")
        for name in traces:
            index = int(name.split("-")[1]) - 1
            parts = open(os.path.join(directory, name)).read().split("---")
            positives = [l.strip() for l in parts[0].split("\n") if l.strip()]
            negatives = [l.strip() for l in parts[1].split("\n") if l.strip()]
            atoms = parts[3].strip().split(",")
            out = os.path.join(root, "sets", family,
                               os.path.splitext(name)[0] + ".trace")
            write_set(out, "finite", atoms,
                      prefix_to_spot(formulas[index], strong_next=True),
                      [" ".join(bits(p) for p in t.split(";")) for t in positives],
                      [" ".join(bits(p) for p in t.split(";")) for t in negatives])
            counts["scarlet", "converted"] += 1


def main():
    counts = collections.Counter()
    ltlf_learning(counts)
    flie(counts)
    scarlet(counts)
    for (source, what), n in sorted(counts.items()):
        print(f"{source}\t{what}\t{n}")


if __name__ == "__main__":
    main()
