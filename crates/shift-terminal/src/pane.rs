use shift_pty::PaneId;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Axis {
    /// Side-by-side (split_vertical in the spec's action names).
    Horizontal,
    /// Stacked.
    Vertical,
}

/// Side of a pane a dragged pane is dropped on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    fn axis(self) -> Axis {
        match self {
            Edge::Left | Edge::Right => Axis::Horizontal,
            Edge::Top | Edge::Bottom => Axis::Vertical,
        }
    }

    /// The edge of `rect` nearest to the point: the drop zone under the
    /// pointer (the pane's half on that side).
    pub fn nearest(rect: [f32; 4], x: f32, y: f32) -> Edge {
        let fx = ((x - rect[0]) / rect[2].max(1.0)).clamp(0.0, 1.0);
        let fy = ((y - rect[1]) / rect[3].max(1.0)).clamp(0.0, 1.0);
        [
            (fx, Edge::Left),
            (1.0 - fx, Edge::Right),
            (fy, Edge::Top),
            (1.0 - fy, Edge::Bottom),
        ]
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, e)| e)
        .unwrap_or(Edge::Right)
    }

    /// The point is in the middle third of `rect` on both axes (a drop
    /// there swaps the two panes instead of splitting).
    pub fn in_centre(rect: [f32; 4], x: f32, y: f32) -> bool {
        let fx = (x - rect[0]) / rect[2].max(1.0);
        let fy = (y - rect[1]) / rect[3].max(1.0);
        (1.0 / 3.0..2.0 / 3.0).contains(&fx) && (1.0 / 3.0..2.0 / 3.0).contains(&fy)
    }

    /// What a drop on this edge does, for the drop-zone label.
    pub fn label(self) -> &'static str {
        match self {
            Edge::Left => "Split left",
            Edge::Right => "Split right",
            Edge::Top => "Split up",
            Edge::Bottom => "Split down",
        }
    }

    /// The half of `rect` a drop on this edge would take.
    pub fn zone(self, rect: [f32; 4]) -> [f32; 4] {
        let [x, y, w, h] = rect;
        match self {
            Edge::Left => [x, y, w / 2.0, h],
            Edge::Right => [x + w / 2.0, y, w / 2.0, h],
            Edge::Top => [x, y, w, h / 2.0],
            Edge::Bottom => [x, y + h / 2.0, w, h / 2.0],
        }
    }
}

/// Tree-based split layout (spec: Split panes).
#[derive(Debug, Clone)]
pub enum PaneNode {
    Leaf(PaneId),
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

/// Path from the root: false = first child, true = second.
pub type NodePath = Vec<bool>;

/// Computed leaf rectangle.
pub struct PaneRect {
    pub id: PaneId,
    pub rect: [f32; 4],
}

/// Resize-handle rectangle between two children of a split.
pub struct HandleRect {
    pub path: NodePath,
    pub rect: [f32; 4],
    pub axis: Axis,
    /// Area of the whole split node (for drag → ratio math).
    pub area: [f32; 4],
}

/// Gap between split panes: the 1px divider line (Warp-style flush panes).
pub const HANDLE: f32 = 1.0;
/// Extra grab width on each side of the divider for dragging it.
pub const HANDLE_GRAB: f32 = 4.0;

/// Leaf id of the file viewer pane: a split leaf with no shell session
/// behind it (session ids count up from 1, so this one never collides).
pub const VIEWER_PANE: PaneId = PaneId::MAX;

impl PaneNode {
    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            PaneNode::Leaf(l) => *l == id,
            PaneNode::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    /// More than one leaf (the tab lays out as a split).
    pub fn is_split(&self) -> bool {
        matches!(self, PaneNode::Split { .. })
    }

    /// This tree without the viewer leaf: the shell panes alone (saved
    /// layouts, launch configs).
    pub fn without_viewer(&self) -> PaneNode {
        let mut root = self.clone();
        if root.contains(VIEWER_PANE) {
            root.remove(VIEWER_PANE);
        }
        root
    }

    pub fn leaves(&self) -> Vec<PaneId> {
        match self {
            PaneNode::Leaf(id) => vec![*id],
            PaneNode::Split { first, second, .. } => {
                let mut v = first.leaves();
                v.extend(second.leaves());
                v
            }
        }
    }

