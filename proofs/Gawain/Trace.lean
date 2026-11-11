/-!
# Trace semantics

A formula over a trace is a predicate on positions, so every formula here is
a `Pred`, and each operator evaluates at a position `t` by looking forward
from it. This is the pointwise reading over infinite traces that Spot uses.

A *template* such as `G p ⇒ F p` holds for all operands exactly when it holds
for every predicate `p`, since validity is closed under uniform substitution.
Every order theorem in this library therefore quantifies over arbitrary
predicates, and a countermodel is a predicate built for the purpose.

Every definition is an `abbrev`, so the anonymous constructor and `rcases`
see through it.
-/

namespace Gawain

abbrev Pred := Nat → Prop

/-- `p` implies `q` at every position of every trace. -/
abbrev Entails (p q : Pred) : Prop := ∀ t, p t → q t

abbrev neg (p : Pred) : Pred := fun t => ¬ p t

/-- `X p` -/
abbrev X (p : Pred) : Pred := fun t => p (t + 1)

/-- `F p` -/
abbrev F (p : Pred) : Pred := fun t => ∃ j, p (t + j)

/-- `G p` -/
abbrev G (p : Pred) : Pred := fun t => ∀ j, p (t + j)

/-- `p U q`: `q` arrives, and `p` holds at every point before it. -/
abbrev U (p q : Pred) : Pred := fun t => ∃ j, q (t + j) ∧ ∀ i, i < j → p (t + i)

/-- `p W q` -/
abbrev W (p q : Pred) : Pred := fun t => U p q t ∨ G p t

/-- `p R q`: `q` holds up to and including the first `p`, or for ever. -/
abbrev R (p q : Pred) : Pred := fun t => ∀ j, q (t + j) ∨ ∃ i, i < j ∧ p (t + i)

end Gawain
