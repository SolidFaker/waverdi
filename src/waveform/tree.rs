#[derive(Clone)]
pub struct ScopeNode {
    pub name: String,
    /// Defining module/definition name of the instance (FSDB records it per
    /// scope; empty for VCD/FST and for old dumps that do not store it).
    pub module: String,
    pub children: Vec<usize>,
    pub signals: Vec<usize>,
}

#[derive(Clone)]
pub struct ScopeTree {
    pub nodes: Vec<ScopeNode>,
    pub root: usize,
}

impl ScopeTree {
    pub fn new() -> Self {
        Self {
            nodes: vec![ScopeNode {
                name: "design".to_string(),
                module: String::new(),
                children: vec![],
                signals: vec![],
            }],
            root: 0,
        }
    }

    pub fn add_scope(&mut self, parent: usize, name: String, module: String) -> usize {
        let id = self.nodes.len();
        self.nodes.push(ScopeNode {
            name,
            module,
            children: vec![],
            signals: vec![],
        });
        self.nodes[parent].children.push(id);
        id
    }

    /// Module the given instance is defined by, if the dump recorded it.
    pub fn module_of(&self, id: usize) -> &str {
        self.nodes
            .get(id)
            .map(|node| node.module.as_str())
            .unwrap_or("")
    }
}

impl Default for ScopeTree {
    fn default() -> Self {
        Self::new()
    }
}
