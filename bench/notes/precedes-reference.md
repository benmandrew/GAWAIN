# RERS precedes: repairs against a reference weakening

Specification weakening has no ground truth ("Evaluation without a ground truth" in `PLAN.md`), so RQ2 of `docs/dev/evaluation.md` ranks repairs by their strength against baselines. The Rigorous Examination of Reactive Systems (RERS) 2016/2017 "precedes" set is a partial exception. Each of its 105 shipped formulas φ has the shape G(Q ∧ P), and the gloss describes the *Dwyer pattern* G(Q → P) ("RERS 2016/2017: the mistranslated pattern properties" in `bench/notes/rers-precedes.md`). Write ψ for that pattern, the *intended weakening*. A person wrote it, and every ψ in the set has a decided verdict. This note asks how often GAWAIN (Genetic Algorithm for Weakening Against Implementations under NSGA-II) reaches ψ from φ, and where its other repairs sit against ψ.

## Instances

GAWAIN reaches these C programs only through the Symbolic Model Verifier (SMV) translation of `bench/rers_seq_smv.py` and its nuXmv back end. A search is affordable only on problems whose translation has no `signed word[32]` variable. On 2016/1 and 2017/1 a search finishes in under a second. A 4×8 search ran past 900 s on 2016/3, 2016/6, 2016/8, 2017/2 and 2017/3, and past 700 s on 2016/4 and 2017/4. The set is therefore the 13 cases on 2016/1 and 2017/1, built by `bench/precedes_prep.py`. ψ holds on 7 of them and fails on 6.

Each model gets `FAIRNESS !dead`, so a check covers only the runs that never take an invalid input, as RERS reads its properties. The translation's start position, at which no event atom holds, stays in the model. nuXmv's check of each ψ on the translated model matches the recorded verdict on all 13 (the `psi_holds_here` column). All 13 instances are *degenerate*: M satisfies ¬φ.

## Method

The instances are degenerate, so the runs use the `forced-` arms. `forced-default` is the evaluation's genetic algorithm (GA) configuration, and `forced-mutation` is the random-mutation baseline under the same budget and filters ("Random-mutation baseline" in `docs/dev/search.md`). Both ran at 30×40 with seeds 1 to 10, on the search of commit `ec8a3f9`, on 2026-10-27. The run and the analysis are the following commands.

```
python3 bench/graded_run.py bench/data/precedes 3 --seeds 1-10 --arms forced-default,forced-mutation --out runs-eval
python3 bench/precedes_stats.py bench/data/precedes bench/data/precedes/eval-stats.csv
```

For each instance and arm, `bench/precedes_stats.py` pools the repairs over the seeds and reduces them to the *maximal set*, the repairs that no other pooled repair strictly implies. Spot then places ψ against each member modulo the *encoding invariant* I, which allows one event per position. A repair is *stronger* than ψ when it implies ψ, and *weaker* when ψ implies it. Per run, the script records whether some repair implies ψ, which makes that repair at least as strong as the intended fix. It also scores ψ and each run's best repair against φ under the *rejection strength*. The rows are in `bench/data/precedes/eval-stats.csv`.

## Reaching ψ

Every run of both arms found a repair, 130 of 130 each. The table counts, on the 7 instances where ψ holds, the runs on which some repair implies ψ modulo I.

| Instance | Reached ψ, GA | Reached ψ, mutation |
|---|---|---|
| 2016/1 p11 | 7/10 | 5/10 |
| 2016/1 p39 | 3/10 | 1/10 |
| 2016/1 p55 | 3/10 | 1/10 |
| 2016/1 p70 | 2/10 | 1/10 |
| 2016/1 p89 | 2/10 | 0/10 |
| 2017/1 p25 | 5/10 | 3/10 |
| 2017/1 p26 | 10/10 | 8/10 |
| Total | 32/70 | 19/70 |

The GA reaches ψ on 13 more runs than random mutation. It is ahead on all seven. A two-sided sign test on 7 of 7 gives p = 0.016. Only on 2017/1 p26 does the GA reach ψ on every seed, and on 2016/1 p89 random mutation never reaches it.

On the 6 instances where ψ fails, no repair can imply ψ, since a repair holds on M. Both arms record 0 runs there.

## Maximal repairs against ψ

The maximal sets are large, between 24 and 65 repairs per instance and arm. Pooled over the 13 instances, the GA's 521 maximal repairs are 35 stronger than ψ, 4 equivalent, 0 weaker and 482 incomparable. Random mutation's 614 are 14 stronger, 2 equivalent, 2 weaker and 596 incomparable. The GA finds a repair equivalent to ψ on 2016/1 p55, p70 and p89, with two on p89. Random mutation finds one on 2016/1 p11 and p55. 18 of the GA's 35 stronger repairs are on 2017/1 p26, where they make up 18 of its 24 maximal repairs.

Most maximal repairs are incomparable with ψ. Each is another weakening of φ that holds on M, neither stronger nor weaker than the human fix. Against this reference they count neither for nor against the search.

## Rejection strength

The CSV gives ψ's rejection strength against φ (`psi_rejection`) and the median over runs of the best repair's (`best_rejection`). In both arms the best repair scores above ψ on 8 of the 13 instances. On the 7 where ψ holds it scores above on 6. The exception is 2016/1 p55, where ψ scores 0.995077 against the GA's 0.989086 and mutation's 0.994584. On the 6 where ψ fails, the best repair scores above ψ on 2 (2016/1 p86 and 2017/1 p29) and below on 4 (2017/1 p5, p38, p50 and p53). The margins are small. ψ scores 0.992649 or 0.995077 on every instance, and the best-repair medians lie between 0.983091 and 1.000000.

## Limitations

The set is 13 instances on two small problems. It checks whether the search can reach a known fix, and is not a broad measure of repair strength. The other RERS problems carry word arithmetic, and their searches ran past the time limits above.

On two small programs the GA reaches a person's fix on 32 of 70 runs where that fix holds. Most of what it returns beside the fix is a different true weakening, which a reference answer can count but cannot rank.
