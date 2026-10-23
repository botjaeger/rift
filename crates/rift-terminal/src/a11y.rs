//! Accessibility tree (AccessKit) projected from app state.
//!
//! Rift renders every surface itself, so screen readers see nothing unless
//! the app describes it. Each frame the app fills a [`Snapshot`] from its
//! state and per-frame hit rects; [`build`] turns it into an AccessKit tree:
//!
//! ```text
//! Window
//! ├─ TabList ─ Tab…              (click selects; a rail search leaves
//! │                               out the tabs it does not match)
//! ├─ Group "pane" per visible pane
//! │  ├─ List "Command blocks" ─ ListItem per block (label = command,
//! │  │                          description = status, value = output)
//! │  ├─ Terminal                 (visible screen text)
//! │  └─ TextInput "Command"      (owned editor, when it is live; "Compose
//! │                               for …" while the rich-input composer is)
//! ├─ Group "Tab rail"             (vertical tabs: SearchInput "Search
//! │                               tabs", the panes-view toggle, pane rows)
//! ├─ Dialog / AlertDialog / Menu (palette, search, settings, confirm, menu)
//! └─ Label (live region)         (command-finished announcements)
//! ```
//!
//! Click is answered by clicking the node's on-screen rect through the
//! app's normal mouse routing, so it behaves exactly like a pointer click.
//! Focus is not advertised: moving focus must never toggle a setting or
//! accept a confirmation.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use accesskit::{Action, Live, Node, NodeId, Rect, Role, Toggled, TreeId, TreeInfo, TreeUpdate};

/// Characters of block output exposed per block (screen readers browse it
/// as the item's value; the full text stays copyable from the block menu).
pub const OUTPUT_LIMIT: usize = 4000;
/// Blocks exposed per pane (newest last).
pub const BLOCK_LIMIT: usize = 50;

pub struct Snapshot {
    /// Logical window size.
    pub size: (f32, f32),
    pub scale: f64,
    pub tabs: Vec<TabNode>,
    /// Tab groups; their tabs name them by index (`TabNode::group`).
    pub groups: Vec<GroupNode>,
    pub panes: Vec<PaneNode>,
    pub overlay: Option<Overlay>,
    pub announcement: Option<String>,
    /// Side panels and the file viewer (non-modal regions).
    pub panels: Vec<PanelNode>,
    /// The vertical tab rail's own controls, while it shows.
    pub rail: Option<RailNode>,
}

/// The vertical tab rail: its search field and the controls under it
/// (the panes-view toggle, then one row per pane of the panes view). Its
/// tab rows are the TabList's tabs.
pub struct RailNode {
    pub query: String,
    /// The search field has the keyboard.
    pub focused: bool,
    pub search_rect: Option<[f32; 4]>,
    pub controls: Vec<Control>,
}

/// A non-modal region: file explorer, code review, file viewer.
pub struct PanelNode {
    pub label: String,
    pub rect: Option<[f32; 4]>,
    /// Read-only text (the viewer's lines), if any.
    pub text: Option<String>,
    pub controls: Vec<Control>,
    /// Keyboard focus is in the panel: its selected control gets focus.
    pub focused: bool,
    /// `text` is being edited (config.toml): a multi-line text input.
    pub editable: bool,
    /// A polite live region under it (the config lint summary).
    pub status: Option<String>,
}

pub struct TabNode {
    pub title: String,
    pub selected: bool,
    pub rect: Option<[f32; 4]>,
    /// Index into `Snapshot::groups`.
    pub group: Option<usize>,
    pub pinned: bool,
    /// One of its panes refuses input.
    pub read_only: bool,
    /// The rail's search does not match it: it is left out of the list.
    pub filtered: bool,
}

/// A tab's label: its title (else "Tab N") and its states.
fn tab_label(i: usize, tab: &TabNode) -> String {
    let mut label = if tab.title.is_empty() {
        format!("Tab {}", i + 1)
    } else {
        // read aloud as shown: secrets masked (`mask_secrets`)
        rift_core::secrets::mask_owned(tab.title.clone())
    };
    if tab.pinned {
        label.push_str(", pinned");
    }
    if tab.read_only {
        label.push_str(", read-only");
    }
    label
}

