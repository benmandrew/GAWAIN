import Gawain.Trace

/-!
# Unary LTL templates

A unary template is a word over `X`, `F` and `G` applied to one atom, such as
`G X F p`. `norm_correct` shows that every word, of any length, is equivalent
to one of five normal forms: `X^k p`, `X^k F p`, `X^k G p`, `F G p` and
`G F p`. `nf_exact` decides the order between normal forms exactly, so
`word_exact` decides the implication between any two unary templates. The
first three forms are the intervals `[k, k]`, `F[k, ∞)` and `G[k, ∞)` of
`Interval.lean`.

`bench/operator_orders.py` measures the same order with Spot for words of up
to three operators: 40 words in 12 classes.
-/

namespace Gawain

inductive Op
  | X
  | F
  | G
  deriving DecidableEq, Repr

def Op.eval : Op → Pred → Pred
  | .X, p => Gawain.X p
  | .F, p => Gawain.F p
  | .G, p => Gawain.G p

/-- A word applied to `p`, outermost operator first. -/
def evalW : List Op → Pred → Pred
  | [], p => p
  | o :: w, p => o.eval (evalW w p)

inductive NF
  | xp (k : Nat)
  | xf (k : Nat)
  | xg (k : Nat)
  | fg
  | gf
  deriving DecidableEq, Repr

def NF.eval : NF → Pred → Pred
  | .xp k, p => fun t => p (t + k)
  | .xf k, p => fun t => ∃ j, p (t + k + j)
  | .xg k, p => fun t => ∀ j, p (t + k + j)
  | .fg, p => fun t => ∃ i, ∀ j, p (t + i + j)
  | .gf, p => fun t => ∀ i, ∃ j, p (t + i + j)

/-- The normal form of one operator applied to a normal form. -/
def NF.step : Op → NF → NF
  | .X, .xp k => .xp (k + 1)
  | .X, .xf k => .xf (k + 1)
  | .X, .xg k => .xg (k + 1)
  | .X, .fg => .fg
  | .X, .gf => .gf
  | .F, .xp k => .xf k
  | .F, .xf k => .xf k
  | .F, .xg _ => .fg
  | .F, .fg => .fg
  | .F, .gf => .gf
  | .G, .xp k => .xg k
  | .G, .xf _ => .gf
  | .G, .xg k => .xg k
  | .G, .fg => .fg
  | .G, .gf => .gf

def norm : List Op → NF
  | [] => .xp 0
  | o :: w => NF.step o (norm w)

theorem Op.eval_congr (o : Op) {A B : Pred} (h : ∀ t, A t ↔ B t) (t : Nat) :
    o.eval A t ↔ o.eval B t := by
  cases o with
  | X => exact h (t + 1)
  | F => exact exists_congr fun j => h (t + j)
  | G => exact forall_congr' fun j => h (t + j)

