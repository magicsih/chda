//! Session restore: the window's tabs, split layout, working directories
//! and tab names, saved as JSON in the data directory and rebuilt on the
//! next launch. Running programs are not restored; each pane gets a shell.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::workspace::{Axis, Node, PaneId, PaneInfo, Workspace};

/// File name inside the data directory.
const FILE_NAME: &str = "session.json";

/// Everything needed to rebuild a window.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedWindow {
    /// Window frame in screen points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<SavedBounds>,
    pub tabs: Vec<SavedTab>,
    /// Index of the active tab.
    pub active: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    /// Name the user gave the tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub root: SavedNode,
    /// Leaf index (in tree order) of the focused pane.
    pub focused: usize,
    /// Leaf index of the zoomed pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoomed: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SavedNode {
    Pane {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
        /// Main worktree of the repository the pane was in, the fallback
        /// when `cwd` is gone.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo: Option<PathBuf>,
    },
    Split {
        /// `horizontal` places the children side by side.
        horizontal: bool,
        ratio: f32,
        first: Box<SavedNode>,
        second: Box<SavedNode>,
    },
}

/// A pane to start after a restore, with where its shell should run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoredPane {
    pub pane: PaneId,
    /// `None`: the home directory.
    pub cwd: Option<PathBuf>,
}

/// Saved directories that no longer exist, with what replaced them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub missing: Vec<(PathBuf, Option<PathBuf>)>,
}

impl SavedWindow {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    /// The saved window, if there is a readable one with at least one tab.
    pub fn load(data_dir: &Path) -> Option<Self> {
        let text = fs::read_to_string(Self::path(data_dir)).ok()?;
        serde_json::from_str::<Self>(&text)
            .ok()
            .filter(|w| !w.tabs.is_empty())
    }

    /// Write the window, or remove the file when there are no tabs left (the
    /// user closed everything, so the next launch starts fresh). Skips the
    /// write when the file already holds the same content.
    pub fn save(&self, data_dir: &Path) -> io::Result<()> {
        let path = Self::path(data_dir);
        if self.tabs.is_empty() {
            return match fs::remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        if fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
            return Ok(());
        }
        fs::create_dir_all(data_dir)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, text)?;
        fs::rename(tmp, path)
    }
}

impl Workspace {
    /// Capture tabs, layout, directories and tab names.
    pub fn snapshot(&self) -> SavedWindow {
        let tabs = self
            .tabs()
            .iter()
            .map(|tab| {
                let leaves = tab.panes();
                let index = |p: PaneId| leaves.iter().position(|l| *l == p);
                SavedTab {
                    title: tab.custom_title.clone(),
                    root: self.save_node(&tab.root),
                    focused: index(tab.focused).unwrap_or(0),
                    zoomed: tab.zoomed.and_then(index),
                }
            })
            .collect();
        SavedWindow {
            bounds: None,
            tabs,
            active: self.active_index().unwrap_or(0),
        }
    }

    fn save_node(&self, node: &Node) -> SavedNode {
        match node {
            Node::Leaf(p) => {
                let info = self.pane(*p);
                SavedNode::Pane {
                    cwd: info.and_then(|i| i.cwd.clone()),
                    repo: info.and_then(|i| i.repo.clone()),
                }
            }
            Node::Split {
                axis,
                ratio,
                first,
                second,
            } => SavedNode::Split {
                horizontal: *axis == Axis::Horizontal,
                ratio: *ratio,
                first: Box::new(self.save_node(first)),
                second: Box::new(self.save_node(second)),
            },
        }
    }

    /// Rebuild a saved window into this (empty) workspace. Returns the panes
    /// to start and the directories that had to be replaced: a missing
    /// directory falls back to its repository root, else the home directory.
    pub fn restore(&mut self, saved: &SavedWindow) -> (Vec<RestoredPane>, RestoreReport) {
        let mut panes = Vec::new();
        let mut report = RestoreReport::default();
        for tab in &saved.tabs {
            let mut leaves = Vec::new();
            let root = self.restore_node(&tab.root, &mut leaves, &mut report);
            let ids: Vec<PaneId> = leaves.iter().map(|(p, _)| *p).collect();
            let focused = ids.get(tab.focused).or(ids.first()).copied();
            let Some(focused) = focused else {
                continue;
            };
            let zoomed = tab.zoomed.and_then(|i| ids.get(i).copied());
            self.push_tab(root, focused, zoomed, tab.title.clone());
            panes.extend(
                leaves
                    .into_iter()
                    .map(|(pane, cwd)| RestoredPane { pane, cwd }),
            );
        }
        self.activate_tab(saved.active.min(self.tabs().len().saturating_sub(1)));
        (panes, report)
    }

