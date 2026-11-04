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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataflowKind {
    Assign,
    Port,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataflowEdge {
    pub from: FollowTarget,
    pub to: FollowTarget,
    pub kind: DataflowKind,
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
                let Some((lhs_node, lhs_module)) = self.signal_node(scope, module, &assign.lhs)
                else {
                    continue;
                };
                for (used, line) in &assign.uses {
                    if used == &assign.lhs {
                        continue;
                    }
                    let Some((used_node, used_module)) = self.signal_node(scope, module, used)
                    else {
                        continue;
                    };
                    let left = target((
                        lhs_node.0.clone(),
                        lhs_module.clone(),
                        lhs_node.1.clone(),
                        def.file.clone(),
                        *line,
                    ));
                    let right = target((
                        used_node.0.clone(),
                        used_module,
                        used_node.1.clone(),
                        def.file.clone(),
                        *line,
                    ));
                    add_edge(&mut graph, used_node, left);
                    add_edge(&mut graph, lhs_node.clone(), right);
                }
            }
            for instance in &def.instances {
                for (child_scope, child_module) in
                    self.instance_scopes(&placements, scope, def, instance)
                {
                    let child_def = self.module(&child_module);
                    for (port, parent_signal, line) in &instance.port_uses {
                        let Some((parent_node, parent_module)) =
                            self.signal_node(scope, module, parent_signal)
                        else {
                            continue;
                        };
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
                            parent_node.clone(),
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
                                scope: parent_node.0,
                                module: parent_module,
                                signal: parent_node.1,
                                file: def.file.clone(),
                                line: *line,
                            },
                        );
                    }
                }
            }
        }

        let start_module = self
            .module_of_scope(scope)
            .map(|def| def.name.clone())
            .unwrap_or_default();
        let start = self
            .signal_node(scope, &start_module, signal)
            .map(|(node, _)| node)
            .unwrap_or_else(|| (scope.to_vec(), base_name(signal).to_string()));
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

    pub fn dataflow(&self, scope: &[String], signal: &str) -> Vec<DataflowEdge> {
        let signal = base_name(signal);
        let Some((owner, def)) = self.signal_owner(scope) else {
            return Vec::new();
        };
        let module = def.name.clone();
        let origin = (scope, signal);
        let placements = self.placements();
        let mut edges = Vec::new();
        for assign in &def.assigns {
            let Some((lhs_node, lhs_module)) = self.signal_node(&owner, &module, &assign.lhs)
            else {
                continue;
            };
            for (used, line) in &assign.uses {
                if used == &assign.lhs {
                    continue;
                }
                let Some((used_node, used_module)) = self.signal_node(&owner, &module, used) else {
                    continue;
                };
                if used_node == lhs_node
                    || (!is_origin(&used_node, origin) && !is_origin(&lhs_node, origin))
                {
                    continue;
                }
                push_dataflow(
                    &mut edges,
                    DataflowEdge {
                        from: target((
                            used_node.0,
                            used_module,
                            used_node.1,
                            def.file.clone(),
                            *line,
                        )),
                        to: target((
                            lhs_node.0.clone(),
                            lhs_module.clone(),
                            lhs_node.1.clone(),
                            def.file.clone(),
                            *line,
                        )),
                        kind: DataflowKind::Assign,
                    },
                );
            }
        }
        for instance in &def.instances {
            for child in self.instance_scopes(&placements, &owner, def, instance) {
                push_port_dataflow(self, &mut edges, &owner, def, instance, child, origin);
            }
        }
        if let Some((instance_name, parent_scope)) = owner.split_last() {
            if let Some(parent) = self.module_at_scope(parent_scope) {
                for instance in &parent.instances {
                    if instance.name != *instance_name || instance.module != module {
                        continue;
                    }
                    for child in self.instance_scopes(&placements, parent_scope, parent, instance) {
                        if child.0 == owner {
                            push_port_dataflow(
                                self,
                                &mut edges,
                                parent_scope,
                                parent,
                                instance,
                                child,
                                origin,
                            );
                        }
                    }
                }
            }
        }
        edges.sort_by(|a, b| {
            a.from
                .scope
                .cmp(&b.from.scope)
                .then_with(|| a.from.signal.cmp(&b.from.signal))
                .then_with(|| a.to.scope.cmp(&b.to.scope))
                .then_with(|| a.to.signal.cmp(&b.to.signal))
        });
        edges
    }

    fn signal_node(
        &self,
        scope: &[String],
        module: &str,
        reference: &str,
    ) -> Option<(Node, String)> {
        let mut parts = reference.split('.');
        let name = base_name(parts.next_back().unwrap_or(reference));
        let chain: Vec<String> = parts.map(str::to_string).collect();
        let bindings = self
            .scope_info(scope)
            .map(|found| found.env)
            .unwrap_or_default();
        let (path, signal) = self.resolve_reference_with(module, &chain, name, &bindings)?;
        let mut scope = scope.to_vec();
        scope.extend(path);
        Some(((scope, signal), module.to_string()))
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

fn push_port_dataflow(
    db: &RtlDb,
    edges: &mut Vec<DataflowEdge>,
    parent_scope: &[String],
    parent: &ModuleDef,
    instance: &Instance,
    child: (Vec<String>, String),
    origin: (&[String], &str),
) {
    let (child_scope, child_module) = child;
    let child_def = db.module(&child_module);
    for (port, parent_signal, line) in &instance.port_uses {
        let Some((parent_node, parent_module)) =
            db.signal_node(parent_scope, &parent.name, parent_signal)
        else {
            continue;
        };
        let at_parent = is_origin(&parent_node, origin);
        let at_child = child_scope == origin.0 && port == origin.1;
        if !at_parent && !at_child {
            continue;
        }
        let declaration = child_def.and_then(|child| child.signal(port));
        let location = child_def
            .and_then(|child| {
                child
                    .signal(port)
                    .map(|decl| (child.file.clone(), decl.line))
            })
            .unwrap_or_else(|| (parent.file.clone(), *line));
        let mut edge = DataflowEdge {
            from: target((
                parent_node.0,
                parent_module,
                parent_node.1,
                parent.file.clone(),
                *line,
            )),
            to: FollowTarget {
                scope: child_scope.clone(),
                module: child_module.clone(),
                signal: port.clone(),
                file: location.0,
                line: location.1,
            },
            kind: DataflowKind::Port,
        };
        match declaration.and_then(|decl| decl.direction.as_deref()) {
            Some("output") => std::mem::swap(&mut edge.from, &mut edge.to),
            Some("inout") => push_dataflow(
                edges,
                DataflowEdge {
                    from: edge.to.clone(),
                    to: edge.from.clone(),
                    kind: DataflowKind::Port,
                },
            ),
            _ => {}
        }
        push_dataflow(edges, edge);
    }
}

fn is_origin(node: &Node, origin: (&[String], &str)) -> bool {
    node.0 == origin.0 && node.1 == origin.1
}

fn push_dataflow(edges: &mut Vec<DataflowEdge>, edge: DataflowEdge) {
    if !edges.contains(&edge) {
        edges.push(edge);
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
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|error| panic!("create {}: {error}", dir.display()));
        let top = dir.join("top.sv");
        let child = dir.join("child.sv");
        let sub = dir.join("sub.sv");
        std::fs::write(
            &top,
            "module top(input rst);\nchild u1(.rst_in(rst));\nchild u2(.rst_in(rst));\nwire result;\nsource u3(.value(result));\nendmodule\n",
        )
        .unwrap();
        std::fs::write(
            &child,
            "module child(input rst_in);\nsub u(.reset(rst_in));\nendmodule\n",
        )
        .unwrap();
        std::fs::write(
            &sub,
            "module sub(input reset);\nwire alias;\nassign alias = reset;\nendmodule\nmodule source(output value);\nendmodule\n",
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

        let edges = db.dataflow(&["top".to_string()], "rst");
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().all(|edge| edge.kind == DataflowKind::Port));
        assert!(edges.iter().any(|edge| {
            edge.from.scope == ["top"]
                && edge.from.signal == "rst"
                && edge.to.scope == ["top", "u1"]
                && edge.to.signal == "rst_in"
        }));
        assert!(edges
            .iter()
            .all(|edge| { edge.to.scope == ["top", "u1"] || edge.to.scope == ["top", "u2"] }));

        let edges = db.dataflow(&["top".to_string(), "u1".to_string()], "rst_in");
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().any(|edge| {
            edge.from.scope == ["top", "u1"]
                && edge.from.signal == "rst_in"
                && edge.to.scope == ["top", "u1", "u"]
                && edge.to.signal == "reset"
        }));
        let edges = db.dataflow(
            &["top".to_string(), "u1".to_string(), "u".to_string()],
            "reset",
        );
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().any(|edge| {
            edge.kind == DataflowKind::Assign
                && edge.from.signal == "reset"
                && edge.to.signal == "alias"
                && edge.from.scope == ["top", "u1", "u"]
                && edge.to.scope == ["top", "u1", "u"]
        }));

        let edges = db.dataflow(&["top".to_string()], "result");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from.scope, ["top", "u3"]);
        assert_eq!(edges[0].from.signal, "value");
        assert_eq!(edges[0].to.scope, ["top"]);
        assert_eq!(edges[0].to.signal, "result");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn struct_members_follow_dataflow_and_trace() {
        let dir =
            std::env::temp_dir().join(format!("waverdi_struct_follow_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("top.sv");
        std::fs::write(
            &file,
            "module top(input clk);\n\
             typedef struct packed { logic r; logic nxt; } pc_struct;\n\
             pc_struct clk_pc;\n\
             pc_struct mem[2];\n\
             logic clk_fetch;\n\
             logic mem_value;\n\
             child u(.reset(clk_pc.r));\n\
             always_ff @(posedge clk) clk_pc.r <= clk_pc.nxt;\n\
             assign clk_fetch = clk_pc.r;\n\
             assign mem_value = mem[0].r;\n\
             assign clk_pc.nxt = clk_pc.r + 1'b1;\n\
             endmodule\n\
             module child(input reset);\n\
             typedef struct packed { logic r; logic nxt; } pc_struct;\n\
             pc_struct clk_pc;\n\
             endmodule\n",
        )
        .unwrap();
        let db = RtlDb::parse_sources(&SourceSet::from_files(vec![file], "test"));
        assert!(db.module("top").unwrap().instances[0]
            .port_uses
            .iter()
            .any(|(_, name, _)| name == "clk_pc.r"));
        let scope = vec!["top".to_string(), "clk_pc".to_string()];
        let targets = db.follow(&scope, "r");
        assert!(targets
            .iter()
            .any(|target| { target.scope == ["top", "clk_pc"] && target.signal == "nxt" }));
        assert!(targets
            .iter()
            .any(|target| target.scope == ["top", "u"] && target.signal == "reset"));

        let edges = db.dataflow(&scope, "r");
        assert!(edges.iter().any(|edge| {
            edge.kind == DataflowKind::Port
                && edge.from.scope == ["top", "clk_pc"]
                && edge.from.signal == "r"
                && edge.to.scope == ["top", "u"]
                && edge.to.signal == "reset"
        }));
        assert!(edges.iter().any(|edge| {
            edge.kind == DataflowKind::Assign
                && edge.from.scope == ["top", "clk_pc"]
                && edge.from.signal == "r"
                && edge.to.signal == "clk_fetch"
        }));
        let (module, trace) = db.trace_at_scope(&scope, "r").expect("struct trace");
        assert_eq!(module, "top");
        assert_eq!(trace.decl.unwrap().line, 3);
        assert_eq!(trace.drivers.len(), 1);
        assert!(trace.loads.len() >= 2);

        let (path, signal) = db
            .resolve_reference("top", &["u".to_string(), "clk_pc".to_string()], "r")
            .expect("instance-qualified struct field");
        assert_eq!(path, ["u", "clk_pc"]);
        assert_eq!(signal, "r");
        let (path, signal) = db
            .resolve_reference("top", &["mem[0]".to_string()], "r")
            .expect("struct array field");
        assert_eq!(path, ["mem[0]"]);
        assert_eq!(signal, "r");
        assert!(db
            .follow(&["top".to_string(), "mem[0]".to_string()], "r")
            .iter()
            .any(|target| target.signal == "mem_value"));
        assert!(!db.trace("top", "mem[0].r").unwrap().loads.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn real_struct_trace_from_optional_filelist() {
        let Ok(list) = std::env::var("WAVERDI_TEST_FILELIST") else {
            return;
        };
        let set = SourceSet::from_filelist(std::path::Path::new(&list)).unwrap();
        let db = RtlDb::parse_sources(&set);
        let scope_text = std::env::var("WAVERDI_TEST_SCOPE")
            .unwrap_or_else(|_| "dp_tst.APP_INST.CPU_INST.clk_pc".to_string());
        let scope: Vec<String> = scope_text.split('.').map(str::to_string).collect();
        assert!(!db.follow(&scope, "r").is_empty());
        assert!(!db.dataflow(&scope, "r").is_empty());
        let (_, trace) = db.trace_at_scope(&scope, "r").expect("real struct trace");
        assert!(!trace.drivers.is_empty());
    }
}
