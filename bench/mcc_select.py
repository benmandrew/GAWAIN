"""Lists the MCC 2026 P/T instances with at most 1e9 reachable states and
their FALSE LTLCardinality and LTLFireability formulas.

State counts are Tedd's StateSpace results (`<instance>-SS.out` in the
oracle archive); verdicts are the contest consensus (`-LTLC.out`,
`-LTLF.out`). Coloured (COL) instances are left out, since the back end
reads P/T nets only; most coloured models also have a P/T unfolding.

usage: python3 -I bench/mcc_select.py [ORACLE_DIR] > candidates.csv
Writes instance,states,examination,formula,verdict,tools to stdout.
"""

import os
import sys

LIMIT = 10**9
EXAMS = {"LTLC": "LTLCardinality", "LTLF": "LTLFireability"}


def states(path):
    with open(path) as f:
        for line in f:
            words = line.split()
            if words[:2] == ["STATE_SPACE", "STATES"]:
                return words[2]
    return None


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    oracle = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data/mcc/oracle")
    print("instance,states,examination,formula,verdict,tools")
    for name in sorted(os.listdir(oracle)):
        if not name.endswith("-SS.out"):
            continue
        instance = name[: -len("-SS.out")]
        if "-PT-" not in instance:
            continue
        count = states(os.path.join(oracle, name))
        # Some counts have thousands of digits; compare the length first.
        if count is None or not count.isdecimal() or len(count) > 10 or int(count) > LIMIT:
            continue
        for short, exam in EXAMS.items():
            path = os.path.join(oracle, f"{instance}-{short}.out")
            if not os.path.exists(path):
                continue
            with open(path) as f:
                for line in f:
                    words = line.split()
                    if words[:1] != ["FORMULA"] or words[2] != "FALSE":
                        continue
                    formula = words[1][len(instance) + 1 :]
                    tools = " ".join(words[5:])
                    print(f"{instance},{count},{exam},{formula},FALSE,{tools}")


main()
