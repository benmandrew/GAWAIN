import Gawain.Trace

/-!
# Interval operators and their exact order

Metric temporal logic over discrete pointwise time. An interval is `[lo, hi]`
with `hi = none` for an unbounded one, and it is well formed (`Itv.WF`) when
it is not empty. `Fi`, `Gi`, `Ui` and `Ri` are the bounded `F`, `G`, until
and release, with the usual reading of until: the left operand holds at every
position before the witness. `Uc` is CEGIW's until, `X^lo (p U[0,hi-lo] q)`
(`~/projects/cegiw/src/logic/mtl.py:144`), where the left operand is needed
only from `lo` onwards.

Each theorem states an implication between two templates for every operand
and every trace, as an `↔` with a condition on the intervals alone. The `←`
half is soundness, and the `→` half is completeness: when the condition
fails, a predicate built for the purpose refutes the implication.
`bench/operator_orders.py` checks the same conditions against Spot for
bounds up to 4.
-/

namespace Gawain

structure Itv where
  lo : Nat
  hi : Option Nat
  deriving DecidableEq, Repr

namespace Itv

/-- `k` lies in the interval. -/
abbrev mem (I : Itv) (k : Nat) : Prop := I.lo ≤ k ∧ ∀ b, I.hi = some b → k ≤ b

/-- The interval is not empty. -/
abbrev WF (I : Itv) : Prop := ∀ b, I.hi = some b → I.lo ≤ b

/-- Inclusion, as sets of positions. -/
abbrev Sub (I J : Itv) : Prop := ∀ k, I.mem k → J.mem k

/-- Inclusion as a test on the bounds. -/
def subB (I J : Itv) : Bool :=
  decide (J.lo ≤ I.lo) &&
    match J.hi, I.hi with
    | none, _ => true
    | some _, none => false
    | some d, some b => decide (b ≤ d)

/-- The two intervals share a position. -/
def meetsB (I J : Itv) : Bool :=
  match I.hi, J.hi with
  | none, none => true
  | none, some d => decide (I.lo ≤ d)
  | some b, none => decide (J.lo ≤ b)
  | some b, some d => decide (I.lo ≤ d) && decide (J.lo ≤ b)

/-- The interval is the single position `[k, k]`. -/
def pointB (I : Itv) : Bool :=
  match I.hi with
  | none => false
  | some b => decide (b = I.lo)

theorem lo_mem {I : Itv} (h : I.WF) : I.mem I.lo := ⟨Nat.le_refl _, h⟩

theorem subB_iff {I J : Itv} (hI : I.WF) : subB I J = true ↔ Sub I J := by
  obtain ⟨a, _ | b⟩ := I <;> obtain ⟨c, _ | d⟩ := J <;> simp [subB, Sub, WF, mem] at *
  · exact ⟨fun h k hk => by omega, fun h => h a (Nat.le_refl _)⟩
  · exact ⟨max a (c + d + 1), by omega, fun _ => by omega⟩
  · exact ⟨fun h k _ _ => by omega, fun h => h a (Nat.le_refl _) hI⟩
  · exact ⟨fun h k _ _ => by omega,
      fun h => ⟨(h a (Nat.le_refl _) hI).1, (h b hI (Nat.le_refl _)).2⟩⟩

theorem meetsB_iff {I J : Itv} (hI : I.WF) (hJ : J.WF) :
    meetsB I J = true ↔ ∃ k, I.mem k ∧ J.mem k := by
  obtain ⟨a, _ | b⟩ := I <;> obtain ⟨c, _ | d⟩ := J <;> simp [meetsB, mem, WF] at *
  · exact ⟨a + c, by omega, by omega⟩
  · exact ⟨fun h => ⟨max a c, by omega, by omega, by omega⟩, fun ⟨k, _, _, _⟩ => by omega⟩
  · exact ⟨fun h => ⟨max a c, ⟨by omega, by omega⟩, by omega⟩, fun ⟨k, ⟨_, _⟩, _⟩ => by omega⟩
  · exact ⟨fun ⟨_, _⟩ => ⟨max a c, ⟨by omega, by omega⟩, by omega, by omega⟩,
      fun ⟨k, ⟨_, _⟩, _, _⟩ => ⟨by omega, by omega⟩⟩

