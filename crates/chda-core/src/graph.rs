//! Read-only history query boundary and page-independent graph geometry.
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

pub use chda_git::{CommitInfo, CommitRef, CommitRefKind, HistoryPage};
pub const HISTORY_PAGE_SIZE: usize = 200;

/// An owning query used only by background tasks. UI receives plain page data.
/// Refresh creates a new query, so refs and ancestry belong to one snapshot.
pub struct GraphQuery {
    repo: PathBuf,
    history: Mutex<Option<chda_git::CommitHistory>>,
}

impl GraphQuery {
    pub fn new(repo: PathBuf) -> Self {
        Self {
            repo,
            history: Mutex::new(None),
        }
    }

    pub fn next_page(&self) -> io::Result<HistoryPage> {
        let mut history = self
            .history
            .lock()
            .map_err(|_| io::Error::other("history query stopped"))?;
        if history.is_none() {
            *history = Some(chda_git::CommitHistory::open(&self.repo)?);
        }
        history
            .as_mut()
            .expect("history was initialized")
            .next_page(HISTORY_PAGE_SIZE)
    }
}

/// A line segment between the top, middle or bottom of one commit row.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphLine {
    pub from: (usize, f32),
    pub to: (usize, f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphRow {
    pub commit: CommitInfo,
    pub column: usize,
    pub columns: usize,
    pub lines: Vec<GraphLine>,
}

/// Expected parent ids, retained across pages. Topological history guarantees
/// that a parent is below every child, even when commit clocks disagree.
#[derive(Default)]
pub struct GraphLayout {
    lanes: Vec<String>,
}

impl GraphLayout {
    pub fn append(&mut self, commit: CommitInfo) -> GraphRow {
        let incoming = self.lanes.contains(&commit.id);
        let column = match self.lanes.iter().position(|id| id == &commit.id) {
            Some(column) => column,
            None => {
                self.lanes.push(commit.id.clone());
                self.lanes.len() - 1
            }
        };
        let before = self.lanes.clone();
        self.lanes.remove(column);
        for (offset, parent) in commit.parents.iter().enumerate() {
            if !self.lanes.contains(parent) {
                self.lanes
                    .insert((column + offset).min(self.lanes.len()), parent.clone());
            }
        }
        let mut lines = Vec::new();
        if incoming {
            lines.push(GraphLine {
                from: (column, 0.0),
                to: (column, 0.5),
            });
        }
        for (from, id) in before.iter().enumerate() {
            if from == column {
                continue;
            }
            if let Some(to) = self.lanes.iter().position(|lane| lane == id) {
                lines.push(GraphLine {
                    from: (from, 0.0),
                    to: (to, 1.0),
                });
            }
        }
        for parent in &commit.parents {
            let to = self
                .lanes
                .iter()
                .position(|lane| lane == parent)
                .expect("parent has a lane");
            lines.push(GraphLine {
                from: (column, 0.5),
                to: (to, 1.0),
            });
        }
        GraphRow {
            columns: before.len().max(self.lanes.len()),
            commit,
            column,
            lines,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn commit(id: &str, parents: &[&str]) -> CommitInfo {
        CommitInfo {
            id: id.into(),
            parents: parents.iter().map(|id| (*id).into()).collect(),
            subject: id.into(),
            refs: Vec::new(),
        }
    }
    #[test]
    fn lanes_join_merge_parents_and_continue_between_pages() {
        let mut layout = GraphLayout::default();
        let merge = layout.append(commit("merge", &["main", "feature"]));
        assert_eq!(merge.columns, 2);
        assert_eq!(merge.lines.len(), 2);
        let main = layout.append(commit("main", &["base"]));
        assert!(main.lines.contains(&GraphLine {
            from: (1, 0.0),
            to: (1, 1.0)
        }));
        let feature = layout.append(commit("feature", &["base"]));
        assert!(feature.lines.contains(&GraphLine {
            from: (1, 0.5),
            to: (0, 1.0)
        }));
        let base = layout.append(commit("base", &[]));
        assert_eq!(
            base.lines,
            vec![GraphLine {
                from: (0, 0.0),
                to: (0, 0.5)
            }]
        );
        for i in 0..201 {
            let row = layout.append(commit(
                &format!("linear-{i}"),
                &[&format!("linear-{}", i + 1)],
            ));
            assert_eq!(row.column, 0);
            if i > 0 {
                assert!(row.lines.contains(&GraphLine {
                    from: (0, 0.0),
                    to: (0, 0.5)
                }));
            }
        }
    }
}
