#[derive(Clone)]
pub struct ScopeNode {
    pub name: String,
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
                children: vec![],
                signals: vec![],
            }],
            root: 0,
        }
    }

    pub fn add_scope(&mut self, parent: usize, name: String) -> usize {
        let id = self.nodes.len();
        self.nodes.push(ScopeNode {
            name,
            children: vec![],
            signals: vec![],
        });
        self.nodes[parent].children.push(id);
        id
    }
}

impl Default for ScopeTree {
    fn default() -> Self {
        Self::new()
    }
}