theorem pointB_iff {I : Itv} (hI : I.WF) : pointB I = true ↔ ∀ k, I.mem k ↔ k = I.lo := by
  obtain ⟨a, _ | b⟩ := I <;> simp [pointB, mem, WF] at *
  · exact ⟨a + 1, fun h => by have := h.1 (by omega); omega⟩
  · exact ⟨fun h k => ⟨fun _ => by omega, fun _ => ⟨by omega, by omega⟩⟩,
      fun h => by have := (h b).1 ⟨hI, Nat.le_refl _⟩; omega⟩

end Itv

open Itv

/-- `F_I p` -/
abbrev Fi (I : Itv) (p : Pred) : Pred := fun t => ∃ k, I.mem k ∧ p (t + k)

/-- `G_I p` -/
abbrev Gi (I : Itv) (p : Pred) : Pred := fun t => ∀ k, I.mem k → p (t + k)

/-- `p U_I q`, the usual reading. -/
abbrev Ui (I : Itv) (p q : Pred) : Pred :=
  fun t => ∃ k, I.mem k ∧ q (t + k) ∧ ∀ j, j < k → p (t + j)

/-- `p U_I q`, CEGIW's reading: `p` only from `I.lo` onwards. -/
abbrev Uc (I : Itv) (p q : Pred) : Pred :=
  fun t => ∃ k, I.mem k ∧ q (t + k) ∧ ∀ j, I.lo ≤ j → j < k → p (t + j)

/-- `p R_I q`: at each position of `I`, `q` holds unless `p` held earlier. -/
abbrev Ri (I : Itv) (p q : Pred) : Pred :=
  fun t => ∀ k, I.mem k → q (t + k) ∨ ∃ j, j < k ∧ p (t + j)

/-! ## Agreement with LTL and duality -/

theorem Fi_full (p : Pred) (t : Nat) : Fi ⟨0, none⟩ p t ↔ F p t :=
  ⟨fun ⟨k, _, h⟩ => ⟨k, h⟩, fun ⟨k, h⟩ => ⟨k, ⟨Nat.zero_le _, fun b hb => by simp at hb⟩, h⟩⟩

theorem Gi_full (p : Pred) (t : Nat) : Gi ⟨0, none⟩ p t ↔ G p t :=
  ⟨fun h j => h j ⟨Nat.zero_le _, fun b hb => by simp at hb⟩, fun h k _ => h k⟩

theorem Ui_full (p q : Pred) (t : Nat) : Ui ⟨0, none⟩ p q t ↔ U p q t :=
  ⟨fun ⟨k, _, hq, hp⟩ => ⟨k, hq, hp⟩,
   fun ⟨k, hq, hp⟩ => ⟨k, ⟨Nat.zero_le _, fun b hb => by simp at hb⟩, hq, hp⟩⟩

/-- `X^k p` is `F[k,k] p`. -/
theorem Fi_point (k : Nat) (p : Pred) (t : Nat) : Fi ⟨k, some k⟩ p t ↔ p (t + k) :=
  ⟨fun ⟨j, ⟨h1, h2⟩, h⟩ => by
      have h1 : k ≤ j := h1
      have := h2 k rfl
      have : j = k := by omega
      subst this; exact h,
   fun h => ⟨k, ⟨Nat.le_refl _, fun b hb => by simp at hb; omega⟩, h⟩⟩

/-- `X^k p` is `G[k,k] p`. -/
theorem Gi_point (k : Nat) (p : Pred) (t : Nat) : Gi ⟨k, some k⟩ p t ↔ p (t + k) :=
  ⟨fun h => h k ⟨Nat.le_refl _, fun b hb => by simp at hb; omega⟩,
   fun h j ⟨h1, h2⟩ => by
      have h1 : k ≤ j := h1
      have := h2 k rfl
      have : j = k := by omega
      subst this; exact h⟩