/// A tab group: its header (a click folds or opens it) holds its tabs.
pub struct GroupNode {
    pub label: String,
    pub expanded: bool,
    pub rect: Option<[f32; 4]>,
}

pub struct PaneNode {
    pub id: u64,
    pub label: String,
    pub active: bool,
    pub rect: Option<[f32; 4]>,
    pub blocks: Vec<BlockNode>,
    /// Visible terminal rows, top to bottom.
    pub screen: String,
    pub editor: Option<EditorNode>,
    /// Title bar buttons (split, menu, close) and welcome rows.
    pub controls: Vec<Control>,
}

pub struct BlockNode {
    pub id: u64,
    pub command: String,
    pub status: String,
    pub output: String,
    pub selected: bool,
    pub rect: Option<[f32; 4]>,
}

pub struct EditorNode {
    /// "Command", or the rich-input composer's "Compose for …".
    pub label: String,
    pub value: String,
    pub rect: Option<[f32; 4]>,
}

pub struct Control {
    pub label: String,
    pub role: Role,
    pub toggled: Option<bool>,
    pub selected: bool,
    pub rect: Option<[f32; 4]>,
    /// Shown but not usable (the review button on the Settings tab).
    pub disabled: bool,
}

pub enum Overlay {
    /// Modal with a text field and a list (palette) or controls (settings).
    Dialog {
        label: String,
        input: Option<(String, String)>,
        status: Option<String>,
        controls: Vec<Control>,
        rect: Option<[f32; 4]>,
    },
    /// Find / block filter bar: a field, its status, and toggle buttons
    /// beside the field (find's match case and regex).
    Find {
        label: String,
        input: (String, String),
        status: String,
        toggles: Vec<Control>,
    },
    /// Blocking confirmation (close with running jobs, multiline paste).
    Alert {
        title: String,
        description: String,
        buttons: Vec<Control>,
    },
    /// A context menu (a block's, a tab's, the ghost text's accept key).
    Menu { label: String, items: Vec<Control> },
}

/// A tree plus the on-screen rect behind each actionable node.
pub struct Built {
    pub update: TreeUpdate,
    pub targets: HashMap<NodeId, [f32; 4]>,
    /// Debug text of the nodes, to skip pushing unchanged trees.
    pub fingerprint: String,
}

const ROOT: NodeId = NodeId(1);
const TAB_LIST: NodeId = NodeId(2);
const LIVE: NodeId = NodeId(3);
const OVERLAY: NodeId = NodeId(4);

fn id_of(parts: impl Hash) -> NodeId {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    // keep clear of the fixed ids above
    NodeId(h.finish() | (1 << 63))
}

fn bounds(r: [f32; 4]) -> Rect {
    Rect {
        x0: r[0] as f64,
        y0: r[1] as f64,
        x1: (r[0] + r[2]) as f64,
        y1: (r[1] + r[3]) as f64,
    }
}

/// Whether an assistive-technology action request performs the node's
/// action. Only Click does; focus requests must not toggle or confirm.
pub fn activates(action: Action) -> bool {
    action == Action::Click
}

struct Tree {
    nodes: Vec<(NodeId, Node)>,
    targets: HashMap<NodeId, [f32; 4]>,
}

impl Tree {
    fn add(&mut self, id: NodeId, mut node: Node, rect: Option<[f32; 4]>, clickable: bool) {
        if let Some(r) = rect {
            node.set_bounds(bounds(r));
            if clickable {
                node.add_action(Action::Click);
                self.targets.insert(id, r);
            }
        }
        self.nodes.push((id, node));
    }

