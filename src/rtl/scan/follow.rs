use super::model::{base_name, Instance, ModuleDef};
use super::RtlDb;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FollowTarget {
    pub scope: Vec<String>,
    pub module: String,
    pub signal: String,
    pub file: PathBuf,
    pub line: usize,
}

type Node = (Vec<String>, String);
type Graph = HashMap<Node, Vec<FollowTarget>>;

impl RtlDb {
    pub fn follow(&self, scope: &[String], signal: &str) -> Vec<FollowTarget> {
        let placements = self.placements();
        let mut graph = Graph::new();
        for (scope, module) in &placements {
            let Some(def) = self.module(module) else {
                continue;
            };
            for assign in &def.assigns {
                if !def.declares(&assign.lhs) {
                    continue;
                }
                for (used, line) in &assign.uses {
                    if used == &assign.lhs || !def.declares(used) {
                        continue;
                    }
                    let lhs = base_name(&assign.lhs).to_string();
                    let used = base_name(used).to_string();
                    let left = (
                        scope.clone(),
                        module.clone(),
                        lhs.clone(),
                        def.file.clone(),
                        *line,
                    );
                    let right = (
                        scope.clone(),
                        module.clone(),
                        used.clone(),
                        def.file.clone(),
                        *line,
                    );
                    add_edge(&mut graph, (scope.clone(), used.clone()), target(left));
                    add_edge(&mut graph, (scope.clone(), lhs.clone()), target(right));
                }
            }
            for instance in &def.instances {
                for (child_scope, child_module) in
                    self.instance_scopes(&placements, scope, def, instance)
                {
                    let child_def = self.module(&child_module);
                    for (port, parent_signal, line) in &instance.port_uses {
                        if !def.declares(parent_signal) {
                            continue;
                        }
                        let parent_signal = base_name(parent_signal).to_string();
                        let child_location = child_def
                            .and_then(|child| {
                                child
                                    .ports
                                    .iter()
                                    .find(|decl| decl.name == *port)
                                    .map(|decl| (child.file.clone(), decl.line))
                            })
                            .unwrap_or_else(|| (def.file.clone(), *line));
                        add_edge(
                            &mut graph,
                            (scope.clone(), parent_signal.clone()),
                            FollowTarget {
                                scope: child_scope.clone(),
                                module: child_module.clone(),
                                signal: port.clone(),
                                file: child_location.0,
                                line: child_location.1,
                            },
                        );
                        add_edge(
                            &mut graph,
                            (child_scope.clone(), port.clone()),
                            FollowTarget {
                                scope: scope.clone(),
                                module: module.clone(),
                                signal: parent_signal.clone(),
                                file: def.file.clone(),
                                line: *line,
                            },
                        );
                    }
                }
            }
        }

        let start = (scope.to_vec(), base_name(signal).to_string());
        let mut visited = HashSet::from([start.clone()]);
        let mut queue = VecDeque::from([start]);
        let mut out = Vec::new();
        while let Some(node) = queue.pop_front() {
            let Some(edges) = graph.get(&node) else {
                continue;
            };
            for target in edges {
                let node = (target.scope.clone(), target.signal.clone());
                if !visited.insert(node.clone()) {
                    continue;
                }
                out.push(target.clone());
                if out.len() >= 256 {
                    break;
                }
                queue.push_back(node);
            }
            if out.len() >= 256 {
                break;
            }
        }
        out.sort_by(|a, b| {
            a.scope
                .len()
                .cmp(&b.scope.len())
                .then_with(|| a.scope.cmp(&b.scope))
                .then_with(|| a.signal.cmp(&b.signal))
        });
        out
    }

    fn instance_scopes(
        &self,
        placements: &[(Vec<String>, String)],
        scope: &[String],
        def: &ModuleDef,
        instance: &Instance,
    ) -> Vec<(Vec<String>, String)> {
        let mut direct = scope.to_vec();
        direct.push(instance.name.clone());
        if self
            .module_at_scope(&direct)
            .is_some_and(|module| module.name == instance.module)
        {
            return vec![(direct, instance.module.clone())];
        }
        placements
            .iter()
            .filter(|(path, module)| {
                *module == instance.module
                    && path.len() > scope.len()
                    && path.starts_with(scope)
                    && path.last().is_some_and(|name| *name == instance.name)
                    && self
                        .module_at_scope(&path[..path.len() - 1])
                        .is_some_and(|parent| parent.name == def.name)
            })
            .cloned()
            .collect()
    }
}

fn target(
    (scope, module, signal, file, line): (Vec<String>, String, String, PathBuf, usize),
) -> FollowTarget {
    FollowTarget {
        scope,
        module,
        signal,
        file,
        line,
    }
}

fn add_edge(graph: &mut Graph, from: Node, to: FollowTarget) {
    if from == (to.scope.clone(), to.signal.clone()) {
        return;
    }
    let edges = graph.entry(from).or_default();
    if !edges.contains(&to) {
        edges.push(to);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtl::SourceSet;

    #[test]
    fn follows_assignments_and_nested_port_connections() {
        let dir = std::env::temp_dir().join(format!("waverdi_follow_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let top = dir.join("top.sv");
        let child = dir.join("child.sv");
        let sub = dir.join("sub.sv");
        std::fs::write(
            &top,
            "module top(input rst);\nchild u1(.rst_in(rst));\nchild u2(.rst_in(rst));\nendmodule\n",
        )
        .unwrap();
        std::fs::write(
            &child,
            "module child(input rst_in);\nsub u(.reset(rst_in));\nendmodule\n",
        )
        .unwrap();
        std::fs::write(
            &sub,
            "module sub(input reset);\nwire alias;\nassign alias = reset;\nendmodule\n",
        )
        .unwrap();
        let db = RtlDb::parse_sources(&SourceSet::from_files(vec![top, child, sub], "test"));
        let targets = db.follow(&["top".to_string()], "rst");
        assert!(targets
            .iter()
            .any(|target| { target.scope == ["top", "u1"] && target.signal == "rst_in" }));
        assert!(targets
            .iter()
            .any(|target| { target.scope == ["top", "u2"] && target.signal == "rst_in" }));
        assert!(targets
            .iter()
            .any(|target| { target.scope == ["top", "u1", "u"] && target.signal == "alias" }));
        assert!(targets.iter().all(|target| target.line > 0));
        let _ = std::fs::remove_dir_all(dir);
    }
}
