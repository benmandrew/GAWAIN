# Evaluation plan

GAWAIN weakens a *linear temporal logic* (LTL) property φ that a model M violates into a φ' that φ implies and M satisfies. Specification weakening has no ground truth, and ⊤ always holds, so a repair is ranked by its strength against baselines ("Evaluation without a ground truth" in `PLAN.md`). This note fixes the research questions, instance sets, configuration, metrics and commands, so that every headline result comes from one configuration.

## Research questions

Each question names its metric, its current status and where the existing evidence lives.

**RQ1, effectiveness.** How often does GAWAIN find a non-trivial weakening? The metric is the *found rate* per family under the final default configuration. The ablations ran with the initial-state filter off, so the headline run below is the first at the defaults.

**RQ2, strength.** How strong are the repairs? Three comparisons answer this. The first is the *closed share* against ⊤ under the rejection strength ("⊤ under the rejection strength" in `bench/notes/baselines.md`). At the ablation configuration its median was 0.9912 [0.9833, 0.9957] over 87 instances. The second is a comparison with CEGIW on its 8 case studies plus abp4 (`bench/notes/baselines.md`). The third compares φ' with the source formula ψ on trace sets (`vs_psi` in `bench/traces_run.py`).

**RQ3, cost of the genetic search.** Does the search earn its cost? The baseline is *random mutation* with the same operators, filters, scoring and number of candidate evaluations, and with no selection during the run. Its final P survivors are chosen by the same NSGA-II (Non-dominated Sorting Genetic Algorithm II) ranking and pass the same output gate ("Random-mutation baseline" in `docs/dev/search.md`). Both searches breed 1 + G × P candidates; the number scored differs slightly, because the validity filter drops a varying share, and each run reports both. The metrics are the found rate, the *net subsumption* between the two arms' pooled maximal repair sets, the *net containment share* between the same sets ("Diversity" below), the closed share under rejection, back-end calls and wall time. Results are paired by instance and seed, with Wilcoxon signed-rank tests and bootstrap 95% intervals, as in "Graded status against binary" in `docs/dev/search.md`.

**RQ4, cost.** The metrics are wall time, back-end calls and cache hit rate per tier. On real genetic algorithm (GA) runs, the cheap tiers answer 91.6% of queries ("Hit rates on GA populations" in `docs/dev/oracle-measurements.md`).

**RQ5, design ablations.** These are already measured and stay in `docs/dev/search.md`, cited rather than rerun. They cover the status variants (binary, graded, graded-feasible and graded-reserve, with binary kept), the model-counting objective, the initial-state filter and the budget (4×8 against 30×40). Filtering at the initial state in the loop raised the found rate from 0.70 to 0.90.

**Qualitative.** A few case studies show readable repairs, among them abp4 `p2false` and the CEGIW foraging robots. A hand classification of the best repair on each of the 89 headline instances with a repair goes with them. Both are in `bench/notes/case-studies.md`.

## Instance sets

`bench/eval_prep.sh` builds three directories. It leaves `bench/data/graded` unchanged, because earlier results read it.

| Directory | Instances | Contents |
|---|---|---|
| `bench/data/eval` | 107 | From `bench/graded_prep.py`: RERS parallel 2016 (52), RERS 2018 problem 101 (14, new), MCC (20), BEEM ERROR pairs (20), LMCS abp4 `p2false` (1) |
| `bench/data/eval-traces` | 126 | The trace-set instances of "Search on a sample" in `docs/dev/trace-sets.md`, 5 per family, seed 1 |
| `bench/data/cegiw` | 9 | CEGIW's 8 case studies, 3 of them degenerate, plus abp4 `p2false`, run with the `forced-` arms |

RERS is the Rigorous Examination of Reactive Systems challenge, and MCC is the Model Checking Contest. BEEM is the BEnchmarks for Explicit Model checkers set, and LMCS names the 2006 benchmarks of a paper in *Logical Methods in Computer Science*. In `bench/data/eval`, the 93 rows outside RERS 2018 are identical to `bench/data/graded`.

**Exclusions.** Seven model instances are left out, as in the ablations. Six are large BEEM instances that gave no result there: anderson 3-2, 3-4, 5-2, 5-4 and 7-4, and lamport 8-4. The seventh is MCC MedleyA-PT-09, which failed on every seed. That leaves 100 model instances.

