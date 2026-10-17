use rift_core::session_state::{SavedAxis, SavedLayout};
use rift_pty::PaneId;

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

    /// The new pane goes before the target (left of it or above it).
    fn before(self) -> bool {
        matches!(self, Edge::Left | Edge::Top)
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

/// Tree-based split layout (spec: Split panes), sized the way Warp sizes its
/// pane group (warp `app/src/pane_group/tree.rs`): a split holds two or more
/// children along one axis, each with a flex weight, and a child's extent is
/// its weight's share of the split. Splitting a pane along its parent's axis
/// adds a sibling there (three panes in a row are one split, a third each);
/// splitting across it nests a two-child split in the pane's place.
#[derive(Debug, Clone)]
pub enum PaneNode {
    Leaf(PaneId),
    Split {
        axis: Axis,
        /// (flex weight, child), first = left or top. Always two or more.
        children: Vec<(f32, PaneNode)>,
    },
}

/// Path from the root: the child index taken at each split.
pub type NodePath = Vec<usize>;

/// Replace `children[i]` when it is a split along `axis` by its own
/// children, each taking its share of the slot (`slot * w_k / Σw`).
fn splice_same_axis(children: &mut Vec<(f32, PaneNode)>, i: usize, axis: Axis) {
    if !matches!(&children[i].1, PaneNode::Split { axis: a, .. } if *a == axis) {
        return;
    }
    let (slot, node) = children.remove(i);
    let PaneNode::Split {
        children: inner, ..
    } = node
    else {
        return;
    };
    let sum: f32 = inner.iter().map(|c| c.0.max(0.0)).sum();
    let n = inner.len() as f32;
    for (k, (w, child)) in inner.into_iter().enumerate() {
        let w = if sum > 0.0 {
            slot * w.max(0.0) / sum
        } else {
            slot / n
        };
        children.insert(i + k, (w, child));
    }
}

/// Computed leaf rectangle.
pub struct PaneRect {
    pub id: PaneId,
    pub rect: [f32; 4],
}

/// Divider between children `index` and `index + 1` of the split at `path`.
pub struct HandleRect {
    pub path: NodePath,
    pub index: usize,
    pub rect: [f32; 4],
    pub axis: Axis,
    /// Area of the whole split node (for drag math).
    pub area: [f32; 4],
}

/// Gap between split panes: the 1px divider line (Warp-style flush panes).
pub const HANDLE: f32 = 1.0;
/// Extra grab width on each side of the divider for dragging it.
pub const HANDLE_GRAB: f32 = 4.0;
/// Flex weight of a new pane (Warp's `DEFAULT_FLEX_VALUE`): it takes an
/// equal share next to siblings that were never resized.
pub const DEFAULT_FLEX: f32 = 1.0;
/// Smallest extent (logical px) a divider move may leave either pane next
/// to it (Warp's `MINIMUM_PANE_SIZE`).
pub const MIN_PANE: f32 = 50.0;
/// One ⌃⌘-arrow resize step, logical px (Warp's `KEYBOARD_RESIZE_DELTA`).
pub const RESIZE_STEP: f32 = 10.0;

/// Leaf id of the file viewer pane: a split leaf with no shell session
/// behind it (session ids count up from 1, so this one never collides).
pub const VIEWER_PANE: PaneId = PaneId::MAX;

enum Removal {
    NotFound,
    /// Removed somewhere below this node.
    Removed,
    /// This node is the leaf to remove (its parent drops it).
    Me,
}

impl PaneNode {
    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            PaneNode::Leaf(l) => *l == id,
            PaneNode::Split { children, .. } => children.iter().any(|(_, c)| c.contains(id)),
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
            PaneNode::Split { children, .. } => {
                children.iter().flat_map(|(_, c)| c.leaves()).collect()
            }
        }
    }

    /// Split the leaf `target`; the new pane goes after it (right or below).
    /// Returns success.
    pub fn split(&mut self, target: PaneId, new: PaneId, axis: Axis) -> bool {
        let edge = match axis {
            Axis::Horizontal => Edge::Right,
            Axis::Vertical => Edge::Bottom,
        };
        self.insert(target, new, edge)
    }

    /// Two leaves along `edge`'s axis, `new` on `edge`'s side of `target`.
    fn pair(target: PaneId, new: PaneId, edge: Edge) -> PaneNode {
        let (a, b) = if edge.before() {
            (new, target)
        } else {
            (target, new)
        };
        PaneNode::Split {
            axis: edge.axis(),
            children: vec![
                (DEFAULT_FLEX, PaneNode::Leaf(a)),
                (DEFAULT_FLEX, PaneNode::Leaf(b)),
            ],
        }
    }

    /// Put `new` on the `edge` side of the leaf `target` (splits and pane
    /// drag and drop). Along the parent split's axis the new pane joins that
    /// split with the default weight (Warp `PaneBranch::split`); across it,
    /// `target` becomes a two-pane split of halves. Returns success.
    pub fn insert(&mut self, target: PaneId, new: PaneId, edge: Edge) -> bool {
        match self {
            PaneNode::Leaf(id) if *id == target => {
                *self = Self::pair(target, new, edge);
                true
            }
            PaneNode::Leaf(_) => false,
            PaneNode::Split { axis, children } => {
                for i in 0..children.len() {
                    if matches!(children[i].1, PaneNode::Leaf(id) if id == target) {
                        if edge.axis() == *axis {
                            let at = if edge.before() { i } else { i + 1 };
                            children.insert(at, (DEFAULT_FLEX, PaneNode::Leaf(new)));
                        } else {
                            children[i].1 = Self::pair(target, new, edge);
                        }
                        return true;
                    }
                    if children[i].1.insert(target, new, edge) {
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Put `new` along the whole tree's `edge`, taking `share` of it (the
    /// file viewer column; a pane moved into a tab with no anchor).
    pub fn insert_root(&mut self, new: PaneId, edge: Edge, share: f32) {
        let share = share.clamp(0.05, 0.95);
        match self {
            PaneNode::Split { axis, children } if *axis == edge.axis() => {
                let sum: f32 = children.iter().map(|c| c.0).sum();
                let w = sum * share / (1.0 - share);
                let at = if edge.before() { 0 } else { children.len() };
                children.insert(at, (w, PaneNode::Leaf(new)));
            }
            _ => {
                let old = std::mem::replace(self, PaneNode::Leaf(new));
                let (a, b) = if edge.before() {
                    ((share, PaneNode::Leaf(new)), (1.0 - share, old))
                } else {
                    ((1.0 - share, old), (share, PaneNode::Leaf(new)))
                };
                *self = PaneNode::Split {
                    axis: edge.axis(),
                    children: vec![a, b],
                };
            }
        }
    }

    /// Exchange the places of leaves `a` and `b` (a drop on a pane's centre).
    pub fn swap(&mut self, a: PaneId, b: PaneId) {
        match self {
            PaneNode::Leaf(id) if *id == a => *id = b,
            PaneNode::Leaf(id) if *id == b => *id = a,
            PaneNode::Leaf(_) => {}
            PaneNode::Split { children, .. } => {
                for (_, c) in children {
                    c.swap(a, b);
                }
            }
        }
    }

    /// Remove a leaf. Its siblings keep their weights, so its space goes to
    /// all of them in proportion (Warp); a split left with one child is
    /// replaced by it. Returns true when the node itself is that leaf.
    pub fn remove(&mut self, target: PaneId) -> bool {
        matches!(self.remove_inner(target), Removal::Me)
    }

    fn remove_inner(&mut self, target: PaneId) -> Removal {
        match self {
            PaneNode::Leaf(id) if *id == target => Removal::Me,
            PaneNode::Leaf(_) => Removal::NotFound,
            PaneNode::Split { axis, children } => {
                let axis = *axis;
                for i in 0..children.len() {
                    match children[i].1.remove_inner(target) {
                        Removal::NotFound => {}
                        Removal::Removed => {
                            // the child may have collapsed into a split along
                            // this axis: join its children into this one
                            splice_same_axis(children, i, axis);
                            return Removal::Removed;
                        }
                        Removal::Me => {
                            children.remove(i);
                            if children.len() == 1
                                && let Some((_, only)) = children.pop()
                            {
                                *self = only;
                            }
                            return Removal::Removed;
                        }
                    }
                }
                Removal::NotFound
            }
        }
    }

    /// Layout the tree into `area`, producing leaf rects and handle rects.
    /// Extents follow the weights, so a window resize keeps every pane's
    /// share (Warp's flex layout).
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
            PaneNode::Split { axis, children } => {
                let [x, y, w, h] = area;
                let flexes: Vec<f32> = children.iter().map(|c| c.0).collect();
                let sizes = flex_sizes(along(*axis, area), &flexes);
                let mut pos = match axis {
                    Axis::Horizontal => x,
                    Axis::Vertical => y,
                };
                for (i, ((_, child), size)) in children.iter().zip(sizes).enumerate() {
                    if i > 0 {
                        handles.push(HandleRect {
                            path: path.clone(),
                            index: i - 1,
                            rect: match axis {
                                Axis::Horizontal => [pos, y, HANDLE, h],
                                Axis::Vertical => [x, pos, w, HANDLE],
                            },
                            axis: *axis,
                            area,
                        });
                        pos += HANDLE;
                    }
                    let rect = match axis {
                        Axis::Horizontal => [pos, y, size, h],
                        Axis::Vertical => [x, pos, w, size],
                    };
                    let mut p = path.clone();
                    p.push(i);
                    child.layout(rect, panes, handles, p);
                    pos += size;
                }
            }
        }
    }

    fn node_at(&self, path: &[usize]) -> Option<&PaneNode> {
        let mut node = self;
        for &i in path {
            match node {
                PaneNode::Split { children, .. } => node = &children.get(i)?.1,
                PaneNode::Leaf(_) => return None,
            }
        }
        Some(node)
    }

    fn node_at_mut(&mut self, path: &[usize]) -> Option<&mut PaneNode> {
        let mut node = self;
        for &i in path {
            match node {
                PaneNode::Split { children, .. } => node = &mut children.get_mut(i)?.1,
                PaneNode::Leaf(_) => return None,
            }
        }
        Some(node)
    }

    /// Give every child of the split at `path` an equal share (Warp's
    /// double-click on a divider). Returns whether anything changed.
    pub fn equalize(&mut self, path: &[usize]) -> bool {
        match self.node_at_mut(path) {
            Some(PaneNode::Split { children, .. }) => {
                let changed = children.iter().any(|c| c.0 != DEFAULT_FLEX);
                for c in children.iter_mut() {
                    c.0 = DEFAULT_FLEX;
                }
                changed
            }
            _ => false,
        }
    }

    /// Equal shares in every split of the tree.
    pub fn equalize_all(&mut self) -> bool {
        match self {
            PaneNode::Leaf(_) => false,
            PaneNode::Split { children, .. } => {
                let mut changed = false;
                for (flex, child) in children.iter_mut() {
                    changed |= *flex != DEFAULT_FLEX;
                    *flex = DEFAULT_FLEX;
                    changed |= child.equalize_all();
                }
                changed
            }
        }
    }

    /// Move divider `index` of the split at `path` (laid out in `area`) by
    /// `delta` px, trading extent between the two panes beside it only
    /// (Warp `PaneBranch::adjust_pane_size`). A move that would leave either
    /// under [`MIN_PANE`] is refused, or with `clamp` stops at it. Returns
    /// whether the divider moved.
    pub fn move_divider(
        &mut self,
        path: &[usize],
        index: usize,
        area: [f32; 4],
        delta: f32,
        clamp: bool,
    ) -> bool {
        let Some(PaneNode::Split { axis, children }) = self.node_at_mut(path) else {
            return false;
        };
        if index + 1 >= children.len() {
            return false;
        }
        let n = children.len() as f32;
        let avail = (along(*axis, area) - HANDLE * (n - 1.0)).max(0.0);
        let sum: f32 = children.iter().map(|c| c.0).sum();
        if sum <= 0.0 {
            return false;
        }
        let (f1, f2) = (children[index].0, children[index + 1].0);
        let (mut s1, mut s2) = (f1 / sum * avail, f2 / sum * avail);
        // a pane under the minimum by weight (a restored 0/1 ratio) shows
        // at the minimum: move from the extents on screen
        let flexes: Vec<f32> = children.iter().map(|c| c.0).collect();
        let shown = flex_sizes(along(*axis, area), &flexes);
        if (shown[index] - s1).abs() > 1.0 || (shown[index + 1] - s2).abs() > 1.0 {
            (s1, s2) = (shown[index], shown[index + 1]);
        }
        // a pane already under the minimum never shrinks further, and the
        // divider never moves against `delta`
        let delta = if clamp {
            let (lo, hi) = ((MIN_PANE - s1).min(0.0), (s2 - MIN_PANE).max(0.0));
            delta.clamp(lo, hi)
        } else if (delta < 0.0 && s1 + delta < MIN_PANE) || (delta > 0.0 && s2 - delta < MIN_PANE) {
            return false;
        } else {
            delta
        };
        if delta.abs() < f32::EPSILON || s1 + s2 <= 0.0 {
            return false;
        }
        let total = f1 + f2;
        let first = ((s1 + delta) / (s1 + s2) * total).clamp(0.0, total);
        children[index].0 = first;
        children[index + 1].0 = total - first;
        true
    }

    /// Drag divider `index` of the split at `path` (laid out in `area`) so
    /// it sits at `pos` along the split's axis; stops [`MIN_PANE`] short of
    /// either neighbour's far edge.
    pub fn drag_divider(&mut self, path: &[usize], index: usize, area: [f32; 4], pos: f32) -> bool {
        let Some(PaneNode::Split { axis, children }) = self.node_at(path) else {
            return false;
        };
        if index + 1 >= children.len() {
            return false;
        }
        // the divider's unrounded position, so the pane under the pointer
        // gets exactly the extent it shows
        let n = children.len() as f32;
        let avail = (along(*axis, area) - HANDLE * (n - 1.0)).max(0.0);
        let sum: f32 = children.iter().map(|c| c.0).sum();
        let before: f32 = children[..=index].iter().map(|c| c.0).sum();
        let start = match axis {
            Axis::Horizontal => area[0],
            Axis::Vertical => area[1],
        };
        let edge = start + avail * before / sum.max(f32::EPSILON) + index as f32 * HANDLE;
        self.move_divider(path, index, area, pos - edge, true)
    }

    /// ⌃⌘-arrow: move the divider beside the leaf `target` in the nearest
    /// split along `axis` by `delta` px — the one after it, or before it
    /// for the last child — when both panes stay at least [`MIN_PANE`]
    /// (Warp `adjust_pane_size_by_id`). `area` is the whole tree's.
    pub fn resize_pane(&mut self, area: [f32; 4], target: PaneId, axis: Axis, delta: f32) -> bool {
        let Some((path, child)) = self.ancestor_along(target, axis) else {
            return false;
        };
        let Some(PaneNode::Split { children, .. }) = self.node_at(&path) else {
            return false;
        };
        let index = child.min(children.len() - 2);
        let mut panes = Vec::new();
        let mut handles = Vec::new();
        self.layout(area, &mut panes, &mut handles, Vec::new());
        let Some(split_area) = handles.iter().find(|h| h.path == path).map(|h| h.area) else {
            return false;
        };
        self.move_divider(&path, index, split_area, delta, false)
    }

    /// The nearest split along `axis` holding the leaf `target`: its path
    /// and the index of the child `target` is in.
    fn ancestor_along(&self, target: PaneId, axis: Axis) -> Option<(NodePath, usize)> {
        fn walk(
            node: &PaneNode,
            target: PaneId,
            axis: Axis,
            path: &mut NodePath,
            best: &mut Option<(NodePath, usize)>,
        ) -> bool {
            match node {
                PaneNode::Leaf(id) => *id == target,
                PaneNode::Split { axis: a, children } => {
                    for (i, (_, child)) in children.iter().enumerate() {
                        path.push(i);
                        let found = walk(child, target, axis, path, best);
                        path.pop();
                        if found {
                            if *a == axis && best.is_none() {
                                *best = Some((path.clone(), i));
                            }
                            return true;
                        }
                    }
                    false
                }
            }
        }
        let mut best = None;
        walk(self, target, axis, &mut Vec::new(), &mut best);
        best
    }

    /// The saved (binary) form: a split of n children becomes a chain of
    /// two-child splits nested in the second child, each ratio its first
    /// child's share of the rest. `leaf` writes one pane.
    pub fn to_saved(&self, leaf: &mut dyn FnMut(PaneId) -> SavedLayout) -> SavedLayout {
        fn chain(
            axis: Axis,
            children: &[(f32, PaneNode)],
            leaf: &mut dyn FnMut(PaneId) -> SavedLayout,
        ) -> SavedLayout {
            match children {
                [] => SavedLayout::pane(None),
                [(_, only)] => only.to_saved(leaf),
                [(w, first), rest @ ..] => {
                    let total: f32 = children.iter().map(|c| c.0).sum();
                    let ratio = if total > 0.0 { w / total } else { 0.5 };
                    let first = first.to_saved(leaf);
                    SavedLayout::split(saved_axis(axis), ratio, first, chain(axis, rest, leaf))
                }
            }
        }
        match self {
            PaneNode::Leaf(id) => leaf(*id),
            PaneNode::Split { axis, children } => chain(*axis, children, leaf),
        }
    }

    /// Take the weights from a saved layout this tree was built from (one
    /// `split` per saved split, so same-axis chains joined one split). Saved
    /// chains flatten with each pane's share kept — this also migrates
    /// layouts saved by the binary tree, nested in either child.
    pub fn apply_saved(&mut self, saved: &SavedLayout) {
        fn flatten<'a>(
            saved: &'a SavedLayout,
            axis: Axis,
            w: f32,
            out: &mut Vec<(f32, &'a SavedLayout)>,
        ) {
            match saved {
                SavedLayout::Split {
                    axis: a,
                    ratio,
                    first,
                    second,
                } if saved_axis(axis) == *a => {
                    let r = if ratio.is_finite() {
                        ratio.clamp(0.01, 0.99)
                    } else {
                        0.5
                    };
                    flatten(first, axis, w * r, out);
                    flatten(second, axis, w * (1.0 - r), out);
                }
                _ => out.push((w, saved)),
            }
        }
        let PaneNode::Split { axis, children } = self else {
            return;
        };
        let mut parts = Vec::new();
        flatten(saved, *axis, 1.0, &mut parts);
        if parts.len() != children.len() {
            return;
        }
        let n = parts.len() as f32;
        for ((flex, child), (w, s)) in children.iter_mut().zip(parts) {
            *flex = (w * n).max(1e-3);
            child.apply_saved(s);
        }
    }
}

fn saved_axis(axis: Axis) -> SavedAxis {
    match axis {
        Axis::Horizontal => SavedAxis::Horizontal,
        Axis::Vertical => SavedAxis::Vertical,
    }
}

/// Extent of `area` along `axis`.
fn along(axis: Axis, area: [f32; 4]) -> f32 {
    match axis {
        Axis::Horizontal => area[2],
        Axis::Vertical => area[3],
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

/// Extents of a split's children along `total` (the dividers excluded), in
/// proportion to `flexes`. Whole-pixel edges keep each 1px divider crisp;
/// the extents always add up to the space, never overflowing it.
fn flex_sizes(total: f32, flexes: &[f32]) -> Vec<f32> {
    let n = flexes.len();
    if n == 0 {
        return Vec::new();
    }
    let avail = (total - HANDLE * (n - 1) as f32).max(0.0);
    let floored = min_extent_flexes(avail, flexes);
    let flexes = floored.as_deref().unwrap_or(flexes);
    let sum: f32 = flexes.iter().map(|f| f.max(0.0)).sum();
    let mut out = Vec::with_capacity(n);
    let (mut acc, mut prev) = (0.0_f32, 0.0_f32);
    for (i, f) in flexes.iter().enumerate() {
        acc += f.max(0.0);
        let edge = if i + 1 == n {
            avail
        } else if sum > 0.0 {
            (avail * acc / sum).round()
        } else {
            (avail * (i + 1) as f32 / n as f32).round()
        };
        out.push((edge - prev).max(0.0));
        prev = edge.max(prev);
    }
    out
}

/// Weights giving every pane at least [`MIN_PANE`] of `avail` when it
/// fits (a restored ratio of 0.0 or 1.0 would leave a sliver); `None`
/// when the weights already do, or the space is too small for all.
fn min_extent_flexes(avail: f32, flexes: &[f32]) -> Option<Vec<f32>> {
    let n = flexes.len();
    let sum: f32 = flexes.iter().map(|f| f.max(0.0)).sum();
    if n < 2 || sum <= 0.0 || avail < MIN_PANE * n as f32 {
        return None;
    }
    let mut fixed = vec![false; n];
    loop {
        let free_sum: f32 = (0..n)
            .filter(|&i| !fixed[i])
            .map(|i| flexes[i].max(0.0))
            .sum();
        let pinned = fixed.iter().filter(|f| **f).count() as f32;
        let free_avail = avail - MIN_PANE * pinned;
        let mut changed = false;
        for i in 0..n {
            if !fixed[i] && flexes[i].max(0.0) / free_sum * free_avail < MIN_PANE - 0.5 {
                fixed[i] = true;
                changed = true;
            }
        }
        if !changed {
            if pinned == 0.0 {
                return None;
            }
            return Some(
                (0..n)
                    .map(|i| {
                        if fixed[i] {
                            MIN_PANE
                        } else {
                            flexes[i].max(0.0) / free_sum * free_avail
                        }
                    })
                    .collect(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_a_pane_never_leaves_a_same_axis_split_nested() {
        // 1 | (2 / (3 | 4)) at 1:1, 3:1 inside; close 2: 1 | 3 | 4 in one row
        let mut root = row(&[1, 2]);
        root.split(2, 3, Axis::Vertical);
        root.split(3, 4, Axis::Horizontal);
        if let PaneNode::Split { children, .. } = &mut root
            && let PaneNode::Split { children: col, .. } = &mut children[1].1
            && let PaneNode::Split {
                children: inner, ..
            } = &mut col[1].1
        {
            inner[0].0 = 3.0;
        }
        root.remove(2);
        let PaneNode::Split { axis, children } = &root else {
            panic!("root is a split");
        };
        assert_eq!(*axis, Axis::Horizontal);
        assert_eq!(children.len(), 3);
        assert!(children.iter().all(|c| matches!(c.1, PaneNode::Leaf(_))));
        // 4 and 3 share the column's half 3:1
        assert_eq!(widths(&root, 1002.0), vec![500.0, 375.0, 125.0]);
    }

    #[test]
    fn restored_sliver_weights_still_lay_out_at_the_minimum() {
        // a saved ratio of 0.0: the first pane gets MIN_PANE, not 0
        let mut root = row(&[1, 2]);
        if let PaneNode::Split { children, .. } = &mut root {
            children[0].0 = 0.0;
        }
        assert_eq!(widths(&root, 401.0), vec![50.0, 350.0]);
        // too narrow for both minimums: the weights rule
        assert_eq!(widths(&root, 81.0), vec![0.0, 80.0]);
        // ⌃⌘→ on it grows it from what it shows
        assert!(root.resize_pane([0.0, 0.0, 401.0, 500.0], 1, Axis::Horizontal, RESIZE_STEP));
        assert_eq!(widths(&root, 401.0), vec![60.0, 340.0]);
    }

    #[test]
    fn a_pane_under_the_minimum_never_shrinks_and_never_jumps_back() {
        // 30 | 80 | 31 px in a row too narrow for the layout floor
        let mut root = row(&[1, 2, 3]);
        if let PaneNode::Split { children, .. } = &mut root {
            children[0].0 = 30.0;
            children[1].0 = 80.0;
            children[2].0 = 31.0;
        }
        let narrow = [0.0, 0.0, 143.0, 500.0];
        let first = |root: &PaneNode| match root {
            PaneNode::Split { children, .. } => children[0].0,
            PaneNode::Leaf(_) => unreachable!(),
        };
        // shrinking the first further is refused in both modes; the clamped
        // drag must not move the divider right instead
        assert!(!root.move_divider(&[], 0, narrow, -5.0, false));
        assert!(!root.move_divider(&[], 0, narrow, -5.0, true));
        assert_eq!(first(&root), 30.0);
        // growing it is allowed, by the step asked
        assert!(root.move_divider(&[], 0, narrow, 5.0, false));
        assert!((first(&root) - 35.0).abs() < 0.01);
    }

    fn rects(root: &PaneNode, area: [f32; 4]) -> (Vec<PaneRect>, Vec<HandleRect>) {
        let mut panes = Vec::new();
        let mut handles = Vec::new();
        root.layout(area, &mut panes, &mut handles, vec![]);
        (panes, handles)
    }

    fn widths(root: &PaneNode, w: f32) -> Vec<f32> {
        rects(root, [0.0, 0.0, w, 500.0])
            .0
            .iter()
            .map(|p| p.rect[2])
            .collect()
    }

    fn row(ids: &[PaneId]) -> PaneNode {
        let mut root = PaneNode::Leaf(ids[0]);
        for pair in ids.windows(2) {
            root.split(pair[0], pair[1], Axis::Horizontal);
        }
        root
    }

    #[test]
    fn splits_never_overflow_the_parent() {
        for total in [0.0, 30.0, 100.0, 130.0, 200.0, 800.0] {
            for flexes in [vec![0.05, 0.95], vec![1.0, 1.0], vec![3.0, 1.0, 1.0, 0.2]] {
                let sizes = flex_sizes(total, &flexes);
                assert!(sizes.iter().all(|s| *s >= 0.0));
                let used = sizes.iter().sum::<f32>() + HANDLE * (flexes.len() - 1) as f32;
                assert!(
                    used <= total.max(HANDLE * 3.0) + 0.001,
                    "{total} {flexes:?}"
                );
            }
        }
        assert_eq!(flex_sizes(801.0, &[1.0, 1.0]), vec![400.0, 400.0]);
        assert_eq!(
            flex_sizes(1002.0, &[1.0, 1.0, 1.0]),
            vec![333.0, 334.0, 333.0]
        );
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
        assert!(!root.is_split());
    }

    #[test]
    fn same_axis_split_joins_the_row_with_an_equal_share() {
        // ⌘D twice: one split of three thirds, not a half and two quarters
        let root = row(&[1, 2, 3]);
        let PaneNode::Split { children, .. } = &root else {
            panic!("{root:?}")
        };
        assert_eq!(children.len(), 3);
        assert_eq!(widths(&root, 1002.0), vec![333.0, 334.0, 333.0]);
        // a fourth pane splitting the first goes right of it, still equal
        let mut root = root;
        root.split(1, 4, Axis::Horizontal);
        assert_eq!(root.leaves(), vec![1, 4, 2, 3]);
        assert_eq!(widths(&root, 1003.0), vec![250.0, 250.0, 250.0, 250.0]);
    }

    #[test]
    fn cross_axis_split_halves_the_pane() {
        // 1 | 2, then 2 split down: 2 and 3 each take half of 2's column
        let mut root = row(&[1, 2]);
        root.split(2, 3, Axis::Vertical);
        let (panes, handles) = rects(&root, [0.0, 0.0, 1001.0, 801.0]);
        assert_eq!(panes[0].rect, [0.0, 0.0, 500.0, 801.0]);
        assert_eq!(panes[1].rect, [501.0, 0.0, 500.0, 400.0]);
        assert_eq!(panes[2].rect, [501.0, 401.0, 500.0, 400.0]);
        assert_eq!(handles.len(), 2);
        assert_eq!(
            (handles[1].path.as_slice(), handles[1].index),
            (&[1][..], 0)
        );
    }

    #[test]
    fn new_pane_takes_the_default_weight_next_to_resized_ones() {
        // Warp: the new child gets flex 1 beside [1.4, 0.6]
        let mut root = row(&[1, 2]);
        assert!(root.move_divider(&[], 0, [0.0, 0.0, 1001.0, 500.0], 200.0, false));
        root.split(2, 3, Axis::Horizontal);
        let w = widths(&root, 1002.0);
        assert_eq!(w, vec![467.0, 200.0, 333.0]);
    }

    #[test]
    fn closing_a_pane_shares_its_space_among_all_siblings() {
        // thirds, close the middle: halves
        let mut root = row(&[1, 2, 3]);
        root.remove(2);
        assert_eq!(widths(&root, 1001.0), vec![500.0, 500.0]);
        // weights 2:1:1, close the first: the other two keep 1:1
        let mut root = row(&[1, 2, 3]);
        if let PaneNode::Split { children, .. } = &mut root {
            children[0].0 = 2.0;
        }
        root.remove(1);
        assert_eq!(widths(&root, 1001.0), vec![500.0, 500.0]);
        // weights 1:2:1, close the first: 2:1 remains
        let mut root = row(&[1, 2, 3]);
        if let PaneNode::Split { children, .. } = &mut root {
            children[1].0 = 2.0;
        }
        root.remove(1);
        assert_eq!(widths(&root, 1001.0), vec![667.0, 333.0]);
    }

    #[test]
    fn closing_in_a_nested_split_collapses_into_the_parent_slot() {
        // 1 | (2 / 3) at 1:3, close 3: 2 keeps the column's share
        let mut root = row(&[1, 2]);
        root.split(2, 3, Axis::Vertical);
        if let PaneNode::Split { children, .. } = &mut root {
            children[1].0 = 3.0;
        }
        root.remove(3);
        assert_eq!(root.leaves(), vec![1, 2]);
        assert_eq!(widths(&root, 1001.0), vec![250.0, 750.0]);
    }

    #[test]
    fn keyboard_resize_moves_the_divider_beside_the_pane_in_steps() {
        let area = [0.0, 0.0, 1001.0, 500.0];
        let mut root = row(&[1, 2]);
        // on the first pane: the divider after it moves right
        assert!(root.resize_pane(area, 1, Axis::Horizontal, RESIZE_STEP));
        assert_eq!(widths(&root, 1001.0), vec![510.0, 490.0]);
        // on the last pane: the divider before it moves (right = it shrinks)
        assert!(root.resize_pane(area, 2, Axis::Horizontal, RESIZE_STEP));
        assert_eq!(widths(&root, 1001.0), vec![520.0, 480.0]);
        // no split along that axis: nothing
        assert!(!root.resize_pane(area, 2, Axis::Vertical, RESIZE_STEP));
        // nested: 1 | (2 / 3), ⌃⌘→ on 3 moves the root divider
        root.split(2, 3, Axis::Vertical);
        assert!(root.resize_pane(area, 3, Axis::Horizontal, -RESIZE_STEP));
        assert_eq!(widths(&root, 1001.0), vec![510.0, 490.0, 490.0]);
        // ...and ⌃⌘↓ on 3 moves the column's own divider
        assert!(root.resize_pane(area, 3, Axis::Vertical, RESIZE_STEP));
        let (panes, _) = rects(&root, area);
        // 249.5 + 10, on whole pixels
        assert_eq!(panes[1].rect[3], 260.0);
    }

    #[test]
    fn three_in_a_row_resize_trades_only_with_the_neighbour() {
        let area = [0.0, 0.0, 1002.0, 500.0];
        let mut root = row(&[1, 2, 3]);
        assert!(root.resize_pane(area, 2, Axis::Horizontal, 30.0));
        assert_eq!(widths(&root, 1002.0), vec![333.0, 364.0, 303.0]);
    }

    #[test]
    fn resize_stops_at_the_minimum_pane_size() {
        let area = [0.0, 0.0, 201.0, 500.0];
        let mut root = row(&[1, 2]);
        let mut moved = 0;
        while root.resize_pane(area, 1, Axis::Horizontal, -RESIZE_STEP) {
            moved += 1;
            assert!(moved < 100);
        }
        // 100 → 50 in five steps; the sixth would leave 40 and is refused
        assert_eq!(moved, 5);
        assert_eq!(widths(&root, 201.0), vec![50.0, 150.0]);
        // a drag past the limit stops at it
        assert!(root.drag_divider(&[], 0, area, 190.0));
        assert_eq!(widths(&root, 201.0), vec![150.0, 50.0]);
        assert!(!root.drag_divider(&[], 0, area, 500.0));
        // a drag inside the limits lands under the pointer
        assert!(root.drag_divider(&[], 0, area, 120.0));
        assert_eq!(widths(&root, 201.0), vec![120.0, 80.0]);
    }

    #[test]
    fn window_resize_keeps_each_share() {
        let mut root = row(&[1, 2, 3]);
        assert!(root.drag_divider(&[], 0, [0.0, 0.0, 1002.0, 500.0], 500.0));
        assert_eq!(widths(&root, 1002.0), vec![500.0, 167.0, 333.0]);
        assert_eq!(widths(&root, 502.0), vec![250.0, 83.0, 167.0]);
        assert_eq!(widths(&root, 2002.0), vec![1000.0, 333.0, 667.0]);
    }

    #[test]
    fn equalize_resets_one_split_or_the_whole_tree() {
        let area = [0.0, 0.0, 1002.0, 801.0];
        let mut root = row(&[1, 2, 3]);
        root.split(3, 4, Axis::Vertical);
        assert!(root.drag_divider(&[], 0, area, 700.0));
        assert!(root.drag_divider(&[2], 0, [668.0, 0.0, 334.0, 801.0], 600.0));
        assert!(root.equalize(&[]));
        assert_eq!(widths(&root, 1002.0)[..3], [333.0, 334.0, 333.0]);
        let (panes, _) = rects(&root, area);
        assert_ne!(panes[2].rect[3], 400.0, "the nested split kept its sizes");
        assert!(root.equalize_all());
        let (panes, _) = rects(&root, area);
        assert_eq!(panes[2].rect[3], 400.0);
        assert!(!root.equalize_all());
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
        assert_eq!(root.ancestor_along(4, Axis::Vertical), Some((vec![0], 0)));
    }

    #[test]
    fn insert_root_takes_a_share_of_the_whole_tree() {
        let mut root = row(&[1, 2]);
        root.insert_root(VIEWER_PANE, Edge::Left, 1.0 / 3.0);
        assert_eq!(root.leaves(), vec![VIEWER_PANE, 1, 2]);
        assert_eq!(widths(&root, 1002.0), vec![333.0, 334.0, 333.0]);
        let mut root = PaneNode::Leaf(1);
        root.split(1, 2, Axis::Vertical);
        root.insert_root(VIEWER_PANE, Edge::Left, 0.25);
        assert_eq!(widths(&root, 1001.0), vec![250.0, 750.0, 750.0]);
        assert_eq!(root.without_viewer().leaves(), vec![1, 2]);
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

    /// What the app's `build_saved` does: one `split` per saved split.
    fn build(saved: &SavedLayout, root: &mut PaneNode, pane: PaneId, next: &mut PaneId) {
        if let SavedLayout::Split {
            axis,
            first,
            second,
            ..
        } = saved
        {
            let axis = match axis {
                SavedAxis::Horizontal => Axis::Horizontal,
                SavedAxis::Vertical => Axis::Vertical,
            };
            *next += 1;
            let new = *next;
            assert!(root.split(pane, new, axis));
            build(first, root, pane, next);
            build(second, root, new, next);
        }
    }

    fn restore(saved: &SavedLayout) -> PaneNode {
        let mut root = PaneNode::Leaf(1);
        build(saved, &mut root, 1, &mut 1);
        root.apply_saved(saved);
        root
    }

    #[test]
    fn saved_layouts_round_trip_the_weights() {
        let area = [0.0, 0.0, 1002.0, 801.0];
        let mut root = row(&[1, 2, 3]);
        root.split(3, 4, Axis::Vertical);
        assert!(root.drag_divider(&[], 0, area, 500.0));
        assert!(root.drag_divider(&[2], 0, [668.0, 0.0, 334.0, 801.0], 200.0));
        let saved = root.to_saved(&mut |_| SavedLayout::pane(None));
        let back = restore(&saved);
        let (a, _) = rects(&root, area);
        let (b, _) = rects(&back, area);
        assert_eq!(
            a.iter().map(|p| p.rect).collect::<Vec<_>>(),
            b.iter().map(|p| p.rect).collect::<Vec<_>>()
        );
    }

    #[test]
    fn binary_layouts_saved_by_older_versions_flatten_with_their_shares() {
        let p = || SavedLayout::pane(None);
        // 1 | (2 | 3) at 0.5 / 0.5: a half and two quarters stay that way
        let old = SavedLayout::split(
            SavedAxis::Horizontal,
            0.5,
            p(),
            SavedLayout::split(SavedAxis::Horizontal, 0.5, p(), p()),
        );
        let root = restore(&old);
        assert_eq!(root.leaves().len(), 3);
        assert!(matches!(&root, PaneNode::Split { children, .. } if children.len() == 3));
        assert_eq!(widths(&root, 1002.0), vec![500.0, 250.0, 250.0]);
        // nested in the first child, with a cross split below it
        let old = SavedLayout::split(
            SavedAxis::Horizontal,
            0.6,
            SavedLayout::split(SavedAxis::Horizontal, 0.5, p(), p()),
            SavedLayout::split(SavedAxis::Vertical, 0.25, p(), p()),
        );
        let root = restore(&old);
        assert_eq!(widths(&root, 1002.0), vec![300.0, 300.0, 400.0, 400.0]);
        let (panes, _) = rects(&root, [0.0, 0.0, 1002.0, 801.0]);
        assert_eq!(panes[2].rect[3], 200.0);
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
