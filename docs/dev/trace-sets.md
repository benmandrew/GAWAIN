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

Across the 109 runs that finished, the oracle answered 78,631 queries: 20,679 from the memo and 57,952 from the back end, with no Spot exec. The back end costs microseconds per call, so it is not what limits the search. All 17 time-outs are in the LTLf families with the largest formulas: every DoubleCounter and Nim run, 4 of 5 SingleCounter runs, and 1 each in RandomBooleanCombinationsofFactors, Random_Conjuncts_from_Basis and Subword. On DoubleCounter `2_5_64_42`, 2 generations took 120.8 s, of which the oracle's 59 back-end calls and the 0.24 s gate are a negligible part. The remainder is `drop_valid`, whose per-generation `ltlfilt --from-ltlf` batch over a 13-atom formula ran into the 120 s Spot limit. A failed batch drops nothing, so the search stays correct, and each generation still pays the full limit. Making that filter cheaper on large LTLf formulas is left open.

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