/-- Release is the dual of the usual until. -/
theorem Ri_dual (I : Itv) (p q : Pred) (t : Nat) : Ri I p q t ↔ ¬ Ui I (neg p) (neg q) t := by
  constructor
  · intro h ⟨k, hk, hq, hp⟩
    rcases h k hk with h | ⟨j, hj, h⟩
    · exact hq h
    · exact hp j hj h
  · intro h k hk
    apply Classical.byContradiction
    intro hn
    apply h
    exact ⟨k, hk, fun hq => hn (Or.inl hq), fun j hj hp => hn (Or.inr ⟨j, hj, hp⟩)⟩

/-! ## The order -/

variable {I J : Itv}

theorem Fi_order (hI : I.WF) :
    (∀ p t, Fi I p t → Fi J p t) ↔ subB I J = true := by
  rw [subB_iff hI]
  constructor
  · intro h k hk
    obtain ⟨k', hk', he⟩ := h (fun n => n = k) 0 ⟨k, hk, by simp⟩
    simp at he
    exact he ▸ hk'
  · intro h p t ⟨k, hk, hp⟩
    exact ⟨k, h k hk, hp⟩

theorem Gi_order (hJ : J.WF) :
    (∀ p t, Gi I p t → Gi J p t) ↔ subB J I = true := by
  rw [subB_iff hJ]
  constructor
  · intro h k hk
    have := h (fun n => I.mem n) 0 (fun k hk => by simpa using hk) k hk
    simpa using this
  · intro h p t hg k hk
    exact hg k (h k hk)

theorem Gi_Fi (hI : I.WF) (hJ : J.WF) :
    (∀ p t, Gi I p t → Fi J p t) ↔ meetsB I J = true := by
  rw [meetsB_iff hI hJ]
  constructor
  · intro h
    obtain ⟨k, hk, hp⟩ := h (fun n => I.mem n) 0 (fun k hk => by simpa using hk)
    exact ⟨k, by simpa using hp, hk⟩
  · intro ⟨k, hkI, hkJ⟩ p t hg
    exact ⟨k, hkJ, hg k hkI⟩

theorem Fi_Gi (hI : I.WF) (hJ : J.WF) :
    (∀ p t, Fi I p t → Gi J p t) ↔ (I = J ∧ pointB I = true) := by
  constructor
  · intro h
    -- Every position of I equals every position of J.
    have key : ∀ m n, I.mem m → J.mem n → n = m := fun m n hm hn => by
      simpa using h (fun x => x = m) 0 ⟨m, hm, by simp⟩ n hn
    obtain ⟨a, _ | b⟩ := I <;> obtain ⟨c, _ | d⟩ := J <;> simp [pointB, mem, WF] at *
    · have := key a (a + c) (by omega) (by omega)
      have := key a (a + c + 1) (by omega) (by omega)
      omega
    · have := key a c (by omega) (Nat.le_refl _) hJ
      have := key (a + 1) c (by omega) (Nat.le_refl _) hJ
      omega
    · have := key a (a + c) (Nat.le_refl _) hI (by omega)
      have := key a (a + c + 1) (Nat.le_refl _) hI (by omega)
      omega
    · have := key a c (Nat.le_refl _) hI (Nat.le_refl _) hJ
      have := key b c hI (Nat.le_refl _) (Nat.le_refl _) hJ
      have := key a d (Nat.le_refl _) hI hJ (Nat.le_refl _)
      refine ⟨⟨?_, ?_⟩, ?_⟩ <;> omega
  · intro ⟨he, hp⟩ p t ⟨k, hk, hpk⟩ n hn
    subst he
    have h1 := ((pointB_iff hI).1 hp k).1 hk
    have h2 := ((pointB_iff hI).1 hp n).1 hn
    rw [h2, ← h1]
    exact hpk

theorem Ui_order (hI : I.WF) :
    (∀ p q t, Ui I p q t → Ui J p q t) ↔ subB I J = true := by
  rw [subB_iff hI]
  constructor
  · intro h k hk
    obtain ⟨k', hk', he, _⟩ := h (fun _ => True) (fun n => n = k) 0 ⟨k, hk, by simp, fun _ _ => trivial⟩
    simp at he
    exact he ▸ hk'
  · intro h p q t ⟨k, hk, hq, hp⟩
    exact ⟨k, h k hk, hq, hp⟩

