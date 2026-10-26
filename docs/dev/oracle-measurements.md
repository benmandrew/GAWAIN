# Oracle measurements

This note measures how long the tiered oracle (`docs/dev/oracle.md`) takes per query on RERS with SPIN and on LMCS-2006 with nuXmv, and how often the cheap tiers answer. The raw data is one CSV row per query in `bench/results/oracle-bench-spin.csv` (6,623 queries) and `bench/results/oracle-bench-nuxmv.csv` (2,089 queries).

## Method

`src/bin/oracle_bench.rs` builds a *candidate stream* per property φ: φ first, then a seeded shuffle of mutants from Spot's `ltlgrind`, drawn from its one-mutation list and from the first 5,000 entries of its two-mutation list. Repeats are kept, since a genetic algorithm (GA) proposes the same candidate more than once and the memo exists for that. Each stream goes through a fresh `Oracle`, as one GA run would. The binary wraps the back end in a timer, so each row has the wall time of the whole query and, for a back-end call, the back end's time alone. The tier that answered comes from the difference in `Counters` across the query.

`bench/oracle_prep.py` writes one job per property and `bench/oracle_run.sh` runs the jobs in parallel, each in its own `systemd-run` scope with `MemoryMax`. `bench/oracle_stats.py` produces every table below from the two CSVs (`python3 bench/oracle_stats.py bench/results/oracle-bench-*.csv`). Percentiles interpolate linearly between order statistics. The *spread* is the central 95% of the times (2.5th to 97.5th percentile). The *mean CI* is a bootstrap 95% confidence interval for the mean, from 10,000 resamples with a fixed seed.

**Inputs.** SPIN ran the 20 `ltl pN { … }` blocks of each RERS parallel problem, 2016 101–106 and 2017 101–105, which is 220 properties. The atoms are each model's `#define`s. nuXmv ran the LMCS-2006 pairs that `bench/lmcs_check.py` checks, on the models converted by `bench/lmcs_fix.py`.

**LMCS abstraction.** `Formula` has only future operators over plain atoms, and LMCS properties use comparisons such as `sender.state=get` and past operators (Y, H, O, S, T). So `oracle_prep.py` turns each comparison or bare variable into a DEFINE `aragon_aK`, and each maximal past subformula into a monitor variable `aragon_mK` with INIT and TRANS constraints added to the main module. `ltlgrind` then mutates only the future skeleton, and every past subformula stays fixed. Two of the 61 pairs cannot be abstracted this way: bc57-sensors `p0` and `p0neg` put F under O, which a monitor cannot track. Those two are absent, so 59 pairs ran. On every pair where nuXmv answered, φ's verdict matched the README, including production-cell `p4` ("probably TRUE"), which holds.

**Mutant settings.** Seed 1 throughout. Fast models drew 40 one-mutation and 40 two-mutation mutants per property (`--one 40 --two 40`): RERS 2016 101–104, 2017 101, and every LMCS model except bc57-sensors, dme5 and dme6. Those three LMCS models drew 5 and 5. So did RERS 2016 105–106 and 2017 102–105, since the SPIN pilot timed out on most of their properties and an `Unknown` is never memoised, so each timed-out candidate costs a full timeout again. A stream holds fewer candidates when `ltlgrind` lists fewer mutants than requested.

**Limits.** SPIN: 120 s and 3,800 MB address space per tool call (each of the two `spin -a` passes, `gcc`, `pan` and the replay), in a 4 GB scope. The limit is per tool call, so a query can run past 120 s. nuXmv: 300 s and 5,800 MB per call, in a 6 GB scope. Spot (`ltlgrind` and `ltlfilt`): 60 s and 2 GB per exec.

