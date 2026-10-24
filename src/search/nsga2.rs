//! NSGA-II ranking and PEREDUR's `nsga2-apportion` survivor step, ported
//! from `include/genetic/nsga2.hpp`, `src/genetic/nsga2.cpp` and
//! `replicate_to_size` in `include/genetic/pipeline.hpp`.
//!
//! Every objective is higher-is-better. The functions work on plain
//! objective vectors, so they are tested without any formula.
//!
//! Each individual is also feasible or not, after Deb's constrained
//! domination: a feasible individual dominates every infeasible one, and
//! two of the same kind compare by [`dominates`]. An infeasible individual
//! is dominated by every feasible one, so it joins a front only after all
//! of them have. Every front is therefore all feasible or all infeasible,
//! and crowding and apportioning never mix the two kinds. With every
//! individual feasible this is plain NSGA-II.

/// True iff `lhs` is at least as good as `rhs` in every objective and
/// strictly better in one.
pub fn dominates(lhs: &[f64], rhs: &[f64]) -> bool {
    assert_eq!(lhs.len(), rhs.len());
    let mut strictly_better = false;
    for (l, r) in lhs.iter().zip(rhs) {
        if l < r {
            return false;
        }
        if l > r {
            strictly_better = true;
        }
    }
    strictly_better
}

/// Constrained domination: `lhs` (feasible iff `lhs_feasible`) beats
/// `rhs` if only `lhs` is feasible, or if both are of one kind and `lhs`
/// [`dominates`] `rhs`.
pub fn constrained_dominates(
    lhs: &[f64],
    lhs_feasible: bool,
    rhs: &[f64],
    rhs_feasible: bool,
) -> bool {
    match (lhs_feasible, rhs_feasible) {
        (true, false) => true,
        (false, true) => false,
        _ => dominates(lhs, rhs),
    }
}

/// Deb's fast non-dominated sort under [`constrained_dominates`]: the
/// front index of each individual, 0 for the Pareto front. `feasible` is
/// indexed like `objectives`.
pub fn non_domination_ranks(objectives: &[Vec<f64>], feasible: &[bool]) -> Vec<usize> {
    assert_eq!(objectives.len(), feasible.len());
    let count = objectives.len();
    let beats = |a: usize, b: usize| {
        constrained_dominates(&objectives[a], feasible[a], &objectives[b], feasible[b])
    };
    let mut dominated: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut domination_count = vec![0usize; count];
    for i in 0..count {
        for j in i + 1..count {
            if beats(i, j) {
                dominated[i].push(j);
                domination_count[j] += 1;
            } else if beats(j, i) {
                dominated[j].push(i);
                domination_count[i] += 1;
            }
        }
    }
    let mut ranks = vec![0; count];
    let mut front: Vec<usize> = (0..count).filter(|&i| domination_count[i] == 0).collect();
    let mut rank = 0;
    while !front.is_empty() {
        let mut next = Vec::new();
        for &member in &front {
            ranks[member] = rank;
            for &loser in &dominated[member] {
                domination_count[loser] -= 1;
                if domination_count[loser] == 0 {
                    next.push(loser);
                }
            }
        }
        rank += 1;
        front = next;
    }
    ranks
}

/// The crowding distance of each individual within its own front. The
/// extremes of an objective get infinity, and an objective constant across a
/// front contributes nothing, not even to its extremes, since on a tie those
/// are arbitrary members.
pub fn crowding_distances(objectives: &[Vec<f64>], ranks: &[usize]) -> Vec<f64> {
    assert_eq!(objectives.len(), ranks.len());
    let mut distances = vec![0.0; objectives.len()];
    let Some(width) = objectives.first().map(Vec::len) else {
        return distances;
    };
    let n_fronts = ranks.iter().max().map_or(0, |&r| r + 1);
    let mut fronts: Vec<Vec<usize>> = vec![Vec::new(); n_fronts];
    for (i, &rank) in ranks.iter().enumerate() {
        fronts[rank].push(i);
    }
    for front in &mut fronts {
        // An objective is a column index into every row, not an item.
        #[allow(clippy::needless_range_loop)]
        for objective in 0..width {
            front.sort_by(|&a, &b| objectives[a][objective].total_cmp(&objectives[b][objective]));
            let lowest = objectives[front[0]][objective];
            let highest = objectives[front[front.len() - 1]][objective];
            let range = highest - lowest;
            if range <= 0.0 {
                continue;
            }
            distances[front[0]] = f64::INFINITY;
            distances[front[front.len() - 1]] = f64::INFINITY;
            for i in 1..front.len().saturating_sub(1) {
                let gap = objectives[front[i + 1]][objective] - objectives[front[i - 1]][objective];
                distances[front[i]] += gap / range;
            }
        }
    }
    distances
}

