use crate::waveform::Waveform;

use super::db::RtlDb;
use super::model::{Body, ParamEnv};
use super::resolve::{block_dump_name, gen_values, taken_branches, unnamed_number};

/// Existing child with this scope name, or a new empty node. Returns `None`
/// when an unindexed sibling with the same base name already exists, because
/// VCS sometimes drops the iteration index (`gen_x` instead of `gen_x[0]`).
fn ensure_tree_child(wf: &mut Waveform, parent: usize, name: &str) -> Option<usize> {
    if let Some(&child) = wf.tree.nodes[parent]
        .children
        .iter()
        .find(|&&child| wf.tree.nodes[child].name == name)
    {
        return Some(child);
    }
    let base = name.split('[').next().unwrap_or(name);
    if base != name
        && wf.tree.nodes[parent].children.iter().any(|&child| {
            let other = wf.tree.nodes[child].name.as_str();
            !other.contains('[') && other == base
        })
    {
        return None;
    }
    Some(wf.tree.add_scope(parent, name.to_string(), String::new()))
}

impl RtlDb {
    /// Fill in labeled generate scopes that the RTL knows about but the dump
    /// does not record (VCS only stores generate blocks that contain dumped
    /// objects). Verdi lists them; this does the same for the Instance pane.
    pub fn merge_generate_scopes(&self, wf: &mut Waveform) {
        let mut steps = Vec::new();
        self.merge_existing_scopes(wf, wf.tree.root, &mut steps);
    }

    fn merge_existing_scopes(&self, wf: &mut Waveform, node: usize, steps: &mut Vec<String>) {
        let children = wf.tree.nodes[node].children.clone();
        for child in children {
            steps.push(wf.tree.nodes[child].name.clone());
            self.merge_existing_scopes(wf, child, steps);
            steps.pop();
        }
        if steps.is_empty() {
            return;
        }
        // Each node gets its own budget (`scope_info` starts a fresh search),
        // so one pathological level cannot exhaust the lookup for the nodes
        // merged after it.
        let Some(found) = self.scope_info(steps) else {
            return;
        };
        // Only instance boundaries own the module body; generate-block nodes
        // are merged through their parent.
        if found.instance_scope.len() != steps.len() {
            return;
        }
        let def = found.module;
        let env = found.env.clone();
        self.merge_generate_items(wf, node, &env, &def.body);
    }

    /// Walk one body and add the labeled generate scopes it declares.
    fn merge_generate_items(&self, wf: &mut Waveform, node: usize, env: &ParamEnv, items: &[Body]) {
        let mut unnamed = 0usize;
        for item in items {
            match item {
                Body::For(gen) => {
                    let number = unnamed_number(gen.label.as_deref(), &mut unnamed);
                    for value in gen_values(gen, env) {
                        let name = block_dump_name(gen.label.as_deref(), number, Some(value));
                        let Some(child) = ensure_tree_child(wf, node, &name) else {
                            continue;
                        };
                        let mut env = env.clone();
                        env.insert(gen.var.clone(), value);
                        self.merge_generate_items(wf, child, &env, &gen.items);
                    }
                }
                Body::If(gen_if) => {
                    for block in taken_branches(gen_if, env) {
                        let number = unnamed_number(block.label.as_deref(), &mut unnamed);
                        let name = block_dump_name(block.label.as_deref(), number, None);
                        if let Some(child) = ensure_tree_child(wf, node, &name) {
                            self.merge_generate_items(wf, child, env, &block.items);
                        }
                    }
                }
                Body::Block(block) => {
                    let number = unnamed_number(block.label.as_deref(), &mut unnamed);
                    let name = block_dump_name(block.label.as_deref(), number, None);
                    if let Some(child) = ensure_tree_child(wf, node, &name) {
                        self.merge_generate_items(wf, child, env, &block.items);
                    }
                }
                Body::Instance(_) => {}
            }
        }
    }
}
