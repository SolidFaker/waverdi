#[derive(Clone)]
pub struct ScopeNode {
    pub name: String,
    /// Defining module/definition name of the instance (FSDB records it per
    /// scope; empty for VCD/FST and for old dumps that do not store it).
    pub module: String,
    /// True for SV struct/union (VHDL record) groups: their fields are dumped
    /// as a scope, but they are values, not instances.
    pub group: bool,
    pub children: Vec<usize>,
    pub signals: Vec<usize>,
}

#[derive(Clone)]
pub struct ScopeTree {
    pub nodes: Vec<ScopeNode>,
    pub root: usize,
    /// Existing children by `(parent, name)`; FSDB dumps may record the same
    /// scope path in several records (e.g. one per dump command) and Verdi
    /// merges them into a single instance.
    index: std::collections::HashMap<(usize, String), usize>,
}

impl ScopeTree {
    pub fn new() -> Self {
        Self {
            nodes: vec![ScopeNode {
                name: "design".to_string(),
                module: String::new(),
                group: false,
                children: vec![],
                signals: vec![],
            }],
            root: 0,
            index: std::collections::HashMap::new(),
        }
    }

    /// Add (or return the existing) child scope of `parent`. Repeated records
    /// of the same path merge into one node; later records only contribute
    /// their variables and sub-scopes.
    pub fn add_scope(&mut self, parent: usize, name: String, module: String) -> usize {
        if let Some(&id) = self.index.get(&(parent, name.clone())) {
            // Old dumps do not record the module; keep the first non-empty.
            if self.nodes[id].module.is_empty() && !module.is_empty() {
                self.nodes[id].module = module;
            }
            return id;
        }
        let id = self.nodes.len();
        self.nodes.push(ScopeNode {
            name: name.clone(),
            module,
            group: false,
            children: vec![],
            signals: vec![],
        });
        self.nodes[parent].children.push(id);
        self.index.insert((parent, name), id);
        id
    }

    /// Detach scopes with the given names from the tree (Verdi pseudo-scopes
    /// such as `$attribute_root`); their whole subtree disappears with them.
    #[cfg_attr(not(fsdb_sdk), allow(dead_code))]
    pub fn remove_scopes_named(&mut self, names: &[&str]) {
        let doomed: Vec<usize> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| names.contains(&node.name.as_str()))
            .map(|(id, _)| id)
            .collect();
        for id in doomed {
            for node in &mut self.nodes {
                node.children.retain(|&child| child != id);
            }
        }
    }

    /// Module the given instance is defined by, if the dump recorded it.
    pub fn module_of(&self, id: usize) -> &str {
        self.nodes
            .get(id)
            .map(|node| node.module.as_str())
            .unwrap_or("")
    }

    /// Scope path from the root down to `id` (root's name excluded).
    pub fn path_of(&self, id: usize) -> Vec<String> {
        fn rec(tree: &ScopeTree, id: usize, target: usize, path: &mut Vec<String>) -> bool {
            if id == target {
                return true;
            }
            for &child in &tree.nodes[id].children {
                path.push(tree.nodes[child].name.clone());
                if rec(tree, child, target, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        let mut path = Vec::new();
        let _ = rec(self, self.root, id, &mut path);
        path
    }
}

impl Default for ScopeTree {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_scope_records_merge_into_one_instance() {
        let mut tree = ScopeTree::new();
        let first = tree.add_scope(tree.root, "dp_tst".into(), "dp_tst".into());
        let child = tree.add_scope(first, "APP_INST".into(), "dp_app_top".into());
        let again = tree.add_scope(tree.root, "dp_tst".into(), String::new());
        assert_eq!(first, again);
        assert_eq!(tree.nodes[tree.root].children, vec![first]);
        // The second record's sub-scope merges recursively.
        let child_again = tree.add_scope(again, "APP_INST".into(), String::new());
        assert_eq!(child, child_again);
        assert_eq!(tree.nodes[first].children, vec![child]);
        assert_eq!(tree.nodes[first].module, "dp_tst");
        assert_eq!(tree.nodes[child].module, "dp_app_top");
    }

    #[test]
    fn pseudo_scopes_are_detached() {
        let mut tree = ScopeTree::new();
        let attr = tree.add_scope(tree.root, "$attribute_root".into(), String::new());
        tree.remove_scopes_named(&["$attribute_root"]);
        assert!(!tree.nodes[tree.root].children.contains(&attr));
    }
}
