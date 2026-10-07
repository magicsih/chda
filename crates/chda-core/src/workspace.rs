//! Tabs, split trees and focus. Pure data; the UI maps pane ids to views.

use std::path::{Path, PathBuf};

use crate::AgentStatus;

/// Pane and tab ids are unique within the process.
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Identifies a terminal pane across the workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(u64);

impl PaneId {
    /// The number agents report back through `CHDA_PANE_ID`.
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// Identifies a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(u64);

/// Split orientation: `Horizontal` places panes side by side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Direction for focus moves and resizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    fn axis(self) -> Axis {
        match self {
            Direction::Left | Direction::Right => Axis::Horizontal,
            Direction::Up | Direction::Down => Axis::Vertical,
        }
    }

    fn forward(self) -> bool {
        matches!(self, Direction::Right | Direction::Down)
    }
}

/// Rectangle in the unit square of a tab's content area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A binary split tree of panes.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf(PaneId),
    Split {
        axis: Axis,
        /// Share of the first child, in `(0, 1)`.
        ratio: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
}

const MIN_RATIO: f32 = 0.1;

impl Node {
    fn contains(&self, pane: PaneId) -> bool {
        match self {
            Node::Leaf(p) => *p == pane,
            Node::Split { first, second, .. } => first.contains(pane) || second.contains(pane),
        }
    }

    fn leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Leaf(p) => out.push(*p),
            Node::Split { first, second, .. } => {
                first.leaves(out);
                second.leaves(out);
            }
        }
    }

    fn first_leaf(&self) -> PaneId {
        match self {
            Node::Leaf(p) => *p,
            Node::Split { first, .. } => first.first_leaf(),
        }
    }

    /// Replace the leaf `pane` with a split of `pane` and `new`.
    fn split(&mut self, pane: PaneId, axis: Axis, new: PaneId) -> bool {
        match self {
            Node::Leaf(p) if *p == pane => {
                *self = Node::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(Node::Leaf(pane)),
                    second: Box::new(Node::Leaf(new)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { first, second, .. } => {
                first.split(pane, axis, new) || second.split(pane, axis, new)
            }
        }
    }

    /// Remove a leaf; its sibling takes the parent's place. Returns false
    /// when the tree is just this leaf.
    fn remove(&mut self, pane: PaneId) -> bool {
        let Node::Split { first, second, .. } = self else {
            return false;
        };
        if **first == Node::Leaf(pane) {
            *self = std::mem::replace(second, Node::Leaf(pane));
            return true;
        }
        if **second == Node::Leaf(pane) {
            *self = std::mem::replace(first, Node::Leaf(pane));
            return true;
        }
        first.remove(pane) || second.remove(pane)
    }

    /// Lay the tree out inside `rect`.
    fn layout(&self, rect: Rect, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            Node::Leaf(p) => out.push((*p, rect)),
            Node::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (a, b) = match axis {
                    Axis::Horizontal => (
                        Rect {
                            w: rect.w * ratio,
                            ..rect
                        },
                        Rect {
                            x: rect.x + rect.w * ratio,
                            w: rect.w * (1.0 - ratio),
                            ..rect
                        },
                    ),
                    Axis::Vertical => (
                        Rect {
                            h: rect.h * ratio,
                            ..rect
                        },
                        Rect {
                            y: rect.y + rect.h * ratio,
                            h: rect.h * (1.0 - ratio),
                            ..rect
                        },
                    ),
                };
                first.layout(a, out);
                second.layout(b, out);
            }
        }
    }

    /// Adjust the ratio of the nearest ancestor split along `direction`'s
    /// axis whose boundary `pane` touches on that side.
    fn resize(&mut self, pane: PaneId, direction: Direction, delta: f32) -> bool {
        let Node::Split {
            axis,
            ratio,
            first,
            second,
        } = self
        else {
            return false;
        };
        if first.resize(pane, direction, delta) || second.resize(pane, direction, delta) {
            return true;
        }
        if *axis != direction.axis() {
            return false;
        }
        // Growing towards the boundary moves it away from the pane's side.
        let in_first = first.contains(pane);
        let grow_first = in_first == direction.forward();
        let change = if grow_first { delta } else { -delta };
        *ratio = (*ratio + change).clamp(MIN_RATIO, 1.0 - MIN_RATIO);
        true
    }

    fn equalize(&mut self) {
        if let Node::Split {
            ratio,
            first,
            second,
            ..
        } = self
        {
            *ratio = 0.5;
            first.equalize();
            second.equalize();
        }
    }
}