**Limitations.** The set carries three known biases. The RERS parallel unknowns cluster in the largest models ("The degeneracy filter" in `docs/dev/search.md`), so the proper instances lean towards smaller state spaces. RERS 2018 problems 102 and 103 are out of reach of the explicit front end: 102 was still exploring at 46 GB when a 3,000 s timeout stopped it, and 103 needs more than the 128-bit state key holds. On 2018 problem 101, the explicit structure keeps deadlocking runs, as SPIN does, while the official answers count only infinite runs (`docs/dev/rers-kripke.md`).

## Configuration

The headline run uses the `search_bench` defaults. These are binary status with the initial-state filter on.

| Setting | Value |
|---|---|
| Budget on models | 30 generations of 40 |
| Budget on trace sets | 30 generations of 30, as in `docs/dev/trace-sets.md` |
| Seeds | 1–10 |
| Per back-end call | 120 s, 4 GB |
| Per run | 3,600 s, 8 GB `systemd` scope |

The arms in `bench/graded_run.py` are `default` and `mutation`. The CEGIW set uses `forced-default` and `forced-mutation`. These runs predate the weakening filter, which became `search_bench`'s default on 2026-11-03, so every arm here passes `--weakening ignore`. Against the filter's `weakening` arm, `default` finds a repair on as many model runs and on fewer trace runs (1,124 of 1,260 against 1,154), with no difference in strength ("Keeping the search on weakenings" in `docs/dev/search.md`).

## Commands

Preparation runs locally in `nix develop`. The directories and a release build are then copied to av2 and av3. Other users share those hosts, so check `uptime` and `free -g` before starting and keep within the job count. Spot 2.15.1 must come first on `PATH` there, since `/usr/local/bin` holds Spot 2.14.5.

The model runs split the seeds between the hosts, with `S` = `1-5` on av2 and `6-10` on av3. The trace runs ran on av2 alone, in one systemd scope. The CEGIW run and the 15×20 budget run ran locally. `F` stands for the filter of the first run, which drops the excluded instances.

```
nix develop -c bench/eval_prep.sh
F='^(?!beem-anderson\.(3-2|3-4|5-2|5-4|7-4)$|beem-lamport\.8-4$|mcc-MedleyA-PT-09-)'
python3 -I bench/graded_run.py bench/data/eval 8 --seeds S --arms default,mutation --out runs-eval --filter "$F"
systemd-run --user --scope -p MemoryMax=32G -p MemorySwapMax=0 -p OOMPolicy=continue \
  python3 -I bench/graded_run.py bench/data/eval-traces 16 --seeds 1-10 --arms default,mutation --budget 30x30 --out runs-eval-dfa --memory-gb 16
python3 bench/graded_run.py bench/data/cegiw 3 --seeds 1-10 --arms forced-default,forced-mutation --out runs-eval
```

The budget runs and the probe of the large BEEM instances use the same arms. The 15×20 run ran locally with 3 jobs. The 60×80 run split its seeds between the hosts (1–3 on av2, 4–5 on av3) and the 120×80 run ran on av2, both with 8 jobs. The probe ran seeds 1–3 locally with 2 jobs and seeds 4–10 on av3 with 4 and then 6 jobs.

```
python3 -I bench/graded_run.py bench/data/eval 3 --seeds 1-5 --arms default,mutation --filter "$F" \
    --budget 15x20 --out runs-b15x20
python3 -I bench/graded_run.py bench/data/eval 8 --seeds S --arms default,mutation --filter "$F" \
    --budget 60x80 --out runs-b60x80 --run-timeout 14400
python3 -I bench/graded_run.py bench/data/eval 8 --seeds 1-2 --arms default,mutation --filter "$F" \
    --budget 120x80 --out runs-b120x80 --run-timeout 14400
python3 -I bench/graded_run.py bench/data/eval J --seeds S --arms default,mutation \
    --filter '^beem-(anderson\.(3-2|3-4|5-2|5-4|7-4)|lamport\.8-4)$' --out runs-beem-probe \
    --memory-gb 10 --run-timeout 14400
```

The probe used `--memory-gb 16` on av3.

