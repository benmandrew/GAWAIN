import Gawain.Interval

/-!
# Nested interval operators

`F_I F_J p` and `F_(I+J) p` are equivalent, and so are `G_I G_J p` and
`G_(I+J) p`, where `I + J` is the interval sum. Discrete intervals are
contiguous, so every position of the sum splits into one position of each.
These are the rewrites a normaliser needs before it compares two formulas.

`FG_GF` decides `F_I G_J p ⇒ G_K F_L p` for bounded intervals: it holds
exactly when `I − K ⊆ L − J`, written here without subtraction.
-/

namespace Gawain

open Itv

/-- The interval sum. -/
def Itv.add (I J : Itv) : Itv :=
  ⟨I.lo + J.lo, match I.hi, J.hi with
    | some b, some d => some (b + d)
    | _, _ => none⟩

theorem Itv.mem_add {I J : Itv} (hI : I.WF) (hJ : J.WF) (n : Nat) :
    (I.add J).mem n ↔ ∃ a b, I.mem a ∧ J.mem b ∧ n = a + b := by
  obtain ⟨a, _ | b⟩ := I <;> obtain ⟨c, _ | d⟩ := J <;> simp [Itv.add, mem, WF] at *
  · exact ⟨fun h => ⟨a, by omega, n - a, by omega, by omega⟩,
      fun ⟨x, hx, y, hy, e⟩ => by omega⟩
  · exact ⟨fun h => ⟨n - c, by omega, c, ⟨by omega, hJ⟩, by omega⟩,
      fun ⟨x, hx, y, ⟨_, _⟩, e⟩ => by omega⟩
  · exact ⟨fun h => ⟨a, ⟨by omega, hI⟩, n - a, by omega, by omega⟩,
      fun ⟨x, ⟨_, _⟩, y, hy, e⟩ => by omega⟩
  · exact ⟨fun ⟨_, _⟩ => ⟨min b (n - c), ⟨by omega, by omega⟩, n - min b (n - c),
        ⟨by omega, by omega⟩, by omega⟩,
      fun ⟨x, ⟨_, _⟩, y, ⟨_, _⟩, e⟩ => ⟨by omega, by omega⟩⟩

theorem Itv.add_WF {I J : Itv} (hI : I.WF) (hJ : J.WF) : (I.add J).WF := by
  obtain ⟨a, _ | b⟩ := I <;> obtain ⟨c, _ | d⟩ := J <;> simp [Itv.add, WF] at * <;> omega

theorem Fi_add {I J : Itv} (hI : I.WF) (hJ : J.WF) (p : Pred) (t : Nat) :
    Fi I (Fi J p) t ↔ Fi (I.add J) p t := by
  constructor
  · intro ⟨a, ha, b, hb, hp⟩
    exact ⟨a + b, (mem_add hI hJ _).2 ⟨a, b, ha, hb, rfl⟩, by rw [← Nat.add_assoc]; exact hp⟩
  · intro ⟨n, hn, hp⟩
    obtain ⟨a, b, ha, hb, e⟩ := (mem_add hI hJ n).1 hn
    exact ⟨a, ha, b, hb, by rw [Nat.add_assoc, ← e]; exact hp⟩

theorem Gi_add {I J : Itv} (hI : I.WF) (hJ : J.WF) (p : Pred) (t : Nat) :
    Gi I (Gi J p) t ↔ Gi (I.add J) p t := by
  constructor
  · intro h n hn
    obtain ⟨a, b, ha, hb, e⟩ := (mem_add hI hJ n).1 hn
    have := h a ha b hb
    rw [Nat.add_assoc, ← e] at this
    exact this
  · intro h a ha b hb
    have := h (a + b) ((mem_add hI hJ _).2 ⟨a, b, ha, hb, rfl⟩)
    rw [← Nat.add_assoc] at this
    exact this

/-- `F_I G_J p ⇒ G_K F_L p` exactly when `I − K ⊆ L − J`, for bounded
intervals `I = [i₁, i₂]`, `J = [j₁, j₂]`, `K = [k₁, k₂]` and `L = [l₁, l₂]`. -/
theorem FG_GF {i₁ i₂ j₁ j₂ k₁ k₂ l₁ l₂ : Nat}
    (hi : i₁ ≤ i₂) (hj : j₁ ≤ j₂) (hk : k₁ ≤ k₂) (hl : l₁ ≤ l₂) :
    (∀ p t, Fi ⟨i₁, some i₂⟩ (Gi ⟨j₁, some j₂⟩ p) t → Gi ⟨k₁, some k₂⟩ (Fi ⟨l₁, some l₂⟩ p) t) ↔
      (l₁ + k₂ ≤ i₁ + j₂ ∧ i₂ + j₁ ≤ l₂ + k₁) := by
  constructor
  · intro h
    -- p holds exactly on i + J, for an extreme witness i of the F.
    have probe : ∀ i k, i₁ ≤ i → i ≤ i₂ → k₁ ≤ k → k ≤ k₂ →
        ∃ l, l₁ ≤ l ∧ l ≤ l₂ ∧ i + j₁ ≤ k + l ∧ k + l ≤ i + j₂ := by
      intro i k h1 h2 h3 h4
      have hf : Fi ⟨i₁, some i₂⟩ (Gi ⟨j₁, some j₂⟩ (fun n => i + j₁ ≤ n ∧ n ≤ i + j₂)) 0 :=
        ⟨i, ⟨h1, fun b hb => by simp at hb; omega⟩, fun j ⟨hj1, hj2⟩ => by
          have hj1 : j₁ ≤ j := hj1
          have := hj2 j₂ rfl
          omega⟩
      obtain ⟨l, ⟨hl1, hl2⟩, hp⟩ := h _ 0 hf k ⟨h3, fun b hb => by simp at hb; omega⟩
      have hl1 : l₁ ≤ l := hl1
      have := hl2 l₂ rfl
      exact ⟨l, hl1, this, by omega, by omega⟩
    obtain ⟨l, h1, _, _, h4⟩ := probe i₁ k₂ (Nat.le_refl _) hi hk (Nat.le_refl _)
    obtain ⟨l', _, h2', h3', _⟩ := probe i₂ k₁ hi (Nat.le_refl _) (Nat.le_refl _) hk
    omega
  · intro ⟨c1, c2⟩ p t ⟨i, ⟨hi1, hi2⟩, hg⟩ k ⟨hk1, hk2⟩
    have hi1 : i₁ ≤ i := hi1
    have hi2 := hi2 i₂ rfl
    have hk1 : k₁ ≤ k := hk1
    have hk2 := hk2 k₂ rfl
    let l := max l₁ (i + j₁ - k)
    have hj := hg (k + l - i) ⟨show j₁ ≤ k + l - i by omega,
      fun b hb => by simp at hb; omega⟩
    refine ⟨l, ⟨show l₁ ≤ l by omega, fun b hb => by simp at hb; omega⟩, ?_⟩
    have e : t + i + (k + l - i) = t + k + l := by omega
    rw [e] at hj
    exact hj

end Gawain