theorem step_correct (o : Op) (n : NF) (p : Pred) (t : Nat) :
    o.eval (n.eval p) t ↔ (NF.step o n).eval p t := by
  cases o <;> cases n <;> simp only [Op.eval, NF.step, NF.eval, Gawain.X, Gawain.F, Gawain.G]
  case X.xp k => rw [show t + 1 + k = t + (k + 1) by omega]
  case X.xf k => simp only [show t + 1 + k = t + (k + 1) by omega]
  case X.xg k => simp only [show t + 1 + k = t + (k + 1) by omega]
  case X.fg =>
    constructor
    · intro ⟨i, h⟩
      exact ⟨1 + i, fun j => by rw [show t + (1 + i) = t + 1 + i by omega]; exact h j⟩
    · intro ⟨i, h⟩
      refine ⟨i, fun j => ?_⟩
      have := h (j + 1)
      rwa [show t + i + (j + 1) = t + 1 + i + j by omega] at this
  case X.gf =>
    constructor
    · intro h i
      obtain ⟨j, hj⟩ := h i
      exact ⟨j + 1, by rw [show t + i + (j + 1) = t + 1 + i + j by omega]; exact hj⟩
    · intro h i
      obtain ⟨j, hj⟩ := h (i + 1)
      exact ⟨j, by rw [show t + 1 + i + j = t + (i + 1) + j by omega]; exact hj⟩
  case F.xp k =>
    constructor
    · intro ⟨j, h⟩
      exact ⟨j, by rw [show t + k + j = t + j + k by omega]; exact h⟩
    · intro ⟨j, h⟩
      exact ⟨j, by rw [show t + j + k = t + k + j by omega]; exact h⟩
  case F.xf k =>
    constructor
    · intro ⟨i, j, h⟩
      exact ⟨i + j, by rw [show t + k + (i + j) = t + i + k + j by omega]; exact h⟩
    · intro ⟨j, h⟩
      exact ⟨0, j, by simpa using h⟩
  case F.xg k =>
    constructor
    · intro ⟨i, h⟩
      exact ⟨i + k, fun j => by rw [show t + (i + k) + j = t + i + k + j by omega]; exact h j⟩
    · intro ⟨i, h⟩
      refine ⟨i, fun j => ?_⟩
      have := h (k + j)
      rwa [show t + i + (k + j) = t + i + k + j by omega] at this
  case F.fg =>
    constructor
    · intro ⟨a, b, h⟩
      exact ⟨a + b, fun j => by rw [show t + (a + b) + j = t + a + b + j by omega]; exact h j⟩
    · intro ⟨i, h⟩
      exact ⟨0, i, by simpa using h⟩
  case F.gf =>
    constructor
    · intro ⟨a, h⟩ i
      obtain ⟨j, hj⟩ := h i
      exact ⟨a + j, by rw [show t + i + (a + j) = t + a + i + j by omega]; exact hj⟩
    · intro h
      exact ⟨0, by simpa using h⟩
  case G.xp k =>
    constructor
    · intro h j
      rw [show t + k + j = t + j + k by omega]; exact h j
    · intro h j
      rw [show t + j + k = t + k + j by omega]; exact h j
  case G.xf k =>
    constructor
    · intro h i
      obtain ⟨j, hj⟩ := h i
      exact ⟨k + j, by rw [show t + i + (k + j) = t + i + k + j by omega]; exact hj⟩
    · intro h i
      obtain ⟨j, hj⟩ := h (i + k)
      exact ⟨j, by rw [show t + i + k + j = t + (i + k) + j by omega]; exact hj⟩
  case G.xg k =>
    constructor
    · intro h j
      simpa using h 0 j
    · intro h i j
      rw [show t + i + k + j = t + k + (i + j) by omega]; exact h _
  case G.fg =>
    constructor
    · intro h
      simpa using h 0
    · intro ⟨i, h⟩ a
      refine ⟨i, fun j => ?_⟩
      rw [show t + a + i + j = t + i + (a + j) by omega]; exact h _
  case G.gf =>
    constructor
    · intro h
      simpa using h 0
    · intro h a i
      obtain ⟨j, hj⟩ := h (a + i)
      exact ⟨j, by rw [show t + a + i + j = t + (a + i) + j by omega]; exact hj⟩

theorem norm_correct (w : List Op) (p : Pred) (t : Nat) : evalW w p t ↔ (norm w).eval p t := by
  induction w generalizing t with
  | nil => simp [evalW, norm, NF.eval]
  | cons o w ih =>
    simp only [evalW, norm]
    exact (Op.eval_congr o ih t).trans (step_correct o (norm w) p t)

/-- The order between normal forms. -/
def nfLe : NF → NF → Bool
  | .xp k, .xp m => decide (k = m)
  | .xp k, .xf m => decide (m ≤ k)
  | .xp _, _ => false
  | .xf k, .xf m => decide (m ≤ k)
  | .xf _, _ => false
  | .xg k, .xp m => decide (k ≤ m)
  | .xg _, .xf _ => true
  | .xg k, .xg m => decide (k ≤ m)
  | .xg _, .fg => true
  | .xg _, .gf => true
  | .fg, .xf _ => true
  | .fg, .fg => true
  | .fg, .gf => true
  | .fg, _ => false
  | .gf, .xf _ => true
  | .gf, .gf => true
  | .gf, _ => false