    /// Split the leaf `target` in two; new pane goes second. Returns success.
    pub fn split(&mut self, target: PaneId, new: PaneId, axis: Axis) -> bool {
        match self {
            PaneNode::Leaf(id) if *id == target => {
                *self = PaneNode::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(PaneNode::Leaf(target)),
                    second: Box::new(PaneNode::Leaf(new)),
                };
                true
            }
            PaneNode::Leaf(_) => false,
            PaneNode::Split { first, second, .. } => {
                first.split(target, new, axis) || second.split(target, new, axis)
            }
        }
    }

    /// Split the leaf `target` and put `new` on its `edge` side (pane drag
    /// and drop). Returns success.
    pub fn insert(&mut self, target: PaneId, new: PaneId, edge: Edge) -> bool {
        match self {
            PaneNode::Leaf(id) if *id == target => {
                let (a, b) = match edge {
                    Edge::Left | Edge::Top => (new, target),
                    Edge::Right | Edge::Bottom => (target, new),
                };
                *self = PaneNode::Split {
                    axis: edge.axis(),
                    ratio: 0.5,
                    first: Box::new(PaneNode::Leaf(a)),
                    second: Box::new(PaneNode::Leaf(b)),
                };
                true
            }
            PaneNode::Leaf(_) => false,
            PaneNode::Split { first, second, .. } => {
                first.insert(target, new, edge) || second.insert(target, new, edge)
            }
        }
    }

    /// Exchange the places of leaves `a` and `b` (a drop on a pane's centre).
    pub fn swap(&mut self, a: PaneId, b: PaneId) {
        match self {
            PaneNode::Leaf(id) if *id == a => *id = b,
            PaneNode::Leaf(id) if *id == b => *id = a,
            PaneNode::Leaf(_) => {}
            PaneNode::Split { first, second, .. } => {
                first.swap(a, b);
                second.swap(a, b);
            }
        }
    }

    /// Remove a leaf; the sibling replaces the parent. Returns true when the
    /// node itself should be removed (single leaf matched).
    pub fn remove(&mut self, target: PaneId) -> bool {
        match self {
            PaneNode::Leaf(id) => *id == target,
            PaneNode::Split { first, second, .. } => {
                if first.remove(target) {
                    *self = (**second).clone();
                    return false;
                }
                if second.remove(target) {
                    *self = (**first).clone();
                    return false;
                }
                false
            }
        }
    }

    /// Layout the tree into `area`, producing leaf rects and handle rects.
    pub fn layout(
        &self,
        area: [f32; 4],
        panes: &mut Vec<PaneRect>,
        handles: &mut Vec<HandleRect>,
        path: NodePath,
    ) {
        match self {
            PaneNode::Leaf(id) => panes.push(PaneRect {
                id: *id,
                rect: area,
            }),
            PaneNode::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let [x, y, w, h] = area;
                match axis {
                    Axis::Horizontal => {
                        let (fw, sw) = split_sizes(w, *ratio);
                        let mut p1 = path.clone();
                        p1.push(false);
                        first.layout([x, y, fw, h], panes, handles, p1);
                        handles.push(HandleRect {
                            path: path.clone(),
                            rect: [x + fw, y, HANDLE, h],
                            axis: Axis::Horizontal,
                            area,
                        });
                        let mut p2 = path;
                        p2.push(true);
                        second.layout([x + fw + HANDLE, y, sw, h], panes, handles, p2);
                    }
                    Axis::Vertical => {
                        let (fh, sh) = split_sizes(h, *ratio);
                        let mut p1 = path.clone();
                        p1.push(false);
                        first.layout([x, y, w, fh], panes, handles, p1);
                        handles.push(HandleRect {
                            path: path.clone(),
                            rect: [x, y + fh, w, HANDLE],
                            axis: Axis::Vertical,
                            area,
                        });
                        let mut p2 = path;
                        p2.push(true);
                        second.layout([x, y + fh + HANDLE, w, sh], panes, handles, p2);
                    }
                }
            }
        }
    }

    /// Ancestor splits of the leaf `target`, root-first:
    /// (path to the split node, its axis, current ratio).
    pub fn ancestors_of(&self, target: PaneId) -> Option<Vec<(NodePath, Axis, f32)>> {
        fn walk(
            node: &PaneNode,
            target: PaneId,
            path: &mut NodePath,
            acc: &mut Vec<(NodePath, Axis, f32)>,
        ) -> bool {
            match node {
                PaneNode::Leaf(id) => *id == target,
                PaneNode::Split {
                    axis,
                    ratio,
                    first,
                    second,
                } => {
                    acc.push((path.clone(), *axis, *ratio));
                    path.push(false);
                    if walk(first, target, path, acc) {
                        path.pop();
                        return true;
                    }
                    *path.last_mut().unwrap() = true;
                    if walk(second, target, path, acc) {
                        path.pop();
                        return true;
                    }
                    path.pop();
                    acc.pop();
                    false
                }
            }
        }
        let mut acc = Vec::new();
        let mut path = Vec::new();
        walk(self, target, &mut path, &mut acc).then_some(acc)
    }

    /// Nudge the ratio of the split at `path` by `delta` (set_ratio clamps).
    pub fn adjust_ratio(&mut self, path: &[bool], delta: f32) {
        if let Some((_, _, r)) = self.ancestors_at(path) {
            self.set_ratio(path, r + delta);
        }
    }

    /// Ratio of the split at `path`, if it exists.
    fn ancestors_at(&self, path: &[bool]) -> Option<(NodePath, Axis, f32)> {
        let mut node = self;
        for &step in path {
            match node {
                PaneNode::Split { first, second, .. } => {
                    node = if step { second } else { first };
                }
                PaneNode::Leaf(_) => return None,
            }
        }
        match node {
            PaneNode::Split { axis, ratio, .. } => Some((path.to_vec(), *axis, *ratio)),
            PaneNode::Leaf(_) => None,
        }
    }

    /// Set the ratio of the split at `path`.
    pub fn set_ratio(&mut self, path: &[bool], ratio: f32) {
        match self {
            PaneNode::Leaf(_) => {}
            PaneNode::Split {
                ratio: r,
                first,
                second,
                ..
            } => {
                if path.is_empty() {
                    *r = ratio.clamp(0.1, 0.9);
                } else if path[0] {
                    second.set_ratio(&path[1..], ratio);
                } else {
                    first.set_ratio(&path[1..], ratio);
                }
            }
        }
    }
}

