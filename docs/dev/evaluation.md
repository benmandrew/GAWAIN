# Evaluation plan

GAWAIN weakens a *linear temporal logic* (LTL) property φ that a model M violates into a φ' that φ implies and M satisfies. Specification weakening has no ground truth, and ⊤ always holds, so a repair is ranked by its strength against baselines ("Evaluation without a ground truth" in `PLAN.md`). This note fixes the research questions, instance sets, configuration, metrics and commands, so that every headline result comes from one configuration.

## Research questions

Each question names its metric, its current status and where the existing evidence lives.

**RQ1, effectiveness.** How often does GAWAIN find a non-trivial weakening? The metric is the *found rate* per family under the final default configuration. The ablations ran with the initial-state filter off, so the headline run below is the first at the defaults.

**RQ2, strength.** How strong are the repairs? Three comparisons answer this. The first is the *closed share* against ⊤ under the rejection strength ("⊤ under the rejection strength" in `bench/notes/baselines.md`). At the ablation configuration its median was 0.9912 [0.9833, 0.9957] over 87 instances. The second is a comparison with CEGIW on its 8 case studies plus abp4 (`bench/notes/baselines.md`). The third compares φ' with the source formula ψ on trace sets (`vs_psi` in `bench/traces_run.py`).

**RQ3, cost of the genetic search.** Does the search earn its cost? The baseline is *random mutation* with the same operators, filters, scoring and number of candidate evaluations, and with no selection during the run. Its final P survivors are chosen by the same NSGA-II (Non-dominated Sorting Genetic Algorithm II) ranking and pass the same output gate ("Random-mutation baseline" in `docs/dev/search.md`, being written). The metrics are the found rate, the *net subsumption* between the two arms' pooled maximal repair sets, the closed share under rejection, back-end calls and wall time. Results are paired by instance and seed, with Wilcoxon signed-rank tests and bootstrap 95% intervals, as in "Graded status against binary" in `docs/dev/search.md`.

**RQ4, cost.** The metrics are wall time, back-end calls and cache hit rate per tier. On real genetic algorithm (GA) runs, the cheap tiers answer 91.6% of queries ("Hit rates on GA populations" in `docs/dev/oracle-measurements.md`).

**RQ5, design ablations.** These are already measured and stay in `docs/dev/search.md`, cited rather than rerun. They cover the status variants (binary, graded, graded-feasible and graded-reserve, with binary kept), the model-counting objective, the initial-state filter and the budget (4×8 against 30×40). Filtering at the initial state in the loop raised the found rate from 0.70 to 0.90.

**Qualitative.** A few case studies show readable repairs, among them abp4 `p2false` and the CEGIW foraging robots. A hand classification of repair shapes on a sample goes with them. Neither is collected yet.

## Instance sets

`bench/eval_prep.sh` builds three directories. It leaves `bench/data/graded` unchanged, because earlier results read it.

| Directory | Instances | Contents |
|---|---|---|
| `bench/data/eval` | 107 | From `bench/graded_prep.py`: RERS parallel 2016 (52), RERS 2018 problem 101 (14, new), MCC (20), BEEM ERROR pairs (20), LMCS abp4 `p2false` (1) |
| `bench/data/eval-traces` | 126 | The trace-set instances of "Search on a sample" in `docs/dev/trace-sets.md`, 5 per family, seed 1 |
| `bench/data/cegiw` | 9 | CEGIW's 8 case studies, 3 of them degenerate, plus abp4 `p2false`, run with the `forced-` arms |

RERS is the Rigorous Examination of Reactive Systems challenge, and MCC is the Model Checking Contest. BEEM is the BEnchmarks for Explicit Model checkers set, and LMCS names the 2006 benchmarks of a paper in *Logical Methods in Computer Science*. In `bench/data/eval`, the 93 rows outside RERS 2018 are identical to `bench/data/graded`.

**Exclusions.** Seven model instances are left out, as in the ablations. Six are large BEEM instances that gave no result there: anderson 3-2, 3-4, 5-2, 5-4 and 7-4, and lamport 8-4. The seventh is MCC MedleyA-PT-09, which failed on every seed. That leaves 100 model instances.

**Limitations.** The set carries three known biases. The RERS parallel unknowns cluster in the largest models ("The degeneracy filter" in `docs/dev/search.md`), so the proper instances lean towards smaller state spaces. RERS 2018 problems 102 and 103 are out of reach of the explicit front end: 102 exceeded 85 GB, and 103 needs about 161 bits of state key. On 2018 problem 101, the explicit structure keeps deadlocking runs, as SPIN does, while the official answers count only infinite runs (`docs/dev/rers-kripke.md`).

## Configuration

The headline run uses the `search_bench` defaults. These are binary status with the initial-state filter on.

| Setting | Value |
|---|---|
| Budget on models | 30 generations of 40 |
| Budget on trace sets | 30 generations of 30, as in `docs/dev/trace-sets.md` |
| Seeds | 1–10 |
| Per back-end call | 120 s, 4 GB |
| Per run | 3,600 s, 8 GB `systemd` scope |

The arms in `bench/graded_run.py` are `default` and `mutation`. The CEGIW set uses `forced-default` and `forced-mutation`.

## Commands

Preparation runs locally in `nix develop`. The directories and a release build are then copied to av2 and av3. Other users share those hosts, so check `uptime` and `free -g` before starting and keep within the job count.

```
nix develop -c bench/eval_prep.sh
bench/graded_remote.sh bench/data/eval 26 eval --seeds 1-10 --arms default,mutation --out runs-eval \
    --filter '^(?!beem-anderson\.(3-2|3-4|5-2|5-4|7-4)$|beem-lamport\.8-4$|mcc-MedleyA-PT-09-)'
bench/graded_remote.sh bench/data/eval-traces 8 eval --seeds 1-10 --arms default,mutation --budget 30x30 --out runs-eval
bench/graded_run.py bench/data/cegiw 2 --seeds 1-10 --arms forced-default,forced-mutation --out runs-eval
```

Three scripts do the analysis. `bench/graded_stats.py` pairs the arms. `bench/baselines_top.py` gives the closed share against ⊤ in its `closed_rejection` column. `bench/ga_hit_rates.py` gives the hit rates.

## Order of work

The random-mutation baseline lands first, since the headline run includes its `mutation` arm. The headline run follows, and the case studies come last. With one configuration fixed in advance, every number in the results traces back to one command and one set of seeds.
