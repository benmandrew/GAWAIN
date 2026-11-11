import Gawain.Interval

/-!
# Single-bound steps generate the interval order

A mutation moves one bound of one interval. For `F`, the weakening steps are
`lower` (the left bound down by one), `raise` (the right bound up by one) and
`unbound` (the right bound to ∞). `F_steps_exact` shows that chains of these
steps reach exactly the intervals that `F` may weaken to, so the steps are
complete for the order: no weakening of an `F` interval needs any other move.
The unbound step is a jump, since no bounded interval lies immediately below
an unbounded one.

For `G` the weakening steps are the same steps reversed: raise the left bound,
lower the right bound, or bound an unbounded interval. `G_steps_exact` gives
the same result for them.
-/

namespace Gawain

open Itv

/-- One weakening step of an `F` interval. -/
inductive FStep : Itv → Itv → Prop
  | lower (a : Nat) (h : Option Nat) : FStep ⟨a + 1, h⟩ ⟨a, h⟩
  | raise (a b : Nat) : FStep ⟨a, some b⟩ ⟨a, some (b + 1)⟩
  | unbound (a b : Nat) : FStep ⟨a, some b⟩ ⟨a, none⟩

/-- Chains of steps of `r`. -/
inductive Reach (r : Itv → Itv → Prop) : Itv → Itv → Prop
  | refl (I : Itv) : Reach r I I
  | tail {I J K : Itv} : Reach r I J → r J K → Reach r I K

theorem Reach.single {r : Itv → Itv → Prop} {I J : Itv} (h : r I J) : Reach r I J :=
  .tail (.refl I) h

theorem Reach.trans {r : Itv → Itv → Prop} {I J K : Itv}
    (h1 : Reach r I J) (h2 : Reach r J K) : Reach r I K := by
  induction h2 with
  | refl => exact h1
  | tail _ hs ih => exact .tail ih hs

theorem FStep.sub {I J : Itv} (h : FStep I J) : Sub I J := by
  cases h with
  | lower a h => intro k ⟨h1, h2⟩; exact ⟨by simp at h1 ⊢; omega, h2⟩
  | raise a b =>
    intro k ⟨h1, h2⟩
    exact ⟨h1, fun d hd => by simp at hd; have := h2 b rfl; omega⟩
  | unbound a b => intro k ⟨h1, _⟩; exact ⟨h1, fun d hd => by simp at hd⟩

theorem Reach.sub {I J : Itv} (h : Reach FStep I J) : Sub I J := by
  induction h with
  | refl => exact fun _ h => h
  | tail _ hs ih => exact fun k hk => hs.sub k (ih k hk)

theorem raise_n (a b : Nat) : ∀ n, Reach FStep ⟨a, some b⟩ ⟨a, some (b + n)⟩
  | 0 => .refl _
  | n + 1 => .tail (raise_n a b n) (.raise a (b + n))

theorem lower_n (c : Nat) (h : Option Nat) : ∀ n, Reach FStep ⟨c + n, h⟩ ⟨c, h⟩
  | 0 => .refl _
  | n + 1 => (Reach.single (.lower (c + n) h)).trans (lower_n c h n)

theorem F_steps_exact {I J : Itv} (hI : I.WF) : Reach FStep I J ↔ subB I J = true := by
  rw [subB_iff hI]
  refine ⟨Reach.sub, fun h => ?_⟩
  have hb := (subB_iff hI).2 h
  obtain ⟨a, _ | b⟩ := I <;> obtain ⟨c, _ | d⟩ := J <;> simp [subB] at hb
  · have := lower_n c none (a - c)
    rwa [show c + (a - c) = a by omega] at this
  · have := lower_n c none (a - c)
    rw [show c + (a - c) = a by omega] at this
    exact (Reach.single (.unbound a b)).trans this
  · obtain ⟨h1, h2⟩ := hb
    have r := raise_n a b (d - b)
    rw [show b + (d - b) = d by omega] at r
    have l := lower_n c (some d) (a - c)
    rw [show c + (a - c) = a by omega] at l
    exact r.trans l

/-- `G` weakens along the `F` steps reversed. -/
abbrev GStep (I J : Itv) : Prop := FStep J I

theorem reach_flip {I J : Itv} (h : Reach GStep I J) : Reach FStep J I := by
  induction h with
  | refl => exact .refl _
  | tail _ hs ih => exact (Reach.single (r := FStep) hs).trans ih

theorem reach_unflip {I J : Itv} (h : Reach FStep J I) : Reach GStep I J := by
  induction h with
  | refl => exact .refl _
  | tail _ hs ih => exact (Reach.single (r := GStep) hs).trans ih

theorem G_steps_exact {I J : Itv} (hJ : J.WF) : Reach GStep I J ↔ subB J I = true :=
  ⟨fun h => (F_steps_exact hJ).1 (reach_flip h), fun h => reach_unflip ((F_steps_exact hJ).2 h)⟩

end Gawain
