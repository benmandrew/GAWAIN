# Trace-set measurements

This note measures the trace-set back end (`docs/dev/oracle.md`, "Trace sets"): the cost of one check, how weakening instances are built from the three sources, and what the search finds on a sample of them. The raw data is in `bench/results/trace-sets-*.csv`.

## Method

`bench/traces_fetch.py` downloads the three sources at pinned versions and writes one set file per instance under `bench/data/<source>/sets/`. Every number below was taken on one laptop (hostname `ben`, a 13th Gen Intel Core i7-13700H with 20 threads and 30 GB), with release builds and Spot 2.15.1. The load average from other work was 1.7–2.1 before the `check` runs and 0.3–6.0 before the searches. Each command ran in its own `systemd-run` scope with `MemoryMax` of 8 or 12 GB.

## Cost of one check

`trace_bench check DIR...` checks each source formula ψ against its own labels and times one back-end check of ψ, which evaluates every positive trace since ψ holds, as the median of 21 runs. The commands were `target/release/trace_bench check bench/data/ltlf-learning/sets bench/data/flie/sets bench/data/scarlet/sets` before and after the packed evaluator, and `python3 -I bench/traces_summary.py check FILE` gives the tables by family.

| Evaluator | Sets | Median | p90 | p99 | Max | Sum |
|---|---|---|---|---|---|---|
| `Lasso::satisfies` (`trace-sets-check-lasso.csv`) | 16,822 | 75.7 µs | 874 µs | 3,433 µs | 35.7 ms | 5.98 s |
| packed (`trace-sets-check.csv`) | 16,822 | 6.3 µs | 45.1 µs | 139 µs | 2.19 ms | 0.31 s |

The median fell 12×, and the sum 19×. The gain is largest on long words with many atoms: Nim (16 to 64 positions) fell from 1,977 µs to 24.6 µs, and DoubleCounter from 578 µs to 10.0 µs. Flie's words have at most 10 positions, and its family medians fell from 1.7–114 µs to 0.4–12.2 µs. The slowest family after the change is Scarlet's `RQ2-benchmarks_Varying_size`, with a median of 86.7 µs over sets of up to 50,000 positive traces.

One `ltlfilt` exec for a single pair costs 8–16 ms (`oracle.md`, "Batched implication"). That is 1,300 to 2,500 median trace-set checks.

## Weakening instances

A set and its source formula ψ give no weakening problem by themselves, since ψ already holds on every positive. A tempting construction takes φ = ¬ψ or some other formula that every positive violates, but then M ⊨ ¬φ, and the search's best answer is degenerate. `trace_bench instances` builds φ the other way round. It draws up to 20 strengthenings of ψ, each one or two `monotone_rewrite(Strengthen)` steps followed by `simplify`. It keeps a draft only when it fails on some positive (M ⊭ φ) and holds on another (M ⊭ ¬φ), and differs from ψ after `canonical`. One `ltlfilt` exec then decides φ ⇒ ψ for every kept draft under the set's semantics, and the first proved draft becomes the instance. ψ is then a known weakening of φ that holds on M, and the degeneracy check is the "holds on another" condition, applied to every instance.

Under `--from-ltlf` that exec takes up to 30 s on the larger LTLf formulas, so building instances from all 16,822 sets was too slow. A first run over every set reached 4,083 sets in 6 minutes, then slowed to about 11 sets a minute on RandomBooleanCombinationsofFactors, and was stopped. `bench/traces_sample.py` instead draws 20 sets per family with seed 1 (fewer where a family is smaller), which is 500 sets:

```
python3 -I bench/traces_sample.py target/release/trace_bench 20 1 bench/data/ltlf-learning/sets bench/data/flie/sets bench/data/scarlet/sets > bench/results/trace-sets-instances.csv
```

It gave 388 instances (`python3 -I bench/traces_summary.py instances bench/results/trace-sets-instances.csv`). Of the other 112 sets, 105 had no draft that failed on some but not all positives, 5 hit the 30 s Spot limit, and in 2 Spot proved no draft implies ψ. The median φ accepts 25–80% of a family's positives, depending on the family.

## Search on a sample

`bench/traces_run.py` draws 5 instances per family with seed 1, 126 in all, and runs `weaken --traces FILE --ltl φ --seed 1 --generations 30 --population 30` on each, two at a time, with each run's process group killed after 300 s. It then compares every repair with ψ in one `ltlfilt` exec per run. The summary comes from `python3 -I bench/traces_summary.py runs bench/results/trace-sets-search.csv`.