Four scripts do the analysis. `bench/graded_stats.py` pairs the arms. `bench/baselines_top.py --arm <arm>` gives the closed share against ⊤ in its `closed_rejection` column, and `bench/ga_hit_rates.py --arm <arm>` gives the hit rates. `bench/budget_curve.py` sets the budgets side by side. `bench/side_stats.py` rescores the repairs of the rejected-side ablation on one scale.

```
python3 bench/budget_curve.py bench/data/eval 15x20=runs-b15x20 30x40=runs-eval 60x80=runs-b60x80 120x80=runs-b120x80
```

## Results

The model runs use the search of commit `7fae687`, and the trace runs that of commit `1c71d8f`. The rows are in `bench/results/eval-{instances,runs,top-default,top-mutation,hit-rates-default,hit-rates-mutation,simplify,traces-instances,traces-runs,beem-probe,cegiw-forced-default,cegiw-forced-mutation}.csv` and `bench/results/eval-budget-{15x20,60x80,120x80}-{instances,runs}.csv`. In the paired tables, the medians are over instances and the p-values come from Wilcoxon signed-rank tests. "Higher/lower/equal" counts the instances on which default is higher than mutation, lower or equal.

### Spot version

The first attempt at the remote runs picked up Spot 2.14.5 from `/usr/local/bin` on av2 and av3. That version lacks `ltlfilt --satisfiable`, so the output gate failed on every run, and all 1,992 model runs reported no repairs. Those runs are set aside in `runs-eval-spot2145`, and the rerun used Spot 2.15.1. `bench/graded_run.py` now refuses to start under any other Spot version.

### Models

The model set ran at 30×40 on 100 instances × 10 seeds × 2 arms. 1,994 of the 2,000 runs gave a result. The 6 without one are on mcc-SieveSingleMsgMbox-PT-d2m06-LTLFireability-00, 1 default and 5 mutation, all at the 3,600 s cap. The *best semantic score* is the model-counting similarity of a run's best repair to φ.

| Measure | Mutation | Default | Paired difference | p | Higher/lower/equal |
|---|---|---|---|---|---|
| Found rate | 0.70 | 0.85 | mean +0.085 | <0.0001 | 36/7/57 |
| Best semantic score | 0.994 | 0.995 | | 0.0004 | 31/14/35 |
| Net subsumption | | | median 0.000 [0.000, 0.222], mean +0.188 | <0.0001 | 38/7/35 |
| Back-end calls | 55 | 65.75 | +6.0 | <0.0001 | 64/28/8 |
| Wall time | 9.0 s | 11.9 s | +0.7 s | 0.029 | |

The GA finds a repair more often than random mutation, and its repairs are stronger. It pays for this with 6.0 more back-end calls per run and 0.7 s more wall time. The gain is in RERS and MCC. On the 66 RERS instances the found rate is 0.60 against 0.50 (p = 0.0001, 27/6/33), and net subsumption favours the GA (p = 0.0008, 22/4/21). On the 19 MCC instances the found rate gives p = 0.042 (6/1/12), and net subsumption has a median of 0.361 (p = 0.0092, 15/2/1). The 14 BEEM instances are level on every measure: the found rate is 2/0/12 (p = 0.5), and net subsumption 0/1/13. On LMCS abp4 the found rate is 1.0 against 0.9.

**Against ⊤.** `bench/baselines_top.py --arm` gives a median closed share under the semantic score of 0.122 [0.029, 0.408] for default and 0.046 [0.016, 0.100] for mutation. Under the rejection strength the median closed share is 0.971 [0.955, 0.988] and 0.953 [0.923, 0.969]. No repair is valid, so every repair is strictly stronger than ⊤. Default's repairs still tie ⊤'s semantic score on 47 instances, and mutation's on 56. By family, default's closed share is 0.294 on RERS (mutation 0.067) and 0.399 on MCC (mutation 0.184). On BEEM it is 0.0007 for both arms, and 100 of the 111 BEEM runs that found a repair score exactly ⊤'s value, with a closed rejection of 0.31. On BEEM the semantic score therefore cannot separate the repairs from ⊤.