    fn control(&mut self, parent_key: &str, i: usize, c: &Control) -> NodeId {
        let id = id_of((parent_key, i, &c.label));
        let mut node = Node::new(c.role);
        node.set_label(c.label.as_str());
        if let Some(t) = c.toggled {
            node.set_toggled(Toggled::from(t));
        }
        if c.selected {
            node.set_selected(true);
        }
        if c.disabled {
            node.set_disabled();
        }
        self.add(id, node, c.rect, !c.disabled);
        id
    }
}

/// Build the whole tree. `focus` picks the focused node: the open overlay's
/// input, else the active pane's editor, else its terminal.
pub fn build(s: &Snapshot) -> Built {
    let mut t = Tree {
        nodes: Vec::new(),
        targets: HashMap::new(),
    };
    let mut root_children = vec![TAB_LIST];
    let mut focus = ROOT;

    // tabs; a group's sit in its node, where its first tab is
    let mut tab_ids = Vec::new();
    let mut group_kids: Vec<Vec<NodeId>> = vec![Vec::new(); s.groups.len()];
    for (i, tab) in s.tabs.iter().enumerate().filter(|(_, t)| !t.filtered) {
        let id = id_of(("tab", i));
        let mut node = Node::new(Role::Tab);
        node.set_label(tab_label(i, tab));
        node.set_selected(tab.selected);
        t.add(id, node, tab.rect, true);
        match tab.group.filter(|g| *g < s.groups.len()) {
            Some(g) => {
                if group_kids[g].is_empty() {
                    tab_ids.push(id_of(("tab-group", g)));
                }
                group_kids[g].push(id);
            }
            None => tab_ids.push(id),
        }
    }
    for (g, kids) in group_kids.into_iter().enumerate() {
        if kids.is_empty() {
            continue;
        }
        let group = &s.groups[g];
        let mut node = Node::new(Role::Group);
        node.set_label(group.label.as_str());
        node.set_expanded(group.expanded);
        node.set_children(kids);
        t.add(id_of(("tab-group", g)), node, group.rect, true);
    }
    let mut list = Node::new(Role::TabList);
    list.set_label("Tabs");
    list.set_children(tab_ids);
    t.nodes.push((TAB_LIST, list));

    // panes
    for pane in &s.panes {
        let pane_id = id_of(("pane", pane.id));
        let mut children = Vec::new();

        let mut block_ids = Vec::new();
        let first = pane.blocks.len().saturating_sub(BLOCK_LIMIT);
        for b in &pane.blocks[first..] {
            let id = id_of(("block", pane.id, b.id));
            let mut node = Node::new(Role::ListItem);
            node.set_label(if b.command.trim().is_empty() {
                "(empty command)".to_string()
            } else {
                b.command.clone()
            });
            node.set_description(b.status.as_str());
            let mut output = b.output.clone();
            if output.chars().count() > OUTPUT_LIMIT {
                let skip = output.chars().count() - OUTPUT_LIMIT;
                output = format!("…{}", output.chars().skip(skip).collect::<String>());
            }
            node.set_value(output);
            node.set_read_only();
            node.set_selected(b.selected);
            t.add(id, node, b.rect, true);
            block_ids.push(id);
        }
        let blocks_id = id_of(("blocks", pane.id));
        let mut blocks = Node::new(Role::List);
        blocks.set_label("Command blocks");
        blocks.set_children(block_ids);
        t.nodes.push((blocks_id, blocks));
        children.push(blocks_id);

        let term_id = id_of(("terminal", pane.id));
        let mut term = Node::new(Role::Terminal);
        term.set_label("Terminal output");
        term.set_value(pane.screen.as_str());
        term.set_read_only();
        t.add(term_id, term, pane.rect, pane.rect.is_some());
        children.push(term_id);
        if pane.active {
            focus = term_id;
        }

        if let Some(ed) = &pane.editor {
            let ed_id = id_of(("editor", pane.id));
            let mut node = Node::new(Role::TextInput);
            node.set_label(ed.label.as_str());
            node.set_value(ed.value.as_str());
            t.add(ed_id, node, ed.rect, ed.rect.is_some());
            children.push(ed_id);
            if pane.active {
                focus = ed_id;
            }
        }

        for (i, c) in pane.controls.iter().enumerate() {
            children.push(t.control(&format!("pane-{}", pane.id), i, c));
        }

        let mut group = Node::new(Role::Group);
        group.set_label(pane.label.as_str());
        group.set_children(children);
        t.add(pane_id, group, pane.rect, false);
        root_children.push(pane_id);
    }

    // the tab rail's search and rows (its field, focused, has the keys
    // over the pane's editor)
    if let Some(rail) = &s.rail {
        let search = id_of("rail-search");
        let mut node = Node::new(Role::SearchInput);
        node.set_label("Search tabs");
        node.set_value(rail.query.as_str());
        t.add(search, node, rail.search_rect, true);
        if rail.focused {
            focus = search;
        }
        let mut children = vec![search];
        for (i, c) in rail.controls.iter().enumerate() {
            children.push(t.control("rail", i, c));
        }
        let id = id_of("rail");
        let mut group = Node::new(Role::Group);
        group.set_label("Tab rail");
        group.set_children(children);
        t.nodes.push((id, group));
        root_children.push(id);
    }

    // side panels and the viewer
    for (pi, panel) in s.panels.iter().enumerate() {
        let mut children = Vec::new();
        if let Some(text) = &panel.text {
            let id = id_of(("panel-text", pi));
            let mut node = Node::new(if panel.editable {
                Role::MultilineTextInput
            } else {
                Role::Document
            });
            node.set_label(panel.label.as_str());
            node.set_value(text.as_str());
            if !panel.editable {
                node.set_read_only();
            }
            t.nodes.push((id, node));
            children.push(id);
            if panel.focused {
                focus = id;
            }
        }
        for (i, c) in panel.controls.iter().enumerate() {
            let id = t.control(&panel.label, i, c);
            if panel.focused && c.selected {
                focus = id;
            }
            children.push(id);
        }
        if let Some(status) = &panel.status {
            let id = id_of(("panel-status", pi));
            let mut node = Node::new(Role::Status);
            node.set_live(Live::Polite);
            node.set_value(status.as_str());
            t.nodes.push((id, node));
            children.push(id);
        }
        let id = id_of(("panel", pi));
        let mut group = Node::new(Role::Group);
        group.set_label(panel.label.as_str());
        group.set_children(children);
        t.add(id, group, panel.rect, false);
        root_children.push(id);
    }

    // overlay
    if let Some(ov) = &s.overlay {
        match ov {
            Overlay::Dialog {
                label,
                input,
                status,
                controls,
                rect,
            } => {
                let mut children = Vec::new();
                if let Some((name, value)) = input {
                    let id = id_of(("overlay-input", label));
                    let mut node = Node::new(Role::SearchInput);
                    node.set_label(name.as_str());
                    node.set_value(value.as_str());
                    t.nodes.push((id, node));
                    children.push(id);
                    focus = id;
                }
                if let Some(st) = status {
                    let id = id_of(("overlay-status", label));
                    let mut node = Node::new(Role::Label);
                    node.set_value(st.as_str());
                    t.nodes.push((id, node));
                    children.push(id);
                }
                if !controls.is_empty() {
                    let list_id = id_of(("overlay-list", label));
                    let mut ids = Vec::new();
                    for (i, c) in controls.iter().enumerate() {
                        let id = t.control(label, i, c);
                        if c.selected && input.is_none() {
                            focus = id;
                        }
                        ids.push(id);
                    }
                    let mut list = Node::new(Role::ListBox);
                    list.set_label(label.as_str());
                    list.set_children(ids);
                    t.nodes.push((list_id, list));
                    children.push(list_id);
                }
                let mut node = Node::new(Role::Dialog);
                node.set_label(label.as_str());
                node.set_modal();
                node.set_children(children);
                t.add(OVERLAY, node, *rect, false);
            }
            Overlay::Find {
                label,
                input: (name, value),
                status,
                toggles,
            } => {
                let id = id_of(("overlay-input", label));
                let mut node = Node::new(Role::SearchInput);
                node.set_label(name.as_str());
                node.set_value(value.as_str());
                t.nodes.push((id, node));
                let mut children = vec![id];
                focus = id;
                let id = id_of(("overlay-status", label));
                let mut node = Node::new(Role::Label);
                node.set_value(status.as_str());
                t.nodes.push((id, node));
                children.push(id);
                for (i, c) in toggles.iter().enumerate() {
                    children.push(t.control(label, i, c));
                }
                let mut node = Node::new(Role::Dialog);
                node.set_label(label.as_str());
                node.set_modal();
                node.set_children(children);
                t.add(OVERLAY, node, None, false);
            }
            Overlay::Alert {
                title,
                description,
                buttons,
            } => {
                let mut ids = Vec::new();
                for (i, c) in buttons.iter().enumerate() {
                    ids.push(t.control("alert", i, c));
                }
                // the button the keyboard stands on, else the last (confirm)
                if let Some(at) = buttons.iter().position(|c| c.selected) {
                    focus = ids[at];
                } else if let Some(last) = ids.last() {
                    focus = *last;
                }
                let mut node = Node::new(Role::AlertDialog);
                node.set_label(title.as_str());
                node.set_description(description.as_str());
                node.set_modal();
                node.set_children(ids);
                t.nodes.push((OVERLAY, node));
            }
            Overlay::Menu { label, items } => {
                let mut ids = Vec::new();
                for (i, c) in items.iter().enumerate() {
                    let id = t.control("menu", i, c);
                    if c.selected {
                        focus = id;
                    }
                    ids.push(id);
                }
                if focus == ROOT || !items.iter().any(|c| c.selected) {
                    focus = ids.first().copied().unwrap_or(focus);
                }
                let mut node = Node::new(Role::Menu);
                node.set_label(label.as_str());
                node.set_children(ids);
                t.nodes.push((OVERLAY, node));
            }
        }
        root_children.push(OVERLAY);
    }

    // live region for completions
    let mut live = Node::new(Role::Label);
    live.set_live(Live::Polite);
    live.set_value(s.announcement.clone().unwrap_or_default());
    t.nodes.push((LIVE, live));
    root_children.push(LIVE);

    let mut root = Node::new(Role::Window);
    root.set_label("Rift");
    root.set_bounds(Rect {
        x0: 0.0,
        y0: 0.0,
        x1: s.size.0 as f64,
        y1: s.size.1 as f64,
    });
    root.set_transform(accesskit::Affine::scale(s.scale));
    root.set_children(root_children);
    t.nodes.push((ROOT, root));

    let fingerprint = format!("{:?}{focus:?}", t.nodes);
    let mut tree = TreeInfo::new(ROOT);
    tree.toolkit_name = Some("Rift".into());
    tree.toolkit_version = Some(env!("CARGO_PKG_VERSION").into());
    Built {
        update: TreeUpdate {
            nodes: t.nodes,
            tree: Some(tree),
            tree_id: TreeId::ROOT,
            focus,
        },
        targets: t.targets,
        fingerprint,
    }
}