| Group | Runs | Timeouts | With a repair | Repair stronger than ψ | Equivalent to ψ | All weaker than ψ | Incomparable | Median wall |
|---|---|---|---|---|---|---|---|---|
| Flie (lassos) | 61 | 0 | 53 | 1 | 22 | 9 | 21 | 0.50 s |
| LTLf benchmarks | 45 | 17 | 26 | 5 | 4 | 3 | 14 | 0.76 s |
| Scarlet | 20 | 0 | 19 | 4 | 10 | 2 | 3 | 0.45 s |
| All | 126 | 17 | 98 | 10 | 36 | 14 | 38 | 0.51 s |

A run counts as *stronger* when some repair is strictly stronger than ψ, which is a better answer than the known one. It counts as *equivalent* when some repair is equivalent to ψ and none is stronger. 46 of the 98 runs with a repair matched or beat ψ. Every repair holds on all positives, since the oracle checks each one. The best repair of each run rejects a median of 98.75% of the negatives, all of them in 49 runs and none in 6, although the search never sees the negatives.

Across the 109 runs that finished, the oracle answered 78,631 queries: 20,679 from the memo and 57,952 from the back end, with no Spot exec. The back end costs microseconds per call, so it is not what limits the search. All 17 time-outs are in the LTLf families with the largest formulas: every DoubleCounter and Nim run, 4 of 5 SingleCounter runs, and 1 each in RandomBooleanCombinationsofFactors, Random_Conjuncts_from_Basis and Subword. "Slow implication batches" below finds the cause and measures the fix.

## Memo only against the full oracle

The same search ran three times on a second sample, 2 instances per family with seed 1 and the DoubleCounter, Nim and SingleCounter families left out, so that Spot's validity filter does not swamp the comparison. That is 45 runs per build, again two at a time with a 300 s limit, on a host whose load average was 0.3–1.5 before the runs. The builds were the full tiered oracle with `Lasso::satisfies` (commit ea69145), the full oracle with the packed evaluator, and `Oracle::memo_only` with the packed evaluator, which is what `weaken --traces` now uses. The data is in `bench/results/trace-sets-tiers-{lasso,packed,memo}.csv`.

| Build | Time-outs | Median wall | Total wall | Lasso hits | Implication hits | Back-end calls | Spot execs |
|---|---|---|---|---|---|---|---|
| full oracle, `Lasso::satisfies` | 2 | 1.68 s | 537.6 s | 15,312 | 4,112 | 2,097 | 4,834 |
| full oracle, packed | 2 | 1.87 s | 548.3 s | 15,312 | 4,112 | 2,097 | 4,834 |
| memo only, packed | 0 | 0.50 s | 75.7 s | 0 | 0 | 21,521 | 0 |

The times and counts cover the 43 runs that finished under all three builds, and in each build the memo answered 8,589 of the 30,110 queries. The packed evaluator changes nothing under the full oracle, since the back end answers only 7% of queries there and Spot's 4,834 execs dominate the time. Memo only cut the total wall time 7.1×, with a median speed-up per run of 3.2× and a range of 0.87× to 68×. All 43 runs reported the same number of repairs, with the same comparison to ψ, under the full oracle and under memo only. Memo only also finished the 2 runs that timed out under the full oracle.

These measurements show that a trace set needs only the memo in front of it. The remaining cost of the search lies in the Spot calls outside the oracle.

## Model counts under LTLf

The similarity objective needs a model count, and the lasso count of `src/count/mod.rs` counts ultimately periodic infinite words. Under LTLf it measures the wrong language. On one atom it gives `X[!] true` the same count as `true`, although no one-position word satisfies the first, and it gives `X false` no words, although under LTLf's weak `X` the one-position words all satisfy it. Two options were measured: the lasso count as it stands, and a count of the finite words of length 1 to 20 accepted by the DFA that `ltlf2dfa -Ht` builds, which `src/count/mod.rs` now counts by state with each edge weighted by the models of its label.

The pairs were the 196 LTLf instances of `trace_bench instances`, each a φ and the known weakening ψ that φ implies under LTLf. `trace_bench counts` counted φ and ψ under both semantics, one pair per process, each in its own `systemd-run` scope with a 600 s limit and `MemoryMax=6G`, on `ben` with a load average of 3–5. The rows are in `bench/results/trace-sets-counts.csv`.

