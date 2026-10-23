# MCC: a counterexample back end and a subset

Checked on 2026-10-23 against the Model Checking Contest (MCC) 2026 mirror at [yanntm/pnmcc-models-2026](https://github.com/yanntm/pnmcc-models-2026). `bench/mcc_fetch.sh` re-fetches everything used here into `bench/data/mcc/` (gitignored): the oracle archive and one archive per instance.

## What the back end needs

The oracle's `Backend` must return a `Lasso` on every failing verdict, with the value of every interface atom at every position. LTLCardinality atoms compare token sums (`integer-le` over `tokens-count` and `integer-constant`), and LTLFireability atoms are `is-fireable` over a set of transitions. Each atom must therefore be evaluable on a marking. The contest semantics extend a run that reaches a dead marking by stuttering on it forever.

## Tools tried

Each tool was run on ResAllocation-PT-R003C002 (20 reachable states) and, for TAPAAL, on a 150-formula sample described below.

| Route | Installs here | Prints an LTL counterexample | Usable as a lasso | Licence |
|---|---|---|---|---|
| LTSmin 3.0.2 `pnml2lts-mc` | Yes, the 2018 Linux binary release runs | Yes, `--trace=f.gcf` after "Accepting cycle FOUND" | No loop marker; the cycle would have to be inferred | BSD-3-Clause |
| ITS-Tools (build 2026-09-11) | Yes, 76 MB Eclipse product, needs Java | No; MCC mode prints verdicts only | No | GPL |
| TAPAAL `verifypn` 4.4.2 | Yes, static binary in the TAPAAL 4.0.4 bundle | Yes, `-t` prints an XML trace with `<loop/>` | Yes, after replay on the net | GPL-3.0-or-later |
| Own PNML to SMV for nuXmv | Not built | — | — | — |
| Own explicit Kripke structure | Not built | — | — | — |

**LTSmin.** `pnml2lts-mc --threads=1 --strategy=ndfs --ltl='<> [] (p_1_1 < 1)' --trace=cex.gcf` found the accepting cycle and wrote a 6-state trace. `ltsmin-printtrace` shows no loop marker, and its last steps print transition `t_0_0` from a marking where it is not enabled. Atoms are comparisons over place variables, so fireability would need translating into conjunctions of the guards `guard_<place>_ge_<w>`. The source build is not in nixpkgs, and the binary dates from 2018.

**ITS-Tools.** `its-tools -pnfolder DIR -examination LTLCardinality` solved all 16 formulas in 4.9 s, by structural reductions, stutter-insensitivity tests and random walks, and printed only `FORMULA … FALSE TECHNIQUES STUTTER_TEST` lines. Its `its-ltl` engine prints Spot runs, but over the GAL model that the Java front end has already reduced, so the transition names are not the net's.

**TAPAAL.** `verifypn -ltl -t -x 1 model.pnml query.xml` reads the contest's XML formulas directly and prints `FORMULA <id> FALSE`, then a `<trace>` of `<transition id=…>` firings on standard error, with `<loop/>` before the cycle. It reads the query from `/dev/stdin`, so no temporary file is needed. The binary ships in `tapaal-4.0.4-linux64.zip` and is pinned in `flake.nix`.

**Own translations.** These were not built, because TAPAAL already gives lassos. A PNML to SMV translation needs a finite range per place, which the bounds oracle (`-UB.out`) gives only for some places, and nuXmv's BDD check on nets with hundreds of places is unmeasured. An explicit Kripke structure fails on size: `src/kripke.rs` allows one atom per state, and the median instance in the subset below has 1.4e6 states while the largest has 9.9e8.

## The TAPAAL back end

`src/oracle/tapaal.rs` writes each candidate as one MCC XML property (`src/mcc.rs`, which expands `W`, `R`, `->` and `<->` into the format's `U`, `!`, `&` and `|`), runs `verifypn`, and replays the trace on its own parse of the PNML net. The marking at each position gives the valuation of every atom of the property. A cycle must return to the marking where `<loop/>` stood. A finite trace is a bad prefix or ends in a dead marking, and the back end closes it by firing the first enabled transition until a marking repeats or a dead marking stutters. Every lasso is then evaluated with `Lasso::satisfies`, and a lasso that satisfies the candidate is rejected.

`verifypn`'s traces are unreliable in a few per cent of cases. On the sample of 150 FALSE formulas (seed 7, `bench/mcc_run.sh` on av3, 12 jobs, 60 s per call), five single configurations gave these counts of usable FALSE verdicts on φ. Each configuration was set through a temporary environment override of `verifypn`'s arguments, since removed; the rows are in `bench/results/mcc-configs.csv`.

| Configuration | Usable FALSE | No trace | Trace does not replay or loop | Lasso satisfies φ | Timeout | TRUE |
|---|---|---|---|---|---|---|
| `-ltl` (Tarjan, defaults) | 128 | 16 | 5 | 0 | 1 | 0 |
| `-ltl -q 0` (no query simplification) | 139 | 0 | 7 | 0 | 3 | 1 |
| `-ltl ndfs` | 123 | 16 | 8 | 2 | 1 | 0 |
| `-ltl -p` (no partial-order reduction) | 127 | 16 | 5 | 0 | 2 | 0 |
| `-ltl -r 0` (no structural reduction) | 128 | 16 | 5 | 0 | 1 | 0 |

A failure that query simplification alone proves has no trace. `-q 0` supplies one, but it answered TRUE for TokenRing-PT-005-LTLFireability-04, which the default configuration refutes with a replayed lasso, and TRUE for the negation of DatabaseWithMutex-PT-02-LTLFireability-09, which the other four refute. `-q 0` is therefore unsound for TRUE. A trace that does not replay is not caused by the reductions: JoinFreeModules-PT-0004-LTLCardinality-07 fires transition `t` in a marking with 2 tokens on `p13` where `t` needs 4, under the defaults, under `-r 0` and under `-p`. `ndfs` gives a usable trace for all 5 formulas whose default trace is broken, and fails on 10 that the default handles.

The back end therefore tries `-ltl`, then `-ltl ndfs`, `-ltl -q 0` and `-ltl ndfs -q 0`. A timeout on the first configuration stands. A later configuration counts only when it returns a lasso that replays and violates the candidate. In the full run below this gives FALSE for 147 of the 150 sample formulas; the other 3 time out. A TRUE verdict cannot be checked by replay, so a first-run TRUE stands only when `-ltl -p` repeats it (see "Confirming TRUE" below).

## Selecting the subset

`bench/mcc_select.py` lists the candidates from the oracle archive. Tedd's StateSpace counts (`<instance>-SS.out`) cover 1,391 of the 1,953 instances, 1,232 of them P/T. 423 P/T instances have at most 1e9 states, and they carry 9,942 FALSE formulas: 4,655 LTLCardinality and 5,287 LTLFireability, of which 47 rest on a single tool's answer. Coloured instances are left out, since the back end reads P/T nets only.

## Results

`bench/mcc_run.sh` ran `mcc_check` on all 9,942 candidates on av3 (32 cores, 125 GB), with 12 instances at a time, a 60 s limit per `verifypn` call, 12 GB per call and a 16 GB scope per instance. Load average was about 15 during the run, from these 12 jobs and other users. The run took 1 h 54 min. Each candidate φ and its negation ¬φ were checked. That run trusted the first configuration's TRUE, and it gave one TRUE that the oracle contradicts, which led to the confirmation step below. The 4,756 candidates with a TRUE answer on φ or ¬φ were then checked again with confirmation, and their TRUE answers were replaced by the new verdicts. FALSE answers were kept, since each carries a replayed lasso. The rows are in `bench/results/mcc-verdicts.csv`, and `bench/mcc_stats.py` gives the tables below.

| Candidates | FALSE (agrees with the oracle) | TRUE (disagrees) | UNKNOWN |
|---|---|---|---|
| 9,942 | 9,779 (98.4%) | 0 | 163 |

Of the 163 unknowns on φ, 152 are timeouts and 11 are formulas where every configuration gave a trace that does not replay or a lasso that satisfies φ.

### Confirming TRUE

Without confirmation, RobotManipulation-PT-00002-LTLFireability-02 came out TRUE, against FALSE from ITS-Tools, TAPAAL, TY and the 2025 gold. `verifypn -ltl` lists stubborn sets (`AUT_STUB`) among its techniques for that answer. `-ltl -p`, which turns partial-order reduction off, and `-ltl ndfs` both answer FALSE, and `-ltl -q 0` and `-ltl -r 0` answer TRUE. The default partial-order reduction is therefore unsound on this formula. A wrong TRUE would be memoised by the oracle and then used to prove other candidates, so the back end now reruns every first-run TRUE under `-ltl -p`. A second TRUE stands. A FALSE with a lasso that replays and violates the candidate overrides the first run. Anything else, including a timeout, is unknown.

Both candidate confirmations were measured on the 4,756 rechecked candidates, as two runs of `bench/mcc_run.sh` at 6 instances each, at the same time on av3 (load average 14 to 16), with the first-run TRUE timed separately from the confirmation:

| Confirmation | Runs | Median (s) | Mean (s) | p2.5–p97.5 (s) | Within 1 s | TRUE | FALSE | Timeout |
|---|---|---|---|---|---|---|---|---|
| `-ltl -p` | 4,735 | 0.033 | 1.68 | 0.008–24.5 | 89.3% | 4,699 | 1 | 35 |
| `-ltl ndfs` | 4,734 | 0.016 | 1.73 | 0.008–27.2 | 89.4% | 4,694 | 1 | 39 |

The two cost about the same. `-p` was chosen because it removes the reduction that gave the wrong answer, while `ndfs` keeps it, and it timed out less often. Both refute only RobotManipulation-PT-00002-LTLFireability-02, so no other TRUE in the run was found wrong. Confirmation roughly doubles the cost of a TRUE answer: the median goes from 0.032 s to 0.065 s and the mean from 1.31 s to 2.33 s. It changed 57 verdicts. The RobotManipulation formula went from TRUE to FALSE, and 56 negations went from TRUE to unknown: 35 because the `-p` run timed out, and 21 because the first run timed out this time, which is the run-to-run variance of a 60 s limit near the edge.

### The subset

If ¬φ is TRUE, no run of the model satisfies φ, and every run is a counterexample; such a candidate is degenerate and is dropped. Of the 9,779 agreeing candidates, ¬φ is TRUE for 4,683 (dropped), FALSE for 4,708 (kept) and unknown for 388 (dropped; 383 of them timeouts). The subset is the 4,708 kept candidates over 410 instances, in `bench/results/mcc-subset.csv` with each lasso's prefix and cycle length.

| Check | Verdict | n | Median (s) | Mean (s) | p2.5–p97.5 (s) | Mean 95% CI (s) |
|---|---|---|---|---|---|---|
| φ | FALSE | 9,779 | 0.093 | 1.13 | 0.008–12.4 | 1.02–1.25 |
| ¬φ | TRUE (confirmed) | 4,683 | 0.065 | 2.33 | 0.016–32.0 | 2.04–2.64 |
| ¬φ | FALSE | 4,708 | 0.093 | 1.60 | 0.017–18.5 | 1.39–1.82 |

The times are wall-clock times per `Backend::check` call, including every configuration the back end tries, the confirmation and the replay. The confidence interval is a bootstrap over candidates. 91.6% of φ checks finish within 1 s and 97.1% within 10 s. The median sits on one of the values where `process::run`'s polling makes times cluster (0.072 s and 0.092 s), so it is accurate only to about 20 ms.