/// The crowded-comparison order: the indices of `objectives` sorted by front
/// ascending, then crowding distance descending. The sort is stable, so ties
/// keep their input order and a seed reproduces. Also returns each
/// individual's front, indexed like `objectives`.
pub fn crowded_order(objectives: &[Vec<f64>], feasible: &[bool]) -> (Vec<usize>, Vec<usize>) {
    let ranks = non_domination_ranks(objectives, feasible);
    let distances = crowding_distances(objectives, &ranks);
    let mut order: Vec<usize> = (0..objectives.len()).collect();
    order.sort_by(|&a, &b| {
        ranks[a]
            .cmp(&ranks[b])
            .then(distances[b].total_cmp(&distances[a]))
    });
    (order, ranks)
}

/// How many copies of each distinct survivor fill `target` slots, given the
/// survivors' fronts in crowded-comparison order. Each keeps one slot, and
/// the spare slots go by weight `1 / (1 + rank)` under the largest-remainder
/// method, remainder ties to the better-ordered survivor. Draws nothing.
/// Requires `1 <= ranks.len() <= target`.
pub fn apportion(ranks: &[usize], target: usize) -> Vec<usize> {
    let count = ranks.len();
    assert!(count >= 1 && count <= target);
    let spare = target - count;
    let weights: Vec<f64> = ranks.iter().map(|&r| 1.0 / (1.0 + r as f64)).collect();
    let total: f64 = weights.iter().sum();
    let mut copies = vec![1usize; count];
    let mut remainders = vec![0.0; count];
    let mut allocated = 0;
    for i in 0..count {
        let quota = spare as f64 * weights[i] / total;
        let whole = quota.floor();
        copies[i] += whole as usize;
        remainders[i] = quota - whole;
        allocated += whole as usize;
    }
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|&a, &b| remainders[b].total_cmp(&remainders[a]));
    let mut i = 0;
    while allocated < spare {
        copies[order[i % count]] += 1;
        allocated += 1;
        i += 1;
    }
    copies
}

#[cfg(test)]
mod tests {
    use super::*;

    fn objs(rows: &[[f64; 2]]) -> Vec<Vec<f64>> {
        rows.iter().map(|r| r.to_vec()).collect()
    }

    #[test]
    fn dominance_needs_one_strict_gain() {
        assert!(dominates(&[1.0, 0.5], &[0.0, 0.5]));
        assert!(!dominates(&[1.0, 0.5], &[1.0, 0.5]));
        assert!(!dominates(&[1.0, 0.0], &[0.0, 1.0]));
    }

    #[test]
    fn hand_built_fronts_get_their_ranks() {
        // Front 0: (1, .2), (0, 1). Front 1: (1, .1), (0, .9). Front 2: (0, .05).
        let o = objs(&[[1.0, 0.1], [0.0, 1.0], [0.0, 0.05], [1.0, 0.2], [0.0, 0.9]]);
        assert_eq!(non_domination_ranks(&o, &[true; 5]), [1, 0, 2, 0, 1]);
    }

    #[test]
    fn crowding_rewards_the_extremes_and_spacing() {
        // One front of four on a line.
        let o = objs(&[[0.0, 1.0], [0.25, 0.75], [0.5, 0.5], [1.0, 0.0]]);
        let ranks = non_domination_ranks(&o, &[true; 4]);
        assert_eq!(ranks, [0, 0, 0, 0]);
        let d = crowding_distances(&o, &ranks);
        assert_eq!(d[0], f64::INFINITY);
        assert_eq!(d[3], f64::INFINITY);
        // Gaps 0.5 on each objective, range 1: 0.5 + 0.5.
        assert!((d[1] - 1.0).abs() < 1e-12);
        // Gaps 0.75 on each.
        assert!((d[2] - 1.5).abs() < 1e-12);
    }