**Cache hit rates.** `bench/ga_hit_rates.py --arm` gives the share of queries that the cheap tiers answer. For default this is 92.1% of 963,457 queries over 999 runs. The memo answers 22.0%, the lasso tier 44.0% and the implication tier 26.1%, and 7.9% reach the back end. For mutation the cheap tiers answer 93.6% of 946,630 queries.

**Simplification.** `bench/simplify_stats.py` rewrites each distinct repair with `ltlfilt -r` and proves the rewrite equivalent in both directions. Over 2,008 distinct repairs the median size falls from 13 to 11 nodes, 889 repairs get shorter, and the total node count falls by 6.9%. By family it falls by 12.5% on MCC, 3.6% on RERS and 2.8% on BEEM, and it rises by 4.4% on LMCS. The search output keeps Spot's form only when it is smaller (the `simplified` field, commit `aa69274`).

### Trace sets

The trace sets ran at 30×30 on 126 instances × 10 seeds × 2 arms, and all 2,520 runs gave a result. They ran on av2 from 22:31 on 30 October to 01:47 on 31 October, 16 at a time at nice 19, in one systemd scope capped at 32 GB. A stub `systemd-run` on `PATH` sent `bench/graded_run.py` to its `prlimit` fallback, so every run stayed inside that scope. No run reached the cap.

A first trace run used the search of commit `7fae687`, whose oracle decided LTLf implications with `ltlfilt --from-ltlf`. Its output gate failed on 318 runs with 427 errors, against 111 runs with 124 errors on commit `1c71d8f`. All 124 later errors are time-outs, 123 of them in `ltlf2dfa`. The first run gave 2,516 results, since 4 runs of ltlf-learning RandomBooleanCombinationsofFactors (trace_length=16, seed=47) died at 16 GB, and its repair sets match the second run's on 2,239 of them. Analysed with `ltlf2dfa`, it gave a net subsumption median of 0.090 [0.000, 0.225] and a mean of +0.151 (p<0.0001, 65/23/30), with calls and wall time level (p = 0.87 and 0.91). Its runs are in `bench/data/eval-traces/runs-eval` and its analysis in `stats-eval`. A 16 GB rerun of it on av3 repeated all 1,254 seed 1–5 runs by mistake, and those repeats are kept in `runs-eval-av3-rerun`.

| Measure | Mutation | Default | Paired difference | p | Higher/lower/equal |
|---|---|---|---|---|---|
| Found rate | | | | 0.66 | 17/11/98 |
| Best semantic score | 0.989 | 0.989 | | <0.0001 | 57/6/55 |
| Net subsumption | | | median 0.077 [0.000, 0.233], mean +0.152 | <0.0001 | 64/24/30 |
| Back-end calls | 661 | 684 | | 0.81 | |
| Wall time | 1.815 s | 1.797 s | median +0.041 s, mean −16.6 s | 0.022 | 78/48/0 |

Both arms find a repair on nearly every run, so the found rate does not separate them. The GA's repairs are stronger at the same number of calls. Default takes a median of 41 ms longer per instance and 16.6 s less on the mean, which a few long instances decide. The gain is in ltlf-learning and scarlet, where on most 5-instance families default has the higher best semantic score on 4 or 5 of the 5. Flie is level. `bench/graded_stats.py` decides the LTLf implications with `ltlf2dfa`, from the minimal DFA of x & !y, and 433 pairs on 4 DoubleCounter and SingleCounter instances stay undecided and count as not implied. On the first run, `ltlfilt --from-ltlf` left 27,727 pairs undecided and gave a median of 0.126 and a mean of +0.162. On 2,000 pairs that route decided, `ltlf2dfa` agrees on 1,999 and times out on 1. The simplification measurement ran out of memory at 16 GB on the trace repairs and was dropped. The search computes the simplified form only under infinite-word semantics.

### CEGIW cases and the precedes set

These two sets ran on the search of commit `ec8a3f9`. On the nine CEGIW instances the GA finds a repair on 66 of 90 runs against 53 for random mutation, and on both foraging absence cases its single maximal repair is equivalent to CEGIW's interval (`bench/notes/case-studies.md`). On the RERS precedes set, on the 7 instances where the intended weakening ψ holds, the GA reached ψ on 32 of 70 runs against 19 of 70 for mutation, ahead on all 7 (sign test p = 0.016; `bench/notes/precedes-reference.md`).

