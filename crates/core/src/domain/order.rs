//! Page-order arithmetic.
//!
//! The move algorithm lives here, pure and free of any project types, so it
//! can be reasoned about and tested on plain indices rather than through the
//! command layer that uses it.

/// The result of planning a move: where every page ends up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovePlan {
    /// `permutation[i]` is the *old* index of the page now at position `i`.
    pub permutation: Vec<usize>,
}

impl MovePlan {
    /// Reorder `items` in place according to [`Self::permutation`].
    pub fn apply<T>(&self, items: &mut Vec<T>) {
        debug_assert_eq!(items.len(), self.permutation.len());
        let mut slots: Vec<Option<T>> = items.drain(..).map(Some).collect();
        items.extend(self.permutation.iter().map(|&old| {
            slots[old]
                .take()
                .expect("permutation visits each index once")
        }));
    }
}

/// Plan moving the pages at `indices` so they sit as one contiguous block
/// immediately before the page originally at `before`.
///
/// `indices` may be unsorted and non-contiguous; duplicates are ignored. Both
/// `indices` and `before` are expressed in *original* list coordinates, which
/// is what a drop position and an arrow press naturally produce. `before` may
/// be `len`, meaning "append after the last page".
///
/// Returns `None` when the move is invalid (an index is out of range, `before`
/// exceeds `len`) or would not change anything.
pub fn plan_move(len: usize, indices: &[usize], before: usize) -> Option<MovePlan> {
    if indices.is_empty() || before > len {
        return None;
    }

    let mut moved: Vec<usize> = indices.to_vec();
    moved.sort_unstable();
    moved.dedup();
    if moved.last().is_some_and(|&i| i >= len) {
        return None;
    }

    let mut is_moved = vec![false; len];
    for &i in &moved {
        is_moved[i] = true;
    }
    let stay = || (0..len).filter(|&i| !is_moved[i]);

    // Insertion point in the list with the moved pages taken out.
    let block_start = before - moved.iter().filter(|&&i| i < before).count();

    let mut permutation = Vec::with_capacity(len);
    permutation.extend(stay().take(block_start));
    permutation.extend(moved.iter().copied());
    permutation.extend(stay().skip(block_start));

    if permutation.iter().copied().eq(0..len) {
        return None;
    }

    Some(MovePlan { permutation })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply a plan to `0..len` so tests can read the resulting order directly.
    fn order(len: usize, indices: &[usize], before: usize) -> Option<Vec<usize>> {
        let plan = plan_move(len, indices, before)?;
        let mut items: Vec<usize> = (0..len).collect();
        plan.apply(&mut items);
        Some(items)
    }

    #[test]
    fn moves_single_page_forward() {
        // "before: 3" means it lands ahead of the page originally at index 3.
        assert_eq!(order(5, &[1], 3), Some(vec![0, 2, 1, 3, 4]));
    }

    #[test]
    fn moves_single_page_backward() {
        assert_eq!(order(5, &[3], 1), Some(vec![0, 3, 1, 2, 4]));
    }

    #[test]
    fn moves_to_start_and_end() {
        assert_eq!(order(4, &[2], 0), Some(vec![2, 0, 1, 3]));
        assert_eq!(order(4, &[1], 4), Some(vec![0, 2, 3, 1]));
    }

    #[test]
    fn gathers_non_contiguous_selection_into_a_block() {
        assert_eq!(order(6, &[1, 4], 3), Some(vec![0, 2, 1, 4, 3, 5]));
    }

    #[test]
    fn unsorted_and_duplicate_indices_are_normalised() {
        assert_eq!(order(6, &[4, 1, 4], 3), order(6, &[1, 4], 3));
    }

    #[test]
    fn moved_pages_land_contiguously() {
        let plan = plan_move(6, &[1, 4], 3).unwrap();
        assert_eq!(&plan.permutation[2..4], &[1, 4]);
    }

    #[test]
    fn insertion_point_accounts_for_pages_removed_before_the_target() {
        // Moving 0 and 1 to the end leaves them starting at index 2.
        let plan = plan_move(4, &[0, 1], 4).unwrap();
        assert_eq!(plan.permutation, vec![2, 3, 0, 1]);
    }

    #[test]
    fn no_op_moves_return_none() {
        assert_eq!(plan_move(5, &[2], 2), None); // before itself
        assert_eq!(plan_move(5, &[2], 3), None); // before its successor
        assert_eq!(plan_move(5, &[0, 1], 0), None); // block already in place
        assert_eq!(plan_move(5, &[0, 1, 2, 3, 4], 0), None); // everything, unchanged
    }

    #[test]
    fn invalid_input_returns_none() {
        assert_eq!(plan_move(5, &[], 0), None);
        assert_eq!(plan_move(5, &[5], 0), None);
        assert_eq!(plan_move(5, &[1], 6), None);
        assert_eq!(plan_move(0, &[0], 0), None);
    }

    #[test]
    fn permutation_is_always_a_permutation() {
        let plan = plan_move(6, &[0, 3, 5], 2).unwrap();
        let mut seen = plan.permutation.clone();
        seen.sort_unstable();
        assert_eq!(seen, (0..6).collect::<Vec<_>>());
    }

    #[test]
    fn reversing_a_move_restores_the_original_order() {
        let mut items: Vec<usize> = (0..5).collect();
        plan_move(5, &[1], 4).unwrap().apply(&mut items);
        assert_eq!(items, vec![0, 2, 3, 1, 4]);
        // Page 1 now sits at position 3; move it back before position 1.
        plan_move(5, &[3], 1).unwrap().apply(&mut items);
        assert_eq!(items, (0..5).collect::<Vec<_>>());
    }
}
