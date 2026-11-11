# Order-guided search and MTL

This note looks at two possible extensions. The first ports PEREDUR's order-guided operators, which use the implication order to choose between weakening and strengthening a candidate. The second widens the input from LTL to *metric temporal logic* (MTL), whose operators carry time intervals, using CEGIW's interval order. It records what each source repository provides, which parts carry over to model checking and which do not, and a proposed order of work. The rules of "LTL templates" are implemented as `RuleSet::Ordered`, and three of PEREDUR's operators as described in "PEREDUR's order features". All of them are off by default.

## What PEREDUR provides

PEREDUR (`~/projects/peredur`) repairs specifications for *realisability* (a controller exists), not against a model. Its recent work adds orders on the fields of a FRETISH requirement, the structured English of NASA's Formal Requirements Elicitation Tool (FRET), and uses them in the genetic algorithm (GA).

**The orders.** `timing_implies` (`src/requirement.cpp:482`) orders timings such as `within n`, `for n`, `next`, `always` and `eventually` under each scope. `scope_implies` (`src/requirement.cpp:570`) orders scopes, and `condition_type_step` (`src/filter/implication_check.cpp:152`) orders the continual and trigger condition types. The timing, scope and condition-type orders are proved exact in Lean for every tick count (`proofs/Fretish/Cpp.lean:100`, `ScopeOrder.lean:500`, `CondOrder.lean:182`). `requirement_implies` (`src/filter/implication_check.cpp:227`) chains single-field steps by a search over the 32 subsets of moved fields, and `classify_move` (`src/genetic/order_moves.cpp:525`) labels a child as equal, stronger, weaker or incomparable to its parent.

**The operators.** All are FRETISH-only and off by default.

