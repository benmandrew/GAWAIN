# GAWAIN: weakening temporal specifications by genetic search

Since 23 October I have been building GAWAIN, short for Genetic Algorithm for Weakening Against Implementations under NSGA-II (the Non-dominated Sorting Genetic Algorithm II). It is a Rust tool that takes a *linear temporal logic* (LTL) property φ that a model M violates, and searches for a weaker property φ' that M satisfies. It reuses the genetic search of my earlier tool [PEREDUR](https://github.com/benmandrew/PEREDUR), which repairs unrealisable specifications, with the realisability check replaced by a call to a model checker. This write-up covers what the tool does, how I benchmarked a problem that has no benchmark, and how the search compares with random mutation under the same budget.

## The problem

The input is a model M and a property φ with M ⊭ φ. The output is a *weakening* φ' with φ ⇒ φ' and M ⊨ φ', and it should be as strong as possible. The constant ⊤ always qualifies, so finding some weakening is trivial, and all of the work lies in finding a strong one.

The only paper I found that states this problem directly is [counterexample-guided interval weakening](https://arxiv.org/html/2604.24540) (CEGIW), which weakens interval bounds only. ["Weakening Goals in Logical Specifications"](https://arxiv.org/html/2510.15718) works in propositional logic and evaluates on a single example with 5 variables. So no public benchmark exists, and I built one from neighbouring problems. There is no ground truth either, which turned out to matter less than I expected, because ⊤ gives a fixed floor and repairs can be ranked by their strength against it and against each other.

One trap shaped the whole benchmark. If φ fails on every run of M, so that M ⊨ ¬φ, then on M the formula ¬φ ∨ χ is equivalent to χ, and any weakening that holds is just some truth of M to which φ contributes nothing. I call such an instance *degenerate* and drop it with one extra model-checking call on ¬φ. Benchmark constructions that negate a true property produce exactly this case, so they are excluded wholesale.

## The approach

### The search

An individual is one formula, and the seed population is P copies of φ. Each of G generations runs four steps, with PEREDUR's operators and rates, which I have not tuned for this problem.

**Breed.** Each slot starts as its parent, is crossed with another member at rate 0.1, is mutated at rate 1.0, and is simplified. Mutation draws atoms from φ only, and a quarter of mutations are *monotone rewrites* that move one subformula in a chosen direction of strength.

**Filter.** Repeats, formulas more than twice φ's size and formulas that Spot proves valid modulo the encoding are dropped before any oracle call.

**Score.** Each survivor gets three objectives, described below.

**Select.** Parents and offspring are pooled, and NSGA-II ranking cuts the pool back to P.

The output gate enforces φ ⇒ φ' once, at the end, with one cached implication check per output. The search never discards a candidate for failing it, so candidates can pass through formulas that are incomparable with φ on their way to a weakening.

Since the evaluation, the search also demotes a candidate that holds on M but that φ is not proved to imply, scoring it as if it had failed. This *weakening filter* (`--weakening demote`) acts on scoring only, and it is now the default. On the model set it left the found rate level and cut the median back-end calls by 9.75 per run, and on the trace sets it found a repair on 1,154 of 1,260 runs against 1,124. The evaluation numbers in this report predate it.

### Objectives

The first objective is the *status*, 1 when the oracle proves M ⊨ φ' and 0 otherwise. The second is syntactic similarity to φ. The third is a model-counting similarity to φ, which is the measure of strength that the evaluation ranks repairs by.

The count took three attempts. PEREDUR counts accepting paths in an automaton, and on 1,200 implied pairs built by monotone rewrites that count broke the implication order on 26. A count of prefixes was monotone, but a finite prefix cannot see liveness, so it tied every liveness weakening with ⊤. The count I kept is the number of *lassos* u·v^ω with |u| + |v| = 20 in the formula's language, combined over φ, φ' and φ ∧ φ' by PEREDUR's logarithmic metric. It gave 0 violations on the same 1,200 pairs, and ⊤ gets the maximum. It was also slow at first, and on LMCS abp4 the search waited 26.8 s for it, until interning the rows of its transition monoid cut the count from 36.6 s to 9.6 s and the wait to 2.3 s, with identical repairs.

For reporting I also use the *rejection strength*, log count(¬φ') / log count(¬φ), the log share of φ's violating lassos that φ' still rejects. ⊤ scores 0 and φ scores 1. It ranks repairs as the similarity does, with a median Kendall τ of 1.000 over 85 instances, but it spreads them out where the similarity sits next to ⊤. It is computed once per repair after the gate and never steers the search.

### The oracle

Each run asks "does M ⊨ φ'?" about a thousand times, and 1,122 times on abp4 at 30 generations of 40. A model-checking call costs a median of 0.87 s on SPIN (Simple Promela Interpreter) and 1.23 s on nuXmv, so the oracle answers from the cheapest tier that can.

| Tier | Check | Answers | Measured cost or effect |
|---|---|---|---|
| 0 | Memo keyed on a canonical form | both verdicts | a hash lookup |
| 1 | Evaluate φ' on every stored counterexample lasso of M | fails | refuted 95.2–95.7% of failing mutants on RERS 2016 problems 101 and 104 |
| 2 | Lasso fingerprints, then Spot implication from known truths | holds | 74–80 µs per pair, batched; proved 62% of true mutants on problem 104 |
| 3 | A back end: explicit Kripke product, nuXmv, SPIN or TAPAAL `verifypn` | both verdicts | 11 ms translation and 0.3 ms product on RERS 2016 problem 104, in process |

Every failing back-end call stores its lasso for tier 1, and every success feeds tier 2. The cache key never renames atoms, which PEREDUR does soundly, because here the atoms name observations of M and `G !p_a` and `G !p_b` can have different verdicts. The tiers were first sized on mutants from Spot's `ltlgrind`. On real genetic algorithm (GA) populations the cheap tiers answered 91.6% of 742,994 queries over 838 runs, and in the headline run 92.1% of 963,457, with 22.0% from the memo, 44.0% from the lassos and 26.1% from implication.

The explicit back end is a Rust front end that explores a RERS Promela model into a *Kripke structure* in memory and checks the product with Spot's automaton for ¬φ'. It explored RERS 2016 problem 107, at 64 million states, in 53.6 s and 9.8 GB on av2, and its verdicts matched the official ones on 100 of 100 properties of problems 101 to 105.

### The output gate

The gate runs once, on the final population. It keeps the proved candidates, asks Spot to prove φ ⇒ φ', drops any candidate that the *encoding invariant* I implies, drops any candidate that another survivor strictly implies, and flags *vacuity*. I says that atoms naming values of one variable never hold together, so `G(s_get & s_wait -> …)` on abp4 counts as trivial. Every reported repair therefore has M ⊨ φ' proved by the oracle and φ ⇒ φ' proved by Spot.

Getting triviality right took three goes. With only a syntactic validity check, the first runs reported one repair on each of two models, and both were valid. Dropping every vacuous repair then dropped every repair on abp4, including `G(s_get -> X F(s_wait & …))`, the `U` of φ weakened to an eventuality, so vacuity became a flag. Last came the initial state. A repair such as `G ψ | resting_p` holds on CEGIW's foraging robots only because they start at rest, so the gate also drops φ' when I together with the initial letters of M implies it. That check marked 867 of 1,771 ablation repairs, and each one I read by hand was trivial. Filtering inside the loop raised the found rate from 0.70 to 0.90 with p < 0.0001, at 10.5 more back-end calls per run, and it has been the default since 26 October.

## Benchmarks

The model set comes from four sources, each with its own back end.

| Family | Instances | Back end |
|---|---|---|
| RERS 2016 parallel | 52 | explicit |
| RERS 2018 problem 101 | 14 | explicit |
| MCC | 20 | TAPAAL `verifypn` |
| BEEM ERROR pairs | 20 | SPIN |
| LMCS abp4 `p2false` | 1 | nuXmv |

The [Rigorous Examination of Reactive Systems](https://rers-challenge.org/) (RERS) challenge gives the most credible properties, randomly instantiated Dwyer patterns that read like requirements. Of 472 false properties in its parallel track, 401 survive the degeneracy filter, 6 are degenerate and 65 stay unknown. The Model Checking Contest (MCC) gives volume, with random nested `X` and `U` formulas on Petri nets. LTSmin's traces there were wrong and ITS-Tools prints none, so I used TAPAAL's `verifypn`, replayed every FALSE as a lasso and confirmed every TRUE with partial-order reduction off, since the default gave one wrong TRUE. On 9,942 FALSE oracle formulas it agreed on 9,779 and disagreed on none.

The BEnchmarks for Explicit Model checkers (BEEM) set ships deliberately buggy variants, which match the framing of a real requirement on a degraded system. Its Promela files break the atomicity of a step in DVE, the input language of the DiVinE model checker, so I wrote a new DVE-to-Promela translator, and all 113 verdicts that BEEM publishes agree with it. Of 96 pairs, 20 fail only on the erroneous variant, and 15 of the 20 are `G F p`. The last model instance is abp4 `p2false` from the 2006 benchmarks of a paper in *Logical Methods in Computer Science* (LMCS), whose files needed rewriting before nuXmv 2.1.0 would accept them.

The evaluation excludes 7 of these 107 instances, as the ablations did. Six are large BEEM instances that gave no result inside the time cap, and MCC MedleyA-PT-09 failed on every seed, so 100 model instances remain. Three smaller sets go beside them. The 126 trace-set instances replace M by positive traces, 5 per family from the LTLf Learning Benchmarks, Flie and Scarlet, where LTLf is LTL over finite traces. CEGIW's 8 case studies plus abp4 give a direct comparison, and the 13 RERS "precedes" instances give a reference answer.

## Evaluation against random mutation

The baseline asks whether selection earns its cost. Random mutation uses the same operators, filters, objectives and gate, and breeds the same 1 + G × P candidates, with parents drawn uniformly from an archive of every distinct candidate so far. Its final P are chosen by the same NSGA-II step, so the two arms differ only in selection during the run. Models ran at 30 generations of 40 and trace sets at 30 of 30, with seeds 1 to 10, 120 s and 4 GB per back-end call, and 3,600 s and 8 GB per run. The pairs are by instance and seed, the p-values come from Wilcoxon signed-rank tests over instances, and "higher/lower/equal" counts the instances on which the GA is higher than mutation.

```
F='^(?!beem-anderson\.(3-2|3-4|5-2|5-4|7-4)$|beem-lamport\.8-4$|mcc-MedleyA-PT-09-)'
python3 -I bench/graded_run.py bench/data/eval 8 --seeds S --arms default,mutation --out runs-eval --filter "$F"
python3 -I bench/graded_run.py bench/data/eval-traces 16 --seeds 1-10 --arms default,mutation --budget 30x30 --out runs-eval-dfa --memory-gb 16
```

The *found rate* is the share of runs that report a repair. *Net subsumption* pools each arm's repairs over the seeds, keeps the *maximal set* that no other pooled repair strictly implies, and takes the share of mutation's set that a GA repair strictly implies, minus the converse.

### Models

The first remote attempt failed in an instructive way. It picked up Spot 2.14.5 from `/usr/local/bin` on the remote hosts, which lacks `ltlfilt --satisfiable`, so the gate failed and all 1,992 model runs reported no repairs. `bench/graded_run.py` now refuses any Spot but 2.15.1. On the rerun, 1,994 of 2,000 runs gave a result, and the other 6 were on SieveSingleMsgMbox at the 3,600 s cap.

| Measure | Mutation | GA | Paired difference | p | Higher/lower/equal |
|---|---|---|---|---|---|
| Found rate | 0.70 | 0.85 | mean +0.085 | <0.0001 | 36/7/57 |
| Best semantic score | 0.994 | 0.995 | | 0.0004 | 31/14/35 |
| Net subsumption | | | median 0.000, mean +0.188 | <0.0001 | 38/7/35 |
| Back-end calls | 55 | 65.75 | +6.0 | <0.0001 | 64/28/8 |
| Wall time | 9.0 s | 11.9 s | +0.7 s | 0.029 | |

The GA finds a repair more often and its repairs are stronger, for 6.0 more back-end calls and 0.7 s more wall time per run. The gain comes from RERS and MCC. On the 66 RERS instances the found rate is 0.60 against 0.50 with p = 0.0001, and on the 19 MCC instances net subsumption has a median of 0.361 with p = 0.0092. BEEM is level on every measure.

Against ⊤, the *closed share* (best − ⊤)/(1 − ⊤) under the semantic score has a median of 0.122 for the GA and 0.046 for mutation, and under the rejection strength 0.971 against 0.953. No repair is valid, so every one is strictly stronger than ⊤. On BEEM, however, 100 of the 111 runs that found a repair score exactly ⊤'s semantic value.

### Trace sets

All 2,520 trace runs gave a result. They ran on the `ltlf2dfa` oracle, after `ltlfilt --from-ltlf` had left 27,727 implication pairs undecided on a first run, against 433 now.

| Measure | Mutation | GA | Paired difference | p | Higher/lower/equal |
|---|---|---|---|---|---|
| Found rate | | | | 0.66 | 17/11/98 |
| Best semantic score | 0.989 | 0.989 | | <0.0001 | 57/6/55 |
| Net subsumption | | | median 0.077, mean +0.152 | <0.0001 | 64/24/30 |
| Back-end calls | 661 | 684 | | 0.81 | |
| Wall time | 1.815 s | 1.797 s | median +0.041 s | 0.022 | 78/48/0 |

Both arms find a repair on nearly every run, so the found rate cannot separate them, but the GA's repairs are stronger at the same number of calls. The gain is in ltlf-learning and Scarlet, and Flie is level. The switch to `ltlf2dfa` also cut the runs with gate errors from 318 to 111.

### Budget

`bench/budget_curve.py` compares four budgets on the 198 instance and seed pairs, from seeds 1 and 2, that gave a result at every budget under both arms. Wall time and calls are medians.

```
python3 bench/budget_curve.py bench/data/eval 15x20=runs-b15x20 30x40=runs-eval 60x80=runs-b60x80 120x80=runs-b120x80
```

| Budget | Found, GA | Found, mutation | Wall, GA | Wall, mutation | Calls, GA | Calls, mutation |
|---|---|---|---|---|---|---|
| 15×20 | 0.485 | 0.449 | 3.4 s | 3.0 s | 26 | 23 |
| 30×40 | 0.677 | 0.566 | 11.5 s | 8.3 s | 64 | 50 |
| 60×80 | 0.864 | 0.672 | 39.9 s | 32.1 s | 124 | 120 |
| 120×80 | 0.939 | 0.697 | 78.7 s | 59.0 s | 164 | 180 |

The gap in found rate grows from 3.6 points at 15×20 to 24.2 points at 120×80. Random mutation levels off near 0.70 while the GA keeps gaining, and at 120×80 net subsumption has a median of 0.325 with p < 0.0001. This is the cleanest evidence that selection does something.

### Large BEEM instances

The 6 excluded BEEM instances ran separately at 30×40, with 4 h and 10–16 GB per run. All 120 runs completed and none hit the cap, and the longest took 6,609 s. The GA found a repair on 51 of 60 runs and mutation on 37 of 60. On the paired runs the GA alone found one on 14 and mutation alone on none, which gives p = 0.0001 by a sign test. The whole gap is on anderson 3-2 and 5-2, where the GA found a repair on all 10 seeds and mutation on 3. These instances fail under the headline's 3,600 s cap only.

### A reference weakening on RERS

RERS 2016 and 2017 ship 105 "precedes" properties as G(Q ∧ P), while their English gloss describes the Dwyer pattern G(Q → P). That pattern ψ is a weakening that a person wrote, so it is the nearest thing this problem has to a ground truth. Searches were affordable only on problems 2016/1 and 2017/1, which gave 13 instances, and ψ holds on 7 of them.

On those 7 the GA produced a repair that implies ψ, and so is at least as strong as the intended fix, on 32 of 70 runs, against 19 of 70 for mutation. It was ahead on all 7, with p = 0.016 by a sign test. Most of the other maximal repairs are incomparable with ψ, 482 of the GA's 521, and each of those is another true weakening of φ that a reference answer can count but cannot rank.

### CEGIW's case studies

On CEGIW's 8 case studies plus abp4, the GA found a repair on 66 of 90 runs against 53 for mutation. On both foraging absence cases its single maximal repair is equivalent to CEGIW's interval. CEGIW is 1.4–10× faster. Its abp4 output `F[1,5]` passes its bounded check at depth 20 but fails on M with a 48-state lasso, it returns a tautology on 2 cases, and on the 2 cases where it proves that no interval weakening exists, GAWAIN finds repairs.

### Repair shapes

I classified by hand the best repair on each of the 89 headline instances where the GA found one.

| Shape | Repairs | RERS | MCC | BEEM | abp4 | Readable | Median closed share | Median size |
|---|---|---|---|---|---|---|---|---|
| Local edit | 38 | 32 | 5 | 1 | | 27 | 0.677 | 13 |
| Prefix | 15 | 2 | | 13 | | 14 | 0.001 | 4 |
| Rewritten | 11 | 6 | 5 | | | 1 | 0.390 | 15 |
| Disjunct | 10 | 4 | 6 | | | 9 | 0.863 | 12.5 |
| Eventual | 7 | 7 | | | | 4 | 0.424 | 7 |
| Dropped conjunct | 6 | 4 | 2 | | | 4 | 0.589 | 9.5 |
| Trigger restriction | 2 | | 1 | | 1 | 2 | 0.452 | 14.5 |
| Total | 89 | 55 | 19 | 14 | 1 | 61 | | |

Local edits are the commonest shape and the ones a person would try first, such as `U` relaxed to `W` on rers2018-101-p16 or `G !p` relaxed to `G F !p` inside a response. Disjuncts keep φ whole and score highest. The BEEM prefixes are the clearest weakness, since a `G F p` becomes `X X F p`, which is readable and correct and nearly as weak as ⊤. 61 of the 89 repairs are readable, and the 25 unreadable ones with a defined closed share have a median of 0.619, against 0.493 for the readable ones. Some of the search's strength therefore comes from formulas that nobody can check by eye.

### Simplification

`bench/simplify_stats.py` rewrites each distinct repair with Spot's `ltlfilt -r` and proves the rewrite equivalent in both directions. Over 2,008 distinct model repairs, the median size falls from 13 to 11 nodes and the total node count by 6.9%. The count rises by 4.4% on LMCS, because Spot spells `a -> b` as `!a | b`, so the output keeps Spot's form only when it is smaller.

## What did not work

### Graded status

The binary status gives every failing candidate 0, which looked wasteful. A graded status scores a failing candidate k/(n + 1), where k of the n stored counterexample lassos satisfy it. On 86 instances and 10 seeds it kept 3.25 fewer proved candidates out of 40, with p < 0.0001 and lower on 76 of 86, and changed nothing else. Under the binary status φ dominates every failing candidate, while under the grade a failing candidate above 0 escapes that dominance and crowds out the candidates that hold.

Two variants did worse. Adding 64 random runs of M as extra lassos gave weaker repairs with p = 0.035. Grading by the share of M's states from which φ' holds rewards formulas that are not weakenings at all, and on RERS 2016 103 p7 the gate kept none of the 21 candidates that held, while grading took a median of 70% of the search's wall time. Their code is at the tag `archive/graded-arms`. A third variant ranked every proved candidate above every other, which filled the population with holders and evicted φ and its neighbours, so the found rate fell on 18 instances and rose on 4 with p = 0.0008, and 74 runs ended with 40 proved candidates and no repair.

### A reserve for unproved candidates

The reserve arm kept the proved-first order but held ⌈P/4⌉ slots for the best unproved candidates, so φ always stays. Every run then ended with 30 proved candidates, 6.5 more than binary with p < 0.0001. Overall the found rate, the repairs and net subsumption were level with binary, at p = 0.31, 0.41 and 0.57. But on RERS it found a repair less often, lower on 10 instances and higher on 2 with p = 0.028, on BEEM binary's repairs were stronger on 9 instances against 2 with p = 0.035, and it cost 3.5 more back-end calls per run. The gate already finds the same repairs from binary's median of 23.5 proved candidates.

### The rejected-side similarity

On BEEM the accepted-side similarity puts every repair within about 1e-11 of ⊤, because φ on anderson accepts about 96% of all lassos, so the search has no gradient towards a stronger repair there. The rejected side applies the same metric to ¬φ, ¬φ' and ¬φ ∧ ¬φ'. On a weakening it scores 2r/(1 + r) for the rejection strength r, and it penalises drift away from φ through its second term. On anderson's φ `G(w -> F c)`, ⊤ and one repair differ by less than 1e-6 on the accepted side and score 0 and 0.326 on the rejected side, so the idea looked promising.

I ran it as the `rejected` arm (`search_bench --semantic rejected`) against the default on the 100 evaluation model instances, at 30×40 with seeds 1 to 10. All 2,000 runs gave a result. Three Sieve runs first hit the 3,600 s cap with 12 jobs on av3, and a rerun at 4 h with 5 jobs finished each in 1,914–2,636 s. The found rate had a median of 0.85 for the default against 0.60 for the rejected side, which was lower on 41 instances and higher on 9, with p < 0.0001. Net subsumption also favoured the default, with a mean of −0.107 and p = 0.040 (rejected stronger on 18 instances, weaker on 26 and level on 41), and on BEEM the default's repairs were stronger on 9 of 13 instances with none the other way, at p = 0.006.

A run's semantic field holds the score of its own side, so I rescored both arms on one scale with the `score` binary through `bench/side_stats.py`. The best rejection strength had a median of 0.971 for the default against 0.973, with p = 0.17, and the accepted-side closed share 0.122 against 0.198, with p = 0.40. On BEEM, the family it was built for, 12 of 13 instances tie at a rejection strength of 0.311. So the rejected side gives the search no gain on BEEM and costs found rate everywhere. The default stays.

## Limitations

The RERS parallel unknowns bias the set. A rerun of the 65 unknown ¬φ checks at 3,600 s decided none of them, since 13 of the 19 that it reached hit pan's 6 GB memory limit and 6 hit the depth bound. 40 of 2017's 41 unknowns are in problems 109 to 115, so the proper instances lean towards smaller state spaces. Unknown instances are still searched.

RERS 2018 problems 102 and 103 are out of reach of the explicit front end. Problem 102 fits its 128-bit state key at 101 bits, and under an 85 GB cap it was still exploring at 46 GB when a 3,000 s timeout stopped it. Problem 103 needs more bits than the key holds, and widening the key costs every problem 5–20% more memory per state, while 103 has 14 more components at the same density, so a wider key would only move its failure from the parser to memory. On problem 101 the explicit structure keeps deadlocking runs, as SPIN does, while the official answers count only infinite runs.

Some MCC instances are too slow for the budget. SieveSingleMsgMbox lost 6 headline runs to the 3,600 s cap, 3 rejected-side runs did the same until a rerun on a quieter host, and 2 mutation runs to the 14,400 s cap at 120×80. MedleyA-PT-09 is excluded outright, after 28 `verifypn` calls in one run hit 120 s each on its 2.3 × 10^8 markings.

On BEEM the semantic score cannot separate the repairs from ⊤. 100 of 111 BEEM runs with a repair score exactly ⊤'s value, and 13 of the 14 BEEM best repairs are prefixes with a median closed share of 0.001. The rejection strength does separate them, at 0.311 against 0.987 for the other instances, but only as a report, and the attempt to search on it failed. The precedes comparison is also small, at 13 instances on two programs, and 2 precedes properties stay unknown.

## Further work

The plainest gap is BEEM, where the search needs an objective that sees the difference between `X X F p` and ⊤ without the found-rate cost of the rejected side. Mutation also draws only φ's atoms, so a repair cannot mention any other variable of the model, and widening the pool to the model's atoms is the obvious next operator. I measured the crossover rate and the monotone share on the model instances. Raising the monotone share from 0.25 to 0.75 found a repair more often (higher on 40 of 99 instances and lower on 10, p < 0.0001), gave stronger repairs (net subsumption 52/15/20, p = 0.0001) and needed 19 fewer back-end calls per run on the median, and a crossover rate of 0.9 or both rates at 0.5 also helped. The evaluation above uses PEREDUR's rates, and random mutation shares the breeding operators, so both arms change and the comparison with random mutation should be rerun at the better rates before they become the default. For new sources, the Lockheed Martin Cyber-Physical Challenges carry 74 discrete-time requirements, about 23 of them violated, but the models need MATLAB, so at most a few small ones could be translated to nuXmv by hand. On the RERS unknowns, pan's bitstate hashing could confirm a violation of ¬φ and so show an instance proper, though it cannot prove the converse.

The search beats random mutation on every family except the BEEM instances under the headline's cap, it is ahead on the large BEEM instances given 4 h, and the gap widens with budget, which is the result I set out to test. What I did not expect was how much of the work went into deciding what a trivial repair is, and the BEEM prefixes show that the count still accepts a few repairs that a reader would call empty.
