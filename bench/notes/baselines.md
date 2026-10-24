# Baselines: ⊤ and CEGIW

GAWAIN has no ground-truth weakening to compare against, so `PLAN.md` names two baselines. The first is ⊤, the trivial weakening, which every φ implies and every model satisfies. The second is *counterexample-guided interval weakening* (CEGIW; `~/projects/cegiw`, [arXiv 2604.24540](https://arxiv.org/html/2604.24540)), which weakens one interval bound of a metric temporal logic (MTL) formula and keeps the rest of the formula fixed. This note gives the method and the results of both. The scripts are `bench/baselines_top.py`, `bench/cegiw_run.py` and `bench/baselines_cegiw.py`, and the results are in `bench/results/baselines-*.csv`.

## ⊤ as the floor

The ⊤ baseline reuses the binary arm of the status ablation ("Graded status against binary" in `docs/dev/search.md`): 87 instances (1 LMCS-2006, 52 RERS 2016, 15 BEEM, 19 MCC) at 30 generations of 40, 838 runs over 10 seeds, of which 801 found a repair. The run rows are `bench/results/graded-runs.csv`. The repair formulas are in the JSON lines of each run, which were copied from `~/aragon-graded-satisfaction/bench/data/graded/runs/*-binary-*.json` on av2 and av3 into `bench/data/baselines/top-runs/` (gitignored). The two hosts held 501 files in common, and each JSON's wall time matches its row in the CSV. MedleyA-PT-09 has no binary run with output, so it is absent.

Two questions apply to each instance. Spot first asks whether each distinct repair is valid, that is equivalent to ⊤ (one `ltlfilt --satisfiable` batch over `true & !φ'`). Then the new `score` binary (`src/bin/score.rs`) scores ⊤ against φ with the same lasso count that the search uses, and the run's best semantic similarity is placed between the two ends. The *closed share* of a run is (best − top) / (1 − top), where top is ⊤'s score: 0 at the floor and 1 at φ itself. The command is the following.

```
nix develop -c python3 bench/baselines_top.py bench/data/baselines/top-runs \
    bench/results/graded-runs.csv bench/results/baselines-top.csv \
    --csv-4x8 bench/results/graded-runs-4x8.csv
```

No repair is equivalent to ⊤. Spot proved all 1,371 distinct repairs strictly stronger than ⊤, as the gate's validity filter requires. The score is a coarser test. 347 of the 1,371 repairs (on 48 of 87 instances) score no higher than ⊤ to six places, and 131 of 801 runs have a best repair that ties with ⊤. The lasso count is monotone under implication, so a strictly stronger formula can tie but never fall below.

| Family | Instances | ⊤ score | Best score | Closed share, 30 × 40 | Closed share, 4 × 8 |
|---|---|---|---|---|---|
| All | 87 | 0.9983 [0.9641, 0.9985] | 0.9992 [0.9940, 0.9992] | 0.49 [0.42, 0.58] | 0.23 [0.02, 0.42] |
| RERS | 52 | 0.9853 | 0.9971 | 0.59 [0.20, 0.80] | 0.43 |
| MCC | 19 | 0.9819 | 0.9939 | 0.49 [0.16, 0.60] | 0.03 |
| BEEM | 15 | 0.9985 | 0.9992 | 0.49 [0.25, 0.49] | 0.00 |
| abp4 `p2false` | 1 | 0.9598 | 0.9821 | 0.55 | 0.24 |

Each cell is the median over instances of the median over seeds, with a bootstrap 95% interval over instances (10,000 resamples, `graded_stats.bootstrap_median`). The 4 × 8 column takes the best scores from `bench/results/graded-runs-4x8.csv`, where 649 of 1,640 binary runs found a repair. The interquartile range of the closed share at 30 × 40 is 0.11 to 0.80 over 80 instances; 35 close more than half the gap and 19 close less than a tenth.

The raw scores sit close to 1 because the similarity is a ratio of logarithms of lasso counts. ⊤ already scores 0.998 against a typical φ, so the raw gap between ⊤ and the best repair is a few thousandths and says little on its own. The closed share normalises that gap. On 7 instances ⊤ scores 1.000000 against φ, for example MCC `F a0 & X X !X G !X a1` and RERS 2016/101 p13 `p_c0_t8 R (p_c0_t8 | !p_c0_t9 | F p_c0_t3)`. φ there misses too few lassos of length 20 for the count to see, so the closed share is undefined and every repair ties with ⊤.

The low end is safety. On RERS `G !p_c0_t2` (2016/102 p19 and 103 p2) ⊤ scores 0.3018, the best repair 0.3244, and the closed share is 0.03. The repairs there are `!p_c0_t2`, `X X !p_c0_t2` and `G F !p_c0_t2`. A violated invariant leaves no near neighbour that holds, and the score reports that honestly. The three BEEM anderson instances (`G(p_wait0 -> F p_cs0)`) have a median closed share of 0: every best repair there is a response variant whose score ties with ⊤.

## CEGIW: the common subset

CEGIW reads an MTL formula, a de Bruijn index naming one temporal operator, and a nuXmv or SPIN model. It widens the interval of an `F` or `U`, or narrows that of a `G` or `R`, until the model satisfies the formula. An unbounded `F` or `U` cannot be weakened (`weaken.py` raises "Cannot weaken interval of F[a, ∞)"), and an unbounded `G` or `R` can only be narrowed to a finite prefix. So an instance is common ground only when φ has a bounded interval, which in LTL means an `X` chain.

The benchmark sets of this repository give almost no such instance.

- **RERS.** None of the 52 RERS formulas of the ablation uses `X`. They are Dwyer patterns over `G`, `F`, `U` and `R`, all unbounded.
- **BEEM.** The 20 pairs are `G F p` and `G(p -> F q)`, with no `X`.
- **MCC.** 16 of the 20 formulas contain `X` chains, such as `G(X X X X a0 U !(a1 & F a0))`. Chains such as `X X a2` read as `F[2,2] a2`, but the models are Petri nets in the Petri Net Markup Language (PNML), which CEGIW cannot read.
- **LMCS-2006.** abp4 `p2false` is `G(s_get -> X(!s_get U ψ))`, and `X θ` is `F[1,1] θ` exactly. This is the one common instance. The other surviving LMCS failures use past-time operators, which GAWAIN's formula core lacks.

The comparison therefore uses abp4 `p2false` and CEGIW's own eight case studies (`case-studies.sh`): two formulas, a response `G(p -> F[1,b] p)` and an absence `G((p & X !p) -> G[1,15] !p)`, on four nuXmv models (foraging robots and a two-state model, each with and without a time limit on searching). The cases are in `bench/sets/cegiw-cases.tsv`. Narrowing an unbounded `G` of a RERS or BEEM formula to a prefix was not run. It would make every repair a statement about the first few steps, and CEGIW's SPIN trace reader has not been tried on the RERS models.

## CEGIW: method

CEGIW ran from a copy of `~/projects/cegiw` at commit `0b74b81`, inside its own flake (nuXmv 2.1.0), three repetitions per case, one run at a time. GAWAIN starts from the same property: `bench/cegiw_run.py` translates each MTL formula with CEGIW's own `mtl_to_ltl` and writes the LTL into `bench/data/cegiw/instances.tsv`. GAWAIN ran the binary arm at 30 × 40 over 10 seeds, also one run at a time (`bench/graded_run.py bench/data/cegiw 1 --seeds 1-10 --arms binary`). Both ran on the local machine (20 cores) between 17:44 and 18:12 on 2026-10-24, at a load of 1.2 to 4.2 from other jobs.

Three of the nine cases are degenerate: M ⊨ ¬φ, so the binary arm refuses them. These are the response formula on both foraging models and the absence formula on the limited two-state model. For them `search_bench` gained `--degenerate search`, and `graded_run.py` an arm `forced` that passes it, so GAWAIN searches anyway. The trap of `PLAN.md` applies there: any truth χ of M gives a weakening `φ | χ`. CEGIW escapes it because the shape of the formula is fixed.

`bench/baselines_cegiw.py` then checks CEGIW's output on the model with a full nuXmv `check_ltlspec`, since CEGIW stops when bounded model checking (BMC) finds no counterexample within its bound. It asks Spot whether the output is valid, plainly and modulo GAWAIN's encoding invariant I, and scores it against φ. Finally it places the output against each member of GAWAIN's maximal set, pooled over seeds, by Spot implication modulo I. Only abp4 has a non-trivial I, `G(!(s_get & s_wait) & !(r_ack0 & r_ack1))`.

## CEGIW: results

| Case | Class | CEGIW | CEGIW wall | GAWAIN found | GAWAIN wall | ⊤ | CEGIW score | GAWAIN best | CEGIW against GAWAIN's maximal set |
|---|---|---|---|---|---|---|---|---|---|
| forage-response | degenerate | no weakening | 0.68 s | 7/10 | 5.28 s | 0.9531 | — | 0.9676 | — |
| forage-absence | proper | `G[1,3]` | 1.34 s | 9/10 | 5.60 s | 0.6217 | 0.7488 | 0.7488 | 1 equivalent, 4 incomparable |
| forage-limit-response | degenerate | `F[1,20]` | 4.57 s | 10/10 | 6.51 s | 0.9531 | 0.9546 | 0.9546 | stronger than 1, incomparable with 1 |
| forage-limit-absence | proper | `G[1,3]` | 1.39 s | 9/10 | 9.33 s | 0.6217 | 0.7488 | 0.7488 | 1 equivalent, 2 incomparable |
| minimal-response | proper | no weakening | 0.50 s | 9/10 | 3.85 s | 0.8737 | — | 0.8936 | — |
| minimal-absence | proper | `G[1,1]`, valid | 0.87 s | 6/10 | 7.06 s | 0.6217 | 0.6217 | 0.6279 | weaker than all 28 |
| minimal-limit-response | proper | `F[1,11]` | 1.21 s | 10/10 | 4.53 s | 0.8737 | 0.8753 | 0.8752 | stronger than 1, incomparable with 3 |
| minimal-limit-absence | degenerate | `G[1,1]`, valid | 0.88 s | 6/10 | 8.83 s | 0.6217 | 0.6217 | 0.6384 | weaker than all 11 |
| abp4 `p2false` | proper | `F[1,5]`, fails on M | 12.84 s | 10/10 | 23.07 s | 0.9598 | 0.9672 | 0.9821 | stronger than 1, incomparable with 13 |

Wall times are medians: over three repetitions for CEGIW, whose outcomes never varied, and over ten seeds for GAWAIN. The rows are in `bench/results/baselines-cegiw.csv` and `bench/results/baselines-cegiw-runs.csv`.

CEGIW is faster on every case, by 1.4× to 10×. It makes a handful of nuXmv BMC calls, where GAWAIN at this budget makes a median of 16.5 to 29 back-end calls on the case studies and 200.5 on abp4.

CEGIW's output fails on abp4. `G(s_get -> F[1,5](!s_get U ψ))` passed BMC at bound 20, but the full nuXmv check rejects it with a lasso of 48 states. Every bounded interval fails: `F[1,1]`, `F[1,10]`, `F[1,20]` and `F[1,40]` have counterexamples of 46, 53, 63 and 83 states, and only the unbounded `G(s_get -> X F(!s_get U ψ))` holds. CEGIW's bound is the larger of 20 and 1.5 times the current upper bound, so on this model it never searches deep enough to find these counterexamples. On the eight case studies, where the models are small, the full check confirms every CEGIW output. GAWAIN's gate reports a repair only after the oracle's full check of M ⊨ φ' and Spot's proof of φ ⇒ φ'.

CEGIW returns a valid formula on both two-state absence cases. `G((a_p & X !a_p) -> G[1,1] !a_p)` is a tautology, since the antecedent already says `X !a_p`, and it scores exactly ⊤'s 0.6217. GAWAIN's gate drops valid candidates, and Spot proves every one of its maximal repairs there strictly stronger than CEGIW's.

Where CEGIW proves that no interval works, GAWAIN still finds repairs. On the two response cases where CEGIW finds none, GAWAIN finds a repair on 7 and 9 of 10 seeds. On the two-state model one of them is `G(a_p -> X(!a_p | G(a_p | X a_p)))`. Many such repairs depend on the initial state, such as `… | resting_p` on the foraging models, which start in `resting`. The encoding invariant does not capture initial states, so these repairs pass the triviality filter.

Where both tools find a non-trivial repair, they agree or differ in shape. On both foraging absence cases GAWAIN found CEGIW's `G[1,3]` repair exactly (`G(resting_p & X !resting_p -> X X(!resting_p & X !resting_p))`), with the same score of 0.7488, and also repairs incomparable with it. On the limited response cases CEGIW's interval is strictly stronger than one GAWAIN maximal repair and incomparable with the rest, and the best scores match to three places (0.9546 against 0.9546, 0.8753 against 0.8752). CEGIW searches one dimension exactly. GAWAIN searches all shapes and lands on the same score where the interval was the right dimension to move.