- `direction_gated` (`src/genetic/mutation.cpp:987`): a realisable parent may only strengthen, an unrealisable one only weaken. A free rewrite that `classify_move` proves to move the forbidden way returns the parent, and an incomparable one is kept with probability `p_sideways`.
- Order crossover (`src/genetic/order_crossover.cpp:85`): two realisable parents give their join, two unrealisable ones their meet, and a mixed pair is *path-relinked* (the realisable parent adopts the unrealisable one's stronger slots).
- `blame_targeted_slots` (`src/genetic/mutation.cpp:1102`): mutate the guarantees that the minimal unrealisable core names.
- `bisect_steps` and `step_scaling`: move a tick count to the midpoint of its chain, or scale a step by the parent's status.
- The parent-verdict filter (`include/genetic/pipeline.hpp:174`): drop offspring proved stronger than a parent already proved unrealisable.
- `maximality_demotion` (`src/genetic/demotion.cpp:122`): add 1 to the front rank of a realisable candidate that another realisable candidate strictly implies.
- `climb_output` (`src/genetic/climb.cpp:94`): after the search, greedily take proved strengthenings of each repair while the output gate still passes.

**The results so far.** The ablation (`experiments/2026-11-05-fretish-order/PLAN.md`, rows in `experiments/results-fretish-order.csv`) ran 270 runs per arm. The count of runs whose repair implies the ideal one is 94 for the base GA, 113 for the full set, 112 with only the climb, and 101 with only the gate. Random selection scores 76 alone and 95 with the climb. The found rate stays near 0.89 in every arm, and no significance test has been run. A pooled strength and diversity pass (`experiments/2026-11-09-fretish-order-pooled/`) was voided by an audit on 2026-11-11, because the climb arms were pooled from the wrong file and 73% of the comparison rows crashed. Its figure for the gate, a median net subsumption of +0.187, held in the rerun at +0.188, and the climb's moved from −0.032 to +0.920 ("PEREDUR's order features").

**What is LTL-generic.** Almost nothing in the orders themselves, which are tied to FRETISH's five fields. `monotone_rewrite` (`src/genetic/monotone.cpp`) is generic, and GAWAIN already has it (`src/search/operators.rs:106`, "Operators" in `search.md`). The slot-wise structure of `classify_move` would work over any per-conjunct implication check.

## What CEGIW provides

CEGIW (`~/projects/cegiw`) has no order between operators. Its order covers only the right bound of one interval that the user names by a de Bruijn path. Lemma 5 of `artefacts/proofs.pdf` proves that pushing out the right bound of an `F` or `U` weakens, and Lemma 6 that pulling in the right bound of a `G` or `R` weakens. Theorem 3 lifts both through any negation-free context, and the code puts the context into a partial negation normal form so that a negative occurrence dualises the operator (`src/logic/ctx.py:306-432`). There is no `G ⇒ F` rule, no rule between `U` and `F`, and no move of the left bound, which the preprint leaves as future work because it gives only a partial order.

The step is counterexample-guided. For each lasso, `Weaken._aux` (`src/weaken.py:273`) computes the smallest change of the bound that makes the lasso satisfy the formula, and `choose_weakest_interval` (`src/analyse_cex.py:116`) merges the results over all lassos. The outer loop (`src/iterative_weaken.py:81`) model-checks, weakens and repeats. The oracle is bounded model checking at a bound of max(20, 1.5·b), which is why its abp4 answer fails on M ("CEGIW: results" in `bench/notes/baselines.md`).

CEGIW's `U` differs from the usual pointwise semantics. `mtl_to_ltl` (`src/logic/mtl.py:144`) expands `φ U[a,b] ψ` to `X^a (φ U[0,b−a] ψ)`, so φ is required only from position a onwards. Under that reading a move of the left bound of `U` is incomparable in general. The usual reading requires φ on every position before the witness, and under it `U[a,b]` is monotone in the interval in both bounds.

## What changes under model checking

Model checking gives exact facts that realisability lacks. Let R be the set of repairs, the formulas ψ with φ ⇒ ψ and M ⊨ ψ.

1. If M ⊭ ψ and χ ⇒ ψ, then M ⊭ χ. Moreover, ψ's refuting lasso refutes χ, so tier 1 of the oracle (`src/oracle/tiered.rs`) answers χ without a back-end call.
2. If M ⊨ ψ and ψ ⇒ χ, then M ⊨ χ, and χ is no stronger than ψ.
3. If M ⊨ ψ₁ and M ⊨ ψ₂, then M ⊨ ψ₁ ∧ ψ₂. If both are repairs, so is ψ₁ ∧ ψ₂, and it implies each. Realisability is not closed under conjunction, so PEREDUR's join crossover is a heuristic there and an exact construction here.
4. If M ⊨ ψ, then φ ∨ ψ is a repair. This projects any holder that φ does not imply into R.

Facts 1 and 2 decide the direction of every monotone rewrite in advance. GAWAIN draws that direction on a fair coin (`src/search/operators.rs:46`). With crossover at 0.1 and the frozen monotone share of 0.75, 0.9 × 0.75 × 0.5 ≈ 0.34 of all offspring are monotone moves in the direction that facts 1 and 2 make useless. That share comes from the rates and has not been measured on a run. A strengthened failing parent fails, and the lasso tier refutes it, so it costs a slot but almost no oracle time. A weakened repair holds but is weaker than its parent, so the gate's maximality step can never report it in place of the parent.

Phase 1 of the paper evaluation already shows the cost of one direction only. On the 100 model instances at 10 seeds (`bench/results/paper-p1-models-instances.csv`, means over instances against the `weakening` arm), `weaken-walk`, a random search whose every mutation weakens, has the highest found rate (0.803 against 0.766 for `tuned`) and the weakest repairs (net subsumption share −0.239 against +0.246). On the trace sets the share is −0.128 against +0.118. A pure weakening walk overshoots the boundary of R, and nothing in it comes back.

## Proposed operators for LTL

Each of these needs no new formula syntax and runs on the existing evaluation sets.

**Status-gated direction.** Pick the monotone direction from the parent's status in place of the coin. This is now built as `Steering::Gated` ("PEREDUR's order features"). A parent that is a proved repair strengthens. A parent that fails weakens, and so does one that holds but that φ is not proved to imply (status 0 under the weakening filter), since weakening keeps it true and can bring it into R. A parent with no verdict also scores 0 and so weakens, because the score cannot tell an Unknown verdict from a failure. The rule tests already cover the change, since `tests/search.rs` checks 1,200 rewrites against Spot for their stated direction. A crossed child gets no gate, as in PEREDUR's `gate_of`.

**Join crossover.** When both parents are proved repairs, offer the child ψ₁ ∧ ψ₂, whose status is known without an oracle call by fact 3. It still needs the bloat cap of `filter` (twice φ's size), which limits joins to small parents. A first version keeps the existing graft for every other pair. PEREDUR's meet and path-relinking have no exact counterpart here, since a disjunction of two failing formulas may hold.

**Counterexample-guided sites.** A failing parent with refuting lasso ℓ should only be weakened at a site s where ℓ satisfies the parent with s replaced by ⊤ (⊥ at a negative site). Every weakening at s implies that replacement, so if ℓ refutes the replacement, it refutes every weakening at s, and the move is wasted. The replacements are the duals of `vacuity_witnesses` (`src/search/operators.rs:200`), and the packed evaluator checks one in about 45 ns ("Graded status" in `trace-sets.md`). This is the LTL analogue of CEGIW's per-lasso step and of PEREDUR's blame. Unlike blame it is exact: it removes only moves that the lasso tier would refute.

**Output climb.** After the search, for each repair in the maximal set, try strengthening rewrites and keep one when the oracle proves it holds and Spot proves φ implies it. This is now built as `search::gate_climbed` ("PEREDUR's order features"). PEREDUR's climb gave the largest single gain there (112 against 94 of 270, and +0.920 net subsumption share in the rerun). It costs wall time: PEREDUR's random arm took a median of about 75 s and 102 s with the climb. At 120 × 80 GAWAIN's GA already spends its extra wall time on proving holders at tier 2 ("Budget" in `PLAN.md`), and a climb adds more of the same work. `gate_wall` records that cost per run.

**Not ported.** The parent-verdict filter duplicates fact 1, which the lasso tier already applies, provided the parent's lasso is stored. Maximality demotion duplicates the semantic objective: the lasso count is monotone under implication (0 violations on 1,200 implied pairs, "Monotonicity" in `search.md`), so a strictly stronger holder already scores at least as high on it. PEREDUR's path count was not monotone, which gave demotion something to add there. Demotion is built anyway as an arm that tests this argument, because PEREDUR's rerun shows a small effect (+0.012). Step scaling and bisection need chains of tick counts, which plain LTL has only as `X` chains. PEREDUR's rerun found neither step scaling nor the parent-verdict filter significant (Holm-corrected p ≥ 0.43).

## PEREDUR's order features

Three of PEREDUR's operators are now ported: status-gated direction, *maximality demotion* and the *output climb*. All three are off by default, and `tuned` is unchanged. With all three off, `search_bench` printed the same JSON as the previous commit, timings aside, on 3 seeds and both status modes of the ack model.

**Why these three.** PEREDUR's rerun of the pooled strength pass (`experiments/fretish-order-pooled-v2`, collected 2026-11-11) has 2,132,728 comparison rows, and none crashed. It was reduced locally with `scripts/pooled_reduce.py --no-balls`. The first pass of 2026-11-09 was voided by an audit: its climb pools were read from `maximal.tsv`, which misses the climb's output, and 73% of its comparison rows were crashes. The rerun compares each single-feature arm with the base GA over 18 subjects. The measure is the net subsumption share, positive when the arm is stronger, and p is Holm-corrected.

| Arm | Net subsumption share | Subjects (+/−) | Holm p |
|---|---|---|---|
| only-climb | +0.920 | 18/0 | 5e-5 |
| only-gate | +0.188 | 18/0 | 5e-5 |
| only-demote | +0.012 | 13/5 | 0.038 |
| full | +0.371 | 18/0 | 8e-6 |

The climb gave the largest gain. Crossover, step scaling, the parent verdict and blame were not significant (Holm p ≥ 0.43). The diversity of the rerun has not been reduced yet. The run-phase measures move little. On the rad cores the base found-repair rate is 0.940, and only-gate and only-xover each lose 8–9 runs and gain 3. On the small set, the runs whose repair implies an ideal one, out of 270, are 94 for base, 112 for only-climb, 101 for only-gate, 99 for only-demote and 113 for full.

**Steering.** `Params::steering` takes a `Steering` enum in place of the `weaken_only` bool, and `search_bench --monotone-direction both|weaken|gated` selects it. `Coin` is the default, and `Weaken` is the weaken-walk arm. Under `Gated`, a parent that holds strengthens and any other parent weakens. Under the default weakening filter, a holder that φ is not proved to imply scores 0, so it weakens. An Unknown verdict also scores 0, so its parent weakens too. This departs from the earlier design, which kept the coin for it, because `Scored` cannot tell Unknown from Fails. A crossed child keeps the coin. The coin is drawn under every variant, so the rest of the random stream does not depend on steering. Only the monotone rewrite is gated, and non-monotone mutations are not checked for direction. PEREDUR's gate also returns the parent in place of a free rewrite proved to move the forbidden way, and GAWAIN has no such check.

**Maximality demotion.** `Params::demotion`, behind `--demotion on`, is implemented by `Demoter` in `src/search/demotion.rs`. At every selection, each distinct holder that another distinct holder of the pool strictly implies moves one front down. `nsga2::crowded_order` takes a `demoted` slice for this and adds 1 to the rank, as PEREDUR does. Each direction of a pair first meets the 256-word *fingerprint* test, `Fingerprint::may_imply`. Spot decides the remaining pairs in batches of 128 under the search's Spot limits, and every verdict is memoised for the run. A pair that is not proved never demotes. Demotion is GA-only, since `random_search` has no selection during the run. Under a feasible-first status a demoted holder can share a front number with failing candidates. The holder/failure cut in `select` is unaffected.

**Output climb.** `search::gate_climbed` runs behind `--climb on`. The gate is split into `maximal` and `finish`. `maximal` keeps the holders that φ implies and that are not valid modulo I, then takes the maximal ones. `finish` sets the vacuity flags and the simplified forms. `gate_climbed` runs `maximal`, then climbs from each maximal repair r. The candidates are every strengthening that `monotone_outcomes` lists under the run's rule set, with each literal over φ's atoms (an atom or its negation) as the added operand. Each candidate c that passes `filter` and is unseen goes to the oracle. For each c that holds, Spot is asked φ ⇒ c and r ⇒ c. c is a step when the first is proved and the second is not, so every step is strict. The step most similar to φ becomes the new r, and ties go to the smaller formula. The climb stops when no step passes or after 32 steps. No validity check is needed, since a strengthening of a repair that is not valid modulo I is not valid modulo I either. The tops are scored and join the repairs, `maximal` runs again over both, and then `finish` runs. The JSON gains `climb_steps`, `climb_checked` and `climb_pairs`, and demotion adds `demoted` and `demotion_pairs`.

**Tests.** The unit tests are `the_gate_strengthens_holders_and_weakens_the_rest` in `src/search/mod.rs`, `a_demoted_individual_drops_one_front` in `nsga2` and `a_pool_the_fingerprints_separate_asks_spot_nothing` in `src/search/demotion.rs`. `tests/search.rs` adds `a_strictly_implied_holder_is_demoted` (Spot), `the_climb_reaches_strict_proved_strengthenings` (nuXmv and Spot) and `search_bench_runs_the_order_arms`. The climb test uses a three-state cycle model with φ = `G(s_get -> X(s_send & s_get))` and the start repair `G(s_get -> X F s_send)`. Under `RuleSet::Peredur` the climb checked 47 strengthenings and took 1 step, to `G(s_get -> G F s_send)`, which is still vacuous in `s_get`. Under `RuleSet::Ordered` it checked 50 and took 1 step, to `G(s_get -> X s_send)`, which is not vacuous. The `search_bench` smoke test runs the ack model at 20 × 30 with all three arms and `--rules ordered`. There, demotion demoted 82 candidates over the run with 110 Spot pairs, and the climb checked 29 strengthenings and found no step. No arm has been run under the paper protocol yet ("Proposed order of work").

## The operator order, measured

`bench/operator_orders.py` computes the order between operator templates exactly. A *template* is a formula over the atoms p and q, such as `p U Xq`. Validity is closed under uniform substitution, so a template implication holds for all operands exactly when it holds on the atoms. A countermodel on the atoms shows that the implication is not schematic. Each pair is decided as the unsatisfiability of A ∧ ¬B in one `ltlfilt --satisfiable` batch, and the whole run takes 2 s. The covers are in `bench/results/operator-orders-ltl.csv` and the interval checks in `bench/results/operator-orders-mtl.csv`.

### LTL templates

The unary set is p under every word of up to three operators from {X, F, G}, 40 templates in 12 equivalence classes. The binary set applies each of `U`, `W`, `R`, `&` and `|` to p and q, each wrapped in nothing, X, F or G, plus each binary node under X, F or G and six small templates such as `Fq` and `Gp`. That makes 101 templates in 78 classes. `M` is left out, because GAWAIN's parser rejects it. The script takes the *covers* of the order (pairs A ⇒ B with no class strictly between them) and asks whether one rule of `rules_at`, without `Constant` and `AddOperand`, rewrites some member of A's class into a member of B's. It reads the rewrites from the Rust code through the `rule_table` binary, which prints every outcome of `monotone_outcomes`, so the counts describe the code and not a Python copy of the rules. Build it first with `nix develop -c cargo build --release --bin rule_table`.

| Set | Classes | Covers | Under `RuleSet::Peredur` | Under `RuleSet::Ordered` | Unsound steps |
|---|---|---|---|---|---|
| Unary | 12 | 15 | 9 | 15 | 0 |
| Binary | 78 | 202 | 90 | 191 | 0 |

Every current rule is sound, but many of them skip levels. 17 of the unary steps and 123 of the binary steps join two classes with another class between them. `G φ → F φ` is the clearest case, since G φ ⇒ φ ⇒ F φ and the middle step has no rule. The same holds for `X φ → F φ`, which skips `F X φ`.

`RuleSet::Ordered` (`src/search/operators.rs:85`) adds rules to PEREDUR's, all applied at any site of known polarity. `Params::rules` and `search_bench --rules peredur|ordered` select the rule set, and the default is `RuleSet::Peredur`, PEREDUR's rules alone. The frozen `tuned` configuration and the trace benchmark generator `trace_bench` stay on `Peredur`. A 20,000-mutation stream at monotone share 1.0 is byte-identical to the build before the change, so the frozen results reproduce. The first six rule pairs are weakenings and their strengthening duals:

| Weaken | Strengthen |
|---|---|
| φ → F φ | F φ → φ |
| G φ → φ | φ → G φ |
| φ U ψ → F ψ | φ U ψ → ψ |
| φ W ψ → φ ∨ ψ | φ W ψ → ψ, φ W ψ → G φ |
| F G φ → G F φ | G F φ → F G φ |
| | φ R ψ → φ ∧ ψ |

With those six alone, every unary cover and 178 of 202 binary covers are one rule away. Nine more rule pairs come from the 24 covers that remain. Each weakens at the left shape of its law and strengthens at the right shape:

| Law | Rules |
|---|---|
| φ R ψ ⇒ φ ∨ Xψ | `ReleaseToNext` / `NextToRelease` |
| φ ∧ Xψ ⇒ φ U ψ | `NextToUntil` / `UntilToNext` |
| F(φ ∧ ψ) ⇒ Fφ ∧ Fψ | `SplitEventually` / `JoinEventually` |
| Gφ ∧ Fψ ⇒ F(φ ∧ ψ), Fφ ∧ Gψ ⇒ F(φ ∧ ψ) | `MeetEventually` / `SplitMeet` |
| F(φ R ψ) ⇒ φ R Fψ, F(φ W ψ) ⇒ Fφ W Fψ | `EventuallyInward` / `EventuallyOutward` |
| φ U Gψ ⇒ G(φ U ψ), φ W Gψ ⇒ G(φ W ψ) | `GloballyOutward` / `GloballyInward` |

With all fifteen pairs, every unary cover and 191 of 202 binary covers are one rule away, and the CSV column that was `proposed_step` is now `ordered_step`. The 11 covers still missing are `F(p & q) => Fp U q`, `Fp R q => Fp | Gq`, `G(p U q) => Gp R Fq`, `G(p W q) => Fp W Gq`, `Gp R q => G(p W q)`, `p & Fq => p | Gq`, `Fp U Gq => F(p R q)`, `Fp U Gq => Gp R Fq`, `Gp & Fq => Gp U q`, `Xp & Fq => Xp | Gq` and `Xp W Fq => F(p W q)`. These are not general laws. Several hold only because of the operand shape. For example, Fp R q ⇒ Fp ∨ Gq holds because ¬Fp means p never holds, while φ R ψ ⇒ φ ∨ Gψ is false in general. `p & Fq => p | Gq` is a cover only because the template set lacks p on its own. The full LTL implication order over fixed atoms has no general covers between distinct shapes in the sense used here. "Reaches every cover" is therefore relative to the template set, and is not a completeness theorem.

No rule carries a weight, and each applicable rule is drawn uniformly, as before. The two introductions, F-introduction and its dual G-introduction, can fire at every node, so they take a share of every draw. The bloat cap and `simplify` remove `F F` and `G G`, but each such wrap still costs a slot.

### MTL intervals

Part B checks closed-form rules over all intervals [a, b] with a ≤ 4 and b ≤ 4 or b = ∞, which gives 20 intervals. `Fi` and `Gi` are expanded to `X` chains by the script, and all 40 expansions are equivalent to Spot's own `F[a:b]`, `G[a:b]`, `F[a:]` and `G[a:]`. `Ui` is the usual bounded until, `Uc` is CEGIW's, and `Ri` is the dual of `Ui`. Every rule matched Spot on every pair.

| Rule | Pairs | Mismatches |
|---|---|---|
| F_I p ⇒ F_J p iff I ⊆ J | 400 | 0 |
| G_I p ⇒ G_J p iff J ⊆ I | 400 | 0 |
| G_I p ⇒ F_J p iff I ∩ J ≠ ∅ | 400 | 0 |
| F_I p ⇒ G_J p iff I = J = [k, k] | 400 | 0 |
| p Ui_I q ⇒ p Ui_J q iff I ⊆ J | 400 | 0 |
| p Ri_J q ⇒ p Ri_I q iff I ⊆ J | 400 | 0 |
| p Uc_I q ⇒ p Uc_J q iff a_I = a_J and I ⊆ J | 400 | 0 |
| p Ui_I q ⇒ F_J q iff I ⊆ J | 400 | 0 |
| G_J q ⇒ p Ri_I q iff I ⊆ J | 400 | 0 |
| p Ui_I q ⇒ p Uc_I q | 20 | 0 |
| p Uc_I q ⇒ p Ui_I q iff a = 0 | 20 | 0 |
| F_I F_J p ≡ F_{I+J} p, both ways | 72 | 0 |
| G_I G_J p ≡ G_{I+J} p, both ways | 72 | 0 |
| F_I G_J p ⇒ G_K F_L p iff I − K ⊆ L − J | 1,296 | 0 |

Here I + J is the interval sum [a + c, b + d], and I − K is [a_I − b_K, b_I − a_K]. The grid covers bounds up to 4, and the nesting rules only bounded intervals with b ≤ 2. Both halves of every rule are now proved in Lean for every interval, unbounded ones included, except the last row, which is proved for every bounded interval ("Proofs" below). The grid therefore serves mainly to confirm the expansions.

The rows give the structure of the order. On one atom, the `F` and `G` families are each ordered by interval inclusion, and they meet only at the singletons, where `F[k,k]`, `G[k,k]` and `X^k` coincide. Every cross implication G_I ⇒ F_J passes through some X^k with k ∈ I ∩ J. So the covers of the combined order are the single-bound steps of one interval, and GAWAIN's `X → F` and `X → G` rules are the case I = [1, 1]. In the `F` family an unbounded interval has no cover from below, since every F[a, b] lies below F[a, ∞), so the move from a bounded to an unbounded interval is a jump. CEGIW's semantics gives `U` the weaker order of its own Lemmas 7 and 8, where the left bound cannot move. Under the usual semantics, `U` follows inclusion in both bounds, as `F` does. GAWAIN should therefore use the usual semantics. CEGIW's case studies use only `F` and `G`, where the two readings agree, so the comparison with CEGIW is not affected.

## A trap in the metrics

Fact 3 lets one output dominate any maximal set. The conjunction of all of a run's maximal repairs is itself a repair that implies every member. An arm that reported it would score at least as well as its own set on net subsumption, and its set of one repair would never enter the diversity test, which needs more than 3 repairs per side. The join crossover moves the search towards this, limited only by the bloat cap. Any arm with the join should therefore report repair size beside net subsumption, and the readable share of the hand classification (68 of 95 at the frozen configuration) should be rechecked on its repairs.

## MTL

### The order on intervals

The order needed is wider than CEGIW's. Take discrete pointwise time, intervals [a, b] with a ≤ b and b possibly ∞, and the usual reading of `U`, where φ holds on every position from the start up to the witness. Then the following hold at a positive occurrence, for I ⊆ J. Each row is checked against Spot in "MTL intervals" above and proved for every interval in `proofs/Gawain/Interval.lean`.

| Stronger | Weaker | Note |
|---|---|---|
| `F_I φ` | `F_J φ` | CEGIW's Lemma 5 for right bounds; the left bound also moves |
| `G_J φ` | `G_I φ` | CEGIW's Lemma 6 for right bounds |
| `φ U_I ψ` | `φ U_J ψ` | under the usual reading only |
| `φ R_J ψ` | `φ R_I ψ` | dual of the row above |
| `G_I φ` | `F_I φ` | I is never empty |
| `G φ` | `G_I φ` | generalises CEGIW's contraction of an unbounded `G` |
| `F_I φ` | `F φ` | |
| `φ U_I ψ` | `F_I ψ` | |
| `G_I ψ` | `φ R_I ψ` | |

`X^k φ`, `F[k,k] φ` and `G[k,k] φ` are equivalent. The single steps on an interval are a − 1 and b + 1 to weaken an `F` or `U`, and a + 1 and b − 1 (while a < b) to strengthen it, with the duals for `G` and `R`. These extend `rules_at` (`src/search/operators.rs:627`) as new rules on the same polarity machinery, since all four operators are monotone in their operands.

The FRETISH timings `within n`, `for n`, `next`, `always` and `eventually` read as `F[0,n]`, `G[0,n]`, `X`, `G` and `F`, and PEREDUR's `global_timing_implies` (`src/requirement.cpp:427`) orders them with a Lean proof. Its edges among these five give a ready-made test table for the rows above.

### The oracle

Spot 2.15.1 parses `F[2:4] a`, `G[1:3] a` and `X[3] a` and expands them to `X` chains at parse time. On the local install, `ltlfilt -f 'F[3:3] a' --equivalent-to 'X X X a'` and the implication checks `G[0:5] a ⇒ G[1:3] a` and `F[1:3] a ⇒ F[0:5] a` all pass. Spot has no bounded `U`, so `φ U[a,b] ψ` needs its own expansion, `G[0,a−1] φ ∧ X^a (φ U[0,b−a] ψ)`, which stays linear in b. The intervals should stay in the abstract syntax tree and expand only at the oracle boundary, so that mutation sees one node per interval and the bloat cap measures the formula as written.

The expansion costs grow with b. Each tool sees a formula of size linear in b per operator, multiplied through nesting, and Spot's automaton for the explicit tier grows with it. The lasso evaluator can read an interval directly over a lasso's positions without expanding it. The SPIN back end refuses `X` except on stepwise Promela models such as BEEM, so MTL candidates reach SPIN only there. Intervals also mean time only on models where one step is one tick, which holds for nuXmv's synchronous models and not for RERS's interleaved processes.

The syntactic similarity needs one change: two interval operators of the same kind should match with partial credit, for example by the overlap of their intervals, so that `F[1,3]` and `F[1,4]` score as close. The lasso count needs none, since it runs on the expanded automaton.

### Benchmarks

Benchmarks are the weak point. The current evaluation sets have almost no intervals. CEGIW's case studies are 8 cases over 2 formulas and 4 small nuXmv models, plus abp4 `p2false`, whose `X` reads as `F[1,1]`. The MCC formulas contain `X` chains, such as `G(X X X X a0 U …)` in 16 of the 20 formulas sampled in `bench/notes/baselines.md`, but they are random formulas over Petri net firings, so a weakened interval says little. CEGIW's FRET survey counts 124 or 127 interval-weakenable requirements out of 419 (the paper gives both), with no models. LMCPS has 74 discrete-time requirements, about 23 violated, but its models need MATLAB ("Investigate LMCPS" in `PLAN.md`). An MTL evaluation therefore needs a new set first, either hand-translated LMCPS models or FRET requirements paired with models written for them.

## Proofs

The orders above are now proved in Lean, using core Lean with `omega` and no Mathlib (`proofs/`, detail in `proofs/README.md`). Every interval row of "MTL intervals" is an exact `↔`, with a countermodel where the condition fails, for every interval including unbounded ones. `F_I F_J` and `G_I G_J` sum their intervals for any well-formed pair, and the `F G`/`G F` row holds for every bounded interval. Chains of single-bound steps reach exactly the `F` and `G` interval orders, so the steps of "The order on intervals" are complete for interval weakening once unbounding the right bound is added to them. Every unary word over `X`, `F` and `G`, of any length, reduces to one of five *normal forms*, which decides the unary order beyond the three operators measured. Every rule of both rule sets moves the whole formula in its stated direction through any context of known polarity, and so does every outcome that `monotone_outcomes` lists. Facts 1 to 4 of "What changes under model checking" are proved over a model taken as a set of traces.

`bench/check_lean_orders.py` compares the Lean verdicts with Spot. All 5,080 MTL cases of `bench/results/operator-orders-mtl.csv` agree, 1,661 of them implications, and so do all 1,600 pairs of unary words of up to three operators, 810 of them implications. As controls, 996 MTL cases disagree when the Spot verdicts are read one row out of step, and 1,244 unary pairs disagree when each pair is swapped.

The same script compares the Lean outcome list `outs` with the Rust `rule_table` on every formula of depth at most 2 over the atoms a and b plus the constants. That grid holds 10,264 distinct formulas, and two directions and two rule sets make 41,056 rows with 652,052 outcomes. None differ, and every row lists its outcomes in the same order. As a control, all 41,056 rows differ from the other rule set's list. On the Spot side, `every_ordered_outcome_moves_the_way_it_says` in `tests/search.rs` checks 7,297 outcomes on 207 subjects, and all move in their stated direction.

The binary LTL order stays measured only. Its rules are proved sound, but the order between binary templates and the reach of `RuleSet::Ordered` (191 of 202 covers) come from `bench/operator_orders.py` alone. The Lean transcription of the rules and of `collect_polarities` is tied to the Rust by running both on the grid above, which is a finite grid and not a proof about the Rust source. The `F G`/`G F` rule is proved for bounded intervals only. The interval and gating theorems are about semantics, since GAWAIN has no MTL parser or oracle yet and the gating proofs do not cover the lasso cache code.

## Proposed order of work

1. The rules of "LTL templates" are implemented as `RuleSet::Ordered`, unweighted, behind `search_bench --rules ordered`, and `tests/search.rs` checks their direction. What remains is the arm run against `tuned` under the paper protocol. This is a change to the operator set, so it is measured as its own arm before the gate.
2. Status-gated direction (`--monotone-direction gated`), maximality demotion (`--demotion on`) and the output climb (`--climb on`) are built ("PEREDUR's order features"). What remains is to run each of them, `--rules ordered` and their combinations against `tuned` under the paper protocol (30 seeds, net subsumption, found rate and diversity). One arm at 30 seeds is 6,780 runs; Phase 2 ran 13,560 runs in 3 h 40 min across av2 and av3. The climb arm is also measured against its wall-time cost in `gate_wall`.
3. Counterexample-guided sites, then the join crossover, both still unbuilt, each as its own arm against the winner of step 2. The join arm reports repair size as well.
4. MTL, once a benchmark set with timed requirements exists. Within it, first the interval order and the expansion at the oracle boundary, then CEGIW's per-lasso interval step as a mutation, then CEGIW itself rerun as a baseline.

The LTL operators are cheap to try, because the polarity machinery, the lasso tier and the evaluation protocol already exist. The MTL extension depends on benchmarks more than on code.