/// A tab is either a terminal split tree or a read-only repository graph.
#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub id: TabId,
    pub content: TabContent,
    /// Title the user typed; overrides the automatic one.
    pub custom_title: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TabContent {
    Terminal(TerminalTab),
    GitGraph { repo: PathBuf },
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalTab {
    pub root: Node,
    pub focused: PaneId,
    /// A pane that is temporarily shown alone.
    pub zoomed: Option<PaneId>,
}

impl Tab {
    pub fn terminal(&self) -> Option<&TerminalTab> {
        match &self.content {
            TabContent::Terminal(terminal) => Some(terminal),
            TabContent::GitGraph { .. } => None,
        }
    }

    fn terminal_mut(&mut self) -> Option<&mut TerminalTab> {
        match &mut self.content {
            TabContent::Terminal(terminal) => Some(terminal),
            TabContent::GitGraph { .. } => None,
        }
    }

    pub fn focused_pane(&self) -> Option<PaneId> {
        self.terminal().map(|terminal| terminal.focused)
    }

    pub fn graph_repo(&self) -> Option<&Path> {
        match &self.content {
            TabContent::GitGraph { repo } => Some(repo),
            TabContent::Terminal(_) => None,
        }
    }

    pub fn panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        if let Some(terminal) = self.terminal() {
            terminal.root.leaves(&mut out);
        }
        out
    }

    /// Pane rectangles in the unit square, honoring zoom.
    pub fn layout(&self) -> Vec<(PaneId, Rect)> {
        let Some(terminal) = self.terminal() else {
            return Vec::new();
        };
        let full = Rect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        };
        if let Some(z) = terminal.zoomed {
            return vec![(z, full)];
        }
        let mut out = Vec::new();
        terminal.root.layout(full, &mut out);
        out
    }
}

/// Per-pane state the UI reports back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaneInfo {
    /// Title from OSC 0/2, if any.
    pub title: String,
    /// Working directory from OSC 7 or the process fallback.
    pub cwd: Option<PathBuf>,
    /// Branch of the worktree containing `cwd`, when known.
    pub branch: Option<String>,
    /// Main worktree path of the repository containing `cwd`, when known.
    pub repo: Option<PathBuf>,
    /// The shell rang the bell since the pane was last focused.
    pub bell: bool,
    /// Milliseconds since the epoch of the last output or input.
    pub last_activity: u64,
    /// Pre-restart work timestamp, separate from this runtime output.
    pub previous_activity: Option<u64>,
    /// The coding agent running in the pane, as its hooks report it.
    pub agent: Option<PaneAgent>,
    /// A runtime event explicitly named this pane; saved conversations and
    /// worktree-only events do not establish a live agent here.
    pub agent_live: bool,
    /// The conversation of the agent running in the pane, kept until the
    /// agent ends so a restored session can reopen it.
    pub agent_session: Option<AgentSessionRef>,
}

/// An agent conversation: which agent and its session id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSessionRef {
    /// Agent id, e.g. `claude` or `codex`.
    pub agent: String,
    pub session: String,
}

/// An agent's state in one pane.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PaneAgent {
    /// Agent id, e.g. `claude` or `codex`.
    pub agent: String,
    /// Idle remains a live session until an exit signal.
    pub status: AgentStatus,
    /// Runtime state timestamp in epoch milliseconds. Review and idle share
    /// the completion/ready time; zero means that time is unavailable.
    pub since: u64,
    /// The user has looked at the pane since it started waiting.
    pub seen: bool,
}

/// How automatic tab titles are chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TitleMode {
    /// Branch name, else directory name.
    #[default]
    Branch,
    /// Directory name.
    Path,
}

/// The whole window: tabs with split panes.
#[derive(Clone, Debug, Default)]
pub struct Workspace {
    tabs: Vec<Tab>,
    active: Option<TabId>,
    panes: std::collections::BTreeMap<PaneId, PaneInfo>,
    pub title_mode: TitleMode,
}

impl Workspace {
    pub fn new() -> Self {
        Self::default()
    }