    fn restore_node(
        &mut self,
        node: &SavedNode,
        leaves: &mut Vec<(PaneId, Option<PathBuf>)>,
        report: &mut RestoreReport,
    ) -> Node {
        match node {
            SavedNode::Pane { cwd, repo } => {
                let resolved = match cwd {
                    Some(dir) if !dir.is_dir() => {
                        let fallback = repo.clone().filter(|r| r.is_dir());
                        report.missing.push((dir.clone(), fallback.clone()));
                        fallback
                    }
                    other => other.clone(),
                };
                let pane = self.add_pane(PaneInfo {
                    cwd: resolved.clone(),
                    ..Default::default()
                });
                leaves.push((pane, resolved));
                Node::Leaf(pane)
            }
            SavedNode::Split {
                horizontal,
                ratio,
                first,
                second,
            } => Node::Split {
                axis: if *horizontal {
                    Axis::Horizontal
                } else {
                    Axis::Vertical
                },
                ratio: ratio.clamp(0.1, 0.9),
                first: Box::new(self.restore_node(first, leaves, report)),
                second: Box::new(self.restore_node(second, leaves, report)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_round_trips_and_missing_directories_fall_back() {
        let tmp = std::env::temp_dir().join(format!("chda-restore-{}", std::process::id()));
        let repo = tmp.join("repo");
        let sub = repo.join("src");
        fs::create_dir_all(&sub).unwrap();

        let mut ws = Workspace::new();
        let (_, a) = ws.new_tab();
        ws.pane_mut(a).unwrap().cwd = Some(sub.clone());
        let b = ws.split(Axis::Vertical).unwrap();
        ws.pane_mut(b).unwrap().cwd = Some(repo.join("gone"));
        ws.pane_mut(b).unwrap().repo = Some(repo.clone());
        ws.resize(crate::Direction::Up, 0.2);
        let tab = ws.active_tab().unwrap().id;
        ws.rename_tab(tab, "work");
        let (_, c) = ws.new_tab();
        ws.pane_mut(c).unwrap().cwd = Some(tmp.join("nowhere"));
        ws.activate_tab(0);

        let Node::Split {
            ratio: saved_ratio, ..
        } = ws.tabs()[0].root
        else {
            panic!("split expected");
        };
        let mut saved = ws.snapshot();
        saved.bounds = Some(SavedBounds {
            x: 10.0,
            y: 20.0,
            width: 800.0,
            height: 600.0,
        });
        saved.save(&tmp).unwrap();
        let loaded = SavedWindow::load(&tmp).unwrap();
        assert_eq!(loaded, saved);

        let mut fresh = Workspace::new();
        let (panes, report) = fresh.restore(&loaded);
        assert_eq!(fresh.tabs().len(), 2);
        assert_eq!(fresh.active_index(), Some(0));
        let first = fresh.tabs()[0].clone();
        assert_eq!(first.custom_title.as_deref(), Some("work"));
        assert_eq!(first.focused, first.panes()[1]);
        let Node::Split { axis, ratio, .. } = first.root else {
            panic!("split expected");
        };
        assert_eq!(axis, Axis::Vertical);
        assert_eq!(ratio, saved_ratio);
        assert_ne!(ratio, 0.5);
        let cwds: Vec<Option<PathBuf>> = panes.iter().map(|p| p.cwd.clone()).collect();
        assert_eq!(cwds, vec![Some(sub), Some(repo.clone()), None]);
        assert_eq!(
            report.missing,
            vec![
                (repo.join("gone"), Some(repo.clone())),
                (tmp.join("nowhere"), None)
            ]
        );
        assert_eq!(fresh.pane(panes[1].pane).unwrap().cwd, Some(repo));

        SavedWindow::default().save(&tmp).unwrap();
        assert!(SavedWindow::load(&tmp).is_none());
        assert!(!SavedWindow::path(&tmp).exists());
        fs::remove_dir_all(&tmp).unwrap();
    }
}
