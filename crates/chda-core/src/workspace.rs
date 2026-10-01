//! Tabs, split trees and focus. Pure data; the UI maps pane ids to views.

use std::path::PathBuf;

/// Identifies a terminal pane across the workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(u64);

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

/// A tab holds one split tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub id: TabId,
    pub root: Node,
    pub focused: PaneId,
    /// A pane that is temporarily shown alone.
    pub zoomed: Option<PaneId>,
    /// Title the user typed; overrides the automatic one.
    pub custom_title: Option<String>,
}

impl Tab {
    pub fn panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.root.leaves(&mut out);
        out
    }

    /// Pane rectangles in the unit square, honoring zoom.
    pub fn layout(&self) -> Vec<(PaneId, Rect)> {
        let full = Rect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        };
        if let Some(z) = self.zoomed {
            return vec![(z, full)];
        }
        let mut out = Vec::new();
        self.root.layout(full, &mut out);
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
    /// The shell rang the bell since the pane was last focused.
    pub bell: bool,
}

/// The whole window: tabs with split panes.
#[derive(Clone, Debug, Default)]
pub struct Workspace {
    tabs: Vec<Tab>,
    active: Option<TabId>,
    panes: std::collections::BTreeMap<PaneId, PaneInfo>,
    next_id: u64,
}

impl Workspace {
    pub fn new() -> Self {
        Self::default()
    }

    fn next(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
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
        self.active_tab().map(|t| t.focused)
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

    /// Title shown on a tab: the focused pane's title, else its cwd's last
    /// component, else a placeholder.
    pub fn tab_title(&self, tab: &Tab) -> String {
        if let Some(t) = &tab.custom_title {
            return t.clone();
        }
        let info = self.panes.get(&tab.focused);
        info.filter(|i| !i.title.is_empty())
            .map(|i| i.title.clone())
            .or_else(|| {
                info.and_then(|i| i.cwd.as_ref())
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "shell".into())
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
                root: Node::Leaf(pane),
                focused: pane,
                zoomed: None,
                custom_title: None,
            },
        );
        self.active = Some(tab);
        (tab, pane)
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
        let tab = self.active_tab_mut()?;
        tab.zoomed = None;
        if !tab.root.split(tab.focused, axis, new) {
            return None;
        }
        tab.focused = new;
        self.panes.insert(new, PaneInfo::default());
        Some(new)
    }

    /// Remove a pane wherever it is. Empty tabs are removed too. Returns the
    /// tab that was closed, if any.
    pub fn close_pane(&mut self, pane: PaneId) -> Option<TabId> {
        self.panes.remove(&pane);
        let idx = self.tabs.iter().position(|t| t.root.contains(pane))?;
        let tab = &mut self.tabs[idx];
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
        let Some(idx) = self.tabs.iter().position(|t| t.root.contains(pane)) else {
            return false;
        };
        self.tabs[idx].focused = pane;
        self.active = Some(self.tabs[idx].id);
        self.clear_bell();
        true
    }

    /// Move focus to the nearest pane in `direction`, by geometry.
    pub fn focus_direction(&mut self, direction: Direction) -> Option<PaneId> {
        let tab = self.active_tab()?;
        let layout = tab.layout();
        let (_, from) = layout.iter().find(|(p, _)| *p == tab.focused)?;
        let (fcx, fcy) = (from.x + from.w / 2.0, from.y + from.h / 2.0);
        let eps = 1e-3;
        let candidate = layout
            .iter()
            .filter(|(p, _)| *p != tab.focused)
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
        let i = panes.iter().position(|p| *p == tab.focused)?;
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
        let Some(tab) = self.active_tab_mut() else {
            return false;
        };
        tab.root.resize(tab.focused, direction, delta)
    }

    pub fn equalize(&mut self) {
        if let Some(tab) = self.active_tab_mut() {
            tab.root.equalize();
        }
    }

    pub fn toggle_zoom(&mut self) {
        if let Some(tab) = self.active_tab_mut() {
            tab.zoomed = match tab.zoomed {
                Some(_) => None,
                None if tab.panes().len() > 1 => Some(tab.focused),
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
        assert!(ws.rename_tab(tab.id, "  build  "));
        let tab = ws.active_tab().unwrap().clone();
        assert_eq!(ws.tab_title(&tab), "build");
        ws.rename_tab(tab.id, "");
        let tab = ws.active_tab().unwrap().clone();
        assert_eq!(ws.tab_title(&tab), "project");
    }
}
