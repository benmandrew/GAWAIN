# GAWAIN order proofs

Lean 4 proofs of the operator orders in `docs/dev/order-search.md`, of the soundness of GAWAIN's monotone rewrites, and of the model-checking facts behind gated search. `bench/operator_orders.py` checks the interval orders against Spot for bounds up to 4. The proofs give them for every interval, unbounded ones included. They use core Lean 4.35.0 with `omega` and no Mathlib, as PEREDUR's do.

## Building

```sh
nix develop '.#proofs'
cd proofs
lake build
```

The `proofs` shell carries `lean4`, the pinned Spot 2.15.1 and `python3`. It is separate from the default shell because `lean4` is a large download that no build, test or benchmark needs. `lean-toolchain` pins 4.35.0 for anyone using elan instead.

## What is proved

The model is pointwise discrete time over infinite traces, and a formula is a predicate on positions. Every template is proved over arbitrary predicates. This is exact, because validity is closed under *uniform substitution*, so a template holds for all operands exactly when it holds for every predicate.

- **`Gawain/Trace.lean`.** The linear temporal logic (LTL) operators `X`, `F`, `G`, `U`, `W` and `R`, each read forward from a position as Spot reads them.
- **`Gawain/Interval.lean`.** The metric temporal logic (MTL) operators over intervals `[lo, hi]`, where `hi` may be ∞. `Fi` and `Gi` are the bounded `F` and `G`, `Ui` is the usual until, `Uc` is CEGIW's until `X^a (p U[0,b−a] q)`, and `Ri` is the dual of `Ui`. Each order is an `↔` with a condition on the intervals alone, proved for every interval. The `←` half is soundness, and the `→` half refutes the implication with a predicate built for the purpose when the condition fails. `Fi_order` holds when I ⊆ J, and `Gi_order` when J ⊆ I. `Gi_Fi` holds when I and J meet, and `Fi_Gi` when I = J is a single point. `Ui_order` and `Ri_order` follow inclusion, and `Uc_order` needs the same lower bound as well. `Ui_Fi` and `Gi_Ri` relate until and release to `F` and `G`. `Ui_Uc` always holds, and `Uc_Ui` holds exactly when lo = 0. `Fi_full`, `Gi_full` and `Ui_full` reduce `[0, ∞)` to plain `F`, `G` and `U`, and `Fi_point` and `Gi_point` reduce `[k, k]` to `X^k`.
- **`Gawain/Nesting.lean`.** `Fi_add` and `Gi_add` prove `F_I F_J p ≡ F_(I+J) p` and `G_I G_J p ≡ G_(I+J) p` for any well-formed intervals, where `I + J` is the interval sum. `FG_GF` shows that, for bounded intervals, `F_I G_J p ⇒ G_K F_L p` holds exactly when I − K ⊆ L − J. The theorem states this without subtraction, as l₁ + k₂ ≤ i₁ + j₂ and i₂ + j₁ ≤ l₂ + k₁.
- **`Gawain/Steps.lean`.** An `F` interval weakens by three single-bound steps, which lower the left bound by one, raise the right bound by one, or unbound the right bound. `F_steps_exact` proves that chains of these steps reach exactly the intervals that `F` may weaken to. `G_steps_exact` proves the same for `G` with the steps reversed. The three moves are therefore complete for interval weakening. The unbound step is a jump, since no bounded interval lies immediately below an unbounded one.
- **`Gawain/Unary.lean`.** `norm_correct` proves that every word over `X`, `F` and `G`, of any length, applied to one atom is equivalent to one of five *normal forms*: `X^k p`, `X^k F p`, `X^k G p`, `F G p` and `G F p`. `nf_exact` decides the order between normal forms exactly. The countermodel for `G F p ⇏ F G p` is the parity predicate. `word_exact` then decides implication between any two unary templates.
- **`Gawain/Rules.lean`.** `Fm` mirrors the formula type in `src/ltl/mod.rs`. A *context* (`Ctx`) is a formula with one hole outside any `<->`, which matches `collect_polarities` in `src/search/operators.rs`. Polarity flips under `!` and in the antecedent of `->`. `ctx_mono` proves monotonicity through any context. `rulesAt` transcribes `rules_at` and `apply_rule` with every coin outcome and an arbitrary drawn literal. `proposedAt` transcribes the extra rules of `RuleSet::Ordered`, one function per Rust `Rule` variant, in the order that `ordered_rules` pushes them. `rulesAt_sound` and `proposedAt_sound` show that every output moves in its stated direction. `rewrite_sound` and `monotone_rewrite_sound` lift this through a context, so every rewrite that `monotone_rewrite` can return moves the whole formula the asked way. `outs` transcribes `monotone_outcomes` and `outcomes_below`. It visits the sites in pre-order, with polarity `some true`, `some false`, or `none` under `<->`. `outs_sound` and `monotone_outcomes_sound` prove that every listed outcome moves the whole formula the asked way, under both rule sets.
- **`Gawain/Gating.lean`.** A model is a set of traces, and `Sat` and `Repair` are defined over it. `holds_up` says that weakening a holder keeps it holding, and `fails_down` that strengthening a failing candidate keeps it failing. `cex_down` adds that the failing candidate's counterexample refutes every strengthening, so no back-end call is needed. `and_repair` proves that the conjunction of two repairs is a repair. `or_repair` proves that `φ ∨ ψ` repairs `φ` when `M ⊨ ψ`.

## Checking against Spot and the Rust

`Tables.lean` prints the verdicts that the proofs decide and the outcome lists of `outs`. `bench/check_lean_orders.py` compares the verdicts with Spot and the outcome lists with the Rust `rule_table` binary. Build the binary, then run the check from the repo root:

```sh
nix develop -c cargo build --release --bin rule_table
nix develop '.#proofs' --command python3 bench/check_lean_orders.py
```

All 5,080 MTL cases of `bench/results/operator-orders-mtl.csv` agree, 1,661 of them implications. All 1,600 ordered pairs of unary words of length at most 3 agree with a fresh Spot run, 810 of them implications. As controls, 996 MTL cases disagree when the Spot verdicts are read one row out of step, and 1,244 unary pairs disagree when each pair is swapped.

The rules table covers every formula of depth at most 2 over the atoms a and b plus the constants. That is 10,264 distinct formulas, and two directions and two rule sets make 41,056 rows with 652,052 outcomes. No row differs between `outs` and `rule_table`, and every row lists its outcomes in the same order. As a control, all 41,056 rows differ from the other rule set's list.

## What is not proved

The order between binary LTL templates over `U`, `W`, `R`, `∧` and `∨` is not proved exact. Every rule over them is proved sound, but the order itself is measured only by Spot in `bench/operator_orders.py`, at small depth. The reach of `RuleSet::Ordered` is also measured only, and it reaches 191 of 202 binary covers. That count is relative to the template set. The full LTL implication order over fixed atoms has no general covers between distinct shapes in the sense of `docs/dev/order-search.md`, so reaching every cover would not be a completeness theorem. `rulesAt`, `proposedAt`, `outs` and `Ctx` are tied to the Rust by running both on the depth-2 grid above, which is a finite grid and not a proof about the Rust source. `FG_GF` covers bounded intervals only. The interval theorems are about semantics, since GAWAIN has no MTL parser or oracle yet. The gating theorems are about semantics too, and they say nothing about the code of GAWAIN's lasso cache.