    #[test]
    fn a_constant_objective_anchors_nothing() {
        let o = objs(&[[0.0, 0.3], [0.0, 0.3], [0.0, 0.3]]);
        let d = crowding_distances(&o, &[0, 0, 0]);
        assert_eq!(d, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn crowded_order_is_rank_then_distance_then_input() {
        let o = objs(&[[0.0, 0.5], [1.0, 0.1], [0.0, 0.5], [0.0, 0.9]]);
        let (order, ranks) = crowded_order(&o, &[true; 4]);
        assert_eq!(ranks, [1, 0, 1, 0]);
        // Front 0 is two extremes, both infinite, kept in input order; the
        // tied pair in front 1 keeps its input order too.
        assert_eq!(order, [1, 3, 0, 2]);
    }

    #[test]
    fn apportion_favours_better_fronts_and_fills_the_target() {
        let copies = apportion(&[0, 0, 1, 3], 12);
        assert_eq!(copies.iter().sum::<usize>(), 12);
        // Spare 8 by weights 1, 1, 1/2, 1/4: quotas 2.91, 2.91, 1.45, 0.73,
        // so 5 whole slots and 3 by remainder to the first, second and last.
        assert_eq!(copies, [4, 4, 2, 2]);
        assert_eq!(apportion(&[0], 5), [5]);
        assert_eq!(apportion(&[0, 2], 2), [1, 1]);
    }

    #[test]
    fn a_feasible_individual_beats_every_infeasible_one() {
        // Worse on every objective, but feasible.
        assert!(constrained_dominates(&[0.0, 0.0], true, &[0.9, 1.0], false));
        assert!(!constrained_dominates(
            &[0.9, 1.0],
            false,
            &[0.0, 0.0],
            true
        ));
        // Two of one kind compare by plain dominance.
        assert!(constrained_dominates(
            &[0.5, 1.0],
            false,
            &[0.2, 1.0],
            false
        ));
        assert!(!constrained_dominates(
            &[0.5, 0.0],
            false,
            &[0.2, 1.0],
            false
        ));
        assert!(constrained_dominates(&[1.0, 0.5], true, &[1.0, 0.4], true));
        assert!(!constrained_dominates(&[1.0, 0.5], true, &[1.0, 0.5], true));
    }

    #[test]
    fn feasible_fronts_come_first_and_never_mix() {
        // Rows 0 and 2 are feasible. The infeasible rows 1 and 3 trade the
        // first objective against the second, and row 1 dominates row 4.
        let o = objs(&[[1.0, 0.2], [0.6, 0.9], [1.0, 0.1], [0.3, 1.0], [0.5, 0.8]]);
        let feasible = [true, false, true, false, false];
        let ranks = non_domination_ranks(&o, &feasible);
        assert_eq!(ranks, [0, 2, 1, 2, 3]);
        // Plain NSGA-II puts the infeasible rows 1 and 3 on the first front.
        assert_eq!(non_domination_ranks(&o, &[true; 5]), [0, 0, 1, 0, 1]);
        let (order, ranks) = crowded_order(&o, &feasible);
        assert_eq!(order, [0, 2, 1, 3, 4]);
        // Front 2 holds only infeasible rows, so they spread on their own
        // objectives: both are extremes.
        let d = crowding_distances(&o, &ranks);
        assert_eq!((d[1], d[3]), (f64::INFINITY, f64::INFINITY));
        // Spare 4 by weights 1, 1/2, 1/3, 1/3, 1/4: quotas 1.66, 0.83,
        // 0.55, 0.55, 0.41, so 1 whole slot and 3 by remainder.
        let sorted: Vec<usize> = order.iter().map(|&i| ranks[i]).collect();
        assert_eq!(apportion(&sorted, 9), [3, 2, 2, 1, 1]);
    }
}
