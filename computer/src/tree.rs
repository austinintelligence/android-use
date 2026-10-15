//! Typed reader for the helper `tree` reply: every accessibility window and its nodes.
//! For tools and the 2.0 core, not an agent command; the helper reports it as status capability bit 3.
use crate::api::{Code, Error, Result};
use serde::Deserialize;

/// Status capability bit for the `tree` op.
pub const CAP_TREE: u32 = 8;
pub const MAX_NODES: usize = 2048;
/// Matches the helper. Each node level is two JSON levels and serde_json stops at 128.
const MAX_DEPTH: usize = 48;

pub const CLICKABLE: u16 = 1;
pub const CHECKABLE: u16 = 2;
pub const CHECKED: u16 = 4;
pub const EDITABLE: u16 = 8;
pub const FOCUSED: u16 = 16;
pub const SCROLLABLE: u16 = 32;
pub const ENABLED: u16 = 64;
pub const SELECTED: u16 = 128;
/// The helper omitted the text of this node.
pub const PASSWORD: u16 = 256;

#[derive(Debug, Deserialize)]
pub struct Tree {
    /// True when a cap or the time budget cut the reply short. `seen` counts every node the helper
    /// visited, left-out ones included, up to a bound.
    pub truncated: bool,
    pub nodes: usize,
    pub seen: usize,
    pub windows: Vec<Window>,
}

/// `kind` is the Android `AccessibilityWindowInfo` type code: 1 application, 2 input method, 3 system.
#[derive(Debug, Deserialize)]
pub struct Window {
    #[serde(rename = "type")]
    pub kind: i32,
    pub layer: i32,
    pub display: i32,
    pub bounds: [i32; 4],
    pub title: String,
    pub pkg: String,
    pub active: bool,
    pub focused: bool,
    pub root: Option<Node>,
}

#[derive(Debug, Deserialize)]
pub struct Node {
    #[serde(rename = "c")]
    pub class: String,
    #[serde(rename = "t", default)]
    pub text: String,
    #[serde(rename = "d", default)]
    pub description: String,
    #[serde(rename = "i", default)]
    pub id: String,
    #[serde(rename = "b")]
    pub bounds: [i32; 4],
    #[serde(rename = "f")]
    pub flags: u16,
    #[serde(rename = "k", default)]
    pub children: Vec<Node>,
}

impl Node {
    pub fn has(&self, flag: u16) -> bool {
        self.flags & flag != 0
    }
    fn count(&self, depth: usize) -> Result<usize> {
        if depth > MAX_DEPTH {
            return Err(Error::new(Code::Bounds, "helper tree is too deep"));
        }
        self.children.iter().try_fold(1, |n, child| Ok(n + child.count(depth + 1)?))
    }
}

/// Reads a helper reply `[seq, 0, {...}]`, refusing a helper that did not report `CAP_TREE`.
pub fn parse(reply: &[serde_json::Value], capabilities: u32) -> Result<Tree> {
    if capabilities & CAP_TREE == 0 {
        return Err(Error::new(Code::Unsupported, "this helper does not report the tree capability"));
    }
    let body = reply.get(2).ok_or_else(|| Error::new(Code::Protocol, "invalid tree reply"))?;
    let tree = Tree::deserialize(body).map_err(|e| Error::new(Code::Protocol, format!("invalid tree reply: {e}")))?;
    let mut counted = 0;
    for window in &tree.windows {
        counted += window.root.as_ref().map_or(Ok(0), |root| root.count(0))?;
    }
    if counted != tree.nodes || counted > MAX_NODES {
        return Err(Error::new(Code::Bounds, "helper tree node count is wrong or over the cap"));
    }
    Ok(tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn golden() -> Vec<Value> {
        let v: Value = serde_json::from_str(include_str!("../../protocol-golden.json")).unwrap();
        v["tree"].as_array().unwrap().clone()
    }

    #[test]
    fn reads_the_golden_the_helper_tests_also_produce() {
        let tree = parse(&golden(), CAP_TREE | 7).unwrap();
        assert!(!tree.truncated);
        assert_eq!((tree.nodes, tree.seen, tree.windows.len()), (5, 5, 2));
        let app = &tree.windows[0];
        assert_eq!((app.kind, app.pkg.as_str(), app.active), (1, "com.example.app", true));
        let fields = &app.root.as_ref().unwrap().children;
        assert_eq!(fields[0].text, "Ada");
        assert!(fields[0].has(EDITABLE | FOCUSED) && !fields[0].has(PASSWORD));
        assert!(fields[1].has(PASSWORD) && fields[1].text.is_empty());
        assert!(fields[2].has(CLICKABLE) && fields[2].has(ENABLED));
        assert_eq!(tree.windows[1].kind, 3);
    }

    #[test]
    fn a_tree_at_the_depth_cap_survives_the_frame_parser() {
        let mut node = json!({"c": "v", "b": [0, 0, 1, 1], "f": 0});
        for _ in 0..MAX_DEPTH {
            node = json!({"c": "v", "b": [0, 0, 1, 1], "f": 0, "k": [node]});
        }
        let window = json!({"type": 1, "layer": 0, "display": 0, "bounds": [0, 0, 1, 1], "title": "", "pkg": "p", "active": true, "focused": true, "root": node});
        let body = json!({"truncated": true, "nodes": MAX_DEPTH + 1, "seen": MAX_DEPTH + 1, "windows": [window]});
        let frame = serde_json::to_vec(&json!([3, 0, body])).unwrap();
        let reply: Vec<Value> = serde_json::from_slice(&frame).unwrap();
        assert_eq!(parse(&reply, CAP_TREE).unwrap().nodes, MAX_DEPTH + 1);
    }

    #[test]
    fn needs_the_capability_bit_and_a_consistent_reply() {
        assert_eq!(parse(&golden(), 7).unwrap_err().code, Code::Unsupported);
        let mut wrong = golden();
        wrong[2]["nodes"] = json!(4);
        assert_eq!(parse(&wrong, CAP_TREE).unwrap_err().code, Code::Bounds);
        assert_eq!(parse(&golden()[..2], CAP_TREE).unwrap_err().code, Code::Protocol);
    }
}