**Hosts.** Both hosts have a 13th Gen Intel Core i9-13900F (32 threads) and 125 GB. av2 (hostname `avlab12`) ran RERS 2016 101–106, 2017 101 and half of 2017 102–105. av3 ran the other half of 2017 102–105 and all of LMCS. av2 ran 22 jobs at once, alongside another user's SPIN runs: its load average was 15.6 at the start, 30.8 at the peak and 1.0 at the end. av3 ran 24 at once, with a load average of 8.9 at the start and 13–18 during the run. Tool versions: SPIN 6.5.2, gcc 13.3.0, nuXmv 2.1.0, Spot 2.15.1. The hosts' own Spot is 2.14.5, whose `ltlfilt` has no `--satisfiable`, so every tier 2 exec failed and tier 2 answered nothing. A first full run was discarded for that reason, and the run reported here used a copy of the local Spot 2.15.1 build.

## Cache hit rate

The cache hit rate is the share of queries that tiers 0 to 2 answered (memo, lasso and implication). Unknown counts the queries that ended `Unknown`, all at the back end.

| Group | Queries | Memo | Lasso | Implication | Back end | Cache hit rate | Unknown |
|---|---|---|---|---|---|---|---|
| rers2016-101 | 1076 | 6.7% | 41.6% | 35.9% | 15.8% | 84.2% | 0 |
| rers2016-102 | 1205 | 5.7% | 33.0% | 43.5% | 17.8% | 82.2% | 0 |
| rers2016-103 | 1029 | 6.6% | 47.2% | 29.2% | 17.0% | 83.0% | 0 |
| rers2016-104 | 1084 | 5.8% | 46.5% | 31.5% | 16.2% | 83.8% | 0 |
| rers2016-105 | 220 | 0.5% | 36.8% | 6.8% | 55.9% | 44.1% | 80 |
| rers2016-106 | 220 | 1.8% | 26.8% | 5.5% | 65.9% | 34.1% | 103 |
| rers2017-101 | 912 | 7.2% | 43.2% | 32.1% | 17.4% | 82.6% | 0 |
| rers2017-102 | 220 | 0.0% | 30.5% | 9.1% | 60.5% | 39.5% | 96 |
| rers2017-103 | 220 | 0.0% | 20.0% | 2.3% | 77.7% | 22.3% | 140 |
| rers2017-104 | 220 | 0.9% | 14.1% | 4.5% | 80.5% | 19.5% | 155 |
| rers2017-105 | 217 | 1.8% | 14.3% | 4.1% | 79.7% | 20.3% | 147 |
| SPIN pooled | 6623 | 5.3% | 38.4% | 28.9% | 27.4% | 72.6% | 721 |
| lmcs-abp4 | 198 | 3.0% | 53.0% | 22.2% | 21.7% | 78.3% | 0 |
| lmcs-bc57-sensors | 55 | 0.0% | 47.3% | 7.3% | 45.5% | 54.5% | 4 |
| lmcs-brp | 261 | 2.7% | 69.0% | 11.1% | 17.2% | 82.8% | 0 |
| lmcs-counter | 19 | 21.1% | 26.3% | 15.8% | 36.8% | 63.2% | 0 |
| lmcs-dme2 | 70 | 11.4% | 50.0% | 17.1% | 21.4% | 78.6% | 0 |
| lmcs-dme3 | 211 | 6.6% | 61.6% | 18.0% | 13.7% | 86.3% | 0 |
| lmcs-dme4 | 211 | 6.6% | 61.1% | 18.0% | 14.2% | 85.8% | 0 |
| lmcs-dme5 | 55 | 5.5% | 47.3% | 14.5% | 32.7% | 67.3% | 0 |
| lmcs-dme6 | 55 | 5.5% | 27.3% | 10.9% | 56.4% | 43.6% | 18 |
| lmcs-mutex | 42 | 14.3% | 38.1% | 23.8% | 23.8% | 76.2% | 0 |
| lmcs-production-cell | 603 | 4.0% | 44.9% | 16.9% | 34.2% | 65.8% | 0 |
| lmcs-ring | 55 | 10.9% | 38.2% | 36.4% | 14.5% | 85.5% | 0 |
| lmcs-short | 42 | 14.3% | 52.4% | 11.9% | 21.4% | 78.6% | 0 |
| lmcs-srg5 | 212 | 2.8% | 63.2% | 18.9% | 15.1% | 84.9% | 0 |
| nuXmv pooled | 2089 | 5.1% | 53.4% | 17.2% | 24.3% | 75.7% | 22 |