### Budget

`bench/budget_curve.py` compares four budgets on the 198 instance and seed pairs, from seeds 1 and 2, that gave a result at every budget under both arms. Wall time and back-end calls are medians.

| Budget | Found, default | Found, mutation | Wall, default | Wall, mutation | Calls, default | Calls, mutation |
|---|---|---|---|---|---|---|
| 15×20 | 0.485 | 0.449 | 3.4 s | 3.0 s | 26 | 23 |
| 30×40 | 0.677 | 0.566 | 11.5 s | 8.3 s | 64 | 50 |
| 60×80 | 0.864 | 0.672 | 39.9 s | 32.1 s | 124 | 120 |
| 120×80 | 0.939 | 0.697 | 78.7 s | 59.0 s | 164 | 180 |

The gap in found rate widens from 3.6 points at 15×20 to 24.2 points at 120×80. Random mutation levels off near 0.70, while the GA keeps gaining. `bench/graded_stats.py` pairs the arms over all seeds of each budget.

- **15×20, 5 seeds.** Found rate 32/9/59 (p = 0.0004). Net subsumption gives p = 0.15, with a mean of 0.093.
- **60×80, 5 seeds.** Found rate 33/4/63 (p < 0.0001). Net subsumption has median 0.200 and mean 0.314 (p < 0.0001, 50/2/31). Default makes 7 more back-end calls (p = 0.0001).
- **120×80, 2 seeds.** 398 of 400 runs gave a result. Two Sieve mutation runs hit the 14,400 s cap. Two rers2018-101-p5 mutation runs died at about 183 s under the 8 GB cap, and a rerun on av3 at 16 GB finished each in 168 s. Found rate 33/1/65 (p < 0.0001). Net subsumption has median 0.325 and mean 0.313 (p < 0.0001, 43/7/26). Back-end calls give p = 0.18.

### Large BEEM instances

The probe ran the 6 BEEM instances that the headline excludes (anderson 3-2, 3-4, 5-2, 5-4 and 7-4, and lamport 8-4) at 30×40, with 4 h and 10–16 GB per run. All 120 runs completed (6 instances, 2 arms, seeds 1–10). No run hit the cap, and the longest took 6,609 s. Default found a repair on 51 of 60 runs, and mutation on 37 of 60. Over the 60 paired runs, default alone found a repair on 14 and mutation alone on none (sign test p = 0.0001). The gap is on anderson 3-2 and 5-2, where default found a repair on all 10 seeds and mutation on 3. Both arms found one on every seed of anderson 3-4, 5-4 and 7-4, and on 1 of 10 seeds of lamport 8-4. The median run took 1,089 s for default and 1,192 s for mutation, with 21 and 22 back-end calls. These instances fail under the headline's 3,600 s cap, and not in general.

## Diversity

Diversity is how far apart the repairs of a set lie in behaviour, and GAWAIN reports it as PEREDUR's paper does (`evaluation.tex` and `experiments/2026-10-06-repair-diversity` in the PEREDUR artifact). `bench/diversity_stats.py` computes it from the same runs and maximal sets as `bench/graded_stats.py`, with A the arm and B the baseline.

**Distance.** `src/bin/diversity.rs` samples 2^16 traces per instance over its atoms, each a lasso with a stem of 0 to 8 positions and a loop of 1 to 8, both uniform, and each atom true at each position by a fair coin. These are the settings of PEREDUR's `sample_words` in its separation run. The distance between two repairs is |a xor b| / |a or b| over their satisfying sets, and 0 when neither holds on any trace. The evaluator is the trace-set back end's packed one (`src/oracle/packed.rs`), so a formula is compiled once and nothing is allocated per trace. On the largest pool of the model set, 232 repairs of rers2016-104-p2, the binary takes 1.6 s for 15.2 million evaluations and the distance matrix on one core.

**Containment.** Each repair's ball reaches to its third-nearest neighbour in its own maximal set (K = 3), and a side's region is the union of its balls. The containment of B in A is the share of B's repairs inside A's region, which is the *improved recall* of Kynkäänniemi et al. The net containment share is the containment of B in A less that of A in B, and it is positive when A's repairs spread wider.