| Count | Pairs counted | Median | p90 | Max | Total |
|---|---|---|---|---|---|
| finite words, `ltlf2dfa` | 196 | 16.6 ms | 37.6 ms | 3.0 s | 10.3 s |
| lassos, `ltl2tgba -D` | 191 | 18.3 ms | 1.64 s | 458 s | 757 s |

Neither count ranked any φ above its ψ. The lasso count gave no value for 5 pairs, 2 of them killed at the memory limit and 1 at the time limit, and its slow tail is the translation: on 61 DoubleCounter formulas `ltl2tgba -D` took 408 s in all and `ltlf2dfa -Ht` 1.4 s. The finite-word count is therefore both the right semantics and the cheaper one, so `weaken --traces` on an LTLf set uses it, and the lasso count stays the only count under LTL. `tests/count.rs` checks the finite count against closed forms and for monotonicity under LTLf implication.

## Slow implication batches

The 10 DoubleCounter and Nim time-outs of "Search on a sample" come from Spot and Ganak calls outside the oracle. The other 7 time-outs, in families with smaller formulas, were not rerun. Each run below is `bench/traces_run.py` with seed 1, 30 generations, a population of 30 and two runs at a time, on `ben` with a load average of 1.2–5.0, in one `systemd-run` scope with `MemoryMax=16G`.

**Cause.** The validity filter sent each generation's offspring to `ltlfilt` in one batch under the oracle's 120 s limit. On DoubleCounter `2_5_64_42`, generation 2 had 29 pairs: alone, 28 of them took a median of 20 ms and at most 0.56 s, and one was still undecided after 60 s. That one pair held the batch to its limit, the batch failed, and the filter lost all 29 answers after 120 s. The counter did the same with its translations: one generation's batch timed out at 120 s, and the 61 formulas retried one at a time took a median of 5.4 s each, 544 s for the generation. The gate had the pattern too: its pruning batch asked 342 ordered pairs of survivors and, under a 10 s limit, took 575 s on retries.

**Fix.** `implications_each` retries a failed batch one pair per exec, four execs at a time, and a pair whose own exec fails reads as not proved, so it stays unfiltered while the rest are filtered. The filter now runs after scoring and asks only about candidates that hold on M, since M ⊨ I makes the others not valid modulo I. The filter, the counter and the gate's pruning give each exec 2 s, and the gate's proof of φ ⇒ φ', whose undecided pair drops a repair, gives it 10 s. On `2_5_64_42` these steps went from no finished run (killed at 471 s within 3 generations) through 11 generations in 600 s with 10 s limits, to all 30 generations and the gate in 868 s with 19 repairs, 575 s of it in the gate's pruning, and to 456 s with the same 19 repairs once pruning used 2 s.

**Ganak.** After the fix, two Nim runs (`heaps=1tokens=4`) still ran past 1,200 s. Their φ has 17 atoms, so many DFA labels mention more than PEREDUR's 16-atom truth-table limit and went to Ganak: one generation made 1,086 Ganak calls and spent 79.5 s in the counter. Counting each such label by truth table up to 20 atoms (`LABEL_TABLE_ATOMS`) gives the same counts with no Ganak call, and the generation took 9.8 s.

| Build | Time-outs | Total wall | Gate | Repairs |
|---|---|---|---|---|
| before, 300 s limit (`trace-sets-search.csv`) | 10 | — | — | — |
| batch retries, 1,200 s limit (`trace-sets-dcnim-ganak.csv`) | 2 | 5,399 s | 518 s | 52 |
| and 20-atom label tables (`trace-sets-dcnim.csv`) | 0 | 4,456 s | 520 s | 52 |

Before the fix all 10 runs timed out at 300 s, and an intermediate build with 10 s limits and one retry at a time still timed out on 8 of them at 600 s, finishing the two `heaps=1tokens=1` Nim runs in 410 s and 408 s. Those two now take 53 s and 34 s. Every run now finishes, in 34–841 s with a median of 484 s. The DoubleCounter runs are unchanged by the label tables, since their labels are narrow. Four runs report no repair: DoubleCounter `2_100_32_42`, Nim `heaps=2tokens=2_100_32_42` and both Nim `heaps=1tokens=4` runs.

## Graded status on trace sets