On the models where the back end answers, the cheap tiers answered 82–86% of the RERS queries and 63–86% of the LMCS queries. On the models where it times out, the rate falls to 20–44%. A timeout stores no lasso and no truth, so later candidates have nothing to hit, and the memo never stores the `Unknown` itself.

## Back-end time

These rows are back-end calls that returned a verdict, timed around `Backend::check` alone. Times are in seconds.

| Group | Verdict | n | Median | Mean | Spread (p2.5–p97.5) | Mean 95% CI |
|---|---|---|---|---|---|---|
| SPIN pooled | holds | 729 | 0.819 | 2.24 | 0.547–10.1 | 1.68–2.91 |
| SPIN pooled | fails | 366 | 0.979 | 6.01 | 0.587–79.7 | 4.31–7.97 |
| nuXmv pooled | holds | 375 | 1.06 | 3.97 | 0.0321–18.1 | 2.95–5.26 |
| nuXmv pooled | fails | 111 | 1.34 | 26.2 | 0.0321–176 | 16.9–36.7 |
| rers2016-101 | holds | 140 | 0.605 | 0.614 | 0.527–0.759 | 0.604–0.625 |
| rers2016-101 | fails | 30 | 0.619 | 0.635 | 0.548–0.831 | 0.611–0.666 |
| rers2016-102 | holds | 159 | 0.669 | 0.681 | 0.596–0.83 | 0.672–0.69 |
| rers2016-102 | fails | 55 | 0.704 | 0.724 | 0.614–0.939 | 0.702–0.747 |
| rers2016-103 | holds | 135 | 0.823 | 0.851 | 0.725–1.11 | 0.833–0.87 |
| rers2016-103 | fails | 40 | 0.879 | 0.896 | 0.78–1.12 | 0.866–0.929 |
| rers2016-104 | holds | 125 | 1.73 | 2.15 | 0.849–9.69 | 1.83–2.52 |
| rers2016-104 | fails | 51 | 1.02 | 1.03 | 0.852–1.32 | 1–1.07 |
| rers2016-105 | holds | 11 | 54.4 | 47.7 | 0.922–111 | 22.3–73.5 |
| rers2016-105 | fails | 32 | 2.1 | 19.6 | 0.576–77.7 | 11.1–29.4 |
| rers2016-106 | holds | 12 | 0.795 | 0.742 | 0.559–0.816 | 0.681–0.797 |
| rers2016-106 | fails | 30 | 3.56 | 33 | 0.603–106 | 19–48.9 |
| rers2017-101 | holds | 121 | 1.44 | 2.58 | 0.902–10.1 | 2.15–3.05 |
| rers2017-101 | fails | 38 | 1.22 | 1.41 | 0.868–2.22 | 1.16–1.86 |
| rers2017-102 | holds | 11 | 13.3 | 17.2 | 0.801–60.4 | 6.7–30.5 |
| rers2017-102 | fails | 26 | 1.21 | 4.68 | 0.772–33.1 | 1.38–10.5 |
| rers2017-103 | holds | 4 | 0.996 | 1.05 | 0.96–1.22 | 0.967–1.18 |
| rers2017-103 | fails | 27 | 1.1 | 3.58 | 0.882–29.2 | 1.28–7.1 |
| rers2017-104 | holds | 5 | 1.34 | 1.32 | 1.15–1.52 | 1.2–1.43 |
| rers2017-104 | fails | 17 | 1.42 | 7.96 | 1.12–45.4 | 2.14–15.6 |
| rers2017-105 | holds | 6 | 1.34 | 1.37 | 1.07–1.64 | 1.21–1.52 |
| rers2017-105 | fails | 20 | 1.49 | 1.42 | 0.885–1.86 | 1.3–1.54 |
| lmcs-abp4 | holds | 38 | 0.0724 | 0.0777 | 0.0522–0.113 | 0.0718–0.0835 |
| lmcs-abp4 | fails | 5 | 0.0926 | 0.0964 | 0.0743–0.112 | 0.0843–0.108 |
| lmcs-bc57-sensors | holds | 13 | 1.48 | 2.53 | 0.481–10.8 | 1.17–4.64 |
| lmcs-bc57-sensors | fails | 8 | 123 | 121 | 43.9–172 | 90.7–147 |
| lmcs-brp | holds | 29 | 0.373 | 0.511 | 0.207–1.3 | 0.408–0.632 |
| lmcs-brp | fails | 16 | 1.43 | 1.48 | 0.403–2.63 | 1.15–1.83 |
| lmcs-counter | holds | 5 | 0.032 | 0.032 | 0.032–0.0321 | 0.032–0.0321 |
| lmcs-counter | fails | 2 | 0.032 | 0.032 | 0.032–0.0321 | 0.032–0.0321 |
| lmcs-dme2 | holds | 8 | 0.0522 | 0.0522 | 0.0521–0.0524 | 0.0522–0.0523 |
| lmcs-dme2 | fails | 7 | 0.575 | 0.443 | 0.0753–0.627 | 0.287–0.593 |
| lmcs-dme3 | holds | 18 | 0.153 | 1.43 | 0.0923–3.82 | 0.685–2.21 |
| lmcs-dme3 | fails | 11 | 4.72 | 5.82 | 0.248–12.3 | 3.54–8.14 |
| lmcs-dme4 | holds | 18 | 0.635 | 6.84 | 0.422–20 | 3.22–10.7 |
| lmcs-dme4 | fails | 12 | 30.9 | 28.6 | 1.33–60.4 | 19–38.7 |
| lmcs-dme5 | holds | 10 | 1.36 | 29.1 | 0.98–105 | 1.25–57.7 |
| lmcs-dme5 | fails | 8 | 159 | 134 | 2.31–252 | 71.2–192 |
| lmcs-dme6 | holds | 8 | 2.8 | 18.5 | 2.26–107 | 2.6–50.2 |
| lmcs-dme6 | fails | 5 | 5.61 | 46.5 | 4.66–111 | 5.09–89.1 |
| lmcs-mutex | holds | 8 | 0.0322 | 0.0322 | 0.032–0.0324 | 0.0321–0.0323 |
| lmcs-mutex | fails | 2 | 0.032 | 0.032 | 0.032–0.0321 | 0.032–0.0321 |
| lmcs-production-cell | holds | 196 | 2.42 | 4.34 | 0.316–17.3 | 3.71–5.03 |
| lmcs-production-cell | fails | 10 | 1.25 | 20.3 | 0.34–81 | 2.56–41.2 |
| lmcs-ring | holds | 6 | 0.0321 | 0.0321 | 0.0319–0.0323 | 0.032–0.0322 |
| lmcs-ring | fails | 2 | 0.0323 | 0.0323 | 0.0321–0.0324 | 0.0321–0.0324 |
| lmcs-short | holds | 4 | 0.0321 | 0.0321 | 0.032–0.0321 | 0.032–0.0321 |
| lmcs-short | fails | 5 | 0.0321 | 0.0321 | 0.032–0.0322 | 0.0321–0.0321 |
| lmcs-srg5 | holds | 14 | 0.0322 | 0.0322 | 0.0321–0.0323 | 0.0321–0.0322 |
| lmcs-srg5 | fails | 18 | 0.0323 | 0.0356 | 0.0321–0.0524 | 0.0323–0.039 |