**Pooling and control.** These follow PEREDUR's `pooled_div2.py`. The seeds both arms ran are split at random into two halves S and S', 5 times. Each side's repairs over one half are pooled and reduced to a maximal set. Cross containment pairs A on S with B on S' and the reverse, so no cross pair shares a seed. The control of A is A on S against A on S', both ways, and likewise for B. All four sets are subsampled to the smallest of their sizes, capped at 200, twice per split, and every figure is the mean over splits and subsamples. A split counts only when that size exceeds K, and an instance enters when some split counts.

**Tests.** Over the instances that enter, the script tests the net share and each direction against its own side's control: B in A against A's control, and A in B against B's. Each test gives the Hodges–Lehmann shift with its exact 95% interval, the exact Wilcoxon signed-rank p, the Holm-adjusted p over the three tests, the rank-biserial r and the signs. `bench/graded_stats.py` now prints the same columns for every measure beside its bootstrap intervals, with Holm over the measures of one block. It also reports net subsumption under PEREDUR's two readings of an undecided pair, and whether its verdict holds under both.

**Deviations from PEREDUR.**

- The trace stream is SplitMix64 (`search::Rng`), not mt19937_64, so the samples are reproducible but differ from PEREDUR's.
- Under LTLf a sampled word is finite, with the stem and loop lengths drawn as above and summed, so the length runs from 1 to 16 positions. A lasso has the same number of distinct positions.
- The atoms are the instance's `atoms` column together with every atom of its repairs. An atom no repair mentions cannot change a distance, so the choice of interface does not matter.
- The 10 seeds split 5 and 5, and an odd count splits floor and ceiling. PEREDUR split 30 seeds 15 and 15.
- The split order is seeded by instance and arm through Python's `random`, so the draws differ from PEREDUR's numpy stream.
- PEREDUR's `hodges_lehmann` helper takes its bounds one Walsh average too far in. On 1, …, 10 it gives [3.5, 7.5] against the textbook [3.0, 8.0], and on (1, …, 8, −9, 10) it excludes zero at p = 0.064, which trips its own assertion. GAWAIN uses the textbook bound, which excludes zero exactly when the exact p is below 0.05 without ties. With 5 or fewer non-zero differences the interval is unbounded.
- Where ties break that duality, PEREDUR's helper raises an error. GAWAIN counts the mismatches and prints the count instead.
- PEREDUR decides a pair in one comparison, so under its `non` reading a pair with either direction undecided is incomparable. GAWAIN's current reading keeps a decided direction, so `net_share_non` can differ from `net_share` as well as from `net_share_imp`.
- The Hodges–Lehmann shift drops zero differences, as PEREDUR's does. Where most instances tie at zero it lies far from the median difference, so the found rate on the model set has a median difference of 0.000 and a shift of 0.200.

**Commands.** A release build of `diversity` goes to the host with the run directory. Copying the stats directory's `implications.json` into OUT first means Spot is asked only about pairs `bench/graded_stats.py` has not seen. Spot 2.15.1 must be first on `PATH`, and LTLf instances also need `ltlf2dfa`.

```
cargo build --release --bin diversity
cp bench/data/eval/stats-eval/implications.json OUT/
python3 -I bench/diversity_stats.py bench/data/eval OUT --runs runs-eval --baseline mutation --jobs 16
python3 -I bench/diversity_stats.py --self-check
```

**Validation.** On the 19 MCC instances of `runs-eval`, 8 enter. The net containment share has median 0.010, mean +0.104 and shift 0.077 [−0.060, 0.349] (p = 0.47, 4/3/1). B in A against A's control gives a shift of 0.020 (p = 0.38), and A in B against B's control −0.068 (p = 0.44). The mean containments are 0.963 for B in A and 0.859 for A in B, against controls of 0.942 and 0.934. The run took 5.7 s on 2 threads with the implication cache warm, and 0.1 to 1.0 s per instance. The other 11 stay out because on every split some half's maximal set holds 3 repairs or fewer. At 10 seeds the MCC set is too small to show a difference in diversity.

## Order of work

The random-mutation baseline lands first, since the headline run includes its `mutation` arm. The headline run follows, and the case studies come last. With one configuration fixed in advance, every number in the results traces back to one command and one set of seeds.