theorem nf_exact (a b : NF) : (∀ p t, a.eval p t → b.eval p t) ↔ nfLe a b = true := by
  cases a <;> cases b <;>
    simp only [nfLe, NF.eval, decide_eq_true_eq, Bool.false_eq_true, iff_true]
  case xp.xp k m =>
    constructor
    · intro h; have := h (fun n => n = k) 0 (by simp); simp at this; omega
    · intro e p t h; subst e; exact h
  case xp.xf k m =>
    constructor
    · intro h; obtain ⟨j, hj⟩ := h (fun n => n = k) 0 (by simp); simp at hj; omega
    · intro e p t h; exact ⟨k - m, by rw [show t + m + (k - m) = t + k by omega]; exact h⟩
  case xp.xg k m =>
    refine ⟨fun h => ?_, False.elim⟩
    have h' := h (fun n => n = k) 0 (by simp)
    have := h' 0; have := h' 1; omega
  case xp.fg k =>
    refine ⟨fun h => ?_, False.elim⟩
    obtain ⟨i, hi⟩ := h (fun n => n = k) 0 (by simp)
    have := hi 0; have := hi 1; omega
  case xp.gf k =>
    refine ⟨fun h => ?_, False.elim⟩
    obtain ⟨j, hj⟩ := h (fun n => n = k) 0 (by simp) (k + 1); simp at hj; omega
  case xf.xp k m =>
    refine ⟨fun h => ?_, False.elim⟩
    have := h (fun n => n = k + m + 1) 0 ⟨m + 1, by simp <;> omega⟩; simp at this <;> omega
  case xf.xf k m =>
    constructor
    · intro h; obtain ⟨j, hj⟩ := h (fun n => n = k) 0 ⟨0, by simp⟩; simp at hj; omega
    · intro e p t ⟨j, h⟩
      exact ⟨k - m + j, by rw [show t + m + (k - m + j) = t + k + j by omega]; exact h⟩
  case xf.xg k m =>
    refine ⟨fun h => ?_, False.elim⟩
    have h' := h (fun n => n = k) 0 ⟨0, by simp⟩
    have := h' 0; have := h' 1; omega
  case xf.fg k =>
    refine ⟨fun h => ?_, False.elim⟩
    obtain ⟨i, hi⟩ := h (fun n => n = k) 0 ⟨0, by simp⟩
    have := hi 0; have := hi 1; omega
  case xf.gf k =>
    refine ⟨fun h => ?_, False.elim⟩
    obtain ⟨j, hj⟩ := h (fun n => n = k) 0 ⟨0, by simp⟩ (k + 1); simp at hj; omega
  case xg.xp k m =>
    constructor
    · intro h; have := h (fun n => k ≤ n) 0 (fun j => by simp); simp at this; omega
    · intro e p t h
      have := h (m - k); rwa [show t + k + (m - k) = t + m by omega] at this
  case xg.xf k m =>
    intro p t h; exact ⟨k, by rw [show t + m + k = t + k + m by omega]; exact h m⟩
  case xg.xg k m =>
    constructor
    · intro h; have := h (fun n => k ≤ n) 0 (fun j => by simp) 0; simp at this; omega
    · intro e p t h j
      rw [show t + m + j = t + k + (m - k + j) by omega]; exact h _
  case xg.fg k => intro p t h; exact ⟨k, h⟩
  case xg.gf k =>
    intro p t h i; exact ⟨k, by rw [show t + i + k = t + k + i by omega]; exact h i⟩
  case fg.xp m =>
    refine ⟨fun h => ?_, False.elim⟩
    have := h (fun n => m + 1 ≤ n) 0 ⟨m + 1, fun j => by simp <;> omega⟩; simp at this <;> omega
  case fg.xf m =>
    intro p t ⟨i, h⟩; exact ⟨i, by rw [show t + m + i = t + i + m by omega]; exact h m⟩
  case fg.xg m =>
    refine ⟨fun h => ?_, False.elim⟩
    have := h (fun n => m + 1 ≤ n) 0 ⟨m + 1, fun j => by simp <;> omega⟩ 0; simp at this <;> omega
  case fg.fg => intro p t h; exact h
  case fg.gf =>
    intro p t ⟨i, h⟩ a; exact ⟨i, by rw [show t + a + i = t + i + a by omega]; exact h a⟩
  case gf.xp m =>
    refine ⟨fun h => ?_, False.elim⟩
    have := h (fun n => n ≠ m) 0 (fun i => ⟨m + 1, by simp <;> omega⟩); simp at this <;> omega
  case gf.xf m => intro p t h; exact h m
  case gf.xg m =>
    refine ⟨fun h => ?_, False.elim⟩
    have := h (fun n => n ≠ m) 0 (fun i => ⟨m + 1, by simp <;> omega⟩) 0; simp at this <;> omega
  case gf.fg =>
    refine ⟨fun h => ?_, False.elim⟩
    obtain ⟨i, hi⟩ := h (fun n => n % 2 = 0) 0 (fun i => ⟨i % 2, by simp <;> omega⟩)
    have h0 := hi 0; have h1 := hi 1; simp at h0 h1; omega
  case gf.gf => intro p t h; exact h

/-- The implication between two unary templates, decided exactly. -/
theorem word_exact (u v : List Op) :
    (∀ p t, evalW u p t → evalW v p t) ↔ nfLe (norm u) (norm v) = true := by
  rw [← nf_exact]
  exact ⟨fun h p t hu => (norm_correct v p t).1 (h p t ((norm_correct u p t).2 hu)),
    fun h p t hu => (norm_correct v p t).2 (h p t ((norm_correct u p t).1 hu))⟩

end Gawain