A single-claim SPIN build sets a floor of about 0.55 s on every call. Medians stay near 1 s, but the means are 2–6 times the medians because a few searches on the larger problems run for tens of seconds. nuXmv calls on the small LMCS models cost 32–100 ms, about the process start-up. Failures on dme5 and bc57-sensors cost minutes. Samples on the slow models are small (4 to 13 calls per verdict), so their mean CIs are wide.

## Time of a whole query

Every query that returned a verdict, at any tier, timed around `Oracle::check`.

| Group | Verdict | n | Median | Mean | Spread (p2.5–p97.5) | Mean 95% CI |
|---|---|---|---|---|---|---|
| SPIN pooled | holds | 2803 | 0.0178 | 0.596 | 2e-06–2.33 | 0.447–0.767 |
| SPIN pooled | fails | 3099 | 1.9e-05 | 0.71 | 3e-06–1.39 | 0.493–0.949 |
| SPIN pooled | both | 5902 | 0.00852 | 0.656 | 2e-06–2 | 0.521–0.805 |
| nuXmv pooled | holds | 783 | 0.0175 | 18 | 1e-06–127 | 15.3–20.9 |
| nuXmv pooled | fails | 1284 | 2.7e-05 | 2.36 | 1e-06–9.61 | 1.47–3.39 |
| nuXmv pooled | both | 2067 | 0.000137 | 8.3 | 1e-06–124 | 7.07–9.58 |