The graded status of `docs/dev/search.md` ("Graded status") scores a candidate φ' that the oracle does not prove as k / (n + 1). Here n is the number of runs of M that violate φ, and k is the number of those runs that φ' satisfies. On a model checker those runs are the lassos stored by tier 1. `weaken --traces` uses `Oracle::memo_only`, which stores none, so the grade was always 0 on trace sets. A trace set holds M as a list of runs instead. `Backend::runs` returns the positive traces, and `Oracle::lassos` returns them in place of tier 1's store. Every positive trace that violates φ is then a ready-made counterexample. The grade is the share of those traces that φ' accepts.

The set is fixed for the whole run. If φ1 implies φ2 under the set's semantics, every trace that satisfies φ1 also satisfies φ2, so the grade is monotone under implication, LTLf implication included. The +1 still matters. A candidate can satisfy every positive that violates φ and fail on a positive that satisfies φ, since candidates need not be weakenings until the gate. Such a candidate scores n / (n + 1), below the 1 of a proved one. `src/search/grade.rs` tests both properties: on four positives, one of 70 positions, and on 50 random sets of finite words against a chain of LTLf implications.

**Cost.** The grader now packs each run of at most 64 positions over φ's atoms and evaluates it with the trace sets' compiled program (`oracle::packed`). Longer runs, and any candidate with an atom outside φ's, which mutation never makes, fall back to `Lasso::satisfies`. On Scarlet `Varying_size` `f_01_nw_50000` (37,458 positives that violate φ, 30 generations of 30, seeds 1–3, `ben` with a load average of 4.4–5.2), grading took 37.2–48.7 s per run with `Lasso::satisfies` and 2.0–2.4 s packed, about 45 ns per evaluation against about 900 ns. On `f_02_nw_2000` it took 1.49–1.78 s and 0.08–0.14 s. Both builds gave the same evaluations, repairs and populations. Over the 1,238 graded runs of the sweep below, grading took a median of 0.28% of the search wall time, and at most 62%.

**Method.** `python3 -I bench/traces_run.py bench/results/trace-sets-instances.csv - - --graded DIR` writes the 126 instances of "Search on a sample" (5 per family, seed 1) as an `instances.tsv` for `bench/graded_run.py`. Each run there is `search_bench --backend traces`. The sweep ran three arms, `binary`, `graded` and `feasible` (`--status graded-feasible`), with seeds 1–10, 30 generations of 30, 3,600 s and an 8 GB systemd scope per run. It ran on av2, 4 at a time, with other users' jobs bringing the load average to 12–23. The binary and graded runs of seeds 1–5 ran on a build of this change on `9f4aaf4` (pre-squash commit `adb61f4`). The rest ran on a build on `ec7e22b` (pre-squash `daa44a4`), which added graded-feasible. 140 binary and graded runs repeated on the second build matched the first in repairs, held candidates, back-end calls and grader evaluations. `bench/graded_stats.py` pairs the arms by seed and reports the same statistics as "Graded status against binary" in `search.md`. For a finite set it decides net subsumption under LTLf (`--from-ltlf`).

**Runs without a result.** 26 binary, 22 graded and 33 feasible runs of the 1,260 per arm ran out of memory in their 8 GB scope. All of them were in seven instances: one RandomBooleanCombinationsofFactors set (every run), four `disjunctedExistence` sets, one SingleCounter set and one Random_Conjuncts_from_Basis set. The memory went to the lasso count of `count`, before its transition monoid was bounded. Rebased onto `651030d`, which contains that bound (`f006d38`), the binary run of `disjunctedExistence` 0067 with seed 1 finishes in 49 s with a peak of 0.75 GB on `ben` (load average 1.4), where both earlier builds ran out of memory. The 0.6 s that `trace-sets-search.csv` records for the same run does not reproduce, not even on `c3acf68`, and its cause is unknown.

| Measure (10 seeds, 125 instances) | Binary | Graded | Difference | p | Higher / lower / tie |
|---|---|---|---|---|---|
| Found-repair rate | 1.00 [1.00, 1.00] | 1.00 [1.00, 1.00] | 0.00 [0.00, 0.00] | 0.042 | 6 / 15 / 104 |
| Proved candidates held (of 30) | 16.0 [16.0, 17.5] | 8.5 [7.5, 10.0] | −7.0 [−8.0, −5.5] | <0.0001 | 4 / 111 / 10 |
| Repairs | 2.0 | 2.0 | 0.0 | 0.0002 | 18 / 48 / 59 |
| Wall (s) | 1.38 [1.27, 1.59] | 1.41 [1.26, 1.61] | 0.006 [−0.008, 0.021] | 0.68 | 65 / 60 / 0 |