theorem Ri_order (hI : I.WF) :
    (∀ p q t, Ri J p q t → Ri I p q t) ↔ subB I J = true := by
  rw [subB_iff hI]
  constructor
  · intro h k hk
    apply Classical.byContradiction
    intro hn
    have hr : Ri J (fun _ => False) (fun n => n ≠ k) 0 := fun m hm =>
      Or.inl (fun he => hn (by simp at he; exact he ▸ hm))
    rcases h _ _ 0 hr k hk with h | ⟨_, _, h⟩
    · simp at h
    · exact h
  · intro h p q t hr k hk
    exact hr k (h k hk)

theorem Uc_order (hI : I.WF) :
    (∀ p q t, Uc I p q t → Uc J p q t) ↔ (I.lo = J.lo ∧ subB I J = true) := by
  rw [subB_iff hI]
  constructor
  · intro h
    have hsub : Sub I J := fun k hk => by
      obtain ⟨k', hk', he, _⟩ :=
        h (fun _ => True) (fun n => n = k) 0 ⟨k, hk, by simp, fun _ _ _ => trivial⟩
      simp at he
      exact he ▸ hk'
    refine ⟨?_, hsub⟩
    have hle := (hsub I.lo (lo_mem hI)).1
    apply Classical.byContradiction
    intro hne
    obtain ⟨k', _, he, hp⟩ := h (fun n => I.lo ≤ n) (fun n => n = I.lo) 0
      ⟨I.lo, lo_mem hI, by simp, fun j h1 h2 => by omega⟩
    simp at he
    have := hp J.lo (Nat.le_refl _) (by omega)
    simp at this
    omega
  · intro ⟨hlo, h⟩ p q t ⟨k, hk, hq, hp⟩
    exact ⟨k, h k hk, hq, fun j hj hjk => hp j (hlo ▸ hj) hjk⟩

theorem Ui_Fi (hI : I.WF) :
    (∀ p q t, Ui I p q t → Fi J q t) ↔ subB I J = true := by
  rw [subB_iff hI]
  constructor
  · intro h k hk
    obtain ⟨k', hk', he⟩ := h (fun _ => True) (fun n => n = k) 0 ⟨k, hk, by simp, fun _ _ => trivial⟩
    simp at he
    exact he ▸ hk'
  · intro h p q t ⟨k, hk, hq, _⟩
    exact ⟨k, h k hk, hq⟩

theorem Gi_Ri (hI : I.WF) :
    (∀ p q t, Gi J q t → Ri I p q t) ↔ subB I J = true := by
  rw [subB_iff hI]
  constructor
  · intro h k hk
    rcases h (fun _ => False) (fun n => J.mem n) 0 (fun k hk => by simpa using hk) k hk with
      h | ⟨_, _, h⟩
    · simpa using h
    · exact h.elim
  · intro h p q t hg k hk
    exact Or.inl (hg k (h k hk))

theorem Ui_Uc (p q : Pred) (t : Nat) : Ui I p q t → Uc I p q t :=
  fun ⟨k, hk, hq, hp⟩ => ⟨k, hk, hq, fun j _ hj => hp j hj⟩

theorem Uc_Ui (hI : I.WF) : (∀ p q t, Uc I p q t → Ui I p q t) ↔ I.lo = 0 := by
  constructor
  · intro h
    apply Classical.byContradiction
    intro hne
    obtain ⟨_, _, _, hp⟩ := h (fun n => I.lo ≤ n) (fun n => n = I.lo) 0
      ⟨I.lo, lo_mem hI, by simp, fun j h1 h2 => by omega⟩
    have := hp 0 (by omega)
    simp at this
    omega
  · intro h p q t ⟨k, hk, hq, hp⟩
    exact ⟨k, hk, hq, fun j hj => hp j (by omega) hj⟩

end Gawain