/// Directional focus: pick the leaf whose center is nearest in `dir` from
/// the active pane's center.
pub fn focus_neighbor(panes: &[PaneRect], active: PaneId, dir: (f32, f32)) -> Option<PaneId> {
    let cur = panes.iter().find(|p| p.id == active)?;
    let (cx, cy) = center(cur.rect);
    panes
        .iter()
        .filter(|p| p.id != active)
        .filter_map(|p| {
            let (px, py) = center(p.rect);
            let (dx, dy) = (px - cx, py - cy);
            let along = dx * dir.0 + dy * dir.1;
            if along > 1.0 {
                let ortho = (dx * dir.1).abs() + (dy * dir.0).abs();
                Some((p.id, along + ortho * 2.0))
            } else {
                None
            }
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

fn center(r: [f32; 4]) -> (f32, f32) {
    (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
}

/// Sizes of the two children of a split along an extent `total` (handle
/// excluded): each keeps 60px when it fits, and the pair never exceeds the
/// space — below 120px the children shrink evenly instead of overflowing.
fn split_sizes(total: f32, ratio: f32) -> (f32, f32) {
    let avail = (total - HANDLE).max(0.0);
    let min = 60.0_f32.min(avail / 2.0);
    // whole pixels, so the 1px divider between the panes stays crisp
    let first = (avail * ratio).round().clamp(min, avail - min);
    (first, avail - first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_splits_never_overflow_the_parent() {
        for total in [0.0, 30.0, 100.0, 130.0, 200.0, 800.0] {
            for ratio in [0.05, 0.5, 0.95] {
                let (a, b) = split_sizes(total, ratio);
                assert!(a >= 0.0 && b >= 0.0);
                assert!(
                    a + b + HANDLE <= total.max(HANDLE) + 0.001,
                    "{total} {ratio}"
                );
            }
        }
        assert_eq!(split_sizes(801.0, 0.5), (400.0, 400.0));
        assert_eq!(split_sizes(801.0, 0.01), (60.0, 740.0));
    }

    #[test]
    fn split_and_remove() {
        let mut root = PaneNode::Leaf(1);
        assert!(root.split(1, 2, Axis::Horizontal));
        assert_eq!(root.leaves(), vec![1, 2]);
        assert!(root.split(2, 3, Axis::Vertical));
        assert_eq!(root.leaves(), vec![1, 2, 3]);
        root.remove(2);
        assert_eq!(root.leaves(), vec![1, 3]);
        root.remove(3);
        assert_eq!(root.leaves(), vec![1]);
    }

    #[test]
    fn insert_on_each_edge() {
        let r = [0.0, 0.0, 100.0, 100.0];
        assert_eq!(Edge::nearest(r, 10.0, 50.0), Edge::Left);
        assert_eq!(Edge::nearest(r, 90.0, 40.0), Edge::Right);
        assert_eq!(Edge::nearest(r, 50.0, 5.0), Edge::Top);
        assert_eq!(Edge::nearest(r, 45.0, 95.0), Edge::Bottom);
        assert_eq!(Edge::Bottom.zone(r), [0.0, 50.0, 100.0, 50.0]);

        // 1 | 2, then 3 dropped left of 2 and 4 above 1
        let mut root = PaneNode::Leaf(1);
        root.split(1, 2, Axis::Horizontal);
        assert!(root.insert(2, 3, Edge::Left));
        assert_eq!(root.leaves(), vec![1, 3, 2]);
        assert!(root.insert(1, 4, Edge::Top));
        assert_eq!(root.leaves(), vec![4, 1, 3, 2]);
        assert!(root.insert(2, 5, Edge::Bottom));
        assert_eq!(root.leaves(), vec![4, 1, 3, 2, 5]);
        assert!(!root.insert(99, 6, Edge::Right));
        let anc = root.ancestors_of(4).unwrap();
        assert_eq!(anc.last().unwrap().1, Axis::Vertical);
    }

    #[test]
    fn centre_drop_swaps_two_leaves() {
        let r = [0.0, 0.0, 90.0, 90.0];
        assert!(Edge::in_centre(r, 45.0, 45.0));
        assert!(!Edge::in_centre(r, 20.0, 45.0));
        assert!(!Edge::in_centre(r, 45.0, 80.0));

        let mut root = PaneNode::Leaf(1);
        root.split(1, 2, Axis::Horizontal);
        root.insert(2, 3, Edge::Bottom);
        root.swap(1, 3);
        assert_eq!(root.leaves(), vec![3, 2, 1]);
        root.swap(3, 1);
        assert_eq!(root.leaves(), vec![1, 2, 3]);
    }

    #[test]
    fn layout_ratio() {
        let mut root = PaneNode::Leaf(1);
        root.split(1, 2, Axis::Horizontal);
        root.set_ratio(&[], 0.6);
        let mut panes = Vec::new();
        let mut handles = Vec::new();
        root.layout([0.0, 0.0, 1001.0, 500.0], &mut panes, &mut handles, vec![]);
        assert_eq!(panes.len(), 2);
        assert_eq!(handles.len(), 1);
        assert!((panes[0].rect[2] - 600.0).abs() < 1.0);
    }

    #[test]
    fn ancestors_and_adjust_ratio() {
        // 1 | (2 / 3): horizontal root, vertical second child
        let mut root = PaneNode::Leaf(1);
        root.split(1, 2, Axis::Horizontal);
        root.split(2, 3, Axis::Vertical);

        let anc = root.ancestors_of(3).unwrap();
        assert_eq!(anc.len(), 2);
        assert_eq!((anc[0].0.as_slice(), anc[0].1), (&[][..], Axis::Horizontal));
        assert_eq!(
            (anc[1].0.as_slice(), anc[1].1),
            (&[true][..], Axis::Vertical)
        );

        let anc1 = root.ancestors_of(1).unwrap();
        assert_eq!(anc1.len(), 1);
        assert!(root.ancestors_of(99).is_none());

        // adjust nudges, set_ratio clamps
        root.adjust_ratio(&[true], 0.2);
        let (_, _, r) = root.ancestors_at(&[true]).unwrap();
        assert!((r - 0.7).abs() < 1e-6);
        for _ in 0..10 {
            root.adjust_ratio(&[true], 0.2);
        }
        let (_, _, r) = root.ancestors_at(&[true]).unwrap();
        assert!((r - 0.9).abs() < 1e-6);
        // path to a leaf: no-op
        root.adjust_ratio(&[false], 0.2);
    }

    #[test]
    fn neighbor_focus() {
        let panes = vec![
            PaneRect {
                id: 1,
                rect: [0.0, 0.0, 100.0, 100.0],
            },
            PaneRect {
                id: 2,
                rect: [110.0, 0.0, 100.0, 100.0],
            },
        ];
        assert_eq!(focus_neighbor(&panes, 1, (1.0, 0.0)), Some(2));
        assert_eq!(focus_neighbor(&panes, 2, (-1.0, 0.0)), Some(1));
        assert_eq!(focus_neighbor(&panes, 1, (-1.0, 0.0)), None);
    }
}