/// Human-readable text of a tree, for `dump` and tests.
pub fn describe(update: &TreeUpdate) -> String {
    let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(i, n)| (*i, n)).collect();
    let mut out = String::new();
    fn walk(
        out: &mut String,
        nodes: &HashMap<NodeId, &Node>,
        id: NodeId,
        depth: usize,
        focus: NodeId,
    ) {
        let Some(n) = nodes.get(&id) else {
            return;
        };
        out.push_str(&"  ".repeat(depth));
        out.push_str(&format!("{:?}", n.role()));
        if let Some(l) = n.label() {
            out.push_str(&format!(" {l:?}"));
        }
        if let Some(d) = n.description() {
            out.push_str(&format!(" desc={d:?}"));
        }
        if let Some(v) = n.value() {
            let v: String = v.chars().take(80).collect();
            out.push_str(&format!(" value={v:?}"));
        }
        if let Some(t) = n.toggled() {
            out.push_str(&format!(" toggled={t:?}"));
        }
        if let Some(e) = n.is_expanded() {
            out.push_str(&format!(" expanded={e}"));
        }
        if n.is_selected() == Some(true) {
            out.push_str(" selected");
        }
        if n.is_disabled() {
            out.push_str(" disabled");
        }
        if id == focus {
            out.push_str(" [focus]");
        }
        out.push('\n');
        for c in n.children() {
            walk(out, nodes, *c, depth + 1, focus);
        }
    }
    walk(&mut out, &nodes, ROOT, 0, update.focus);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        Snapshot {
            size: (800.0, 600.0),
            scale: 2.0,
            tabs: vec![
                TabNode {
                    title: "zsh".into(),
                    selected: true,
                    rect: Some([10.0, 0.0, 100.0, 30.0]),
                    group: None,
                    pinned: false,
                    read_only: false,
                    filtered: false,
                },
                TabNode {
                    title: String::new(),
                    selected: false,
                    rect: None,
                    group: None,
                    pinned: false,
                    read_only: false,
                    filtered: false,
                },
            ],
            groups: Vec::new(),
            panes: vec![PaneNode {
                id: 7,
                label: "Pane ~/src".into(),
                active: true,
                rect: Some([0.0, 40.0, 800.0, 500.0]),
                blocks: vec![BlockNode {
                    id: 1,
                    command: "ls".into(),
                    status: "exit 0".into(),
                    output: "a\nb\n".into(),
                    selected: false,
                    rect: Some([0.0, 50.0, 800.0, 20.0]),
                }],
                screen: "a\nb".into(),
                editor: Some(EditorNode {
                    label: "Command".into(),
                    value: "git st".into(),
                    rect: Some([0.0, 540.0, 800.0, 40.0]),
                }),
                controls: vec![Control {
                    label: "Close pane".into(),
                    role: Role::Button,
                    toggled: None,
                    selected: false,
                    rect: Some([780.0, 40.0, 20.0, 20.0]),
                    disabled: false,
                }],
            }],
            overlay: None,
            announcement: Some("ls finished".into()),
            panels: Vec::new(),
            rail: None,
        }
    }

    #[test]
    fn the_rail_search_takes_focus_and_filters_the_tabs() {
        let mut s = snapshot();
        s.tabs[0].pinned = true;
        s.tabs[0].read_only = true;
        let text = describe(&build(&s).update);
        assert!(
            text.contains("Tab \"zsh, pinned, read-only\" selected"),
            "{text}"
        );
        s.tabs[1].filtered = true;
        s.rail = Some(RailNode {
            query: "zs".into(),
            focused: true,
            search_rect: Some([0.0, 40.0, 200.0, 28.0]),
            controls: vec![Control {
                label: "build, pane, active, +3 -1 lines".into(),
                role: Role::ListItem,
                toggled: None,
                selected: true,
                rect: Some([0.0, 80.0, 200.0, 28.0]),
                disabled: false,
            }],
        });
        let built = build(&s);
        let text = describe(&built.update);
        assert!(
            text.contains("TabList \"Tabs\"\n    Tab \"zsh, pinned, read-only\" selected\n  Group"),
            "{text}"
        );
        assert!(!text.contains("Tab \"Tab 2\""), "{text}");
        assert!(
            text.contains(
                "Group \"Tab rail\"\n    SearchInput \"Search tabs\" value=\"zs\" [focus]"
            ),
            "{text}"
        );
        assert!(
            text.contains("ListItem \"build, pane, active, +3 -1 lines\" selected"),
            "{text}"
        );
        // the command line no longer claims the focus
        assert!(!text.contains("value=\"git st\" [focus]"), "{text}");
        assert_eq!(text.lines().count(), built.update.nodes.len());
    }

    #[test]
    fn tree_shape_and_focus() {
        let built = build(&snapshot());
        let text = describe(&built.update);
        assert!(text.starts_with("Window \"Rift\""), "{text}");
        assert!(text.contains("  TabList \"Tabs\"\n    Tab \"zsh\" selected\n    Tab \"Tab 2\""));
        assert!(text.contains("ListItem \"ls\" desc=\"exit 0\" value=\"a\\nb\\n\""));
        assert!(text.contains("TextInput \"Command\" value=\"git st\" [focus]"));
        assert!(text.contains("Label value=\"ls finished\""));
        // every node is reachable from the root exactly once
        assert_eq!(text.lines().count(), built.update.nodes.len());
        // the pane's title bar buttons sit in its group
        assert!(text.contains("Button \"Close pane\""), "{text}");
        // clickable nodes carry their rect for action routing
        assert_eq!(built.targets.len(), 5);
    }

    #[test]
    fn overlay_takes_focus() {
        let mut s = snapshot();
        s.overlay = Some(Overlay::Dialog {
            label: "Command palette".into(),
            input: Some(("Search commands".into(), "spl".into())),
            status: None,
            controls: vec![Control {
                label: "Split pane right".into(),
                role: Role::ListBoxOption,
                toggled: None,
                selected: true,
                rect: Some([100.0, 100.0, 300.0, 30.0]),
                disabled: false,
            }],
            rect: None,
        });
        let text = describe(&build(&s).update);
        assert!(text.contains("SearchInput \"Search commands\" value=\"spl\" [focus]"));
        assert!(text.contains("ListBoxOption \"Split pane right\" selected"));
    }

    #[test]
    fn long_output_is_bounded() {
        let mut s = snapshot();
        s.panes[0].blocks[0].output = "x".repeat(OUTPUT_LIMIT * 3);
        let built = build(&s);
        let block = built
            .update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::ListItem)
            .unwrap();
        assert_eq!(block.1.value().unwrap().chars().count(), OUTPUT_LIMIT + 1);
    }

    #[test]
    fn alert_focuses_confirm_button() {
        let mut s = snapshot();
        s.overlay = Some(Overlay::Alert {
            title: "Paste and run?".into(),
            description: "2 lines".into(),
            buttons: vec![
                Control {
                    label: "Cancel".into(),
                    role: Role::Button,
                    toggled: None,
                    selected: false,
                    rect: Some([0.0; 4]),
                    disabled: false,
                },
                Control {
                    label: "Paste".into(),
                    role: Role::Button,
                    toggled: None,
                    selected: false,
                    rect: Some([0.0; 4]),
                    disabled: false,
                },
            ],
        });
        let text = describe(&build(&s).update);
        assert!(text.contains("AlertDialog \"Paste and run?\" desc=\"2 lines\""));
        assert!(text.contains("Button \"Paste\" [focus]"));
    }

    #[test]
    fn a_disabled_control_says_so_and_takes_no_click() {
        let mut s = snapshot();
        s.overlay = Some(Overlay::Alert {
            title: "t".into(),
            description: String::new(),
            buttons: vec![Control {
                label: "No repository on the Settings tab".into(),
                role: Role::Button,
                toggled: None,
                selected: false,
                rect: Some([1.0, 1.0, 10.0, 10.0]),
                disabled: true,
            }],
        });
        let built = build(&s);
        let text = describe(&built.update);
        assert!(
            text.contains("Button \"No repository on the Settings tab\" disabled"),
            "{text}"
        );
        assert!(
            !built.targets.values().any(|r| *r == [1.0, 1.0, 10.0, 10.0]),
            "a disabled control takes no click"
        );
    }

    #[test]
    fn only_click_activates() {
        let built = build(&snapshot());
        for (id, node) in &built.update.nodes {
            assert!(
                !node.supports_action(Action::Focus),
                "{id:?} advertises Focus"
            );
            if built.targets.contains_key(id) {
                assert!(node.supports_action(Action::Click));
            }
        }
        assert!(activates(Action::Click));
        assert!(!activates(Action::Focus));
    }
}
