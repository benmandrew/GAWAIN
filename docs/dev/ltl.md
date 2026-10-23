# LTL formulas

The `ltl` module (`src/ltl/`) holds the formula type that every later layer shares. It is a port of PEREDUR's `Formula` (`~/projects/peredur/include/prop_formula.hpp`), cut down to what this crate uses now. This file records the design decisions that span its submodules.

## Shared nodes, no hash-consing

A `Formula` is an `Arc` to an immutable node. Cloning one costs a refcount increment, and a rewrite that leaves a subtree alone shares that subtree with its input. Each node also caches its structural hash and its size at construction. Hashing a formula therefore costs O(1), and so does `n_subformulae`, which makes a formula cheap to use as a `HashMap` key.

*Hash-consing* (one global table, so that equal subtrees are one allocation) was rejected. PEREDUR counts subformulae as a multiset, so `a & a` has three nodes, and `syntactic_similarity` matches repeats with multiplicity. A tree of shared nodes keeps that count without a second notion of size. A global table would also need a lock or a per-thread arena once the search runs in parallel. Equality stays structural, with a pointer check and a hash check first, so two formulas built apart still compare equal.

## Weak until in SPIN and nuXmv

nuXmv has no W operator. SPIN has one, but both back ends receive the same operator set, so that a disagreement between them comes from the tools and not from the input. Before either printer runs, every `a W b` is replaced by `(a U b) | G a`. The rewrite works bottom-up, so a nested W is expanded inside its own expansion. The left operand is copied, so a chain of W nested on the left grows exponentially with its depth. TLSF specifications nest W shallowly, so this has not been a problem.

Both printers put parentheses around every operand of a binary operator unless it is an atom or a constant. SPIN and nuXmv each have their own precedence tables, and neither matches Spot's. Full parentheses make the printers independent of all three tables. A letter operator is followed by a space (`X a`), because `Xa` lexes as one identifier in both languages.

## Spot-style word reading

The parser reads Spot syntax with Spot's rules. Spot reads a word that starts with a run of `F`, `G` and `X` as that run of unary operators applied to the rest of the word. So `GFa` is `G(F(a))` and `Grant` is `G(rant)`. An atom with such a name must be quoted, as in `"Grant"`. The Spot printer quotes it automatically, and quotes any other name that Spot would misread (`U`, `true`, names with spaces). PEREDUR's parser reads `Grant` as one atom instead. The two parsers therefore disagree on such names, and GAWAIN follows Spot because Spot is the tool that reads the output.

The parser also accepts SPIN's spellings (`[]`, `<>`, `V`, `&&`, `||`) and nuXmv's constants (`TRUE`, `FALSE`), so the output of all three printers parses back. It rejects `xor` and `M`, which are outside the TLSF operator set.

## What `canonical()` normalises

`canonical()` gives one key for many spellings of one formula. It flattens chains of `&` and of `|`, sorts the operands, removes duplicates and rebuilds the chain leaning left, the way the parser associates. It sorts the two operands of `<->` but does not remove a duplicate, because `a <-> a` is `true` and not `a`. It removes double negation. PEREDUR measured that operand order alone caused 16.4% of its `simplify_ltl` executions over 14 specifications.

It folds no constants and recognises no tautologies, because that is the job of `simplify()`. It is for cache keys only. Replacing a stored formula with its canonical form would change the shape that `rewrite_post_order` walks, and with it the order of random draws in a seeded run.

PEREDUR also has a key that renames atoms, for caches whose values do not depend on atom names. Here an atom names a variable of the model, so two formulas that differ only in atom names can get different verdicts. That key is therefore not ported.