A lasso refutation costs about 20 µs and a tier 2 proof about 17 ms (one `ltlfilt` exec), so the median query is far below the median back-end call. The nuXmv mean for truths is inflated by one model. On production-cell, 131 of 196 back-end truths spent 60 s or more in `ltlfilt`, against a median of 2.4 s in nuXmv itself: 76 of them hit the 60 s limit twice, once in the tier 2 lookup that failed before the back-end call and once in the exec that evicts implied truths after it. Spot's complete check on large formulas cost more than the model checker it stood in front of. "Tier 2 eviction" below diagnoses this and measures the fix.

## Tier 2 eviction

Tier 2 and eviction both asked Spot's complete check (`ltlfilt --satisfiable`, which translates `ψ & !φ` to an automaton) under the 60 s limit, with no cheaper step first. On production-cell that check cost minutes, and this section finds out why, then measures the replacement now in `src/oracle/tiered.rs`.

**Diagnosis method.** A diagnostic patch (`bench/notes/tier2-spot-log.patch`, against commit 69992f2) makes the oracle log every `ltlfilt` batch: lookup or eviction, its pairs, its time, the store size and the result. The 10 production-cell jobs ran with it on av2 for 40 minutes at the old code and logged 263 batches, and a *fast set* of 134 jobs (RERS 2016 101–104 and 2017 101, and every LMCS model except production-cell, bc57-sensors, dme5 and dme6) ran to the end and logged 2,401. `bench/tier2_replay.py` then ran each distinct pair alone, once with the complete check under a 60 s limit and once with the rewriting-rule check described below. It replayed all 5,586 fast-set pairs and 1,261 of the 1,557 production-cell pairs before its 3-hour cap. The per-pair rows are in `bench/results/tier2-pairs-fast.csv` and `bench/results/tier2-pairs-production-cell.csv`.

**Why production-cell stalls.** Its properties `p3` and `p4` and their negations are chains of 13 to 15 nested `U` over 24 to 33 atoms, and the mutants differ from them in one or two operands. The batches were small: a lookup held a median of 3 pairs (at most 22) and an eviction 6 (at most 19). Alone, the complete check proved a pair in a median of 8.8 ms (at most 1.14 s), but it refuted one in a median of 87 ms with a 97.5th percentile of 34 s, and 319 of the 1,261 pairs ran past 60 s. So 68 of 182 lookup batches and 54 of 81 eviction batches timed out. A timed-out batch held a pair that times out alone, or several slow refutations that summed past 60 s. One hard pair does stall the whole batch: 30 lookup pairs and 8 eviction pairs that the complete check proves alone sat in batches that timed out. Failed evictions also let the store grow to 26 truths, which made later batches larger. Only `p3`, `p4` and their negations stalled, and the fast set had no timed-out batch.

**The rewriting rules.** `ltlfilt -r2` rewrites a formula with Spot's basic rules and its *syntactic implication* rules, and builds no automaton. A line `(ψ) & !(φ)` that it rewrites to `0` proves ψ ⇒ φ, and `--ap=0 --format=%L,%f` prints only the lines that became a constant (`syntactic_implications` in `src/oracle/spot.rs`). On every replayed pair it answered in at most 18 ms. On production-cell it proved 340 pairs, every pair the complete check proved there and 22 on which the complete check timed out. On the fast set it missed 481 of the 5,435 pairs the complete check proved (426 lookup pairs and 55 eviction pairs), all of them proved by the complete check in at most 17 ms.

