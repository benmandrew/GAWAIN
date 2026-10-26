# The weakening search

`src/search/` breeds candidate weakenings φ' of one LTL property φ that the model M violates, scores them with the tiered oracle, and selects under NSGA-II. An output gate then keeps only the candidates proved to be weakenings that hold on M and that no other survivor beats. The `weaken` binary (`src/main.rs`) runs it over the nuXmv back end. This file records the loop, the objectives, the gate, and which parts of PEREDUR (`~/projects/peredur`) were left out and why.

## The degeneracy filter

An instance (M, φ) is worth weakening only when M ⊭ φ and M ⊭ ¬φ. If M ⊨ φ, there is nothing to weaken. If M ⊨ ¬φ, φ fails on every run of M, so on M the weakening ¬φ ∨ χ is equivalent to χ, and every repair that holds is some truth of M to which φ contributes nothing (see "Evaluation without a ground truth" in `PLAN.md`). `instance::classify` asks the search's own oracle about φ and then ¬φ, and returns `Proper`, `Satisfied` or `Degenerate`. Both verdicts are memoised, and each refuting lasso goes into the lasso tier, so the search starts with them. ¬φ is not asked when M ⊨ φ.

The `weaken` binary runs `classify` before the search. It prints one line to stderr and exits with status 1 for `Satisfied` and 3 for `Degenerate`, without searching; status 2 stays the usage error. A check with no verdict (a timeout or a tool error) does not drop the instance. The binary prints a warning naming the formula and the reason, and searches anyway, because an `Unknown` proves neither case and a filter that dropped it would make the benchmark set depend on machine load. Earlier versions exited with status 2 when φ's own verdict was unknown. A proper instance prints `instance: proper time=…`, the wall time of the two checks. The bench scripts `bench/rers_filter.py` and `bench/lmcs_check.py` run the same two checks in Python, with SPIN and nuXmv directly; a Rust benchmark loader should call `classify`.

The added cost was measured on abp4 `p2false` (seed 1, 30 × 40), against the commit before the filter, on the local machine (20 cores, release build, five repetitions with the arm order alternated, other jobs at a load of 7 to 12, `bench/degeneracy_ab.sh`, rows in `bench/results/degeneracy-ab.csv`). The table gives medians.

| Arm | Checks before the search | Wall | Back-end calls | Lasso hits | Repairs |
|---|---|---|---|---|---|
| φ only (before) | — | 37.46 s | 164 | 654 | 2 |
| φ and ¬φ | 0.17 s | 39.03 s | 165 | 654 | 2 |

The two checks took 0.15 to 0.17 s, and ¬φ costs one nuXmv call. The repairs and every other counter were identical on all ten runs. The φ lasso already seeded the lasso tier before this change, since the binary checked φ first. The new seed is the lasso that refutes ¬φ, a run of M on which φ holds, and it refuted no candidate that the other lassos did not, so the lasso hits stayed at 654. The difference in median wall time, 1.6 s, is inside the spread of either arm (37.1 to 41.5 s before, 35.0 to 39.8 s after), so the filter has no measurable effect on the total.

## The loop

An individual is one formula. The seed population is P copies of φ, collapsed by the survivor step to one distinct member replicated P times. Each of G generations then runs four steps.

1. **Breed.** Each of the P slots starts as its parent, is crossed with a parent drawn from the whole population at rate 0.1, is mutated at rate 1.0, and is passed through `simplify`. Crossover and mutation interleave per slot, as in PEREDUR's `breed_offspring`, so the draw stream depends on that order.
2. **Filter.** Repeats up to `canonical()` are dropped, then anything larger than twice φ's size (PEREDUR's bloat cap), then anything `simplify` folds to `true`, then anything Spot proves valid modulo the encoding invariant (see below). A dropped candidate costs no oracle call.
3. **Score.** Each survivor is checked by the oracle, in population order.
4. **Select.** Parents and offspring are pooled, as in a (μ+λ) step, and PEREDUR's `nsga2-apportion` survivor step applies. Repeats up to `canonical()` are dropped, the rest are sorted by the crowded-comparison order (front rank, then crowding distance, stable), and the result is truncated to P or replicated up to P. Replication gives each survivor one slot and shares the spare slots by weight `1 / (1 + rank)` under the largest-remainder method.

Mutation draws atoms from φ's atoms only. The model's other atoms, listed in `--atoms`, are out of scope for now, though `--atoms` must still cover φ's, since a lasso reads an unlisted atom as false.

The random source is SplitMix64 with Lemire's bounded reduction (`src/search/rng.rs`). PEREDUR's rule holds: each draw goes into its own local, never two draws as arguments of one call, and a probability of 0 or 1 costs no draw. The stream is this crate's own and does not match PEREDUR's mt19937 stream.

## Operators

Mutation (`src/search/operators.rs`) is `tlsf_mutate` with the slot and section draws removed. With probability 0.25 it is a monotone rewrite, in a direction drawn on a fair coin. Otherwise, with probability 0.2, it is Brizzio et al.'s temporal mutation, which may add, drop or swap temporal operators, with PEREDUR's arm that keeps a binary node's connective. The remaining mutations rewrite only the maximal propositional subtrees, through PEREDUR's `mutate_formula`, so the temporal skeleton is kept exactly.

The monotone rewrite draws a site uniformly over the nodes of determinate *polarity*, and then one rule that moves that node the asked way. A weaker subformula weakens the whole only at a positive occurrence, so `!` and the antecedent of `->` flip the direction, and nothing under `<->` is a site. `tests/search.rs` checks 1,200 rewrites (600 each way, random formulas of depth 4 over three atoms) against Spot in one batch, and every one moves in its stated direction.

One rule differs from PEREDUR. Its `GloballyToInfinitelyOften` builds `G` of the whole node, which gives `G G φ`, a formula equivalent to its input. This port builds the documented `G F φ`.

Crossover is one side of `tlsf_crossover`. On a fair coin, a subformula of the first parent is replaced by one of the second, or joined with one under `&`, `|`, `U` or `W`. The graft sites are the distinct temporal subformulae, or the whole formula when there are none, and only the first occurrence of the chosen site gives way.

## Objectives

`score` takes one generation's survivors and returns three objectives for each, all higher-is-better.

- **Status.** 1 if the oracle answers `Holds`, 0 if it answers `Fails` or `Unknown`. The oracle never memoises `Unknown`, so a candidate that timed out is asked again if it is bred again. A graded alternative was measured and not adopted; see "Graded status" below.
- **Syntactic similarity** to φ (`Formula::syntactic_similarity`).
- **Model-counting similarity** to φ (`count::Counter`, next section), the measure of strength that PLAN.md ranks repairs by. A candidate whose counts fail scores 0, the worst value.

The NSGA-II functions take objective vectors of any width, so nothing else changed. No trait was introduced for the objectives, because each has one implementation. The `weaken` binary prints both similarities on each repair line, as `repair<TAB>syntactic<TAB>semantic<TAB>vacuity<TAB>formula`.

### Graded status

`Status::Graded` (`src/search/grade.rs`) replaces the 0 of a candidate that is not proved with a grade. Let n be the number of lassos the oracle has stored that violate φ, and k the number of those that φ' satisfies. A candidate the oracle proves scores 1, and any other scores k / (n + 1). φ satisfies none of its own counterexamples, so it scores 0, and for a fixed set of lassos the grade is monotone under implication. It costs no back-end call, since the lassos are runs of M that tier 1 already keeps, and it works on every back end that returns counterexamples. On a trace set the grading set is the fixed set of positive traces instead, measured in "Graded status on trace sets" in `trace-sets.md`.

Three choices fix the grade.

- **The +1.** A `Fails` or `Unknown` candidate can satisfy all n stored counterexamples, because M's run that refutes it may not be stored yet. The denominator n + 1 keeps every such candidate below 1, so a proved candidate always beats one that is merely consistent with the stored runs. With no stored lasso the grade is 0, the binary value.
- **No deduplication.** Tier 1 stores a back-end counterexample only when no stored lasso already refutes the query, so a second copy of a stored lasso can never be added. Lassos that differ but agree on every candidate still count separately, and the grade does not try to merge them.
- **Rescoring.** The set only grows, so a grade computed in generation 3 is out of date in generation 10. Before each selection `Grader::rescore` sorts the new lassos into those that violate φ and regrades every member of the pool that does not hold, parents included, against one set.

The ablation under "Measured runs" found that the grade crowds out the candidates that hold. Under the binary status φ itself (status 0, both similarities 1) dominates every candidate that fails, so the front holds φ plus the candidates that hold. Under the grade a failing candidate that satisfies some counterexample has status above 0, so φ no longer dominates it, and it takes a place on the front. The final population held a median of 3.25 fewer proved candidates out of 40, while the found-repair rate and the strength of the repairs did not change. The default is therefore `Status::Binary`, and `search_bench --status graded` keeps the grade for the ablation. The `weaken` binary has no flag for it.

Two variants were also measured on RERS and dropped. Their code is at commit `ddd9648` on the unmerged branch `exp/graded-arms` (`src/oracle/explicit.rs` there).