    fn next(&mut self) -> u64 {
        NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| Some(t.id) == self.active)
    }

    fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let active = self.active?;
        self.tabs.iter_mut().find(|t| t.id == active)
    }

    pub fn active_index(&self) -> Option<usize> {
        self.tabs.iter().position(|t| Some(t.id) == self.active)
    }

    /// The focused pane of the active tab.
    pub fn focused_pane(&self) -> Option<PaneId> {
        self.active_tab().and_then(Tab::focused_pane)
    }

    pub fn pane(&self, pane: PaneId) -> Option<&PaneInfo> {
        self.panes.get(&pane)
    }

    pub fn pane_mut(&mut self, pane: PaneId) -> Option<&mut PaneInfo> {
        self.panes.get_mut(&pane)
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// Title shown on a tab: the user's name, else (by mode) the branch or
    /// directory name, else the OSC title, else a placeholder.
    pub fn tab_title(&self, tab: &Tab) -> String {
        if let Some(t) = &tab.custom_title {
            return t.clone();
        }
        if let Some(repo) = tab.graph_repo() {
            return format!(
                "Git tree · {}",
                repo.file_name().unwrap_or_default().to_string_lossy()
            );
        }
        let Some(info) = tab.focused_pane().and_then(|pane| self.panes.get(&pane)) else {
            return "shell".into();
        };
        let dir = info
            .cwd
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        let auto = match self.title_mode {
            TitleMode::Branch => info.branch.clone().or(dir),
            TitleMode::Path => dir,
        };
        auto.or_else(|| (!info.title.is_empty()).then(|| info.title.clone()))
            .unwrap_or_else(|| "shell".into())
    }

    /// Repository (main worktree path) a tab belongs to, from its focused pane.
    pub fn tab_repo(&self, tab: &Tab) -> Option<PathBuf> {
        tab.graph_repo().map(Path::to_path_buf).or_else(|| {
            tab.focused_pane()
                .and_then(|pane| self.panes.get(&pane))
                .and_then(|i| i.repo.clone())
        })
    }

    /// Tabs whose focused pane is in `repo` (`None`: outside any repository).
    pub fn tabs_in(&self, repo: Option<&Path>) -> Vec<&Tab> {
        self.tabs
            .iter()
            .filter(|t| self.tab_repo(t).as_deref() == repo)
            .collect()
    }

    /// Index of the active tab within its repository group.
    pub fn active_index_in(&self, repo: Option<&Path>) -> Option<usize> {
        self.tabs_in(repo)
            .iter()
            .position(|t| Some(t.id) == self.active)
    }

    /// Activate the `index`-th tab of a repository group.
    pub fn activate_tab_in(&mut self, repo: Option<&Path>, index: usize) -> bool {
        match self.tabs_in(repo).get(index).map(|t| t.id) {
            Some(id) => self.activate_tab_id(id),
            None => false,
        }
    }

    /// Move to the next or previous tab within the active tab's group.
    pub fn cycle_tab_in_group(&mut self, forward: bool) {
        let Some(active) = self.active_tab() else {
            return;
        };
        let repo = self.tab_repo(active);
        let ids: Vec<TabId> = self.tabs_in(repo.as_deref()).iter().map(|t| t.id).collect();
        let Some(i) = ids.iter().position(|id| Some(*id) == self.active) else {
            return;
        };
        let n = ids.len();
        let j = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        self.activate_tab_id(ids[j]);
    }

    /// Every tab with its title, repository and last activity, in tab order.
    pub fn tabs_with_activity(&self) -> Vec<(TabId, String, Option<PathBuf>, u64)> {
        self.tabs
            .iter()
            .map(|t| {
                let activity = t
                    .panes()
                    .iter()
                    .filter_map(|p| self.panes.get(p))
                    .map(|i| i.last_activity)
                    .max()
                    .unwrap_or(0);
                (t.id, self.tab_title(t), self.tab_repo(t), activity)
            })
            .collect()
    }

    /// The pane an agent reported through `CHDA_PANE_ID`.
    pub fn pane_by_raw(&self, raw: u64) -> Option<PaneId> {
        self.panes.keys().copied().find(|p| p.0 == raw)
    }

    /// Record an agent's status in a pane, including live idle. Returns whether
    /// anything changed.
    pub fn set_agent_status(
        &mut self,
        pane: PaneId,
        agent: &str,
        status: AgentStatus,
        at: u64,
    ) -> bool {
        let Some(info) = self.panes.get_mut(&pane) else {
            return false;
        };
        let next = Some(match &info.agent {
            Some(a) if a.status == status && a.agent == agent => a.clone(),
            _ => PaneAgent {
                agent: agent.to_owned(),
                status,
                since: at,
                seen: false,
            },
        });
        let changed = info.agent != next;
        info.agent = next;
        changed
    }

    /// Remember the conversation an agent reported from a pane, or forget it
    /// when the session ended. Returns whether anything changed.
    pub fn set_agent_session(&mut self, pane: PaneId, session: Option<AgentSessionRef>) -> bool {
        let Some(info) = self.panes.get_mut(&pane) else {
            return false;
        };
        let changed = info.agent_session != session;
        info.agent_session = session;
        changed
    }

    /// End runtime tracking without confusing a saved conversation with life.
    pub fn end_agent(&mut self, pane: PaneId) -> bool {
        let Some(info) = self.panes.get_mut(&pane) else {
            return false;
        };
        let changed = info.agent.take().is_some() || info.agent_live;
        info.agent_live = false;
        changed
    }

    /// The user looked at a pane: a finished turn becomes idle and
    /// a waiting agent counts as seen. Returns whether anything changed.
    pub fn mark_seen(&mut self, pane: PaneId) -> bool {
        let Some(info) = self.panes.get_mut(&pane) else {
            return false;
        };
        match &mut info.agent {
            Some(a) if a.status == AgentStatus::Review => {
                a.status = AgentStatus::Idle;
                true
            }
            Some(a) if a.status == AgentStatus::WaitingInput && !a.seen => {
                a.seen = true;
                true
            }
            _ => false,
        }
    }

    /// The most urgent agent among a tab's panes.
    pub fn tab_agent(&self, tab: &Tab) -> Option<&PaneAgent> {
        tab.panes()
            .iter()
            .filter_map(|p| self.panes.get(p)?.agent.as_ref())
            .max_by_key(|a| (a.status.urgency(), a.since))
    }

    /// Panes that need the user: waiting for input first, then finished and
    /// not yet reviewed, newest first within each group.
    pub fn attention_panes(&self) -> Vec<PaneId> {
        let mut out: Vec<(PaneId, &PaneAgent)> = self
            .panes
            .iter()
            .filter_map(|(p, i)| Some((*p, i.agent.as_ref()?)))
            .filter(|(_, a)| matches!(a.status, AgentStatus::WaitingInput | AgentStatus::Review))
            .collect();
        out.sort_by_key(|(_, a)| {
            (
                std::cmp::Reverse(a.status.urgency()),
                std::cmp::Reverse(a.since),
            )
        });
        out.into_iter().map(|(p, _)| p).collect()
    }

    /// Agents waiting for input in panes the user has not looked at since.
    pub fn unseen_waiting(&self) -> usize {
        self.panes
            .values()
            .filter_map(|i| i.agent.as_ref())
            .filter(|a| a.status == AgentStatus::WaitingInput && !a.seen)
            .count()
    }

    /// Open a tab with one pane after the active tab and focus it.
    pub fn new_tab(&mut self) -> (TabId, PaneId) {
        let pane = PaneId(self.next());
        let tab = TabId(self.next());
        self.panes.insert(pane, PaneInfo::default());
        let at = self
            .active_index()
            .map(|i| i + 1)
            .unwrap_or(self.tabs.len());
        self.tabs.insert(
            at,
            Tab {
                id: tab,
                content: TabContent::Terminal(TerminalTab {
                    root: Node::Leaf(pane),
                    focused: pane,
                    zoomed: None,
                }),
                custom_title: None,
            },
        );
        self.active = Some(tab);
        (tab, pane)
    }

    /// Open a graph after the active tab, or activate the existing graph.
    pub fn open_graph(&mut self, repo: PathBuf) -> TabId {
        if let Some(tab) = self
            .tabs
            .iter()
            .find(|tab| tab.graph_repo() == Some(repo.as_path()))
        {
            let id = tab.id;
            self.activate_tab_id(id);
            return id;
        }
        let id = TabId(self.next());
        let at = self
            .active_index()
            .map(|i| i + 1)
            .unwrap_or(self.tabs.len());
        self.tabs.insert(
            at,
            Tab {
                id,
                content: TabContent::GitGraph { repo },
                custom_title: None,
            },
        );
        self.active = Some(id);
        id
    }

    /// Remove a complete tab and all its domain panes.
    pub fn close_tab(&mut self, id: TabId) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        for pane in self.tabs[index].panes() {
            self.panes.remove(&pane);
        }
        self.tabs.remove(index);
        if self.active == Some(id) {
            self.active = self
                .tabs
                .get(index.min(self.tabs.len().saturating_sub(1)))
                .map(|tab| tab.id);
        }
        true
    }

    /// Register a pane that is not in any tab yet (restore builds trees).
    pub(crate) fn add_pane(&mut self, info: PaneInfo) -> PaneId {
        let pane = PaneId(self.next());
        self.panes.insert(pane, info);
        pane
    }

    /// Register a pane under an id an earlier chda process gave it, so the
    /// programs inside keep reporting to it; later ids come after it.
    pub(crate) fn add_pane_with_id(&mut self, id: u64, info: PaneInfo) -> PaneId {
        NEXT_ID.fetch_max(id + 1, std::sync::atomic::Ordering::Relaxed);
        let pane = PaneId(id);
        self.panes.insert(pane, info);
        pane
    }

    /// Append a tab built from registered panes.
    pub(crate) fn push_tab(
        &mut self,
        root: Node,
        focused: PaneId,
        zoomed: Option<PaneId>,
        custom_title: Option<String>,
    ) -> TabId {
        let id = TabId(self.next());
        self.tabs.push(Tab {
            id,
            content: TabContent::Terminal(TerminalTab {
                root,
                focused,
                zoomed,
            }),
            custom_title,
        });
        self.active.get_or_insert(id);
        id
    }

    /// Set or clear (empty string) a tab's custom title.
    pub fn rename_tab(&mut self, id: TabId, title: &str) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        let title = title.trim();
        tab.custom_title = (!title.is_empty()).then(|| title.to_owned());
        true
    }

    pub fn activate_tab(&mut self, index: usize) -> bool {
        match self.tabs.get(index) {
            Some(t) => {
                self.active = Some(t.id);
                self.clear_bell();
                true
            }
            None => false,
        }
    }

    pub fn activate_tab_id(&mut self, id: TabId) -> bool {
        match self.tabs.iter().position(|t| t.id == id) {
            Some(i) => self.activate_tab(i),
            None => false,
        }
    }

    /// Move to the next or previous tab, wrapping around.
    pub fn cycle_tab(&mut self, forward: bool) {
        let Some(i) = self.active_index() else {
            return;
        };
        let n = self.tabs.len();
        let j = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        self.activate_tab(j);
    }

    /// Split the focused pane; the new pane takes focus.
    pub fn split(&mut self, axis: Axis) -> Option<PaneId> {
        let new = PaneId(self.next());
        let tab = self.active_tab_mut()?.terminal_mut()?;
        tab.zoomed = None;
        if !tab.root.split(tab.focused, axis, new) {
            return None;
        }
        tab.focused = new;
        self.panes.insert(new, PaneInfo::default());
        Some(new)
    }

    /// Split the active tab's largest pane along its longer side, for
    /// opening several panes at once: in a landscape tab two panes sit side
    /// by side and four form a 2x2 grid. `aspect` is the tab's width over
    /// its height. The new pane takes focus.
    pub fn split_largest(&mut self, aspect: f32) -> Option<PaneId> {
        let tab = self.active_tab()?;
        let mut largest: Option<(PaneId, Rect)> = None;
        for (pane, rect) in tab.layout() {
            let area = rect.w * rect.h;
            // Strictly larger, so ties go to the first pane in tree order.
            if largest.is_none_or(|(_, r)| area > r.w * r.h + f32::EPSILON) {
                largest = Some((pane, rect));
            }
        }
        let (pane, rect) = largest?;
        let axis = if rect.w * aspect >= rect.h {
            Axis::Horizontal
        } else {
            Axis::Vertical
        };
        self.active_tab_mut()?.terminal_mut()?.focused = pane;
        self.split(axis)
    }

    /// Remove a pane wherever it is. Empty tabs are removed too. Returns the
    /// tab that was closed, if any.
    pub fn close_pane(&mut self, pane: PaneId) -> Option<TabId> {
        self.panes.remove(&pane);
        let idx = self.tabs.iter().position(|t| t.panes().contains(&pane))?;
        let tab = self.tabs[idx].terminal_mut()?;
        if tab.zoomed == Some(pane) {
            tab.zoomed = None;
        }
        if tab.root.remove(pane) {
            if tab.focused == pane {
                tab.focused = tab.root.first_leaf();
            }
            return None;
        }
        let closed = self.tabs.remove(idx).id;
        if self.active == Some(closed) {
            self.active = self
                .tabs
                .get(idx.min(self.tabs.len().saturating_sub(1)))
                .map(|t| t.id);
        }
        Some(closed)
    }

    pub fn focus_pane(&mut self, pane: PaneId) -> bool {
        let Some(idx) = self.tabs.iter().position(|t| t.panes().contains(&pane)) else {
            return false;
        };
        self.tabs[idx]
            .terminal_mut()
            .expect("pane belongs to a terminal")
            .focused = pane;
        self.active = Some(self.tabs[idx].id);
        self.clear_bell();
        true
    }

    /// Move focus to the nearest pane in `direction`, by geometry.
    pub fn focus_direction(&mut self, direction: Direction) -> Option<PaneId> {
        let tab = self.active_tab()?;
        let layout = tab.layout();
        let (_, from) = layout
            .iter()
            .find(|(p, _)| Some(*p) == tab.focused_pane())?;
        let (fcx, fcy) = (from.x + from.w / 2.0, from.y + from.h / 2.0);
        let eps = 1e-3;
        let candidate = layout
            .iter()
            .filter(|(p, _)| Some(*p) != tab.focused_pane())
            .filter(|(_, r)| match direction {
                Direction::Left => (r.x + r.w - from.x).abs() < eps,
                Direction::Right => (r.x - (from.x + from.w)).abs() < eps,
                Direction::Up => (r.y + r.h - from.y).abs() < eps,
                Direction::Down => (r.y - (from.y + from.h)).abs() < eps,
            })
            .filter(|(_, r)| match direction.axis() {
                Axis::Horizontal => r.y < from.y + from.h - eps && r.y + r.h > from.y + eps,
                Axis::Vertical => r.x < from.x + from.w - eps && r.x + r.w > from.x + eps,
            })
            .min_by(|(_, a), (_, b)| {
                let da = (a.x + a.w / 2.0 - fcx).abs() + (a.y + a.h / 2.0 - fcy).abs();
                let db = (b.x + b.w / 2.0 - fcx).abs() + (b.y + b.h / 2.0 - fcy).abs();
                da.total_cmp(&db)
            })
            .map(|(p, _)| *p)?;
        self.focus_pane(candidate);
        Some(candidate)
    }

    /// Focus the next or previous pane in tree order, wrapping around.
    pub fn cycle_pane(&mut self, forward: bool) -> Option<PaneId> {
        let tab = self.active_tab()?;
        let panes = tab.panes();
        let i = panes.iter().position(|p| Some(*p) == tab.focused_pane())?;
        let n = panes.len();
        let j = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        let target = panes[j];
        self.focus_pane(target);
        Some(target)
    }

    /// Grow the focused pane towards `direction` by `delta` of the tab.
    pub fn resize(&mut self, direction: Direction, delta: f32) -> bool {
        let Some(tab) = self.active_tab_mut().and_then(Tab::terminal_mut) else {
            return false;
        };
        tab.root.resize(tab.focused, direction, delta)
    }

    pub fn equalize(&mut self) {
        if let Some(tab) = self.active_tab_mut().and_then(Tab::terminal_mut) {
            tab.root.equalize();
        }
    }

    pub fn toggle_zoom(&mut self) {
        if let Some(tab) = self.active_tab_mut().and_then(Tab::terminal_mut) {
            tab.zoomed = match tab.zoomed {
                Some(_) => None,
                None if !matches!(tab.root, Node::Leaf(_)) => Some(tab.focused),
                None => None,
            };
        }
    }

    fn clear_bell(&mut self) {
        if let Some(p) = self.focused_pane()
            && let Some(info) = self.panes.get_mut(&p)
        {
            info.bell = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_of(ws: &Workspace, pane: PaneId) -> Rect {
        ws.active_tab()
            .unwrap()
            .layout()
            .into_iter()
            .find(|(p, _)| *p == pane)
            .unwrap()
            .1
    }

    #[test]
    fn tabs_open_after_active_and_cycle() {
        let mut ws = Workspace::new();
        let (t1, _) = ws.new_tab();
        let (t3, _) = ws.new_tab();
        ws.activate_tab_id(t1);
        let (t2, _) = ws.new_tab();
        let ids: Vec<TabId> = ws.tabs().iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![t1, t2, t3]);
        assert_eq!(ws.active_index(), Some(1));
        ws.cycle_tab(true);
        assert_eq!(ws.active_index(), Some(2));
        ws.cycle_tab(true);
        assert_eq!(ws.active_index(), Some(0));
        ws.cycle_tab(false);
        assert_eq!(ws.active_index(), Some(2));
    }

    #[test]
    fn split_layout_focus_and_close() {
        let mut ws = Workspace::new();
        let (_, a) = ws.new_tab();
        let b = ws.split(Axis::Horizontal).unwrap();
        let c = ws.split(Axis::Vertical).unwrap();
        // a | (b / c)
        assert_eq!(ws.focused_pane(), Some(c));
        assert_eq!(
            rect_of(&ws, a),
            Rect {
                x: 0.0,
                y: 0.0,
                w: 0.5,
                h: 1.0
            }
        );
        assert_eq!(
            rect_of(&ws, b),
            Rect {
                x: 0.5,
                y: 0.0,
                w: 0.5,
                h: 0.5
            }
        );
        assert_eq!(
            rect_of(&ws, c),
            Rect {
                x: 0.5,
                y: 0.5,
                w: 0.5,
                h: 0.5
            }
        );

        assert_eq!(ws.focus_direction(Direction::Up), Some(b));
        assert_eq!(ws.focus_direction(Direction::Left), Some(a));
        assert_eq!(ws.focus_direction(Direction::Left), None);
        assert_eq!(ws.focus_direction(Direction::Right), Some(b));
        assert_eq!(ws.cycle_pane(true), Some(c));
        assert_eq!(ws.cycle_pane(true), Some(a));

        assert!(ws.resize(Direction::Right, 0.2));
        assert_eq!(rect_of(&ws, a).w, 0.7);
        ws.equalize();
        assert_eq!(rect_of(&ws, a).w, 0.5);

        ws.toggle_zoom();
        assert_eq!(ws.active_tab().unwrap().layout().len(), 1);
        ws.toggle_zoom();

        assert_eq!(ws.close_pane(a), None);
        assert_eq!(ws.focused_pane(), Some(b));
        assert_eq!(
            rect_of(&ws, b),
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 0.5
            }
        );
        assert_eq!(ws.close_pane(b), None);
        let tab = ws.active_tab().unwrap().id;
        assert_eq!(ws.close_pane(c), Some(tab));
        assert!(ws.is_empty());
    }

    #[test]
    fn closing_the_active_tab_moves_to_a_neighbor() {
        let mut ws = Workspace::new();
        let (_, p1) = ws.new_tab();
        let (t2, p2) = ws.new_tab();
        let (t3, _) = ws.new_tab();
        ws.activate_tab_id(t2);
        assert_eq!(ws.close_pane(p2), Some(t2));
        assert_eq!(ws.active_tab().map(|t| t.id), Some(t3));
        ws.activate_tab(0);
        ws.pane_mut(p1).unwrap().title = "vim".into();
        let tab = ws.active_tab().unwrap().clone();
        assert_eq!(ws.tab_title(&tab), "vim");
        ws.pane_mut(p1).unwrap().title.clear();
        ws.pane_mut(p1).unwrap().cwd = Some("/tmp/project".into());
        assert_eq!(ws.tab_title(&tab), "project");
        ws.pane_mut(p1).unwrap().branch = Some("feat/x".into());
        ws.pane_mut(p1).unwrap().repo = Some("/tmp/project".into());
        assert_eq!(ws.tab_title(&tab), "feat/x");
        ws.title_mode = TitleMode::Path;
        assert_eq!(ws.tab_title(&tab), "project");
        ws.title_mode = TitleMode::Branch;
        assert_eq!(ws.tabs_in(Some(Path::new("/tmp/project"))).len(), 1);
        assert_eq!(ws.tabs_in(None).len(), 1);
        assert_eq!(ws.active_index_in(Some(Path::new("/tmp/project"))), Some(0));
        ws.pane_mut(p1).unwrap().last_activity = 5;
        assert_eq!(ws.tabs_with_activity()[0].1, "feat/x");
        assert!(ws.rename_tab(tab.id, "  build  "));
        let tab = ws.active_tab().unwrap().clone();
        assert_eq!(ws.tab_title(&tab), "build");
        ws.rename_tab(tab.id, "");
        let tab = ws.active_tab().unwrap().clone();
        assert_eq!(ws.tab_title(&tab), "feat/x");
    }

    #[test]
    fn activity_changes_preserve_tab_order_and_include_unfocused_splits() {
        let mut ws = Workspace::new();
        let (first, p1) = ws.new_tab();
        let split = ws.split(Axis::Horizontal).unwrap();
        let (second, p2) = ws.new_tab();
        for (pane, at) in [(p1, 100), (p2, 200), (split, 300), (p2, 400)] {
            ws.pane_mut(pane).unwrap().last_activity = at;
            let rows = ws.tabs_with_activity();
            assert_eq!(
                rows.iter().map(|r| r.0).collect::<Vec<_>>(),
                vec![first, second]
            );
        }
        ws.activate_tab_id(first);
        assert_eq!(ws.tabs_with_activity()[0].3, 300);
        assert_eq!(ws.tabs_with_activity()[1].3, 400);
        // Creating after the focused tab and closing follow the actual tab order.
        let (third, p3) = ws.new_tab();
        assert_eq!(
            ws.tabs_with_activity()
                .iter()
                .map(|r| r.0)
                .collect::<Vec<_>>(),
            vec![first, third, second]
        );
        ws.close_pane(p3);
        assert_eq!(
            ws.tabs_with_activity()
                .iter()
                .map(|r| r.0)
                .collect::<Vec<_>>(),
            vec![first, second]
        );
    }

    #[test]
    fn split_largest_tiles_a_landscape_tab_into_a_grid() {
        let mut ws = Workspace::new();
        let (_, a) = ws.new_tab();
        let b = ws.split_largest(1.6).unwrap();
        let c = ws.split_largest(1.6).unwrap();
        let d = ws.split_largest(1.6).unwrap();
        let rects: std::collections::HashMap<PaneId, Rect> =
            ws.active_tab().unwrap().layout().into_iter().collect();
        let at = |p: PaneId| (rects[&p].x, rects[&p].y, rects[&p].w, rects[&p].h);
        assert_eq!(at(a), (0.0, 0.0, 0.5, 0.5));
        assert_eq!(at(c), (0.0, 0.5, 0.5, 0.5));
        assert_eq!(at(b), (0.5, 0.0, 0.5, 0.5));
        assert_eq!(at(d), (0.5, 0.5, 0.5, 0.5));
        assert_eq!(ws.focused_pane(), Some(d));

        // A portrait tab stacks the first two.
        let mut ws = Workspace::new();
        let (_, a) = ws.new_tab();
        let b = ws.split_largest(0.5).unwrap();
        let rects: std::collections::HashMap<PaneId, Rect> =
            ws.active_tab().unwrap().layout().into_iter().collect();
        assert_eq!((rects[&a].h, rects[&b].y), (0.5, 0.5));
    }

    #[test]
    fn pane_agents_drive_tab_status_and_attention_order() {
        let mut ws = Workspace::new();
        let (_, a) = ws.new_tab();
        let b = ws.split(Axis::Horizontal).unwrap();
        let (_, c) = ws.new_tab();
        assert_eq!(ws.pane_by_raw(a.raw()), Some(a));
        assert_eq!(ws.pane_by_raw(999), None);

        assert!(ws.set_agent_status(a, "claude", AgentStatus::Working, 1));
        assert!(!ws.set_agent_status(a, "claude", AgentStatus::Working, 2));
        assert!(ws.set_agent_status(b, "codex", AgentStatus::Review, 3));
        assert!(ws.set_agent_status(c, "claude", AgentStatus::WaitingInput, 4));
        let first = ws.tabs()[0].clone();
        assert_eq!(
            ws.tab_agent(&first).map(|a| a.status),
            Some(AgentStatus::Working)
        );
        assert_eq!(ws.attention_panes(), vec![c, b]);
        assert_eq!(ws.unseen_waiting(), 1);

        // Looking at the waiting pane keeps it waiting but seen; looking at
        // the reviewed one clears it.
        assert!(ws.mark_seen(c));
        assert!(!ws.mark_seen(c));
        assert_eq!(ws.unseen_waiting(), 0);
        assert!(ws.mark_seen(b));
        assert_eq!(ws.attention_panes(), vec![c]);
        assert!(ws.set_agent_status(c, "claude", AgentStatus::Idle, 5));
        assert!(ws.attention_panes().is_empty());
        assert_eq!(
            ws.pane(c).unwrap().agent.as_ref().unwrap().status,
            AgentStatus::Idle
        );
        assert!(ws.end_agent(c));
        assert_eq!(ws.pane(c).unwrap().agent, None);
    }
    #[test]
    fn graphs_have_no_panes_and_terminal_commands_do_not_affect_them() {
        let mut ws = Workspace::new();
        let (terminal, pane) = ws.new_tab();
        let graph = ws.open_graph(PathBuf::from("/repo"));
        assert_eq!(ws.focused_pane(), None);
        assert!(ws.active_tab().unwrap().panes().is_empty());
        assert_eq!(ws.split(Axis::Horizontal), None);
        assert_eq!(ws.split_largest(1.0), None);
        assert_eq!(ws.focus_direction(Direction::Left), None);
        assert_eq!(ws.cycle_pane(true), None);
        assert!(!ws.resize(Direction::Left, 0.1));
        ws.toggle_zoom();
        ws.equalize();
        assert_eq!(ws.open_graph(PathBuf::from("/repo")), graph);
        assert_eq!(ws.tabs().len(), 2);
        assert!(ws.close_tab(graph));
        assert_eq!(ws.active_tab().unwrap().id, terminal);
        assert_eq!(ws.focused_pane(), Some(pane));
    }
}