**Policies.** `bench/tier2_simulate.py` replayed each logged batch under each policy, timing a batch as one start-up plus the solo time of each pair, and a batch that passes its limit as proving nothing. The table counts batches that proved at least one pair, which for a lookup is a tier 2 hit, over the 168 production-cell lookup batches whose pairs were all replayed and all 2,101 fast-set lookup batches.

| Policy | Production-cell hits | Production-cell mean time | Fast-set hits | Fast-set mean time |
|---|---|---|---|---|
| Complete check, 60 s (old) | 102 | 19.8 s | 2,083 | 8.10 ms |
| Rules only | 119 | 9.87 ms | 1,897 | 7.82 ms |
| Rules, then complete check for 250 ms | 119 | 73.3 ms | 2,083 | 8.63 ms |
| Rules, then complete check for 1 s | 119 | 253 ms | 2,083 | 8.63 ms |
| Rules, then complete check for 60 s | 119 | 13.7 s | 2,083 | 8.63 ms |

Rules alone lose 186 fast-set hits (9%), and each lost hit becomes a back-end call of about 0.6 s on RERS. So a lookup runs the rules first and the complete check only when the rules prove nothing, for at most 250 ms (`COMPLETE_CHECK_BUDGET`), about 15 times the slowest proof the rules missed (17 ms). On eviction the complete check made 33 more fast-set batches evict something and none on production-cell, where its 69 replayed eviction batches cost 2,798 s at the 60 s limit and 11.4 s with a 250 ms budget, against 0.69 s for the rules. Eviction runs the rules only, since a store that keeps an implied truth still answers correctly. The store never grew past 14 truths on the fast set, so a size bound was not needed. Splitting batches per pair would have rescued the 38 stalled proofs, but the rules already prove all of them in one exec.

**Rerun.** The full job set ran again at the new code on av2 (14 jobs at once; the load average was 0.9 before the diagnosis runs and about 14 during the rerun), with the settings in "Method". The rows are in `bench/results/oracle-bench-spin-eviction.csv` and `bench/results/oracle-bench-nuxmv-eviction.csv`; the earlier CSVs are unchanged. `python3 bench/oracle_compare.py BEFORE AFTER` matched every row on bench, property and sequence number: every candidate was the same, and all 5,900 SPIN and 2,067 nuXmv queries that both runs decided got the same verdict. `bench/oracle_overhead.py` gives the time outside the back end. The query medians and means in this table include the queries that ended `Unknown`, unlike those in "Time of a whole query".

| Group | Run | Implication | Back end | Median query (s) | Mean query (s) | Total query (s) | Outside the back end (s) | Tier 2 answer, median (ms) |
|---|---|---|---|---|---|---|---|---|
| production-cell | before | 16.9% | 34.2% | 0.0136 | 22.9 | 13,787 | 12,734 | 16.4 |
| production-cell | after | 24.0% | 27.0% | 0.00804 | 1.09 | 658 | 25 | 8.3 |
| nuXmv pooled | before | 17.2% | 24.3% | 0.000146 | 11.4 | 23,753 | 12,753 | 16.4 |
| nuXmv pooled | after | 19.2% | 22.1% | 0.000144 | 4.24 | 8,854 | 30 | 8.3 |
| SPIN pooled | before | 28.9% | 27.4% | 0.0116 | 13.8 | 91,198 | 38.9 | 16.7 |
| SPIN pooled | after | 28.9% | 27.4% | 0.00811 | 13.6 | 90,071 | 21.0 | 8.3 |