- **B with random runs.** Before the search, 64 random walks of M (seeded from the run's seed, at most 10,000 steps each) were stored in the oracle, so the grade had more counterexamples to work with from generation 0. Few walks violate φ (a median of 7 refuting lassos per run against 3), and the crowding was the same.
- **Option C, the share of states.** The status was h / (N + 1), where h is the number of M's N states from which φ' holds, from a product of M with ¬φ' and a backward pass over its bad strongly connected components. The share does not depend on φ, so it rewards formulas that are not weakenings of φ at all. On RERS 2016 103 p7, seed 1, the gate kept none of the 21 candidates that held. Each new formula costs one Spot translation and one product, which took a median of 70% of the search's wall time.

#### Proved candidates first

`Status::GradedFeasible` keeps the grade and ranks every proved candidate above every other, after Deb's constrained domination. A candidate is feasible when it holds. In `nsga2::constrained_dominates` a feasible candidate dominates every infeasible one, two feasible candidates compare by plain dominance as before, and two infeasible ones compare by plain dominance on the full vector, grade included. `select` passes the feasibility of each candidate to the sort, and the other arms pass every candidate as feasible, which is plain NSGA-II. An infeasible candidate is dominated by every feasible one, so it joins a front only after every feasible candidate has one. Every front is therefore all feasible or all infeasible, and the crowding distance and the apportioning by front need no change: the feasible fronts take the low ranks and the most copies, and the grade spreads the infeasible fronts. The unit tests `feasible_fronts_come_first_and_never_mix` (in `nsga2.rs`) and `feasible_first_selection_keeps_holders_over_graded_failures` (in `mod.rs`) check the ordering, the crowding and the apportioning on a mixed pool.

The three arms ran on one commit (`ad408ff`, this change on `main` at `9f4aaf4`) with the harness, instances, budget and statistics of "Graded status against binary": 30 × 40, 10 seeds, `bench/graded_run.py --arms binary,graded,feasible`. RERS and BEEM ran on av2 and abp4 and MCC on av3, at 14 concurrent runs per host, with other agents' jobs also on av2. The six large BEEM instances that gave no result before (anderson 3-2, 3-4, 5-2, 5-4 and 7-4, and lamport 8-4) were left out from the start, so 86 instances pair as before. 34 binary, 34 graded and 32 feasible runs gave no result, at the 8 GB cap or the 3600 s limit; MedleyA-PT-09 failed on every seed again. The table gives the paired difference from binary over the 86 instances, with its bootstrap 95% interval, the Wilcoxon p-value, and the instances where the arm is higher, lower or tied.

| Measure | Graded − binary | p | +/−/0 | Feasible − binary | p | +/−/0 |
|---|---|---|---|---|---|---|
| Found-repair rate | 0.00 [0.00, 0.00] | 0.89 | 10 / 11 / 65 | 0.00 [0.00, 0.00] | 0.0008 | 4 / 18 / 64 |
| Best semantic similarity | 0.000 [0.000, 0.000] | 0.57 | 22 / 20 / 44 | 0.000 [0.000, 0.000] | 0.063 | 16 / 32 / 38 |
| Repairs | 0 [0, 0] | 0.30 | 15 / 23 / 48 | 0 [0, 0] | 0.28 | 16 / 26 / 44 |
| Candidates that hold, of 40 | −3.25 [−5.5, −2.25] | < 0.0001 | 7 / 74 / 5 | +16.5 [16.0, 17.5] | < 0.0001 | 86 / 0 / 0 |
| Wall time | −0.01 s [−0.17, 0.11] | 0.62 | 41 / 45 / 0 | +0.54 s [0.19, 0.91] | 0.083 | 58 / 28 / 0 |
| Back-end calls | +0.75 [0.0, 1.5] | 0.52 | 49 / 33 / 4 | +2.0 [0.75, 4.25] | 0.009 | 54 / 28 / 4 |
| Net subsumption share | 0.000 [−0.038, 0.000] | 0.56 | 25 / 34 / 27 | 0.000 [−0.167, 0.000] | 0.005 | 19 / 42 / 25 |

The graded arm repeats the earlier ablation: 3.25 fewer proved candidates and nothing else changed. Ranking proved candidates first more than recovers them. In every one of the 838 feasible runs all 40 members of the final population hold, against a median of 23.5 under binary. That has a cost. φ never holds, so once 40 candidates hold it is no longer in the population, and neither is any failing candidate a later weakening could be bred from. The found-repair rate dropped on 18 instances and rose on 4 (p = 0.0008), 14 of the 18 on RERS, where the rate fell from 1.0 to 0.4 on 2016 104 p12 and 104 p15 (5 paired seeds each) and to 0.6 on 105 p7 (10 seeds). 74 feasible runs ended without a repair although all 40 candidates held, so none of them was a proved, non-trivial weakening of φ. The repairs were also weaker. Feasible's maximal set was stronger than binary's on 19 instances and weaker on 42 (p = 0.005), and BEEM alone gave −0.25 [−0.25, −0.05] (p = 0.016, 1 stronger and 10 weaker). Against graded directly the picture is the same: the found-repair rate fell on 20 instances and rose on 5 (p = 0.0005), and the net subsumption share favoured graded on 40 instances against 21 (p = 0.005). Feasible made 2 more back-end calls per run (p = 0.009), and its grading took a median of 0.05% of the search's wall time.

At 4 × 8 with 20 seeds (83 instances, everything but MCC locally at two concurrent runs, MCC on av3 at two), feasible held 1.0 more proved candidates than binary [0.5, 2.0] (p < 0.0001, 59 higher and 10 lower). The population is too young there to fill with holders. The found-repair rate (0.40 against 0.35, p = 0.30), the repairs (p = 0.30), the back-end calls (p = 0.97) and the net subsumption share (p = 0.26) did not differ, and the best semantic similarity was lower on 19 instances and higher on 7 (p = 0.019).

SPIN rechecked every RERS repair of the three arms (`bench/graded_crosscheck.py --runs runs-3arm`). Of 1714 distinct repairs, 1083 hold, 443 use `X` and were skipped, 188 left SPIN's search incomplete, and none is violated; the 678 repairs that feasible reported include 393 that hold and none violated. The results are in `bench/results/graded-feasible-runs.csv`, `graded-feasible-instances.csv` (against binary), `graded-feasible-instances-vs-graded.csv`, the `-4x8` files of each and `graded-feasible-crosscheck.csv`.

Ranking proved candidates first answers the question it was asked, and the answer is a population of holders that has lost φ and its neighbours. The default stays `Status::Binary`, and `search_bench --status graded-feasible` keeps the arm for the record.

#### A reserve for unproved candidates

`Status::GradedReserve(d)` keeps graded-feasible's order and keeps ⌈population / d⌉ slots for candidates that do not hold. When `select` cuts the pool, it takes the best unproved candidates for the reserve, as far as there are any, and the best holders for the other slots. A kind with too few candidates leaves its slots to the other. Fronts never mix the two kinds, so each kind's part of the constrained crowded order is that kind's own NSGA-II order, with graded dominance among the unproved. The cut therefore needs no second sort: it takes a prefix of each part of the one order. A pool small enough to keep whole is replicated as before. φ has both similarities at 1, so no other unproved candidate dominates it, and a reserve of one slot or more keeps it in the population. The unit tests `a_reserve_keeps_failures_beside_the_holders` and `a_short_kind_leaves_its_slots_to_the_other` in `mod.rs` check the cut.

Four arms ran on one commit (`3cfd3cc`, this change on `main` at `ec7e22b`): binary, feasible, and the reserve at d = 4 (`reserve4`, 10 slots of 40) and d = 2 (`reserve2`, 20 slots). The harness, budget, statistics and 86 instances are those of "Proved candidates first", with 10 seeds, all on av3 at 26 concurrent runs and no other load. The branch was then rebased onto `f006d38`, whose lasso-count speed-up leaves every count the same, so the selection is the same. 33 binary, 30 feasible, 39 `reserve4` and 53 `reserve2` runs gave no result. MedleyA-PT-09 failed on every seed again. Most of `reserve2`'s extra failures hit the 3600 s limit on BEEM lamport 6-4 and peterson 6-4, 6 seeds each. The table gives the paired difference from binary over the 86 instances, as before.

| Measure | Feasible − binary | p | +/−/0 | Reserve 1/4 − binary | p | +/−/0 | Reserve 1/2 − binary | p | +/−/0 |
|---|---|---|---|---|---|---|---|---|---|
| Found-repair rate | 0.00 [0.00, 0.00] | 0.001 | 5 / 17 / 64 | 0.00 [0.00, 0.00] | 0.31 | 8 / 11 / 67 | 0.00 [0.00, 0.00] | 0.92 | 10 / 11 / 65 |
| Repairs | 0 [0, 0] | 0.34 | 16 / 25 / 45 | 0 [0, 0] | 0.41 | 24 / 14 / 48 | 0 [0, 0] | 0.056 | 12 / 21 / 53 |
| Candidates that hold, of 40 | +16.5 [16.0, 17.5] | < 0.0001 | 86 / 0 / 0 | +6.5 [5.75, 7.5] | < 0.0001 | 84 / 2 / 0 | −3.5 [−4.0, −2.5] | < 0.0001 | 3 / 79 / 4 |
| Wall time | +0.91 s [0.20, 1.22] | 0.078 | 60 / 26 / 0 | +0.97 s [0.51, 1.46] | < 0.0001 | 62 / 24 / 0 | +0.55 s [0.38, 1.00] | < 0.0001 | 63 / 23 / 0 |
| Back-end calls | +2.0 [0.75, 4.75] | 0.009 | 54 / 28 / 4 | +3.5 [2.25, 5.5] | < 0.0001 | 61 / 23 / 2 | +4.5 [2.25, 6.25] | < 0.0001 | 65 / 17 / 4 |
| Net subsumption share | 0.000 [−0.158, 0.000] | 0.012 | 20 / 41 / 25 | 0.000 [0.000, 0.000] | 0.57 | 31 / 30 / 25 | 0.000 [0.000, 0.000] | 0.42 | 23 / 29 / 34 |

Feasible repeats its earlier result on this host: 40 holders, a lower found-repair rate and weaker repairs. A reserve of a quarter keeps 6.5 of its 16.5 extra holders. Every `reserve4` run ended with exactly 30 holders, so the reserve was always full and the holders always filled the rest. It also gives back what feasible lost. Over all 86 instances the found-repair rate, the number of repairs and the net subsumption share do not differ from binary. 40 of 831 runs ended without a repair, against 38 of 837 under binary and 73 of 840 under feasible. Against feasible, `reserve4` found a repair more often on 17 instances and less often on 2 (p = 0.0004), and reported more repairs (p = 0.016). The families do not all agree. On RERS the found-repair rate was lower on 10 instances and higher on 2 (p = 0.028); the largest falls were 2016 106 p16 (0.67 to 0.33 over 3 paired seeds) and 104 p12 (1.0 to 0.75 over 4). On BEEM it was higher on 4 instances and lower on none (p = 0.089), but binary's maximal set was stronger on 9 instances and weaker on 2 (p = 0.035). The cost is 3.5 more back-end calls per run and 0.97 s more wall time (both p < 0.0001). A reserve of a half leaves fewer holders than binary (20, −3.5), so it does not answer the question.

SPIN rechecked every RERS repair of the four arms (`bench/graded_crosscheck.py --runs runs-reserve`). Of 2213 distinct repairs, 1357 hold, 620 use `X` and were skipped, 236 left SPIN's search incomplete, and none is violated. The 730 repairs that `reserve4` reported include 455 that hold and none violated. The results are in `bench/results/graded-reserve-runs.csv`, `graded-reserve-instances.csv` (against binary), `graded-reserve-instances-vs-feasible.csv` and `graded-reserve-crosscheck.csv`.

A quarter reserve keeps a population of 30 proved candidates without losing the found-repair rate or the strength of the repairs overall. The repairs it reports are no better than binary's, though, and it pays 5% more back-end calls. The proved candidates matter only as parents and as gate input, and on this budget the gate already finds the same repairs from binary's 23.5. The default therefore stays `Status::Binary`, and `search_bench --status graded-reserve-D` keeps the arm.

## Model-counting similarity

`src/count/` follows PEREDUR's semantic similarity (`semantic_similarity.cpp`, `model_counter.cpp`, `exhaustive_count.cpp` and `runner/ganak.cpp`), at its defaults, with one change of definition. PEREDUR counts accepting paths in a transfer matrix (`transfer_matrix.cpp`), and that count is not monotone under implication. The count here is a count of lassos, which is monotone and sees liveness (see "Monotonicity" below).

- **Count.** Under LTL, the number of pairs (u, v) of finite words over the valuations of the *atom universe*, with |u| + |v| = 20 (PEREDUR's `default_model_counting_bound`) and |v| ≥ 1, such that the infinite word u v^ω is in the formula's language. `ltl2tgba -D -H` builds a generalised Büchi automaton, trimmed to its *live* states, the states that reach a cycle visiting every acceptance set. A subset construction runs over u, one subset per prefix. The loops v run through the automaton's transition monoid: the relation of v links state p to state q when some run on v leads from p to q, and it records, per acceptance set, whether some such run visits it. v^ω is accepted from p when p reaches, in the graph of that relation, a strongly connected component that has an internal edge for every acceptance set. An infinite run may take each edge of that component by a different run of v on each pass, so the union over runs is exact. For each split |u| = 20 − |v|, the count is the sum, over each prefix subset and each loop relation, of their weights when some state of the subset accepts v^ω.
- **Pairs, not words.** Different pairs can give the same infinite word: `(a, a)` and `(ε, aa)` both give `a^ω`. The count takes pairs, which is simpler and still monotone, since a language that accepts more words accepts at least the same pairs. A word with many lasso shapes, such as a constant word, weighs more than a word with one, and this weight is the same for every formula that accepts the word.
- **Universe.** The atoms of φ and φ' together, so the three counts below share it. Mutation draws only φ's atoms, so in practice the universe is φ's atoms.
- **Similarity.** PEREDUR's default `Logarithmic` metric over the counts of φ, φ' and `φ & φ'`: the harmonic mean of `log shared / log count(φ)` and `log shared / log count(φ')`, each clamped to [0, 1]. Two empty languages score 1, one empty against one non-empty 0, and a count of at most 1 falls back to membership. A candidate equal to φ up to `canonical()` scores 1 with no count, as an unchanged requirement pair does in PEREDUR.
- **Letters.** The valuations are split once per automaton into classes by the set of live-edge labels they satisfy, so the valuations of one class move the automaton the same way, and each class is one letter weighted by its number of valuations. When the labels mention at most 16 atoms together (`k_exhaustive_count_max_atoms`), each label becomes one 64-bit truth table over those atoms, and a class is a set of rows. Its count is multiplied by 2 for each universe atom the labels do not mention. Wider labels make each class a conjunction. A conjunction, or under LTLf a single label, on at most 20 atoms is counted by truth table (`LABEL_TABLE_ATOMS`), and a wider one goes to Ganak 2.5.3 as a Tseitin CNF with full bi-implications, so each auxiliary variable is determined and the CNF keeps the label's count. PEREDUR sends everything past 16 atoms to Ganak, but one Ganak exec costs about 70 ms against about a millisecond for a 20-atom table ("Slow implication batches" in `trace-sets.md`). `tests/count.rs` drives Ganak with a 21-atom φ and checks the result against a closed-form count.

Under LTLf (`--traces` on an LTLf set) a lasso count measures the wrong semantics: it cannot tell `X[!] true` from `true`, and it gives weak `X false` no words at all. The count there is the number of finite words of each length from 1 to 20 that satisfy the formula under LTLf. `ltlf2dfa` builds a deterministic automaton with transition-based acceptance, which accepts a word when its last transition is accepting, and the words are counted by the state they reach, each edge weighted by the models of its own label. A larger LTLf language accepts every word a smaller one does, so the count is monotone under LTLf implication. `tests/count.rs` checks seven closed forms over one atom, and checks the 1,196 of the 1,200 implied pairs above that Spot also proves under LTLf: 0 violations, no count above ⊤, and 784 pairs strictly ordered. The choice between the two counts is measured in "Model counts under LTLf" in `trace-sets.md`.

Two other details differ from PEREDUR. Counts are kept as base-2 logarithms in `f64`, and the weights are rescaled to a maximum of 1 after each step, because Rust has no x87 `long double`. This removes PEREDUR's clamp of the bound to `max_representable_step_count`, which no longer has a limit to enforce. PEREDUR also raises the bound to the *timing horizon* of a FRETISH requirement. Plain LTL input has no timings, so that rule has nothing to act on here.

### Monotonicity

PEREDUR's count is the number of weighted paths of exactly 20 edges from the initial state to a state carrying every acceptance set. That is a property of the automaton, not of the language, in two ways. Spot's `-D` only prefers a deterministic automaton, so in a nondeterministic one a word with two accepting-state paths counts twice. And a finite path that ends in an accepting state is not the same as a prefix of the language: it can end in a state with no accepting future, and a prefix whose path is between visits is missed. On the 1,200 implied pairs ψ ⇒ χ that `tests/search.rs` builds by monotone rewrites (depth-4 formulas over three atoms, each implication proved by Spot), PEREDUR's count gave count(ψ) > count(χ) on 26 pairs (2.2%), and 69 of the 2,400 counts exceeded the count of ⊤. On abp4, the repair `G F(… -> X(!s_get W (…)))` scored 0.9697 against 0.9623 for `G(s_get -> X F(s_wait & (…)))`, which implies it.

The lasso count is monotone by definition. If L(ψ) ⊆ L(χ), every pair (u, v) with u v^ω in L(ψ) has u v^ω in L(χ). ⊤ gets the maximum, 20 · 2^(20n) pairs over n atoms, and equivalent formulas have equal languages and so similarity 1. `tests/count.rs` checks all three on the same 1,200 pairs: 0 violations, no count above ⊤, and 796 pairs strictly ordered. It also checks five equivalent pairs, such as `a U b` against `b | a & X (a U b)`, for similarity 1, and checks that on `ack.smv`'s φ `G(req -> X ack)` the order 1 > `G(req -> F ack)` > `G F req -> G F ack` > ⊤ holds strictly.

Two other monotone counts were built and measured, and both were rejected.

- **Prefixes.** The number of words of length 20 that are prefixes of some word of the language, by a subset construction over the live states. It is monotone (0 violations, no count above ⊤), but it orders only 679 of the 1,200 pairs strictly, and a finite prefix cannot see liveness: every finite word extends to one that satisfies `F p`, so it measures only a formula's safety closure. `G(req -> F ack)` scored 0.8906, the score of ⊤, and on abp4 every repair of every arm but the two `G(s_get & r_ack0 -> …)` ones scored 0.9599, again the score of ⊤. It cost 0.34 s in process on abp4.
- **Sampled lassos.** A fixed seeded sample of N lassos over the universe, with |u| uniform in [0, 20) and |v| = 20 − |u|, each formula evaluated in process by `Lasso::satisfies`, and the similarity taken over the sample fractions. It is monotone by construction (0 violations), but uniform lassos almost never satisfy a safety specification. With N = 10,000, φ on `ack.smv` held on 56 lassos and abp4's φ on 39, so its similarity rested on a few dozen samples: `G(req -> F ack)` scored 0.6100 against 0.6083 for ⊤. On the 1,200 pairs it ordered 669 strictly, and in 384 of them ψ had no hit at all. It is also no cheaper: 2,400 formulas took 28.7 s in process at N = 10,000 (2.7 s at N = 1,000, with 668 pairs strictly ordered and 386 empty), against 0.59 s for the exact lasso count of the same formulas. These figures come from a one-off test on the local machine, not kept in the tree.

### Cost and caching

Each new candidate costs two automata, one for φ' and one for `φ & φ'`. φ's own count is computed once per universe. All three caches key on `canonical()` and never rename atoms. The similarity is relative to the fixed φ, and φ's atoms name model variables. The caches hold the log count per (formula, universe), the similarity per candidate, and the Ganak count per label. Within one count, each row and each monoid element is interned once, with its successor under each letter, and each element with its accepting states (see "Interned rows"). A failed translation is cached as a failure, as PEREDUR caches an `ltl2tgba` timeout.

Every formula of one generation goes to `ltl2tgba` (`ltlf2dfa` under LTLf) on one stdin, one formula per line, and the HOA documents come back in order. If the batch fails, each formula is retried in its own exec, so one formula that blows up costs only itself. `score` also runs the counts on a second thread while the oracle checks the same candidates, since the two are independent. The counter reports `time` (wall time in the counter), `tool_time` (the part spent in `ltl2tgba` and Ganak) and `wait` (the part the search waited for after the oracle had finished).

Both optimisations were measured with PEREDUR's path count, on av3 (Spot 2.15.1 built from source, release build, five interleaved repetitions with other jobs at a load of 5 to 8, `bench/model_counting_batching.sh`). The slow arm, built by `bench/model_counting_seq.py`, runs one exec per formula and counts after the oracle. Both arms report identical repairs. The table gives medians, and every run is in `bench/results/model-counting-ab.csv`.

| Run | Arm | `ltl2tgba` execs | Count time | Wait | Wall |
|---|---|---|---|---|---|
| `ack.smv`, seed 4 | one exec per formula, sequential | 1005 | 14.35 s | 14.35 s | 15.75 s |
| `ack.smv`, seed 4 | batched, beside the oracle | 30 | 0.70 s | 0.10 s | 1.40 s |
| abp4 `p2false`, seed 1 | one exec per formula, sequential | 1981 | 27.10 s | 27.10 s | 40.63 s |
| abp4 `p2false`, seed 1 | batched, beside the oracle | 30 | 2.67 s | 0.00 s | 10.65 s |

Batching cut the count time by 10× to 20×. Per exec, `ltl2tgba` start-up costs 7 to 17 ms on av3, against 0.7 to 1.3 ms per formula inside a batch. The thread then hides nearly all of what is left: on abp4 the oracle is always the slower side, and on `ack.smv` the search waited 0.10 s of 1.40 s. In process, the truth tables and the path count took 0.02 s to 0.14 s per run, 3% to 4% of the counter's time.

The prefix count was then measured against the path count on av3 (release build, five repetitions with the order of the arms rotated, other jobs at a load of 12 to 15, `bench/model_counting_ab.sh`, rows `monotone` in the CSV). The first version built each class as a conjunction of labels and truth-tabled it from scratch, and on abp4 it spent 11.2 s of a 14.6 s count in process. Keeping one truth table per label and each class as a set of rows brought that to 0.34 s. The table gives medians for the final version.

| Run | Count | Count time | In process | `ltl2tgba` | Wait |
|---|---|---|---|---|---|
| `ack.smv`, seed 4 | paths | 0.98 s | 0.01 s | 0.96 s | 0.19 s |
| `ack.smv`, seed 4 | prefixes | 1.00 s | 0.04 s | 0.96 s | 0.05 s |
| abp4 `p2false`, seed 1 | paths | 4.29 s | 0.13 s | 4.16 s | 0.00 s |
| abp4 `p2false`, seed 1 | prefixes | 3.76 s | 0.34 s | 3.42 s | 0.02 s |

The subset construction costs 2.5× to 4× the path count in process, which is under 10% of the counter's time. Dropping `-S` made the automata smaller, so `ltl2tgba` itself got faster, and on abp4 the whole count got 12% cheaper. The searches differ after the first generation, so these runs translate different formulas (1073 and 2025 for the prefix count, against 1005 and 1981).

The lasso count costs more. Its monoid grows with the automaton: over the 2,015 formulas of one abp4 run, the median automaton had 12 distinct loop relations, the 99th percentile 6,554 and the largest, with 23 states and 20 letters, 117,303. Profiling on the local machine (abp4, seed 1, 30 × 40, release build, timers around each phase) put the time in the step from a relation to its successors. The first version spent 96 s of the count in process. Iterating set bits by `trailing_zeros` instead of testing all 64 brought the count to 72 s, and caching each relation's successors in a flat table, with the loop weights in a vector indexed by relation and a multiplicative hash for interning, brought it to 29.4 s. The four counts were then compared on av3 (release build, five repetitions with the order of the arms rotated, other jobs at a load of about 20 of 32 cores, `bench/model_counting_ab.sh`, rows `lasso-ab` in the CSV). The table gives medians.

| Run | Count | Count time | In process | `ltl2tgba` | Wait | Wall |
|---|---|---|---|---|---|---|
| `ack.smv`, seed 4 | paths | 1.04 s | 0.02 s | 1.02 s | 0.14 s | 2.47 s |
| `ack.smv`, seed 4 | prefixes | 1.05 s | 0.04 s | 1.01 s | 0.02 s | 2.85 s |
| `ack.smv`, seed 4 | lassos | 1.03 s | 0.06 s | 0.97 s | 0.11 s | 2.21 s |
| abp4 `p2false`, seed 1 | paths | 5.02 s | 0.14 s | 4.88 s | 0.00 s | 19.43 s |
| abp4 `p2false`, seed 1 | prefixes | 4.50 s | 0.39 s | 4.11 s | 0.05 s | 17.37 s |
| abp4 `p2false`, seed 1 | lassos | 36.74 s | 32.99 s | 3.75 s | 26.77 s | 43.32 s |

As first built, on abp4 the lasso count took 8.2× the prefix count's time and the search waited 26.8 s for it, so the run took 2.5× the wall time; "Interned rows" below cuts that wait to 2.3 s. On `ack.smv` it costs nothing measurable. The cost stays under the 10× limit set for keeping the exact count over the sampled one, and the sample is slower at a useful size (see "Monotonicity"). Unoptimised, the count is about 20× slower again, and the abp4 test in `tests/search.rs` took 570 s, so `Cargo.toml` builds the dev profile at `opt-level = 2`. That test then took 36 s.

### Interned rows

Callgrind on the release build (abp4, seed 1, 15 generations, collection limited to the count; `perf` is blocked on both hosts) put 70% of the instructions in the step from a relation to its successors, mostly in the bit loops over the rows, and 5% in interning. The 50 slowest automata of a 30-generation run took 98% of the time in process. The slowest, with 46 states, 28 letters and 5 planes, had 47,544 relations and took 9.4 s. Its rows were sparse, at 2.4 targets per row in plane 0 and 9.3 in the other planes together.

Row p of v·a depends only on row p of v and on a. The monoid therefore interns rows on their own, a relation is its list of row ids, and the step of each (row, letter) pair is computed once and kept in a flat table. A step of a relation is then one table lookup per state and an intern of a list of 32-bit ids, in place of a pass over every bit of every plane and an intern of the whole bit matrix. Each relation's closure for its accepting states is Warshall's algorithm on bit rows. On the 15-generation profile, instructions in the count fell from 166.7 × 10⁹, with the row layout of the last item below, to 27.8 × 10⁹. The count stays exact: every test gives the same counts, and the runs below give the same repairs, scores and call counts.

The comparison ran on av3 (release build, every run pinned to cores 0 to 7 by `taskset`, five repetitions with the order of the arms alternated, other jobs at a load of 17 to 20 of 32 cores, `bench/lasso_count_opt.sh`, rows `opt-final` in `bench/results/model-counting-ab.csv`). The table gives medians.

| Run | Count | Count time | In process | Wait | Wall |
|---|---|---|---|---|---|
| `ack.smv`, seed 4 | whole relations | 1.03 s | 0.07 s | 0.11 s | 2.32 s |
| `ack.smv`, seed 4 | interned rows | 1.02 s | 0.06 s | 0.12 s | 2.19 s |
| abp4 `p2false`, seed 1 | whole relations | 36.59 s | 33.24 s | 27.54 s | 42.39 s |
| abp4 `p2false`, seed 1 | interned rows | 9.56 s | 6.06 s | 2.26 s | 17.06 s |

On abp4 the wait fell from 27.5 s to 2.3 s and the wall time from 42.4 s to 17.1 s. The count in process is 5.5× cheaper, and `ltl2tgba` is now a third of the count. `ack.smv` is unchanged, since its count was already almost all `ltl2tgba`.

Five other ideas were measured and left out.

- **Threads by candidate.** A pool that counted the automata of one generation on every core, each thread taking the next automaton as it finished one. On the same av3 setup (rows `opt-arms`) it cut the in-process time on abp4 from 7.07 s to 5.71 s and the wait from 2.46 s to 1.81 s, but the wall time stayed at 18.7 s, because the slowest automaton of a generation still runs on one thread and the oracle is the longer side. It is not worth a thread pool and eight busy cores.
- **Sharing counts between automata.** Of the 2,021 automata of one abp4 run, 47 were exact repeats of an earlier one, and their counts took 0.002 s in all, so a cache keyed on the automaton has nothing to save. The counts of φ and of each `φ & φ'` are already cached on `canonical()` across generations (76 hits of 1,086 queries).
- **State-based Büchi automata.** `ltl2tgba -B -D` gives one acceptance set, so two planes, but more states. The local abp4 run went past its 8 GB limit and was killed.
- **A cheaper hash.** Reading the hashed bytes 8 at a time, in place of copying each chunk into a buffer, removed the `memcpy` calls from the profile and changed no measured time. It stays, since it is no longer than the copy.
- **Laying out each relation's planes by row.** Before rows were interned, keeping the planes of one row next to each other and iterating set bits directly cut the local abp4 count from 33.6 s to 23.1 s. Interned rows replaced that step, so only the layout of a row survives.

The remaining ideas were not built. The v-DP is already computed once per automaton, independent of u. Symmetry between atoms would merge letters, but the letters are already classes of valuations that move the automaton the same way.

### Bounded monoid

The transition monoid of an automaton can grow exponentially with its states, and until this change nothing bounded it. MedleyA-PT-09 `LTLCardinality-09` ran out of memory on every seed of the graded ablation ("Graded status against binary"), at 8 GB and again at 32 GB. One seed was rerun on av2 under `bench/mcc_memory.sh`, which samples once a second the scope's memory and the peak resident set (`VmHWM`) of every process in it. At a 16 GB cap the scope was killed after 702 s, and the process holding the memory was `search_bench` itself, at 16,120 MB. Its children stayed small: `verifypn` peaked at 1,150 MB under its 4 GB address-space limit, the `-p` confirmation of a TRUE answer at 276 MB, and `ltl2tgba` at 21 MB. The search's own memory sat between 35 and 77 MB for its first 300 s, then grew in steps of 1 to 4 GB while the oracle waited on `verifypn`, which is when the counting thread runs.

The diagnostic patch `bench/notes/count-monoid-log.patch` prints each count's automaton size and, after each loop length, the monoid's relations and the time. With it, one automaton of the same run, with 85 states and 16 letters, had 304,654 relations at loop length 5, 1,409,192 at 6 and 4,567,696 at 7, after 36 s, and was killed at a 4 GB cap on its way to length 20. The subset construction over the prefixes stays small: no count in any run below held more than 2,805 subsets.

Each `Monoid` now keeps an estimate of the bytes it holds: per relation 4 bytes per state for its row ids, 8 per letter for its successors and 160 for the hash map, the lists and the loop weights, and per row its bit planes, 4 bytes per letter and the same 160. When the estimate passes `MONOID_BYTES`, 1 GiB, the count fails and its candidate scores 0, the rule already applied to a translation that times out (`Counter::similarities`; `search_bench` reports the failures as `count_failures`). The estimate tracks the real memory: on AutoFlight-PT-02a seed 2 the largest monoid, 4,468,330 relations over 64 states and 8 letters, was estimated at 2,050 MiB, and the search's peak resident set was 2,162 MB. The bound is on bookkeeping, so a seeded run makes the same search under any load, which a wall-clock limit would not. 1 GiB leaves room under the ablation's 8 GB scope for `verifypn`, which may reach its 4 GB limit, and for the allocator, which does not return every freed count to the system.

The pattern is not specific to MedleyA. Every MCC instance of the ablation ran with seeds 1 to 3 on av2, before and after the change, both built with the diagnostic patch, at the ablation's budget (30 × 40, 120 s and 4 GB per back-end call, 3600 s per run) in an 8 GB scope each (`bench/mcc_memory.sh`, 2 or 3 runs at a time; other users' jobs kept the load average between 7 and 29 on 32 cores). `bench/mcc_memory_stats.py` gives the table below, and the rows are in `bench/results/mcc-memory.csv`. Before the change, five instances built a monoid estimated at more than 1 GiB: AutoFlight, CircadianClock, FireWire, MedleyA-PT-09 and Railroad. Here FireWire seed 3, Railroad seed 2 and two seeds of MedleyA-PT-09 ran out of memory, and CircadianClock seed 1 reached the 3600 s cap at 5.9 GB, inside one count over an automaton of 2,144 states. In the table, the search peak is the largest `VmHWM` of the search process over the three seeds, the largest monoid counts relations, and a count is cut when the bound stopped it.

| Instance | Before: runs | Before: search peak (MB) | Before: largest monoid | After: runs | After: search peak (MB) | After: counts cut |
|---|---|---|---|---|---|---|
| AirplaneLD-PT-0010-LTLFireability-04 | 3 ok | 63 | 147,536 | 3 ok | 62 | 0 |
| AutoFlight-PT-02a-LTLFireability-02 | 3 ok | 2,162 | 4,468,330 | 3 ok | 1,652 | 2 |
| BridgeAndVehicles-PT-V10P10N10-LTLFireability-13 | 3 ok | 24 | 34,132 | 3 ok | 22 | 0 |
| CO4-PT-05-LTLFireability-04 | 3 ok | 12 | 19,720 | 3 ok | 12 | 0 |
| CircadianClock-PT-000010-LTLFireability-02 | 2 ok, 1 timeout | 5,915 | 705,502 | 3 ok | 1,033 | 1 |
| CloudDeployment-PT-4a-LTLCardinality-15 | 3 ok | 15 | 30,942 | 3 ok | 15 | 0 |
| Diffusion2D-PT-D05N010-LTLCardinality-01 | 3 ok | 439 | 1,000,649 | 3 ok | 438 | 0 |
| FireWire-PT-01-LTLCardinality-07 | 2 ok, 1 OOM | 8,109 | 6,995,889 | 3 ok | 931 | 1 |
| MedleyA-PT-03-LTLFireability-08 | 3 ok | 10 | 6,685 | 3 ok | 10 | 0 |
| MedleyA-PT-09-LTLCardinality-09 | 2 OOM, 1 timeout | 7,493 | 12,138,401 | 3 timeout | 1,119 | 10 |
| ParamProductionCell-PT-1-LTLCardinality-13 | 3 ok | 303 | 609,614 | 3 ok | 302 | 0 |
| Peterson-PT-2-LTLFireability-09 | 3 ok | 7 | 3,133 | 3 ok | 7 | 0 |
| Peterson-PT-4-LTLCardinality-01 | 3 ok | 206 | 12,120 | 3 ok | 209 | 0 |
| Railroad-PT-010-LTLCardinality-01 | 2 ok, 1 OOM | 8,161 | 14,661,684 | 3 ok | 1,232 | 5 |
| ResAllocation-PT-R003C005-LTLCardinality-13 | 3 ok | 10 | 877 | 3 ok | 10 | 0 |
| ResAllocation-PT-R010C002-LTLCardinality-01 | 3 ok | 12 | 2,480 | 3 ok | 12 | 0 |
| ShieldPPPt-PT-001B-LTLCardinality-04 | 3 ok | 75 | 39,746 | 3 ok | 74 | 0 |
| SieveSingleMsgMbox-PT-d2m06-LTLFireability-00 | 3 ok | 64 | 11,505 | 3 ok | 64 | 0 |
| StigmergyElection-PT-02a-LTLFireability-12 | 3 ok | 651 | 835,660 | 3 ok | 650 | 0 |
| TCPcondis-PT-05-LTLFireability-14 | 3 ok | 18 | 1,147 | 3 ok | 18 | 0 |

After the change no run ran out of memory, and the search's peak resident set fell from at most 8,161 MB to at most 1,652 MB. That maximum is above 1 GiB because a sequence of counts, each under the bound, can leave the heap larger than any one of them. No child came near the scope's cap in either arm: `verifypn` peaked at 1,689 MB, its `-p` confirmation at 1,261 MB and `ltl2tgba` at 133 MB, and no Ganak call ran. The bound changes the search only through a count it cuts. Of the 54 runs that finished in both arms, 53 gave the same repairs with the same similarities, and Railroad seed 3, where one count was cut, found a stronger best repair (similarity 0.9876 against 0.9861). Wall times moved per instance by between −42 s and +75 s in the median, with no consistent sign, under that shared load, so the runs make no claim about time.

MedleyA-PT-09 now keeps its search under 1,119 MB, and its scope under 1,475 MB, but at this budget it still gives no result within 3600 s, on any seed. A separate run of seed 1 that logged every `verifypn` call spent 3,508 s in 269 calls, 3,362 s of it in 28 calls that reached the 120 s limit. The instance has 2.3 × 10⁸ reachable markings, and `bench/graded_prep.py` chose it because φ and ¬φ each take under 1 s; its mutants do not. That cost is the back end's, and the 120 s limit already reports it as unknown.

The bound limits memory, not time. A count that stays under 1 GiB can still be slow, and on AutoFlight seed 2 the search waited 507 s of its 538 s on the counter. The slowest case was CircadianClock's 2,144-state automaton: each relation's accepting states came from Warshall's closure, which costs n³/64 word operations per relation, so the count took 361 s to reach loop length 10. "Lasso count time" below removes that cost and adds a bound on work.

### Lasso count time

The lasso count spent most of its time in two places, and "Bounded monoid" left both in place. Each relation's accepting states came from Warshall's closure, at n³/64 word operations per relation, and each step of a relation was one table lookup and one intern per state. Four changes cut the count's time in process by 3.0× over the replayed corpus below, and every count is the same f64 as before.

**Replays.** A search's counts depend on the candidates it meets, so timing whole searches mixes the count with the oracle and with the load. `bench/notes/count-dump.patch` makes `Counter::similarities` write each call's candidates to a file (with `GAWAIN_COUNT_DUMP` set), and adds a timer per phase and a line per automaton (with `GAWAIN_COUNT_PROFILE` set). The `count_bench` binary replays one such dump on a fresh counter, call by call, and prints every log2 count exactly, so two builds can be compared with `cmp`. `bench/count_replay.sh` runs several builds over many dumps, rotating the order of the builds, and `bench/count_replay_stats.py` compares the counts and totals the times. The corpus is the search of seed 1 of every instance of `bench/data/graded/instances.tsv` in the binary arm, at the ablation's budget, plus AutoFlight-PT-02a seed 2, the run that waited 507 s: 93 dumps, from 20 MCC, 52 RERS, 20 BEEM and 1 abp4 searches, with a median of 1,789 formulas each. MedleyA-PT-09 is missing, since its search had not reached its 3600 s cap when the replays ran.

**Where the time went.** The profile of the first 85 dumps, on the build before these changes, put AutoFlight seed 2 at 470.6 s in process, of which 269.1 s was in the closures and 140.3 s in the steps of relations. CircadianClock seed 1 spent 413.7 s in process, and 332 s of it in two counts over its 2,144-state automaton, which the monoid bound cut after the closures of every relation met so far. The prefix subsets, the letters and the dot products took under 3 s in every dump.

**Closure by components.** A relation's accepting states are those that reach a *strongly connected component* (SCC) with a cycle and an edge inside it on every acceptance plane. Tarjan's algorithm finds the components in time linear in the states and edges, and it closes each component after every component it reaches, so one pass also marks the states that reach an accepting one. `Components` keeps its buffers between relations and copies plane 0 of each row into one contiguous array before the search. A unit test compares it with the old closure on 600 random graphs of 1 to 150 states and 1 to 3 planes.

**Blocks of rows.** Rows p to p + 7 of v·a depend only on the same rows of v and on a, so 8 consecutive row ids are interned as a *block*, with a table of block steps, and a relation is its list of block ids. A step of a relation is then one lookup per 8 states. On the AutoFlight profile the steps took 4.11 s at 4 rows a block, 4.48 s at 8 and 5.03 s at 16, and on CircadianClock 1.52 s at 8, 1.60 s at 16 and 2.02 s at 32 (local machine, one run each); 8 balances the two.

**Flat interning.** Blocks and relations are stored end to end in one `Vec<u32>` each and interned by open addressing, so a list costs no allocation and no `Rc` of its own. The successor table is one flat `Vec<u32>`.

**Two passes.** The loop weights of each length need only the monoid, so a first pass steps every length to build it and check the bound, and a second pass computes the closures. A count that the monoid bound cuts now fails before any closure. CircadianClock's failed 2,144-state count fell from 11.6 s, with the closure by components, to 0.29 s.

Each change was measured on the local machine (20 cores, load 3 to 4, release build, one run each), on a dump of three calls from AutoFlight seed 2 whose automata have 54 to 71 states and up to 843,720 relations. The closure by components brought the count to 14.6 s, blocks to 13.3 s, the contiguous plane 0 to 11.7 s and flat interning to 10.2 s. Callgrind on the same dump counted 176 × 10⁹ instructions before blocks and 142 × 10⁹ after. The build before all four took 25.3 s and peaked at 431 MB resident (`/usr/bin/time`), and the final build took 11.3 s and peaked at 199 MB.

The full corpus ran on av2 (release builds, the before and after builds of each dump run at the same time, 6 jobs, other users' jobs keeping the load average between 12 and 21 on 32 cores, `bench/count_replay.sh r3 6 "before final"`). Count time includes `ltl2tgba`, and in process excludes it. Every row is in `bench/results/count-time-corpus.csv`, whose `work` arm is the final build with the work bound below, run alone at 12 jobs.

| Dump | Before: count time | Before: in process | After: count time | After: in process |
|---|---|---|---|---|
| AutoFlight-PT-02a seed 2 | 490.1 s | 470.6 s | 197.1 s | 177.0 s |
| CircadianClock-PT-000010 seed 1 | 428.3 s | 413.7 s | 29.6 s | 15.5 s |
| Railroad-PT-010 seed 1 | 328.9 s | 258.7 s | 158.2 s | 90.4 s |
| AutoFlight-PT-02a seed 1 | 264.9 s | 252.8 s | 124.9 s | 112.0 s |
| RERS 2016 106 p16 seed 1 | 111.0 s | 101.7 s | 53.9 s | 44.3 s |
| All 85 dumps | 2,370.8 s | 2,067.6 s | 988.9 s | 684.8 s |

Over the 20 dumps whose count took more than 5 s in process, the speed-up ranged from 1.4× to 26.7×, with a median of 2.3×. The 60 dumps under 5 s took 108.9 s before and 106.0 s after, nearly all of it `ltl2tgba`. The counts of 84 dumps were identical. In Railroad seed 1, one formula's translation timed out at the replay's 2 s limit in one arm and not in the other; at a 30 s limit the two builds printed identical counts for all 1,925 formulas. The last 8 dumps, from 7 slow BEEM searches and SieveSingleMsgMbox, ran later against the final build with the work bound, at a load of 8 to 15: their counts were identical, at 6.2 s of count time in each build, nearly all of it `ltl2tgba`.

**Full searches.** The two slowest searches ran again on av2 at the ablation's budget (`search_bench`, 30 × 40, 120 s and 4 GB per back-end call), the build before these changes and the final build at the same time, four runs in all in 8 GB scopes, at a load of 8 to 15 on 32 cores, one run each (`bench/results/count-time-searches.csv`). Both builds gave the same repairs, the same 3 and 2 failed counts, and the same queries and back-end calls.

| Run | Build | Count time | Wait | Wall |
|---|---|---|---|---|
| AutoFlight-PT-02a seed 2 | before | 372.8 s | 345.5 s | 373.9 s |
| AutoFlight-PT-02a seed 2 | after | 192.1 s | 159.3 s | 193.6 s |
| CircadianClock-PT-000010 seed 1 | before | 333.9 s | 328.0 s | 410.1 s |
| CircadianClock-PT-000010 seed 1 | after | 28.0 s | 20.3 s | 126.2 s |

AutoFlight's wall time halved, and the search still waits on the counter for 82% of it. CircadianClock now waits 20.3 s, and the rest of its 126.2 s is the oracle.

**Work bound.** A count under the monoid bound can still be slow, so each count also counts its *steps*: per prefix subset and letter, per relation interned and per block id stepped, per row of a block stepped and per word combined into a row, and per state, word and edge of each closure. A count past `COUNT_WORK`, 2³² steps, fails as one past `MONOID_BYTES` does, and the check runs in both passes. Steps make the bound deterministic, so a seeded search makes the same choices under any load. On av2 a step took 4 to 19 ns, 7 ns in the median, over the 120 counts of the corpus with more than 10⁸ steps, so the bound is about 30 s of count. The largest count of the corpus took 1.28 × 10⁹ steps, 3.4× under the bound, and no count of the corpus reached it.

`MONOID_BYTES` keeps its formula, 4 bytes per state for each relation as when a relation was its list of row ids, so the same counts stop at the same point. With blocks a relation holds 4 bytes per 8 states, so the estimate now overstates the monoid's memory.

Three ideas were measured and left out. A cache keyed on the whole automaton would have saved 0.27 s of 55.7 s on a partial AutoFlight replay, as "Interned rows" found on abp4. Pruning the states that reach no accepting cycle saves nothing, since in every profiled count over 1 s every state was live, and `letters()` already drops edges into dead states. Blocks of 16 or 32 rows were slower than 8, as above.

What remains is the steps of relations and the closures, in about equal parts, on monoids of up to 2,233,707 relations in the corpus. A smaller monoid needs a smaller automaton or fewer letters, which is a change to the count rather than to its implementation.

## The output gate

The gate (`search::gate`) runs once, on the final population.

1. Keep the distinct candidates with status 1.
2. In one `ltlfilt` batch, ask for each candidate whether φ implies it and whether the encoding invariant I implies it (see below). Keep the ones proved implied by φ and not proved implied by I. A failed batch is retried one pair per exec, so a pair Spot cannot decide drops only its own candidate.
3. In a second batch, ask every ordered pair of survivors whether one implies the other. Drop a candidate that a strictly stronger survivor implies. Of two equivalent survivors, keep the one more similar to φ, then the smaller in formula order. A pair Spot cannot decide, even alone, proves no implication, so neither of its candidates drops on its account.
4. Flag where each surviving repair is vacuous on M (see below). This step asks the oracle, not Spot, and never drops a repair.

So every reported repair φ' has M ⊨ φ' proved by the oracle and φ ⇒ φ' proved by Spot. Spot does not prove I ⇒ φ', and does not prove any other reported repair stronger than it.

## Triviality: validity modulo the encoding

A candidate that holds on M can still hold for a trivial reason. On abp4, `G(s_get & s_wait -> X(…))` holds only because `s_get` and `s_wait` are two values of one enumerated variable, `sender.state`, and never hold together. Spot cannot see this, since it does not know the model, so plain validity lets the candidate through.

The fix is to tell Spot what the encoding guarantees. The atoms fall into groups of mutually exclusive atoms, and the encoding invariant I is `G` of `!(a & b)` over every pair within a group (`search::invariant`). A candidate φ' is trivial when I ⇒ φ' is valid, that is, when `I & !φ'` is unsatisfiable. I takes the place of `true` in the validity batch of each generation's filter and in the gate, so the check adds no process. With no groups, I is `true` and the check is plain validity.

The groups come from three places.

- `nuxmv::exclusive_groups` reads the DEFINEs of `MODULE main`. DEFINEs of the form `name := v = c` with the same `v` and distinct constants `c` form a group, and `name := v` and `name := !v` count as `v = TRUE` and `v = FALSE`. Two names for the same constant are equivalent, not exclusive, so only the first joins. Any other DEFINE is skipped.
- Each `--exclusive a,b,…` flag adds a group by hand.
- On a RERS model, at most one action atom holds per state, so all action atoms would form one group. The `weaken` binary runs only the nuXmv back end, so no code derives this group yet; `--exclusive` can give it.

The binary keeps only the atoms listed in `--atoms` in each group, drops groups of one, and prints each group at start-up as `exclusive: a, b`.

## Vacuity is a flag, not a filter

The standard test of vacuity (Beer, Ben-David, Eisner and Rodeh; Kupferman and Vardi) asks whether M also satisfies φ' with a subformula occurrence ψ replaced by the constant that makes φ' stronger: false where ψ has positive polarity, true where it has negative polarity. That stronger formula is a *witness*. If M satisfies it, φ' is vacuous in ψ.

A first version of the gate dropped every candidate vacuous on M. On abp4 it dropped every repair, including `G(s_get -> X F(s_wait & (…)))`, the `U` of φ weakened to an eventuality. That repair is vacuous in `s_get`, because M satisfies `G X F(s_wait & (…))`. But it is still a useful answer: it is the strongest weakening found that φ implies. Vacuity cannot tell a repair that says nothing from one that is weaker than some other truth of M that φ does not imply. Validity modulo the encoding can tell these apart, since it asks only about the encoding and not about M. So the filter is validity modulo I, and vacuity is a flag on each reported repair.

`operators::vacuity_witnesses` builds one witness per subformula occurrence, simplified, paired with its ψ. It uses the polarity rules of the monotone rewrite, so it skips both children of `<->` and everything under them. It also skips the whole formula and the constants. A witness that simplifies to false is left out, because it cannot hold on a model with a run. A witness that simplifies to true holds with no query. `search::vacuity` asks the same oracle as the search about every witness of each reported repair, so the cheap tiers answer most of them. A witness is stronger than the repair, so a lasso that refutes the repair refutes the witness too.

The `weaken` binary prints the flag as the third column of each repair line: the subformulae in which the repair is vacuous, separated by `; `, or `-` when there are none. A witness whose verdict is unknown adds `?`. A line `vacuity: queries=… unknowns=… backend_calls=…` gives the cost, and the gate's own time is on the `wall` line.

## Triviality at the initial state

Validity modulo I misses a second kind of trivial repair. CEGIW's foraging robots start at rest, and the gate reports `G(resting_p -> X(resting_p | X(resting_p | X resting_p))) | resting_p` on them. The disjunct `resting_p` is true at position 0 of every run, so the whole formula holds whatever M does afterwards. I says nothing about the first position, so the repair passes the triviality filter.

### The candidate filter

Let *Init* be the *initial-state predicate* of M over φ's atoms: the disjunction of the letters that start some run of M, each letter a conjunction of literals over every atom of φ. The candidate filter drops φ' when (I ∧ Init) → φ' is valid, read with Init at position 0 only. It is the triviality check with I ∧ Init in place of I, so it runs in the same `ltlfilt` batches as before, under the oracle's semantics, and adds no Spot process.

`instance::initial` finds Init through the oracle, so one routine serves every back end: SMV `INIT` and `init()` assignments, Promela initial values, a PNML initial marking and the first letters of a trace set's positive traces. It starts from the first letters of the runs the oracle has stored, which after `classify` include the run that refutes φ. It then asks whether M satisfies their disjunction. Each refutation starts with a letter outside the disjunction, which joins it, so the loop ends with the exact projection of M's initial states onto φ's atoms. It gives up after 16 letters or on an `Unknown`, since a partial disjunction would be stronger than Init. A run then keeps I alone.

`search_bench --initial flag` finds Init and marks each repair that I ∧ Init implies, in one more `ltlfilt` batch after the gate, without changing the search. `--initial filter` also hands I ∧ Init to the search's validity filter and to the gate, so these candidates leave the population as soon as they are scored. The default, `ignore`, asks nothing. The `weaken` binary has no such option.

### Is position 0 the right line?

Treating "true at the first position" as trivial has a cost. A repair `G ψ | init_p` is strictly stronger than ⊤ as a language, and on a second implementation that starts elsewhere it still says `G ψ`. Read as a statement about M, though, it holds on every model with M's encoding and M's first letter, whatever the model does next. That is the same reason I-validity counts as trivial: the fact comes from the model's interface and start, not from its behaviour. Reachable-state invariants are left out on purpose. They are truths of M's behaviour, and dropping every φ' implied by one would drop every repair.

Vacuity overlaps with the check. `G ψ | init_p` is vacuous in `G ψ`, since its witness `init_p` holds on M, so the existing flag already marks the plain disjunctive case. The check is not the same as vacuity, however. Vacuity flags real repairs such as abp4's `G(s_get -> X F …)`, and the initial-state check does not; it also needs no oracle query per subformula. So three uses are possible: a filter in the loop and the gate, a flag on each repair beside vacuity, or a filter at the gate only. The last loses repairs without letting the search look elsewhere, so only the first two were measured.

### Measured on the ablation set and CEGIW's cases

Two arms ran on one commit, at 30 × 40 over seeds 1–10, on av3 (32 cores, 28 runs at once, no other load). `initflag` is the binary arm with `--initial flag`, so its search is the binary arm's and its marks give the post-hoc count. `initfilter` uses `--initial filter`. The instances are the 86 of the status ablation and CEGIW's nine cases, the degenerate ones searched (`forced-initflag` and `forced-initfilter`). The commands were the following, with the results in `bench/results/initial-*.csv`.

```
bench/graded_remote.sh bench/data/graded 28 init --seeds 1-10 --arms initflag,initfilter \
    --out runs-init --filter '^(?!beem-anderson\.(3-2|3-4|5-2|5-4|7-4)$|beem-lamport\.8-4$|mcc-MedleyA-PT-09-)'
bench/graded_run.py bench/data/cegiw 2 --seeds 1-10 --arms forced-initflag,forced-initfilter --out runs-init
python3 bench/initial_stats.py bench/data/graded bench/results initial-graded --top bench/results/baselines-top.csv
python3 bench/initial_stats.py bench/data/cegiw bench/results initial-cegiw \
    --flag forced-initflag --filter forced-initfilter --top bench/results/baselines-cegiw.csv
```

The flag arm reproduces the binary arm of the reserve ablation, which ran the previous commit's search. Of 837 runs on the same instances and seeds, 815 report the same repairs; 19 of the 22 others had a back-end `Unknown` in one of the two runs, and the last 3 were not traced. All 860 flag runs and 851 of 860 filter runs finished; the other 9 filter runs passed the 3,600 s cap (lamport.6-4 three times, SieveSingleMsgMbox four times, anderson.7-2 and peterson.6-4 once each), where their flag twins took 1,514–2,732 s.

| Family | Instances | Repairs | Marked | Marked and vacuous | Runs left with none | Of those, the filter run finds one | Instances left with none |
|---|---|---|---|---|---|---|---|
| RERS | 52 | 965 | 532 (55%) | 492 | 249 | 82 | 12 |
| MCC | 19 | 541 | 220 (41%) | 217 | 45 | 32 | 1 |
| BEEM | 14 | 228 | 111 (49%) | 110 | 28 | 7 | 0 |
| abp4 `p2false` | 1 | 37 | 4 (11%) | 4 | 0 | 0 | 0 |
| CEGIW cases | 9 | 166 | 32 (19%) | 32 | 11 | 2 | 0 |

Repairs count over the flag arm's runs. A run is left with none when it reported repairs and every one is marked; an instance is left with none when that holds for all ten seeds. Every instance but one has at least one marked repair.

Half the repairs are marked: 867 of 1,771 on the ablation set. The share is highest on RERS because every RERS run starts with the empty letter, before any action, so Init is the conjunction of every atom negated. Any top-level disjunct `!a` then holds at position 0. Representative marks, with Init after the instance, are these.

- **RERS** 2016/103 p16, `!p_c0_t1`: `F G !p_c0_t1 | !p_c0_t1`, for φ = `F G !p_c0_t1`, on 7 seeds. 2016/104 p13, `!p_c3_t3 & !p_c3_t5`: `!p_c3_t3 | p_c3_t5`, which is the right side of φ's release read at position 0 alone, on 9 seeds. 2016/102 p19: `!p_c0_t2` for φ = `G !p_c0_t2`, the repair the ⊤ baseline already noted.
- **BEEM** anderson.1-4 and six others, `!p_someoneincs`: `G F p_someoneincs | !p_someoneincs` on 5 seeds each. lamport.6-4: `!p_someoneincs W X F p_someoneincs`, which carries no vacuity flag.
- **MCC** Peterson-PT-4 Cardinality-01, `a0 & !a1 & !a2`: `a0 | F a2`. Peterson-PT-2 Fireability-09, `!a0 & !a1`: `X G !F a1 | !a1`. ResAllocation-R010C002, `!a0 & a1 & …`: `F(!a0 | a1 & !(a2 & a3))`, whose `F` is met at position 0.
- **abp4**, `s_get & !s_abp & …`: φ itself `| !s_abp`, since the sender starts with bit 0.
- **CEGIW**, `resting_p`: `G(resting_p -> X(resting_p | X(resting_p | X resting_p))) | resting_p` and `resting_p U X(resting_p -> X F X X resting_p)` on forage-response, and the same `… | a_p` shapes on the two-state model.

By construction, a marked formula holds on every word that satisfies I and starts in Init, so it says nothing about M after its first letter. The 32 CEGIW marks, the 36 distinct marks without a vacuity flag and about 20 others were also read by hand, and each reads as trivial. 823 of the 867 marked repairs already carry a vacuity flag; the 44 others (36 distinct) include `!p_c0_t2`, which has no subformula to replace, and conjunctions such as `G !(a & b) & !a` that restate I ∧ Init. The lasso count does not see the problem: 142 of 653 distinct marked repairs on the ablation set tie with ⊤, and the rest score above it. On forage-response the best score in the CEGIW comparison above, 0.9676, belongs to a marked repair; the filter arm finds no repair there on any seed.

The filter in the loop beats removing the marks afterwards.

| Family | Found rate, flag after removal | Found rate, filter | Higher / lower / equal | p | Back-end calls, flag → filter | Wall, flag → filter |
|---|---|---|---|---|---|---|
| All 86 | 0.70 | 0.90 | 36 / 10 / 40 | <0.0001 | 49 → 67.25 (+10.5, p<0.0001) | 17.7 → 20.8 s (+1.0 s, p<0.0001) |
| RERS | 0.40 | 0.60 | 25 / 9 / 18 | 0.0014 | 63.75 → 89.75 | 12.2 → 12.0 s |
| MCC | 0.90 | 1.00 | 10 / 0 / 9 | 0.0057 | 48 → 68.5 | 131.8 → 130.8 s |
| BEEM | 1.00 | 1.00 | 1 / 1 / 12 | 1.0 | 14.75 → 17.25 | 14.3 → 15.0 s |

Each cell is the median over instances of the per-instance value over the seeds both arms finished, and p is the Wilcoxon signed-rank test of the paired per-instance differences. The differences are medians of per-instance differences, so they are not the differences of the two medians. Of the 13 instances left with no repair after removal, the filter finds one on 10, at rates of 0.1 to 0.8. It finds none on RERS 2016/103 p4, 105 p12 and 105 p17, nor on CEGIW's degenerate forage-response. Of 322 flag runs left with none, the filter run on the same seed found a repair in 121. The repairs it finds are as strong as the survivors of removal: the net subsumption share of the filter's pooled maximal set against the flag arm's, after removal, has median 0 (22 instances favour the filter, 16 the flag arm, 31 tie; p = 0.91).

The cost is in the search, not in Init. `instance::initial` took a median of 18 ms and at most 0.95 s, and exactly one back-end call on every run, since the stored runs always held every initial letter. The filter arm makes a median 10.5 more back-end calls per run, while the final population holds as many proved candidates (median 24 against 23). On the four instances whose calls take tens of seconds, the extra calls pushed 9 of 40 runs past the cap.

The filter removes half of what the gate reported, and what it removes is trivial by construction. Its price is about ten more back-end calls per run, which matters only where each call takes tens of seconds.

## The validity filter

The brief asked for a syntactic validity check only. That was not enough. On the first runs, with only the `simplify` check in the filter, the gate reported one repair on each of the two end-to-end models, and both were valid: `G(ack -> X(!req <-> ack) | ack)` and a formula ending in `s_wait W (… | !s_wait)`. A valid candidate holds on every model and so wins the status objective outright, and the search crowded towards them. PEREDUR's vacuity filter asks a solver the same question per guarantee, so the filter now sends each generation's offspring to `ltlfilt` in one batch, and the gate asks again. The question is now validity modulo I, as described above. The filter runs after scoring and asks only about the candidates that hold on M. M satisfies I, so a candidate that fails on M is not valid modulo I. A batch that fails is retried one candidate per exec (`implications_each`), and a candidate whose own exec fails is kept unfiltered, so the search never depends on machine load. The filter, the counter and the gate's pruning of dominated repairs give each Spot exec 2 s, and the gate's proof of φ ⇒ φ' gives it 10 s (`SEARCH_LIMITS` and `GATE_LIMITS` in `src/main.rs`), against 120 s for the oracle's batches. "Slow implication batches" in `trace-sets.md` has the measurement behind these choices.

## Measured runs

The runs below come from `cargo test --test search -- --nocapture` (debug build, local machine). The test re-checks every reported repair with a direct nuXmv call and a Spot implication check. The first two rows are from before the model-counting objective, the next two from PEREDUR's path count, the two after from the prefix count, and the last two from the lasso count with interned rows, with the dev profile at `opt-level = 2` and the degeneracy filter, which adds one query and one back-end call on abp4.

| Model | φ | G × P, seed | Repairs | Wall | Queries | Back-end calls | Lasso hits | Implication hits | Memo hits | Spot execs |
|---|---|---|---|---|---|---|---|---|---|---|
| `ack.smv` (in the test) | `G (req -> X ack)` | 30 × 40, 4 | 2 | 2.35 s | 843 | 27 | 429 | 97 | 290 | 102 |
| LMCS-2006 abp4 | `p2false` | 30 × 40, 1 | 2 | 12.5 s | 1121 | 140 | 676 | 199 | 106 | 228 |
| `ack.smv`, path count | `G (req -> X ack)` | 30 × 40, 4 | 2 | 2.63 s | 790 | 21 | 398 | 87 | 284 | 92 |
| abp4, path count | `p2false` | 30 × 40, 1 | 3 | 16.1 s | 1129 | 184 | 521 | 309 | 115 | 357 |
| `ack.smv`, prefix count | `G (req -> X ack)` | 30 × 40, 4 | 3 | 3.01 s | 782 | 24 | 387 | 129 | 242 | 133 |
| abp4, prefix count | `p2false` | 30 × 40, 1 | 1 | 15.1 s | 1101 | 180 | 648 | 190 | 83 | 247 |
| `ack.smv`, lasso count | `G (req -> X ack)` | 30 × 40, 4 | 2 | 2.35 s | 801 | 22 | 408 | 80 | 291 | 82 |
| abp4, lasso count | `p2false` | 30 × 40, 1 | 2 | 14.6 s | 1122 | 165 | 654 | 197 | 106 | 240 |

The counts include the vacuity flags. On `ack.smv` there are no groups, and without the third objective the repairs are `req -> F X ack` and `G(req -> F ack)`, the same as before the flags. Neither is vacuous. The flags cost 7 queries and no back-end call.

On abp4 the propositions of `p2false` are DEFINEs appended to `main` (`s_get` for `sender.state = get`, and so on). The binary derives two groups, `{r_ack0, r_ack1}` and `{s_get, s_wait}`. Before the invariant, the gate reported `G(s_get & s_wait -> X(…))`. Now that candidate is dropped as trivial, and the search takes a different path from the same seed. The repairs are as follows.

- `G(s_get -> X F(s_wait & (…)))`, flagged vacuous in `s_get`, in the disjunct `!s_abp & r_ack0` and in the atoms under it. The protocol reaches both acknowledgements infinitely often.
- `G(s_get & r_ack0 -> X((s_abp | !s_get) U (s_wait & (…))))`, flagged vacuous in `s_get` and in every subformula of its consequent. Its antecedent never holds on M, but the encoding does not say so: the sender's state and the channel tag are different variables.

The flags cost 32 queries, no back-end call and 0.23 s. Against the run before the invariant (1097 queries, 132 back-end calls, 12.1 s), the run made 24 more queries and 8 more back-end calls. That difference comes from the changed search path, not from extra processes.

Seed 2 on abp4, with the same budget, ended with no repair after 237 back-end calls and 20.8 s. A run that finds nothing is a normal outcome at this budget.

### With the model-counting objective

The third objective changes the search path from the same seed. Four arms ran on av3 (release build, five repetitions with the order of the arms rotated, other jobs at a load of about 20 of 32 cores, `bench/model_counting_ab.sh`, rows `lasso-ab` in `bench/results/model-counting-ab.csv`): no counting objective, PEREDUR's path count, the prefix count and the lasso count. Every arm gave the same repairs and the same call counts on all five repetitions. The table gives medians.

| Run | Arm | Repairs | Back-end calls | Count time | Wait | Wall |
|---|---|---|---|---|---|---|
| `ack.smv`, seed 4 | two objectives | 2 | 27 | — | — | 2.82 s |
| `ack.smv`, seed 4 | path count | 2 | 21 | 1.04 s | 0.14 s | 2.47 s |
| `ack.smv`, seed 4 | prefix count | 3 | 24 | 1.05 s | 0.02 s | 2.85 s |
| `ack.smv`, seed 4 | lasso count | 2 | 22 | 1.03 s | 0.11 s | 2.21 s |
| abp4 `p2false`, seed 1 | two objectives | 2 | 140 | — | — | 14.59 s |
| abp4 `p2false`, seed 1 | path count | 3 | 184 | 5.02 s | 0.00 s | 19.43 s |
| abp4 `p2false`, seed 1 | prefix count | 1 | 180 | 4.50 s | 0.05 s | 17.37 s |
| abp4 `p2false`, seed 1 | lasso count | 2 | 164 | 36.74 s | 26.77 s | 43.32 s |

Earlier runs at lower load (rows `ab` and `monotone`) gave the same repairs and call counts. Spot compared every lasso-count repair with every repair of the other three arms, and each repair below is scored by the lasso count.

- `ack.smv`. ⊤ scores 0.8987. The lasso count finds `G F(req -> X ack)` at 0.8989 and `req -> F X ack` at 0.8987. `G(req -> F ack)`, found by the two-objective and path arms, scores 0.8993 and implies `G F(req -> X ack)`. The prefix arm's `req -> F ack` and the path arm's `X G req | F X ack | !req` also score 0.8987. These three only constrain the first request, so their languages miss few lassos and they equal ⊤ to four places, while the liveness repairs sit strictly between φ and ⊤.
- abp4. ⊤ scores 0.9598. The lasso count finds `G(s_get & r_ack0 -> X(!s_get U (s_wait & (…))))` and the same with `r_ack1`, both at 0.9821, the highest score of any arm. The first implies the `G(s_get & r_ack0 -> …)` repairs of the two-objective arm (0.9734) and the path arm (0.9751): it keeps φ's `!s_get` where they weaken it. No other repair scores above 0.9751. The prefix arm's `F(…)` repair scores 0.9598, a tie with ⊤, and the path arm's `G(s_get -> X(… | s_get))` 0.9599.

On abp4 the lasso count gives the strongest repairs found by any arm, and no repair of its own ties with ⊤. Two seeds are too few to judge the objective in general.

### Graded status against binary

The ablation compares the binary status with the graded one (option B, "Graded status" above) at a fixed budget of 30 generations of 40, over 10 seeds per instance. The instance set comes from `bench/graded_prep.py` and has 93 instances.

- LMCS-2006 abp4 `p2false` on nuXmv. Every other LMCS failure that survives the filter uses past-time operators, which the formula core lacks.
- 52 RERS 2016 parallel properties (problems 101 to 106, every filter survivor) on the explicit back end.
- The 20 BEEM pairs of `bench/beem/search_models.py` on SPIN, in its stepwise mode.
- 20 MCC LTL properties on TAPAAL, one per instance, drawn with seed 1 from those with at least two atoms whose two checks each take under 1 s.

Each run is one `search_bench` process (release build), with 120 s and 4 GB per back-end call and 3600 s per run, in its own systemd scope capped at 8 GB (`bench/graded_run.py`). The runs went to av3, except BEEM and option C, which went to av2, at 14 concurrent runs per host, seed-major with the arms interleaved, so both arms of a pair ran on the same host under the same load. Other users' jobs were not controlled, so wall times compare pairs and not hosts. The code was this change on `main` at `76966b1`, before the slow-call limits of `6d5590a`; the check after the rebase is at the end of this section. `bench/graded_stats.py` pairs the arms by seed, takes each instance's median over its seeds, and reports the median over instances with a bootstrap 95% confidence interval (10,000 resamples), the median paired difference with its interval, and a Wilcoxon signed-rank test over instances. Repair strength follows PEREDUR. Each arm's repairs of an instance are pooled over seeds and reduced to the maximal set, and the net subsumption share is the share of binary's maximal set strictly implied by a graded repair, minus the converse, so it is positive when graded is stronger. The results are in `bench/results/graded-runs.csv` (one row per run) and `bench/results/graded-instances.csv` (one row per instance).

| Measure (86 instances) | Binary | Graded | Difference (graded − binary) | p | Graded higher / lower / tie |
|---|---|---|---|---|---|
| Found-repair rate | 1.00 [1.00, 1.00] | 1.00 [1.00, 1.00] | 0.00 [0.00, 0.00] | 0.74 | 8 / 10 / 68 |
| Best semantic similarity | 0.999 [0.994, 0.999] | 0.999 [0.989, 0.999] | 0.000 [0.000, 0.000] | 0.53 | 22 / 20 / 44 |
| Repairs | 2 [1, 2] | 2 [1, 2] | 0 [0, 0] | 0.10 | 14 / 21 / 51 |
| Candidates that hold, of 40 | 23.5 [22.5, 24.0] | 19.0 [17.75, 20.0] | −3.25 [−5.25, −2.25] | < 0.0001 | 5 / 76 / 5 |
| Wall time | 9.77 s [7.59, 14.81] | 9.35 s [7.35, 14.86] | +0.01 s [−0.20, 0.28] | 0.33 | 43 / 43 / 0 |
| Back-end calls | 67 [54.5, 81] | 67 [56.25, 78.5] | +0.5 [−1, 1] | 0.50 | 44 / 37 / 5 |
| Net subsumption share | | | 0.000 [−0.088, 0.000] | 0.34 | 24 / 36 / 26 |

The 86 instances are those where both arms have at least one paired seed. MedleyA-PT-09 `LTLCardinality-09` ran out of memory on every seed of both arms, and six large BEEM instances (anderson 3-2, 3-4, 5-2, 5-4 and 7-4, and lamport 8-4) ran only seeds 1 to 3, of which most reached the 3600 s cap without a result. Over the whole sweep 76 binary and 75 graded runs gave no result, at a cap of 8 GB. A rerun of those at 32 GB recovered 11; the rest were killed by the out-of-memory killer again, in a Spot or Ganak child. The losses are spread evenly over the arms. On the MCC instances the memory was later traced to the search process itself, in the lasso count's transition monoid, which is now bounded (see "Bounded monoid").

By family, the drop in candidates that hold is −3.0 on RERS (47 of 52 instances lower), −7.0 on MCC (16 of 19) and −1.0 on BEEM (12 of 14), each at p < 0.004. MCC is the one family where the drop reaches the gate: graded found a median of 0.5 fewer repairs (p = 0.024) and its net subsumption share was −0.10 [−0.17, 0.00] (p = 0.070, 4 instances stronger and 12 weaker). On BEEM graded ran 1.1 s longer (p = 0.0001, all 14 instances) and made 1.75 more back-end calls (p = 0.021). Nothing else differs at p < 0.05.

Grading is cheap. Over 842 graded runs it took a median of 0.15% of the search's wall time and at most 7.3%. A run ends with a median of 3 stored lassos that violate φ (at most 17), so the grade can take few distinct values.

The budget could hide a difference, since every arm finds a repair at 30 × 40. The same instances ran at 4 × 8 with 20 seeds (83 instances; the small BEEM pairs locally, MCC on av3 at two concurrent runs, everything else locally at two concurrent runs). The found-repair rate was 0.40 for both arms (p = 0.69), with 24 instances higher and 17 lower under graded. Graded held fewer proved candidates (9 higher and 23 lower, p = 0.009) and made more back-end calls (32 higher and 16 lower, p = 0.005), with medians equal. The net subsumption share did not differ (p = 0.11). Runs ended with a median of 1 refuting lasso (at most 5). These results are in `bench/results/graded-runs-4x8.csv` and `bench/results/graded-instances-4x8.csv`.

The two variants of "Graded status" ran on RERS only, against the same binary runs.

| Arm (52 RERS instances) | Seeds | Candidates that hold, difference | Back-end calls, difference | Wall, difference | Net subsumption share (p, higher / lower) |
|---|---|---|---|---|---|
| B with 64 random runs | 10 | −4.0 [−5.5, −2.25], p < 0.0001 | −6.25 [−9.25, −3.0], p < 0.0001 | +0.08 s, p = 0.36 | p = 0.035, 10 / 22 |
| C, share of states | 1 to 4 | −5.0 [−5.5, −3.0], p < 0.0001 | −1.75 [−5.5, 0.0], p = 0.064 | +7.7 s [3.2, 254], p < 0.0001 | p = 0.58, 13 / 16 |

The random runs save back-end calls, since a stored walk refutes some candidates before tier 3 sees them, but their repairs were weaker than binary's on more instances. Option C cost a median of 70% of the search's wall time and gave no stronger repairs, and C was stopped after four seeds.

SPIN rechecked every repair that the explicit back end reported on RERS, as ground truth (`bench/graded_crosscheck.py`, `ltl2tgba --spin` and `pan -a` on the original Promela model, 600 s per check). At 4 × 8 there were 345 distinct repairs: 228 hold, 77 use `X` and were skipped (the Promela model takes hidden steps, so only a stutter-invariant formula has the same verdict on both; see `rers-kripke.md`), 40 left SPIN's search incomplete, and none is violated. At 30 × 40, over all four arms, there were 1735 distinct repairs: 1185 hold, 337 use `X` and were skipped, 213 left SPIN's search incomplete, and none is violated. The two back ends never disagree. The rows are in `bench/results/graded-crosscheck.csv` and `bench/results/graded-crosscheck-4x8.csv`.

The search changed under this work: `6d5590a` gave each Spot exec of the validity filter, the counter and the gate's pruning 2 s, and moved the validity filter after scoring. After the rebase onto it, RERS and abp4 ran again on av2 (10 seeds, both arms, 14 concurrent runs, rows in `bench/results/graded-runs-rebased.csv` and `bench/results/graded-instances-rebased.csv`; 37 of 1060 runs gave no result). The conclusion holds. On the 52 RERS instances graded held 3.0 fewer proved candidates [−5.5, −2.25] (p < 0.0001, 47 lower), and the found-repair rate (p = 0.67), the repairs (p = 0.50) and the net subsumption share (p = 0.72, 18 higher and 16 lower) did not differ. Graded ran 0.15 s faster [−0.39, −0.03] (p = 0.021). Both arms made more back-end calls than before the rebase (binary's median rose from 86.75 to 102.75), so the absolute numbers in the tables above belong to `76966b1` and do not carry over.

## Left out of PEREDUR

| PEREDUR piece | Why it is not here |
|---|---|
| Sections, sides, slots, assumptions and guarantees | The individual is one formula, so there is nothing to choose between. |
| `tlsf_add_assumption`, clone-assumption, `tlsf_remove_guarantee` and tombstones | All act on lists of conjuncts. A single property has no list. |
| The FRETISH front end, timings, timing donation, scopes and condition types | FRETISH is a different input language. The input here is plain LTL. |
| Per-section atom pools and initial-condition rules | There are no sections. The pool is φ's atoms. |
| Crossover's target and donor draws | With one slot per parent, each would always draw index 0. |
| `remove_double_negation` after `mutate_formula` | Breeding runs `simplify` on every offspring, which removes it. |
| Elitism (`elitism_rate` 0.1) | The (μ+λ) pool already keeps the best parents. PEREDUR's own notes say both arguments point at 0, and elites would bypass the filters. |
| `selection_rate` (the target size is half the population) | The population stays at P throughout, one offspring per slot. |
| The pad stage and the filter fallback | Under (μ+λ) an empty offspring set leaves the parents, so the run continues without them. |
| `nsga2-truncate` and the weighted-average scheme | Only `nsga2-apportion`, PEREDUR's default, has a caller. |
| Termination by individuals or wall time | Only a generation count is needed now. |
| The repair accumulator and streaming maximality | The gate runs once, on the final population. |
| MUC repair mode | It splits a specification into cores; one formula has none. |
| The atom-renaming cache key | Atoms name model variables, so renaming is unsound (see `CLAUDE.md`). |
| The timing horizon in the model-counting bound | It exists for FRETISH timings; plain LTL has none. |
| `max_representable_step_count` | Counts are logarithms with rescaled weights, so no bound overflows. |
| The transfer-matrix path count (`transfer_matrix.cpp`) | It is not monotone under implication, so the lasso count replaces it (see "Monotonicity"). |
| The `Direct` similarity metric and the keyword mix | Only the default `Logarithmic` metric has a caller. |
| The renamed cache keys of `cached_count_traces` and Ganak | The score is relative to the fixed φ, whose atoms name model variables. Ganak's label cache does renumber the label's own atoms, which a propositional count cannot tell apart. |
| mt19937 and PEREDUR's draw stream | Each tool must reproduce its own runs, not the other's. |

The rates kept (0.1 crossover, 1.0 mutation, 0.25 monotone, 0.2 temporal, a bloat ratio of 2) are PEREDUR's defaults and are not tuned here. None has been measured on this problem.
