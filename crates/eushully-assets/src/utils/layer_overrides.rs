// layer_overrides.rs — Which entries are shadowed by a later copy of the
// same file in a layered archive set (Parente: base < Append < Patch).
//
// Entries must be supplied in engine load order; the LAST occurrence of a
// name is the one the game actually loads. Names compare case-insensitively
// (Unity bundle names are lower-case, and the engine runs on Windows).

use std::collections::HashMap;

/// For each entry, the index of the entry that overrides it, or `None` when
/// the entry itself is the effective copy.
pub fn find_overriding_entries<'a>(
    entry_names_in_load_order: impl DoubleEndedIterator<Item = &'a str> + ExactSizeIterator,
) -> Vec<Option<usize>> {
    let entry_count = entry_names_in_load_order.len();
    let mut overriding_entry_by_index = vec![None; entry_count];
    let mut effective_index_by_name: HashMap<String, usize> = HashMap::with_capacity(entry_count);

    for (index, name) in entry_names_in_load_order.enumerate().rev() {
        let name_key = name.to_lowercase();
        match effective_index_by_name.get(&name_key) {
            Some(&effective_index) => overriding_entry_by_index[index] = Some(effective_index),
            None => { effective_index_by_name.insert(name_key, index); }
        }
    }
    overriding_entry_by_index
}

/// Order entry indices for extraction: load order (ascending), minus entries
/// whose effective copy is ALSO being extracted. Copies of one file share an
/// output path, so without this the write order — and therefore which tier's
/// version you get — would be arbitrary. If only shadowed copies are chosen
/// they are all kept; load order makes the latest of them land last.
pub fn extraction_order(
    mut entry_indices: Vec<usize>,
    overriding_entry_by_index: &[Option<usize>],
) -> Vec<usize> {
    entry_indices.sort_unstable();
    entry_indices.dedup();
    let chosen: std::collections::HashSet<usize> = entry_indices.iter().copied().collect();
    entry_indices.retain(|index| {
        let overriding = overriding_entry_by_index.get(*index).copied().flatten();
        !overriding.is_some_and(|winner| chosen.contains(&winner))
    });
    entry_indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extraction_order_sorts_and_drops_shadowed_when_winner_chosen() {
        // 0 and 2 are shadowed by 3; 1 is unique.
        let overrides = [Some(3), None, Some(3), None];
        assert_eq!(extraction_order(vec![3, 1, 0, 2], &overrides), vec![1, 3]);
        // Winner not chosen: keep shadowed copies, in load order.
        assert_eq!(extraction_order(vec![2, 0], &overrides), vec![0, 2]);
        // AGE titles have no override data.
        assert_eq!(extraction_order(vec![5, 4], &[]), vec![4, 5]);
    }

    #[test]
    fn last_occurrence_wins_and_earlier_copies_point_at_it() {
        let names = ["src/a.txt", "img/b.bytes", "SRC/A.txt", "src/a.txt"];
        assert_eq!(
            find_overriding_entries(names.iter().copied()),
            vec![Some(3), None, Some(3), None]
        );
    }

    #[test]
    fn unique_names_are_all_effective() {
        let names = ["a", "b", "c"];
        assert_eq!(find_overriding_entries(names.iter().copied()), vec![None, None, None]);
    }
}