On production-cell the 153 back-end truths now cost a median of 2.38 s and a mean of 3.40 s per query, against 64.8 s and 68.6 s before, and no query spent more than 0.30 s outside nuXmv. Tier 2 proved 43 more production-cell queries, which no longer reach nuXmv, and its hit rate rose from 16.9% to 24.0%. The nuXmv mean for truths fell from 18.0 s to 1.29 s. On SPIN the hit rates are unchanged except for one query on rers2016-105, whose stream lacked a truth because an earlier SPIN call timed out in one run and not the other. A tier 2 answer costs 8.3 ms instead of 16.4–16.7 ms, since a rules exec skips the automaton translation. The back-end medians moved too (SPIN truths 0.819 s before, 0.508 s after), but the code did not touch the back ends, so that difference is host load: the earlier SPIN run shared av2 with another user's jobs at load averages up to 30.8, and the earlier nuXmv run was on av3.

**Process start-up.** The rest of the query path now costs 30 s of 8,854 s on nuXmv (0.34%) and 21 s of 90,071 s on SPIN (0.02%), over 763 and 2,341 `ltlfilt` execs. A long-lived `ltlfilt` would save at most the 8 ms start-up per exec, about 6 s and 19 s, under 0.1% of either total. The bench does not justify it, so the oracle still starts one process per batch.

## Unknown verdicts

Every `Unknown` was a back-end timeout. SPIN gave 721: 80 on rers2016-105, 103 on 106, and 96, 140, 155 and 147 on rers2017-102 to 105. nuXmv gave 22: 4 on bc57-sensors (all on `p3`) and 18 on dme6. The rerun in "Tier 2 eviction" gave 719 and 18. No candidate contained X, since no RERS property uses it and `ltlgrind` never introduced it, so SPIN's X refusal never fired.

## Caveats

The candidate streams are `ltlgrind` mutants of each φ, which stand in for GA populations. A real GA run differs in two ways that move these numbers: later generations hold more true candidates, which only tier 2 can answer, and a GA re-proposes candidates more often than the overlap between two mutant lists does, which raises the memo rate. "Hit rates on GA populations" below re-measures them on real GA runs. On LMCS, mutants never touch a past subformula, since the abstraction makes each one an atom. Timings on av2 shared the machine with another user's SPIN jobs, so per-call times there carry some contention.

## Hit rates on GA populations

The search records the same counters for every query of a run, its output gate included. `bench/ga_hit_rates.py` reads them from the 838 binary-arm runs at 30 × 40 in `bench/data/baselines/top-runs` (87 instances, seeds 1–10, initial-state filter off) and writes one row per run to `bench/results/ga-hit-rates.csv`. In every run the four tiers sum to the queries.

```
python3 -I bench/ga_hit_rates.py bench/data/baselines/top-runs \
    bench/results/graded-runs.csv bench/results/ga-hit-rates.csv
```

| Family | Runs | Queries | Memo | Lasso | Implication | Back end | Cache hit rate | Median per run | Unknown |
|---|---|---|---|---|---|---|---|---|---|
| BEEM | 141 | 92,696 | 60.8% | 22.9% | 12.1% | 4.2% | 95.8% | 97.3% | 1,647 |
| LMCS abp4 | 10 | 11,628 | 11.3% | 52.3% | 19.3% | 17.0% | 83.0% | 82.9% | 0 |
| MCC | 186 | 194,630 | 14.6% | 48.6% | 29.1% | 7.7% | 92.3% | 93.1% | 189 |
| RERS | 501 | 444,040 | 21.6% | 49.4% | 19.7% | 9.3% | 90.7% | 91.6% | 0 |
| All | 838 | 742,994 | 24.5% | 46.0% | 21.2% | 8.4% | 91.6% | 92.8% | 1,836 |

The cheap tiers answer 91.6% of the queries of a GA run, against 72.6% (SPIN) and 75.7% (nuXmv) on the mutant streams. The lowest run still reaches 72.2%. The memo tier moved most, from about 5% to 24.5%, as the caveat above predicted: selection keeps re-proposing the formulas it already holds. On BEEM the memo answers 60.8%, and the back end gets 4.2% of the queries. 1,647 of BEEM's 3,938 back-end calls ended `Unknown`, almost all on four instances (lamport.6-4, peterson.6-4, rether.4-5 and rether.6-5, about 350 each), where SPIN times out on most candidates. On abp4 the rate is 83.0%, close to the 78.3% that the mutants gave there.

