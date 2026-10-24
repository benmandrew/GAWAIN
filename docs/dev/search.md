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

`Status::Graded` (`src/search/grade.rs`) replaces the 0 of a candidate that is not proved with a grade. Let n be the number of lassos the oracle has stored that violate φ, and k the number of those that φ' satisfies. A candidate the oracle proves scores 1, and any other scores k / (n + 1). φ satisfies none of its own counterexamples, so it scores 0, and for a fixed set of lassos the grade is monotone under implication. It costs no back-end call, since the lassos are runs of M that tier 1 already keeps, and it works on every back end that returns counterexamples.

Three choices fix the grade.

- **The +1.** A `Fails` or `Unknown` candidate can satisfy all n stored counterexamples, because M's run that refutes it may not be stored yet. The denominator n + 1 keeps every such candidate below 1, so a proved candidate always beats one that is merely consistent with the stored runs. With no stored lasso the grade is 0, the binary value.
- **No deduplication.** Tier 1 stores a back-end counterexample only when no stored lasso already refutes the query, so a second copy of a stored lasso can never be added. Lassos that differ but agree on every candidate still count separately, and the grade does not try to merge them.
- **Rescoring.** The set only grows, so a grade computed in generation 3 is out of date in generation 10. Before each selection `Grader::rescore` sorts the new lassos into those that violate φ and regrades every member of the pool that does not hold, parents included, against one set.

The ablation under "Measured runs" found that the grade crowds out the candidates that hold. Under the binary status φ itself (status 0, both similarities 1) dominates every candidate that fails, so the front holds φ plus the candidates that hold. Under the grade a failing candidate that satisfies some counterexample has status above 0, so φ no longer dominates it, and it takes a place on the front. The final population held a median of 3.25 fewer proved candidates out of 40, while the found-repair rate and the strength of the repairs did not change. The default is therefore `Status::Binary`, and `search_bench --status graded` keeps the grade for the ablation. The `weaken` binary has no flag for it.

Two variants were also measured on RERS and dropped. Their code is at commit `ddd9648` on the unmerged branch `exp/graded-arms` (`src/oracle/explicit.rs` there).

- **B with random runs.** Before the search, 64 random walks of M (seeded from the run's seed, at most 10,000 steps each) were stored in the oracle, so the grade had more counterexamples to work with from generation 0. Few walks violate φ (a median of 7 refuting lassos per run against 3), and the crowding was the same.
- **Option C, the share of states.** The status was h / (N + 1), where h is the number of M's N states from which φ' holds, from a product of M with ¬φ' and a backward pass over its bad strongly connected components. The share does not depend on φ, so it rewards formulas that are not weakenings of φ at all. On RERS 2016 103 p7, seed 1, the gate kept none of the 21 candidates that held. Each new formula costs one Spot translation and one product, which took a median of 70% of the search's wall time.

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

The 86 instances are those where both arms have at least one paired seed. MedleyA-PT-09 `LTLCardinality-09` ran out of memory on every seed of both arms, and six large BEEM instances (anderson 3-2, 3-4, 5-2, 5-4 and 7-4, and lamport 8-4) ran only seeds 1 to 3, of which most reached the 3600 s cap without a result. Over the whole sweep 76 binary and 75 graded runs gave no result, at a cap of 8 GB. A rerun of those at 32 GB recovered 11; the rest were killed by the out-of-memory killer again, in a Spot or Ganak child. The losses are spread evenly over the arms.

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
