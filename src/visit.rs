//! A small visitor over the serialized tree (`to_json`). Nodes are the JSON objects that have a
//! string `type`; `walk` calls `enter` before a node's children and `leave` after them, in source
//! order of the fields as serialized.

use serde_json::Value;

pub trait Visitor {
    /// Called when entering a node. `ancestors` runs from the root to the parent.
    fn enter(&mut self, _node: &Value, _ancestors: &[&Value]) {}
    fn leave(&mut self, _node: &Value, _ancestors: &[&Value]) {}
}

impl<F: FnMut(&Value, &[&Value])> Visitor for F {
    fn enter(&mut self, node: &Value, ancestors: &[&Value]) {
        self(node, ancestors)
    }
}

pub fn is_node(v: &Value) -> bool {
    v.get("type").is_some_and(Value::is_string)
}

/// Walk every node under (and including) `root`.
pub fn walk(root: &Value, visitor: &mut impl Visitor) {
    fn go<'a>(v: &'a Value, stack: &mut Vec<&'a Value>, vis: &mut impl Visitor) {
        match v {
            Value::Object(m) => {
                let node = is_node(v);
                if node {
                    vis.enter(v, stack);
                    stack.push(v);
                }
                for (k, c) in m {
                    // Comments are data, not tree.
                    if matches!(k.as_str(), "comments" | "leadingComments" | "trailingComments" | "tokens") {
                        continue;
                    }
                    go(c, stack, vis);
                }
                if node {
                    stack.pop();
                    vis.leave(v, stack);
                }
            }
            Value::Array(a) => a.iter().for_each(|c| go(c, stack, vis)),
            _ => {}
        }
    }
    go(root, &mut Vec::new(), visitor);
}

/// The node's `type`.
pub fn kind(v: &Value) -> &str {
    v.get("type").and_then(Value::as_str).unwrap_or("")
}