| Measure (10 seeds, 124 instances) | Binary | Feasible | Difference | p | Higher / lower / tie |
|---|---|---|---|---|---|
| Found-repair rate | 1.00 [1.00, 1.00] | 1.00 [1.00, 1.00] | 0.00 [0.00, 0.00] | 0.040 | 7 / 16 / 101 |
| Proved candidates held (of 30) | 16.0 [15.5, 17.5] | 30.0 [30.0, 30.0] | 11.5 [10.5, 13.0] | <0.0001 | 115 / 0 / 9 |
| Repairs | 2.0 | 2.0 | 0.0 [0.0, 0.5] | <0.0001 | 59 / 22 / 43 |
| Wall (s) | 1.37 [1.27, 1.58] | 1.34 [1.19, 1.44] | −0.054 [−0.090, −0.032] | 0.006 | 37 / 87 / 0 |

Feasible against graded held a median of 19.5 more proved candidates, with no change in found rate (13 / 13 / 98, p = 0.96). Every number in these two tables covers all 10 seeds and comes from `bench/results/trace-grading-runs.csv`. They omit net subsumption, which is partial.

**Strength (partial).** The full 10-seed strength comparison had not finished on av2 when this section was written, because the pooled maximal sets, and so the Spot pairs, grow with the seeds. The net subsumption share below covers seeds 1–5, binary against graded only (`bench/results/trace-grading-instances-s1-5.csv`). Its Spot limits were 30 s per batch and 2 s per retried pair. Spot left 14,336 of the 66,196 LTLf pairs undecided, 22%, and all 7,090 LTL pairs decided. An undecided pair reads as not implied, so the share on the LTLf families is weak evidence. Over the 116 instances where both arms found a repair, the median net share was 0.000 [0.000, 0.000], with graded stronger on 35, weaker on 34 and tied on 47 (p = 0.32, mean −0.041). DoubleCounter (mean −0.38) and Scarlet RQ1 (−0.35) leaned towards binary, and Flie `5to10OneThree` (+0.20) towards graded, on 5 instances each.

**Undecided pairs.** Over all 10 seeds the binary-against-graded comparison asks 228,274 LTLf pairs, and the 2 s limit left 42,616 of them undecided, 18.7%. They lie in 17 of the 118 compared instances, 41,427 of them in eight DoubleCounter and SingleCounter sets. `bench/graded_recheck.py` asked Spot about them again, one pair per exec in a 6–7 GB scope, on av3 and av2 (load average 2–26). On 480 pairs drawn at random, 10 s decided 22 (4.6%), 60 s decided 56 in all (11.7%), and 600 s decided 26 of 147 sampled 60 s time-outs, which puts the 600 s share near 23%. A further 113 of the 480 (23.5%) fail at once or within 60 s because their automaton needs more than Spot's 32 acceptance sets, which no limit fixes, and no exec reached its memory limit. Only 3 of the 82 decided pairs were implications, so reading an undecided pair as not implied is mostly right. Every undecided pair of the nine smaller instances, 1,189 in all, also ran at 10 s and 60 s, which decided 650, and the Nim and RandomBooleanCombinationsofFactors pairs ran at 600 s. With the 741 decided pairs merged into the cache, the 10-seed net share is 0.000 [0.000, 0.000] over 118 instances, graded stronger on 36, weaker on 37 and tied on 45 (p = 0.72, mean −0.014), against p = 0.81 and 36 / 36 / 46 before. On the 104 instances with no undecided pair, where the share is exact, it is 0.000 with 29 / 32 / 43 (p = 0.65). The rows are in `bench/results/trace-grading-recheck.csv`. A longer pair limit in the search would buy little, since 10 s decides a twentieth of these pairs and a quarter cannot be decided at all. `ltlf2dfa`, which builds a DFA and so has no acceptance-set limit, decides 58 of 60 sampled undecided pairs at a median of 47 ms. `bench/graded_stats.py` now uses it ("Trace sets" in `docs/dev/evaluation.md`), and the comparison above has not been rerun with it.

The grade does not help on trace sets. It costs the population about half its proved candidates and gains no strength or speed, as on the model-checking benchmarks, even though here it measures the share of positives accepted. Graded-feasible keeps every slot for proved candidates once 30 exist, finds repairs at the binary rate and runs a little faster. It still adds no measured strength, which the partial comparison above cannot settle for LTLf.
