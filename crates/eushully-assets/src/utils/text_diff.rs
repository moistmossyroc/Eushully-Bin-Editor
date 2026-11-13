// text_diff.rs — Side-by-side line diff rows for the Base vs Patch view.
//
// A translation patch replaces most lines (JP → EN) while keeping the
// script's structure, so replaced blocks are PAIRED line by line rather than
// shown as "all deleted, then all inserted": translated line N sits next to
// its original. Diffing is time-boxed; past the deadline `similar` falls
// back to a coarser (still correct) diff.

use std::time::Duration;

use similar::{DiffOp, TextDiff};

const DIFF_DEADLINE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowChange {
    Same,
    Changed,
    OnlyLeft,
    OnlyRight,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffSide {
    /// 1-based line number in its file.
    pub line_number: usize,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRow {
    pub left: Option<DiffSide>,
    pub right: Option<DiffSide>,
    pub change: RowChange,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffSummary {
    pub same: usize,
    pub changed: usize,
    pub only_left: usize,
    pub only_right: usize,
}

fn strip_line_ending(line: &str) -> String {
    line.trim_end_matches(['\n', '\r']).to_string()
}

fn side(lines: &[&str], index: usize) -> Option<DiffSide> {
    lines.get(index).map(|line| DiffSide { line_number: index + 1, text: strip_line_ending(line) })
}

pub fn side_by_side_rows(left_text: &str, right_text: &str) -> Vec<DiffRow> {
    let diff = TextDiff::configure().timeout(DIFF_DEADLINE).diff_lines(left_text, right_text);
    let left_lines = diff.old_slices();
    let right_lines = diff.new_slices();
    let mut rows = Vec::with_capacity(left_lines.len().max(right_lines.len()));

    for op in diff.ops() {
        match *op {
            DiffOp::Equal { old_index, new_index, len } => {
                for offset in 0..len {
                    rows.push(DiffRow {
                        left: side(left_lines, old_index + offset),
                        right: side(right_lines, new_index + offset),
                        change: RowChange::Same,
                    });
                }
            }
            DiffOp::Delete { old_index, old_len, .. } => {
                rows.extend((0..old_len).map(|offset| DiffRow {
                    left: side(left_lines, old_index + offset),
                    right: None,
                    change: RowChange::OnlyLeft,
                }));
            }
            DiffOp::Insert { new_index, new_len, .. } => {
                rows.extend((0..new_len).map(|offset| DiffRow {
                    left: None,
                    right: side(right_lines, new_index + offset),
                    change: RowChange::OnlyRight,
                }));
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                for offset in 0..old_len.max(new_len) {
                    let left = (offset < old_len).then(|| side(left_lines, old_index + offset)).flatten();
                    let right = (offset < new_len).then(|| side(right_lines, new_index + offset)).flatten();
                    let change = match (&left, &right) {
                        (Some(_), Some(_)) => RowChange::Changed,
                        (Some(_), None) => RowChange::OnlyLeft,
                        _ => RowChange::OnlyRight,
                    };
                    rows.push(DiffRow { left, right, change });
                }
            }
        }
    }
    rows
}

pub fn summarize(rows: &[DiffRow]) -> DiffSummary {
    rows.iter().fold(DiffSummary::default(), |mut summary, row| {
        match row.change {
            RowChange::Same => summary.same += 1,
            RowChange::Changed => summary.changed += 1,
            RowChange::OnlyLeft => summary.only_left += 1,
            RowChange::OnlyRight => summary.only_right += 1,
        }
        summary
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaced_lines_pair_up_side_by_side() {
        let rows = side_by_side_rows("a\nこんにちは\nb\n", "a\nHello\nb\n");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].change, RowChange::Changed);
        assert_eq!(rows[1].left.as_ref().unwrap().text, "こんにちは");
        assert_eq!(rows[1].right.as_ref().unwrap().text, "Hello");
        assert_eq!(rows[2].right.as_ref().unwrap().line_number, 3);
    }

    #[test]
    fn uneven_replace_and_pure_insert_delete() {
        let rows = side_by_side_rows("x\nold1\nold2\ny\n", "x\nnew1\ny\nz\n");
        let summary = summarize(&rows);
        assert_eq!(summary, DiffSummary { same: 2, changed: 1, only_left: 1, only_right: 1 });
    }

    #[test]
    fn crlf_endings_are_stripped() {
        let rows = side_by_side_rows("a\r\n", "a\r\n");
        assert_eq!(rows[0].left.as_ref().unwrap().text, "a");
    }
}
