//! Session restore: the window's tabs, split layout, working directories
//! and tab names, saved as JSON in the data directory and rebuilt on the
//! next launch. Running programs are not restored; each pane gets a shell,
//! except that a pane running a coding agent remembers the agent and its
//! session so the UI can reopen that conversation.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::workspace::{AgentSessionRef, Axis, Node, PaneId, PaneInfo, Workspace};

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
    #[serde(flatten)]
    pub content: SavedTabContent,
}

/// Terminal fields retain their original JSON shape, so old sessions load.
/// Graph tabs store only their repository; history is always queried afresh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SavedTabContent {
    Terminal {
        root: SavedNode,
        focused: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zoomed: Option<usize>,
    },
    GitGraph {
        graph: PathBuf,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SavedNode {
    Pane {
        #[serde(default)]
        last_activity: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
        /// Main worktree of the repository the pane was in, the fallback
        /// when `cwd` is gone.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo: Option<PathBuf>,
        /// Agent id running in the pane, e.g. `claude`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
        /// That agent's session id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
        /// The pane's id, kept only when handing running panes to a new
        /// chda process (their programs report to it).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<u64>,
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
    /// The agent conversation the pane had open.
    pub agent: Option<AgentSessionRef>,
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
        let saved = SavedSession::load(data_dir)?;
        saved
            .windows
            .get(saved.active_window)
            .or(saved.windows.first())
            .cloned()
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
        self.snapshot_inner(false)
    }

    /// [`Workspace::snapshot`] with pane ids, for a live upgrade.
    pub fn handoff_snapshot(&self) -> SavedWindow {
        self.snapshot_inner(true)
    }

    fn snapshot_inner(&self, ids: bool) -> SavedWindow {
        let tabs = self
            .tabs()
            .iter()
            .map(|tab| {
                let leaves = tab.panes();
                let index = |p: PaneId| leaves.iter().position(|l| *l == p);
                let content = match &tab.content {
                    crate::TabContent::Terminal(terminal) => SavedTabContent::Terminal {
                        root: self.save_node(&terminal.root, ids),
                        focused: index(terminal.focused).unwrap_or(0),
                        zoomed: terminal.zoomed.and_then(index),
                    },
                    crate::TabContent::GitGraph { repo } => SavedTabContent::GitGraph {
                        graph: repo.clone(),
                    },
                };
                SavedTab {
                    title: tab.custom_title.clone(),
                    content,
                }
            })
            .collect();
        SavedWindow {
            bounds: None,
            tabs,
            active: self.active_index().unwrap_or(0),
        }
    }

    fn save_node(&self, node: &Node, ids: bool) -> SavedNode {
        match node {
            Node::Leaf(p) => {
                let info = self.pane(*p);
                let agent = info.and_then(|i| i.agent_session.clone());
                SavedNode::Pane {
                    last_activity: info.map_or(0, |i| i.last_activity),
                    cwd: info.and_then(|i| i.cwd.clone()),
                    repo: info.and_then(|i| i.repo.clone()),
                    agent: agent.as_ref().map(|a| a.agent.clone()),
                    session: agent.map(|a| a.session),
                    pane: ids.then_some(p.raw()),
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
                first: Box::new(self.save_node(first, ids)),
                second: Box::new(self.save_node(second, ids)),
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
            match &tab.content {
                SavedTabContent::GitGraph { graph } => {
                    let id = self.open_graph(graph.clone());
                    if let Some(title) = &tab.title {
                        self.rename_tab(id, title);
                    }
                }
                SavedTabContent::Terminal {
                    root,
                    focused,
                    zoomed,
                } => {
                    let mut leaves = Vec::new();
                    let root = self.restore_node(root, &mut leaves, &mut report);
                    let ids: Vec<PaneId> = leaves.iter().map(|p| p.pane).collect();
                    let focused = ids.get(*focused).or(ids.first()).copied();
                    let Some(focused) = focused else {
                        continue;
                    };
                    let zoomed = zoomed.and_then(|i| ids.get(i).copied());
                    self.push_tab(root, focused, zoomed, tab.title.clone());
                    panes.extend(leaves);
                }
            }
            self.activate_tab(self.tabs().len().saturating_sub(1));
        }
        self.activate_tab(saved.active.min(self.tabs().len().saturating_sub(1)));
        (panes, report)
    }

    fn restore_node(
        &mut self,
        node: &SavedNode,
        leaves: &mut Vec<RestoredPane>,
        report: &mut RestoreReport,
    ) -> Node {
        match node {
            SavedNode::Pane {
                last_activity,
                cwd,
                repo,
                agent,
                session,
                pane,
            } => {
                let resolved = match cwd {
                    Some(dir) if !dir.is_dir() => {
                        let fallback = repo.clone().filter(|r| r.is_dir());
                        report.missing.push((dir.clone(), fallback.clone()));
                        fallback
                    }
                    other => other.clone(),
                };
                let agent = agent
                    .clone()
                    .zip(session.clone())
                    .map(|(agent, session)| AgentSessionRef { agent, session });
                let info = PaneInfo {
                    cwd: resolved.clone(),
                    last_activity: *last_activity,
                    previous_activity: (*last_activity > 0).then_some(*last_activity),
                    agent_session: agent.clone(),
                    ..Default::default()
                };
                let pane = match pane {
                    Some(id) => self.add_pane_with_id(*id, info),
                    None => self.add_pane(info),
                };
                leaves.push(RestoredPane {
                    pane,
                    cwd: resolved,
                    agent,
                });
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
        let conversation = AgentSessionRef {
            agent: "claude".into(),
            session: "s1".into(),
        };
        assert!(ws.set_agent_session(b, Some(conversation.clone())));
        ws.resize(crate::Direction::Up, 0.2);
        let tab = ws.active_tab().unwrap().id;
        ws.rename_tab(tab, "work");
        let (_, c) = ws.new_tab();
        ws.pane_mut(c).unwrap().cwd = Some(tmp.join("nowhere"));
        ws.activate_tab(0);

        let Node::Split {
            ratio: saved_ratio, ..
        } = ws.tabs()[0].terminal().unwrap().root
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
        assert_eq!(first.focused_pane(), Some(first.panes()[1]));
        let Node::Split { axis, ratio, .. } = first.terminal().unwrap().root else {
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
        let agents: Vec<_> = panes.iter().map(|p| p.agent.clone()).collect();
        assert_eq!(agents, vec![None, Some(conversation.clone()), None]);
        assert_eq!(
            fresh.pane(panes[1].pane).unwrap().agent_session,
            Some(conversation)
        );

        // Files written before agents were saved still load.
        let old = r#"{"tabs":[{"root":{"kind":"pane","cwd":"/tmp"},"focused":0}],"active":0}"#;
        fs::write(SavedWindow::path(&tmp), old).unwrap();
        let old = SavedWindow::load(&tmp).unwrap();
        let (panes, _) = Workspace::new().restore(&old);
        assert_eq!(panes[0].agent, None);

        SavedWindow::default().save(&tmp).unwrap();
        assert!(SavedWindow::load(&tmp).is_none());
        assert!(!SavedWindow::path(&tmp).exists());
        fs::remove_dir_all(&tmp).unwrap();
    }
    #[test]
    fn mixed_tab_types_order_names_and_active_position_round_trip() {
        let mut ws = Workspace::new();
        ws.new_tab();
        let graph = ws.open_graph(PathBuf::from("/repo/a"));
        ws.rename_tab(graph, "History A");
        ws.new_tab();
        ws.open_graph(PathBuf::from("/repo/b"));
        ws.activate_tab(1);
        let saved = ws.snapshot();
        let text = serde_json::to_string(&saved).unwrap();
        assert!(text.contains("\"graph\":\"/repo/a\""));
        let loaded = serde_json::from_str(&text).unwrap();
        let mut restored = Workspace::new();
        let (panes, _) = restored.restore(&loaded);
        assert_eq!(panes.len(), 2);
        assert_eq!(restored.snapshot(), saved);
        assert_eq!(
            restored.tabs()[1].custom_title.as_deref(),
            Some("History A")
        );
        assert_eq!(restored.active_index(), Some(1));
    }
}

/// All open windows, with the active window separate from each active tab.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedSession {
    pub windows: Vec<SavedWindow>,
    pub active_window: usize,
}
impl SavedSession {
    pub fn load(data_dir: &Path) -> Option<Self> {
        let text = fs::read_to_string(SavedWindow::path(data_dir)).ok()?;
        serde_json::from_str::<Self>(&text)
            .ok()
            .filter(|s| !s.windows.is_empty())
            .or_else(|| {
                serde_json::from_str::<SavedWindow>(&text)
                    .ok()
                    .filter(|w| !w.tabs.is_empty())
                    .map(|window| Self {
                        windows: vec![window],
                        active_window: 0,
                    })
            })
    }
    pub fn save(&self, data_dir: &Path) -> io::Result<()> {
        if self.windows.is_empty() {
            return SavedWindow::default().save(data_dir);
        }
        let path = SavedWindow::path(data_dir);
        let text = serde_json::to_string_pretty(self)?;
        if fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
            return Ok(());
        }
        fs::create_dir_all(data_dir)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, text)?;
        fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod handoff_tests {
    use super::*;
    use crate::workspace::Axis;

    #[test]
    fn handoff_snapshot_keeps_pane_ids_and_later_ids_follow() {
        let mut ws = Workspace::new();
        let (_, a) = ws.new_tab();
        let b = ws.split(Axis::Vertical).unwrap();
        let saved = ws.handoff_snapshot();
        assert!(
            !serde_json::to_string(&ws.snapshot())
                .unwrap()
                .contains("\"pane\":")
        );

        let mut next = Workspace::new();
        let (panes, _) = next.restore(&saved);
        let ids: Vec<PaneId> = panes.iter().map(|p| p.pane).collect();
        assert_eq!(ids, [a, b]);
        let (_, c) = next.new_tab();
        assert!(c.raw() > b.raw(), "a new pane does not reuse an adopted id");
    }
}
