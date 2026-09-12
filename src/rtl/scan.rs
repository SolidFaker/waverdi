//! Lightweight SystemVerilog scanner for the RTL source view and tracing.
//!
//! This is intentionally not a full parser: it recognises modules, signal
//! declarations, continuous assignments, `always`/`initial` blocks and module
//! instantiations together with their source lines — enough to browse RTL and
//! to trace a signal to its declaration, drivers and loads.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::SourceSet;

/// File + line reference into the parsed sources.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Location {
    pub file: PathBuf,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct SignalDecl {
    pub name: String,
    /// `wire`, `reg`, `logic`, ... (kept for the signal tooltip/bus slicing).
    #[allow(dead_code)]
    pub kind: String,
    #[allow(dead_code)]
    pub direction: Option<String>,
    /// Raw range text as written, e.g. `7:0` (kept for bus slicing).
    #[allow(dead_code)]
    pub range: Option<String>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct Assign {
    pub lhs: String,
    pub line: usize,
    /// Identifiers used on the right-hand side.
    pub uses: Vec<(String, usize)>,
}

#[derive(Clone, Debug)]
pub struct AlwaysBlock {
    pub start: usize,
    pub end: usize,
    /// Signals in the sensitivity list (`@(posedge clk)`).
    pub sensitivity: Vec<(String, usize)>,
    /// Assignments performed inside the block.
    pub drivers: Vec<(String, usize)>,
    /// Identifiers read inside the block / sensitivity list.
    pub uses: Vec<(String, usize)>,
}

#[derive(Clone, Debug)]
pub struct Instance {
    /// Instantiated module type (kept for "jump to instantiation").
    #[allow(dead_code)]
    pub module: String,
    #[allow(dead_code)]
    pub name: String,
    #[allow(dead_code)]
    pub line: usize,
    /// `#(.NAME(expr))` parameter overrides, in source order.
    pub overrides: Vec<(String, Expr)>,
}

/// Small constant expression used for parameters and generate bounds.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(i64),
    Name(String),
    Unary(char, Box<Expr>),
    Binary(Box<Expr>, BinOp, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    LAnd,
    LOr,
}

/// `generate for` loop; `step` yields the next value of `var`.
#[derive(Clone, Debug)]
pub struct GenFor {
    pub label: Option<String>,
    pub var: String,
    pub init: Expr,
    pub cond: Expr,
    pub step: Expr,
    pub items: Vec<Body>,
    /// Kept for jump-to-generate.
    #[allow(dead_code)]
    pub line: usize,
}

/// One `begin : label ... end` generate block (also the body of a for loop).
#[derive(Clone, Debug)]
pub struct GenBlock {
    pub label: Option<String>,
    pub items: Vec<Body>,
    #[allow(dead_code)]
    pub line: usize,
}

/// `generate if` / `else if` / `else`, one entry per branch.
#[derive(Clone, Debug)]
pub struct GenIf {
    /// `(condition, block)`; the condition is `None` for the final `else`.
    pub branches: Vec<(Option<Expr>, GenBlock)>,
    #[allow(dead_code)]
    pub line: usize,
}

/// Structural item of a module body (statements are kept in the flat lists).
#[derive(Clone, Debug)]
pub enum Body {
    Instance(Instance),
    For(GenFor),
    Block(GenBlock),
    If(GenIf),
}

#[derive(Clone, Debug)]
pub struct ModuleDef {
    pub name: String,
    pub file: PathBuf,
    pub start: usize,
    pub end: usize,
    /// Module parameter/localparam values (evaluated from defaults), if known.
    pub params: BTreeMap<String, Option<i64>>,
    pub ports: Vec<SignalDecl>,
    pub signals: Vec<SignalDecl>,
    pub assigns: Vec<Assign>,
    pub always: Vec<AlwaysBlock>,
    /// Flat list of every instantiation, including those inside generate.
    pub instances: Vec<Instance>,
    /// Hierarchical body: instantiations, `generate for` and `generate if`.
    pub body: Vec<Body>,
}

impl ModuleDef {
    pub fn signal(&self, name: &str) -> Option<&SignalDecl> {
        self.signals.iter().find(|decl| decl.name == name)
    }

    /// Whether `name` is declared in this module (port or internal signal,
    /// range-suffixed names like `count[7:0]` match their base).
    pub fn declares(&self, name: &str) -> bool {
        let want = base_name(name);
        self.signals
            .iter()
            .any(|decl| decl.name == name || base_name(&decl.name) == want)
    }
}

/// Identifier without a packed range: `count[7:0]` -> `count`.
fn base_name(text: &str) -> &str {
    text.split('[').next().unwrap_or(text)
}

/// Values a `generate for` loop takes: `(label, index)` names in a dump scope
/// look like `gen_clusters[0]`.
fn gen_values(gen: &GenFor, env: &ParamEnv) -> Vec<i64> {
    let mut values = Vec::new();
    let mut value = eval(&gen.init, env);
    let mut guard = 0usize;
    while let Some(current) = value {
        guard += 1;
        if guard > 4096 {
            break;
        }
        let mut env = env.clone();
        env.insert(gen.var.clone(), current);
        if eval(&gen.cond, &env) == Some(0) {
            break;
        }
        values.push(current);
        value = eval(&gen.step, &env);
    }
    values
}

/// Split `gen_clusters[0]` into `("gen_clusters", Some(0))`; a plain name
/// yields `(name, None)`.
fn split_block_index(step: &str) -> Option<(&str, Option<i64>)> {
    if let Some(open) = step.find('[') {
        let label = &step[..open];
        let index = step[open + 1..].strip_suffix(']')?.parse::<i64>().ok()?;
        Some((label, Some(index)))
    } else {
        Some((step, None))
    }
}

/// Generate-if branches that are instantiated: conditions that are known
/// false are skipped, a known-true branch ends the chain, and branches with
/// unknown conditions are kept (they cannot be ruled out).
fn taken_branches<'a>(gen_if: &'a GenIf, env: &ParamEnv) -> Vec<&'a GenBlock> {
    let mut taken = Vec::new();
    for (cond, block) in &gen_if.branches {
        match cond {
            None => taken.push(block),
            Some(expr) => match eval(expr, env) {
                Some(0) => {}
                Some(_) => {
                    taken.push(block);
                    break;
                }
                None => taken.push(block),
            },
        }
    }
    taken
}

/// Scope names that cannot be produced by the source AST: unnamed generate
/// blocks (`genblk1[0]`), tool-generated scopes (`unnamed$$_0`) and FSDB
/// pseudo scopes (`$attribute_root`).
fn generated_scope_name(step: &str) -> bool {
    step.starts_with("genblk")
        || step.starts_with("unnamed")
        || step.starts_with('$')
        || step.contains('[')
}

/// Parse the tool-assigned name of an unnamed generate block: `genblk1[0]`
/// -> `(1, Some(0))`, `genblk2` -> `(2, None)`.
fn genblk_step(step: &str) -> Option<(usize, Option<i64>)> {
    let rest = step.strip_prefix("genblk")?;
    if let Some(open) = rest.find('[') {
        let number = rest[..open].parse::<usize>().ok()?;
        let index = rest[open + 1..].strip_suffix(']')?.parse::<i64>().ok()?;
        Some((number, Some(index)))
    } else {
        Some((rest.parse::<usize>().ok()?, None))
    }
}

/// Index of an unnamed generate construct: `genblk<N>` names are assigned in
/// source order to the unnamed `for`/`if` blocks of a module.
fn unnamed_ordinal(unnamed: &mut usize) -> usize {
    *unnamed += 1;
    *unnamed
}

/// Dump scope name of a generate block: the label or the tool-assigned
/// `genblk<N>`, with the loop index appended for `for` loops.
fn block_dump_name(label: Option<&str>, number: Option<usize>, index: Option<i64>) -> String {
    let base = match label {
        Some(label) => label.to_string(),
        None => format!("genblk{}", number.unwrap_or(1)),
    };
    match index {
        Some(value) => format!("{base}[{value}]"),
        None => base,
    }
}

/// Does the dump scope name `step` name this generate block? Returns the loop
/// index written in the name (`None` when the name has no index).
fn block_step_matches(
    step: &str,
    label: Option<&str>,
    number: Option<usize>,
) -> Option<Option<i64>> {
    if let Some(label) = label {
        let (name, index) = split_block_index(step).unwrap_or((step, None));
        return (name == label).then_some(index);
    }
    let number = number?;
    let (n, index) = genblk_step(step)?;
    (n == number).then_some(index)
}

/// Declaration, drivers and loads of one signal inside a module.
#[derive(Clone, Debug, Default)]
pub struct SignalTrace {
    pub decl: Option<Location>,
    pub drivers: Vec<Location>,
    pub loads: Vec<Location>,
}

/// All modules found in the RTL sources.
#[derive(Clone, Debug, Default)]
pub struct RtlDb {
    pub modules: BTreeMap<String, ModuleDef>,
    pub files: Vec<PathBuf>,
}

impl RtlDb {
    /// Parse every `.v`/`.sv` file of a source set.
    pub fn parse_sources(set: &SourceSet) -> RtlDb {
        let mut db = RtlDb::default();
        for path in &set.files {
            if !is_verilog(path) {
                continue;
            }
            let Ok(text) = fs::read_to_string(path) else {
                continue;
            };
            db.files.push(path.clone());
            for module in parse_module_text(&text, path) {
                db.modules.entry(module.name.clone()).or_insert(module);
            }
        }
        db
    }

    pub fn module(&self, name: &str) -> Option<&ModuleDef> {
        self.modules.get(name)
    }

    /// Modules that no other parsed module instantiates: the roots of the
    /// elaborated design hierarchy.
    pub fn top_modules(&self) -> Vec<String> {
        let mut instantiated: HashSet<&str> = HashSet::new();
        for module in self.modules.values() {
            for instance in &module.instances {
                instantiated.insert(instance.module.as_str());
            }
        }
        self.modules
            .keys()
            .filter(|name| !instantiated.contains(name.as_str()))
            .cloned()
            .collect()
    }

    /// Known parameter/genvar values of a module (default overrides).
    pub(crate) fn default_env(&self, def: &ModuleDef) -> ParamEnv {
        module_env(def)
    }

    /// Parameter environment of an instance: child defaults plus overrides.
    fn instance_env(&self, instance: &Instance, child: &ModuleDef, parent: &ParamEnv) -> ParamEnv {
        let mut env = self.default_env(child);
        for (name, expr) in &instance.overrides {
            if let Some(value) = eval(expr, parent) {
                env.insert(name.clone(), value);
            }
        }
        env
    }

    /// Walk the elaborated design hierarchy (through `generate for` loops and
    /// `generate if` branches) to the module instantiated at a dump scope
    /// path, e.g. `["tb", "gen_clusters[0]", "u_cluster"]` -> `cluster`.
    pub fn module_at_scope(&self, steps: &[String]) -> Option<&ModuleDef> {
        if steps.is_empty() {
            return None;
        }
        for top in self.top_modules() {
            let Some(def) = self.modules.get(&top) else {
                continue;
            };
            let Some(start) = steps.iter().position(|step| step == &top) else {
                continue;
            };
            let env = self.default_env(def);
            if let Some(module) = self.walk_scope(def, &env, &def.body, &steps[start + 1..]) {
                return Some(module);
            }
        }
        None
    }

    fn walk_scope<'a>(
        &'a self,
        def: &'a ModuleDef,
        env: &ParamEnv,
        items: &[Body],
        steps: &[String],
    ) -> Option<&'a ModuleDef> {
        if steps.is_empty() {
            return Some(def);
        }
        let step = &steps[0];
        let rest = &steps[1..];
        let mut unnamed = 0usize;
        for item in items {
            match item {
                Body::Instance(instance) => {
                    if instance.name != *step {
                        continue;
                    }
                    let Some(child) = self.modules.get(&instance.module) else {
                        continue;
                    };
                    let env = self.instance_env(instance, child, env);
                    if let Some(found) = self.walk_scope(child, &env, &child.body, rest) {
                        return Some(found);
                    }
                }
                Body::For(gen) => {
                    let number = if gen.label.is_none() {
                        Some(unnamed_ordinal(&mut unnamed))
                    } else {
                        None
                    };
                    let Some(index) = block_step_matches(step, gen.label.as_deref(), number) else {
                        continue;
                    };
                    for value in gen_values(gen, env) {
                        if index.is_some_and(|wanted| wanted != value) {
                            continue;
                        }
                        let mut env = env.clone();
                        env.insert(gen.var.clone(), value);
                        if let Some(found) = self.walk_scope(def, &env, &gen.items, rest) {
                            return Some(found);
                        }
                    }
                }
                Body::If(gen_if) => {
                    for block in taken_branches(gen_if, env) {
                        let number = if block.label.is_none() {
                            Some(unnamed_ordinal(&mut unnamed))
                        } else {
                            None
                        };
                        let Some(index) = block_step_matches(step, block.label.as_deref(), number)
                        else {
                            continue;
                        };
                        if index.is_some() {
                            continue;
                        }
                        if let Some(found) = self.walk_scope(def, env, &block.items, rest) {
                            return Some(found);
                        }
                    }
                }
                Body::Block(block) => {
                    let number = if block.label.is_none() {
                        Some(unnamed_ordinal(&mut unnamed))
                    } else {
                        None
                    };
                    let Some(index) = block_step_matches(step, block.label.as_deref(), number)
                    else {
                        continue;
                    };
                    if index.is_some() {
                        continue;
                    }
                    if let Some(found) = self.walk_scope(def, env, &block.items, rest) {
                        return Some(found);
                    }
                }
            }
        }
        // Unnamed generate blocks and tool-generated helper scopes carry no
        // meaning for the source mapping: skip them.
        if generated_scope_name(step) {
            return self.walk_scope(def, env, items, rest);
        }
        None
    }

    /// Resolve a reference inside `module`: `name`, optionally qualified by a
    /// chain of instance names (`u_dut.count`) or generate blocks
    /// (`gen_pes[0].u_pe.count`). Returns the instance path taken and the
    /// declared signal name, or `None` when the AST says the reference is not
    /// a signal (a keyword, an instance or an unknown name).
    pub fn resolve_reference(
        &self,
        module: &str,
        chain: &[String],
        name: &str,
    ) -> Option<(Vec<String>, String)> {
        let def = self.module(module)?;
        let env = self.default_env(def);
        let (path, target) = self.walk_chain(def, &env, &def.body, chain)?;
        if target.declares(name) {
            let signal = target
                .signals
                .iter()
                .find(|decl| decl.name == name || base_name(&decl.name) == base_name(name))
                .map(|decl| decl.name.clone())
                .unwrap_or_else(|| name.to_string());
            return Some((path, signal));
        }
        // `sig.field`: the head signal is the target, the field is a slice.
        if let Some(head) = chain.first() {
            if def.declares(head) {
                return Some((Vec::new(), head.clone()));
            }
        }
        None
    }

    /// Walk an instance/generate chain inside a body, returning the scope
    /// segments taken and the module that contains the final target.
    fn walk_chain<'a>(
        &'a self,
        def: &'a ModuleDef,
        env: &ParamEnv,
        items: &'a [Body],
        chain: &[String],
    ) -> Option<(Vec<String>, &'a ModuleDef)> {
        if chain.is_empty() {
            return Some((Vec::new(), def));
        }
        let step = &chain[0];
        // Direct children: instances and explicitly named generate blocks.
        let mut unnamed = 0usize;
        for item in items {
            match item {
                Body::Instance(instance) if instance.name == *step => {
                    let child = self.modules.get(&instance.module)?;
                    let env = self.instance_env(instance, child, env);
                    let (rest, target) = self.walk_chain(child, &env, &child.body, &chain[1..])?;
                    let mut path = vec![instance.name.clone()];
                    path.extend(rest);
                    return Some((path, target));
                }
                Body::For(gen) => {
                    let number = if gen.label.is_none() {
                        Some(unnamed_ordinal(&mut unnamed))
                    } else {
                        None
                    };
                    let Some(index) = block_step_matches(step, gen.label.as_deref(), number) else {
                        continue;
                    };
                    for value in gen_values(gen, env) {
                        if index.is_some_and(|wanted| wanted != value) {
                            continue;
                        }
                        let mut env = env.clone();
                        env.insert(gen.var.clone(), value);
                        if let Some((rest, target)) =
                            self.walk_chain(def, &env, &gen.items, &chain[1..])
                        {
                            let mut path =
                                vec![block_dump_name(gen.label.as_deref(), number, Some(value))];
                            path.extend(rest);
                            return Some((path, target));
                        }
                    }
                }
                Body::If(gen_if) => {
                    for block in taken_branches(gen_if, env) {
                        let number = if block.label.is_none() {
                            Some(unnamed_ordinal(&mut unnamed))
                        } else {
                            None
                        };
                        let Some(index) = block_step_matches(step, block.label.as_deref(), number)
                        else {
                            continue;
                        };
                        if index.is_some() {
                            continue;
                        }
                        if let Some((rest, target)) =
                            self.walk_chain(def, env, &block.items, &chain[1..])
                        {
                            let mut path =
                                vec![block_dump_name(block.label.as_deref(), number, None)];
                            path.extend(rest);
                            return Some((path, target));
                        }
                    }
                }
                Body::Block(block) => {
                    let number = if block.label.is_none() {
                        Some(unnamed_ordinal(&mut unnamed))
                    } else {
                        None
                    };
                    let Some(index) = block_step_matches(step, block.label.as_deref(), number)
                    else {
                        continue;
                    };
                    if index.is_some() {
                        continue;
                    }
                    if let Some((rest, target)) =
                        self.walk_chain(def, env, &block.items, &chain[1..])
                    {
                        let mut path = vec![block_dump_name(block.label.as_deref(), number, None)];
                        path.extend(rest);
                        return Some((path, target));
                    }
                }
                _ => {}
            }
        }
        // Plain names inside generate blocks: search the generated scopes.
        let mut unnamed = 0usize;
        for item in items {
            match item {
                Body::For(gen) => {
                    let number = if gen.label.is_none() {
                        Some(unnamed_ordinal(&mut unnamed))
                    } else {
                        None
                    };
                    for value in gen_values(gen, env) {
                        let mut env = env.clone();
                        env.insert(gen.var.clone(), value);
                        if let Some((rest, target)) = self.walk_chain(def, &env, &gen.items, chain)
                        {
                            let mut path =
                                vec![block_dump_name(gen.label.as_deref(), number, Some(value))];
                            path.extend(rest);
                            return Some((path, target));
                        }
                    }
                }
                Body::If(gen_if) => {
                    for block in taken_branches(gen_if, env) {
                        let number = if block.label.is_none() {
                            Some(unnamed_ordinal(&mut unnamed))
                        } else {
                            None
                        };
                        if let Some((rest, target)) = self.walk_chain(def, env, &block.items, chain)
                        {
                            let mut path =
                                vec![block_dump_name(block.label.as_deref(), number, None)];
                            path.extend(rest);
                            return Some((path, target));
                        }
                    }
                }
                Body::Block(block) => {
                    let number = if block.label.is_none() {
                        Some(unnamed_ordinal(&mut unnamed))
                    } else {
                        None
                    };
                    if let Some((rest, target)) = self.walk_chain(def, env, &block.items, chain) {
                        let mut path = vec![block_dump_name(block.label.as_deref(), number, None)];
                        path.extend(rest);
                        return Some((path, target));
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Every elaborated instance path in the design: `(scope, module)` where
    /// `scope` starts with the top instance name and includes generate block
    /// names, e.g. `(["tb", "gen_clusters[0]", "u_cluster"], "cluster")`.
    /// Recursive instantiations are cut off.
    pub fn placements(&self) -> Vec<(Vec<String>, String)> {
        let mut out = Vec::new();
        for top in self.top_modules() {
            let Some(def) = self.modules.get(&top) else {
                continue;
            };
            let env = self.default_env(def);
            let mut path = vec![top.clone()];
            let mut stack = vec![top.clone()];
            out.push((path.clone(), top.clone()));
            self.walk_body_placements(&env, &def.body, &mut path, &mut out, &mut stack);
        }
        out
    }

    fn walk_body_placements(
        &self,
        env: &ParamEnv,
        items: &[Body],
        path: &mut Vec<String>,
        out: &mut Vec<(Vec<String>, String)>,
        stack: &mut Vec<String>,
    ) {
        let mut unnamed = 0usize;
        for item in items {
            match item {
                Body::Instance(instance) => {
                    path.push(instance.name.clone());
                    out.push((path.clone(), instance.module.clone()));
                    if let Some(child) = self.modules.get(&instance.module) {
                        if !stack.contains(&instance.module) {
                            let env = self.instance_env(instance, child, env);
                            stack.push(instance.module.clone());
                            self.walk_body_placements(&env, &child.body, path, out, stack);
                            stack.pop();
                        }
                    }
                    path.pop();
                }
                Body::For(gen) => {
                    let number = if gen.label.is_none() {
                        Some(unnamed_ordinal(&mut unnamed))
                    } else {
                        None
                    };
                    for value in gen_values(gen, env) {
                        let mut env = env.clone();
                        env.insert(gen.var.clone(), value);
                        path.push(block_dump_name(gen.label.as_deref(), number, Some(value)));
                        self.walk_body_placements(&env, &gen.items, path, out, stack);
                        path.pop();
                    }
                }
                Body::If(gen_if) => {
                    for block in taken_branches(gen_if, env) {
                        let number = if block.label.is_none() {
                            Some(unnamed_ordinal(&mut unnamed))
                        } else {
                            None
                        };
                        path.push(block_dump_name(block.label.as_deref(), number, None));
                        self.walk_body_placements(env, &block.items, path, out, stack);
                        path.pop();
                    }
                }
                Body::Block(block) => {
                    let number = if block.label.is_none() {
                        Some(unnamed_ordinal(&mut unnamed))
                    } else {
                        None
                    };
                    path.push(block_dump_name(block.label.as_deref(), number, None));
                    self.walk_body_placements(env, &block.items, path, out, stack);
                    path.pop();
                }
            }
        }
    }

    /// Declaration / driver / load lines of a signal in a module.
    pub fn trace(&self, module: &str, signal: &str) -> Option<SignalTrace> {
        let m = self.modules.get(module)?;
        let mut trace = SignalTrace::default();
        let at = |line: usize| Location {
            file: m.file.clone(),
            line,
        };
        trace.decl = m.signal(signal).map(|decl| at(decl.line));
        for assign in &m.assigns {
            if assign.lhs == signal {
                trace.drivers.push(at(assign.line));
            }
            for (name, line) in &assign.uses {
                if name == signal {
                    trace.loads.push(at(*line));
                }
            }
        }
        for block in &m.always {
            for (name, line) in &block.drivers {
                if name == signal {
                    trace.drivers.push(at(*line));
                }
            }
            for (name, line) in block.uses.iter().chain(&block.sensitivity) {
                if name == signal {
                    trace.loads.push(at(*line));
                }
            }
        }
        trace.drivers.sort();
        trace.drivers.dedup();
        trace.loads.retain(|loc| !trace.drivers.contains(loc));
        trace.loads.sort();
        trace.loads.dedup();
        Some(trace)
    }
}

fn is_verilog(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("v" | "sv")
    )
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Num(String),
    Str,
    Punct(char),
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    line: usize,
}

fn lex(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            c if c.is_whitespace() => i += 1,
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    if i < chars.len() && chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
                i += 1;
                out.push(Token {
                    tok: Tok::Str,
                    line,
                });
            }
            '\\' => {
                let start = i;
                i += 1;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                out.push(Token {
                    tok: Tok::Ident(name),
                    line,
                });
            }
            '`' => {
                i += 1;
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                if i > start {
                    let name: String = chars[start..i].iter().collect();
                    out.push(Token {
                        tok: Tok::Ident(name),
                        line,
                    });
                }
            }
            '$' | 'a'..='z' | 'A'..='Z' | '_' => {
                let start = i;
                i += 1;
                while i < chars.len()
                    && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$')
                {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                out.push(Token {
                    tok: Tok::Ident(name),
                    line,
                });
            }
            '0'..='9' => {
                let start = i;
                i += 1;
                while i < chars.len()
                    && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '\'' | '.' | '?'))
                {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                out.push(Token {
                    tok: Tok::Num(text),
                    line,
                });
            }
            _ => {
                out.push(Token {
                    tok: Tok::Punct(c),
                    line,
                });
                i += 1;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos).map(|t| &t.tok)
    }

    fn peek_at(&self, offset: usize) -> Option<&Tok> {
        self.tokens.get(self.pos + offset).map(|t| &t.tok)
    }

    fn line(&self) -> usize {
        self.tokens
            .get(self.pos)
            .map(|t| t.line)
            .unwrap_or_else(|| self.tokens.last().map(|t| t.line).unwrap_or(1))
    }

    fn next(&mut self) -> Option<&Tok> {
        let tok = self.tokens.get(self.pos).map(|t| &t.tok);
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    fn at_ident(&self, name: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(ident)) if ident == name)
    }

    fn eat_ident(&mut self, name: &str) -> bool {
        if self.at_ident(name) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn take_ident(&mut self) -> Option<(String, usize)> {
        let line = self.line();
        match self.peek() {
            Some(Tok::Ident(name)) => {
                let name = name.clone();
                self.pos += 1;
                Some((name, line))
            }
            _ => None,
        }
    }

    /// Skip one balanced group assuming the current token is the opener.
    fn skip_balanced(&mut self) {
        let open = match self.peek() {
            Some(Tok::Punct(c @ ('(' | '[' | '{'))) => *c,
            _ => return,
        };
        let close = match open {
            '(' => ')',
            '[' => ']',
            _ => '}',
        };
        let mut depth = 0i32;
        while let Some(tok) = self.next() {
            match tok {
                Tok::Punct(c) if *c == open => depth += 1,
                Tok::Punct(c) if *c == close => {
                    depth -= 1;
                    if depth <= 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    /// Skip to the end of the current statement.
    fn skip_statement(&mut self) {
        let mut depth = 0i32;
        while let Some(tok) = self.next() {
            match tok {
                Tok::Punct('(' | '[' | '{') => depth += 1,
                Tok::Punct(')' | ']' | '}') => depth -= 1,
                Tok::Punct(';') if depth <= 0 => return,
                _ => {}
            }
        }
    }

    /// Skip a block until one of the given end keywords at begin/end depth 0.
    fn skip_to(&mut self, end_keywords: &[&str]) {
        let mut depth = 0i32;
        while let Some(tok) = self.next() {
            if let Tok::Ident(name) = tok {
                match name.as_str() {
                    "begin" | "fork" => depth += 1,
                    "end" | "join" | "join_any" | "join_none" => depth -= 1,
                    other if end_keywords.contains(&other) && depth <= 0 => return,
                    _ => {}
                }
            }
        }
    }
}

fn parse_module_text(text: &str, path: &Path) -> Vec<ModuleDef> {
    let mut parser = Parser::new(lex(text));
    let mut modules = Vec::new();
    while parser.pos < parser.tokens.len() {
        if parser.eat_ident("module") || parser.eat_ident("macromodule") {
            if let Some(module) = parse_module(&mut parser, path) {
                modules.push(module);
            }
        } else {
            parser.next();
        }
    }
    modules
}

fn parse_module(parser: &mut Parser, path: &Path) -> Option<ModuleDef> {
    let start = parser.line();
    let (name, _) = parser.take_ident()?;
    let mut module = ModuleDef {
        name,
        file: path.to_path_buf(),
        start,
        end: start,
        params: BTreeMap::new(),
        ports: Vec::new(),
        signals: Vec::new(),
        assigns: Vec::new(),
        always: Vec::new(),
        instances: Vec::new(),
        body: Vec::new(),
    };

    // Optional parameter list, then the port list (ANSI declarations inside).
    if matches!(parser.peek(), Some(Tok::Punct('#'))) {
        parser.next();
        if matches!(parser.peek(), Some(Tok::Punct('('))) {
            parser.next();
            parse_param_decls(parser, &mut module);
            parser.eat_punct(')');
        }
    }
    if matches!(parser.peek(), Some(Tok::Punct('('))) {
        parser.next();
        parse_declarations(parser, &mut module, true);
        parser.eat_punct(')');
    }
    // Skip anything else before the body (e.g. `;`).
    if matches!(parser.peek(), Some(Tok::Punct(';'))) {
        parser.next();
    }

    module.body = parse_body(parser, &mut module);
    module.end = parser.line();
    if parser.at_ident("endmodule") {
        parser.next();
    }
    Some(module)
}

/// Parse module-body items until `end`/`endmodule` (not consumed).
fn parse_body(parser: &mut Parser, module: &mut ModuleDef) -> Vec<Body> {
    let mut items = Vec::new();
    loop {
        match parser.peek() {
            None => break,
            Some(Tok::Ident(word)) if word == "end" || word == "endmodule" => break,
            _ => {}
        }
        if let Some(item) = parse_body_item(parser, module) {
            items.push(item);
        }
    }
    items
}

/// Parse one module-body statement; structural items come back as `Body`.
fn parse_body_item(parser: &mut Parser, module: &mut ModuleDef) -> Option<Body> {
    let Tok::Ident(word) = parser.peek().cloned()? else {
        parser.next();
        return None;
    };
    match word.as_str() {
        "generate" | "endgenerate" => {
            parser.next();
            None
        }
        "input" | "output" | "inout" | "wire" | "reg" | "logic" | "bit" | "integer" | "real"
        | "time" | "genvar" | "supply0" | "supply1" | "tri" | "triand" | "trior" | "wand"
        | "wor" | "trireg" => {
            parse_declarations(parser, module, false);
            None
        }
        "parameter" | "localparam" => {
            parse_param_decls(parser, module);
            None
        }
        "assign" => {
            parser.next();
            parse_assign(parser, module);
            None
        }
        "always" | "always_ff" | "always_comb" | "always_latch" | "initial" | "final" => {
            parser.next();
            parse_process(parser, module);
            None
        }
        "function" => {
            parser.skip_to(&["endfunction"]);
            None
        }
        "task" => {
            parser.skip_to(&["endtask"]);
            None
        }
        "defparam" | "specify" | "endspecify" => {
            parser.skip_statement();
            None
        }
        "case" | "casex" | "casez" => {
            parser.skip_to(&["endcase"]);
            None
        }
        "for" => parse_gen_for(parser, module).map(Body::For),
        "if" => parse_gen_if(parser, module).map(Body::If),
        "begin" => Some(Body::Block(parse_gen_block(parser, module))),
        "end" | "endmodule" => None,
        _ => {
            if let Some(instance) = parse_instance(parser, module) {
                module.instances.push(instance.clone());
                Some(Body::Instance(instance))
            } else {
                parser.next();
                None
            }
        }
    }
}

/// `for (i = init; cond; i++) begin : label ... end`
fn parse_gen_for(parser: &mut Parser, module: &mut ModuleDef) -> Option<GenFor> {
    let line = parser.line();
    parser.eat_ident("for");
    if !parser.eat_punct('(') {
        return None;
    }
    let var = parser.take_ident()?.0;
    if !parser.eat_punct('=') {
        return None;
    }
    let init = parse_expr(parser);
    if !parser.eat_punct(';') {
        return None;
    }
    let cond = parse_expr(parser);
    if !parser.eat_punct(';') {
        return None;
    }
    let step = parse_for_step(parser, &var);
    parser.eat_punct(')');
    let block = parse_gen_block(parser, module);
    Some(GenFor {
        label: block.label,
        var,
        init,
        cond,
        step,
        items: block.items,
        line,
    })
}

/// `i++` / `i--` / `i += n` / `i = expr`; yields the next value of `var`.
fn parse_for_step(parser: &mut Parser, var: &str) -> Expr {
    let current = Expr::Name(var.to_string());
    if parser.at_ident(var) {
        parser.next();
        match (parser.peek(), parser.peek_at(1)) {
            (Some(Tok::Punct('+')), Some(Tok::Punct('+'))) => {
                parser.next();
                parser.next();
                return Expr::Binary(Box::new(current), BinOp::Add, Box::new(Expr::Num(1)));
            }
            (Some(Tok::Punct('-')), Some(Tok::Punct('-'))) => {
                parser.next();
                parser.next();
                return Expr::Binary(Box::new(current), BinOp::Sub, Box::new(Expr::Num(1)));
            }
            (Some(Tok::Punct('+')), Some(Tok::Punct('='))) => {
                parser.next();
                parser.next();
                let delta = parse_expr(parser);
                return Expr::Binary(Box::new(current), BinOp::Add, Box::new(delta));
            }
            (Some(Tok::Punct('-')), Some(Tok::Punct('='))) => {
                parser.next();
                parser.next();
                let delta = parse_expr(parser);
                return Expr::Binary(Box::new(current), BinOp::Sub, Box::new(delta));
            }
            (Some(Tok::Punct('=')), _) => {
                parser.next();
                return parse_expr(parser);
            }
            _ => {}
        }
    }
    parser.skip_expr();
    current
}

/// `begin : label ... end`, a single item, or a nested generate statement.
fn parse_gen_block(parser: &mut Parser, module: &mut ModuleDef) -> GenBlock {
    let line = parser.line();
    if parser.eat_ident("begin") {
        let label = if parser.eat_punct(':') {
            parser.take_ident().map(|(name, _)| name)
        } else {
            None
        };
        let items = parse_body(parser, module);
        parser.eat_ident("end");
        GenBlock { label, items, line }
    } else {
        let items = parse_body_item(parser, module).into_iter().collect();
        GenBlock {
            label: None,
            items,
            line,
        }
    }
}

/// `if (cond) ... else if (cond) ... else ...`
fn parse_gen_if(parser: &mut Parser, module: &mut ModuleDef) -> Option<GenIf> {
    let line = parser.line();
    parser.eat_ident("if");
    let cond = parse_paren_expr(parser);
    let mut branches = vec![(cond, parse_gen_block(parser, module))];
    while parser.eat_ident("else") {
        if parser.eat_ident("if") {
            let cond = parse_paren_expr(parser);
            branches.push((cond, parse_gen_block(parser, module)));
        } else {
            branches.push((None, parse_gen_block(parser, module)));
            break;
        }
    }
    Some(GenIf { branches, line })
}

fn parse_paren_expr(parser: &mut Parser) -> Option<Expr> {
    if !parser.eat_punct('(') {
        return None;
    }
    let expr = parse_expr(parser);
    parser.eat_punct(')');
    Some(expr)
}

/// Parse `parameter`/`localparam` declarations into the module parameter map.
fn parse_param_decls(parser: &mut Parser, module: &mut ModuleDef) {
    loop {
        if matches!(parser.peek(), Some(Tok::Punct(')')) | None) {
            return;
        }
        // Skip the keyword, type and qualifiers (but keep the range).
        loop {
            match parser.peek() {
                Some(Tok::Ident(word)) => match word.as_str() {
                    "parameter" | "localparam" | "int" | "integer" | "logic" | "bit" | "reg"
                    | "signed" | "unsigned" | "real" | "time" | "shortint" | "longint" | "byte"
                    | "var" | "const" | "static" | "automatic" => {
                        parser.next();
                    }
                    _ => break,
                },
                Some(Tok::Punct('[')) => parser.skip_balanced(),
                _ => break,
            }
        }
        let Some((name, _)) = parser.take_ident() else {
            parser.skip_statement();
            return;
        };
        let mut value = None;
        if parser.eat_punct('=') {
            let expr = parse_expr(parser);
            value = eval(&expr, &module_env(module));
        }
        module.params.insert(name, value);
        match parser.peek() {
            Some(Tok::Punct(',')) => {
                parser.next();
            }
            Some(Tok::Punct(';')) => {
                parser.next();
                return;
            }
            Some(Tok::Punct(')')) | None => return,
            _ => {
                parser.skip_expr();
                if parser.eat_punct(';') {
                    return;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Constant expressions (parameters, generate bounds)
// ---------------------------------------------------------------------------

/// Known parameter/genvar values during elaboration.
pub(crate) type ParamEnv = BTreeMap<String, i64>;

fn module_env(module: &ModuleDef) -> ParamEnv {
    module
        .params
        .iter()
        .filter_map(|(name, value)| value.map(|value| (name.clone(), value)))
        .collect()
}

fn parse_expr(parser: &mut Parser) -> Expr {
    let cond = parse_binary(parser, 0);
    if parser.eat_punct('?') {
        let yes = parse_expr(parser);
        parser.eat_punct(':');
        let no = parse_expr(parser);
        return Expr::Ternary(Box::new(cond), Box::new(yes), Box::new(no));
    }
    cond
}

fn parse_binary(parser: &mut Parser, min_precedence: u8) -> Expr {
    let mut lhs = parse_unary(parser);
    while let Some((op, precedence, width)) = peek_binop(parser) {
        if precedence < min_precedence {
            break;
        }
        for _ in 0..width {
            parser.next();
        }
        let rhs = parse_binary(parser, precedence + 1);
        lhs = Expr::Binary(Box::new(lhs), op, Box::new(rhs));
    }
    lhs
}

/// `(operator, precedence, token width)` of the current binary operator.
fn peek_binop(parser: &Parser) -> Option<(BinOp, u8, u8)> {
    let Some(Tok::Punct(c)) = parser.peek() else {
        return None;
    };
    let next = parser.peek_at(1);
    let double = |ch: char| matches!(next, Some(Tok::Punct(other)) if *other == ch);
    Some(match c {
        '|' if double('|') => (BinOp::LOr, 1, 2),
        '&' if double('&') => (BinOp::LAnd, 2, 2),
        '=' if double('=') => (BinOp::Eq, 3, 2),
        '!' if double('=') => (BinOp::Ne, 3, 2),
        '<' if double('=') => (BinOp::Le, 4, 2),
        '>' if double('=') => (BinOp::Ge, 4, 2),
        '<' if double('<') => (BinOp::Shl, 7, 2),
        '>' if double('>') => (BinOp::Shr, 7, 2),
        '<' => (BinOp::Lt, 4, 1),
        '>' => (BinOp::Gt, 4, 1),
        '+' => (BinOp::Add, 5, 1),
        '-' => (BinOp::Sub, 5, 1),
        '*' => (BinOp::Mul, 6, 1),
        '/' => (BinOp::Div, 6, 1),
        '%' => (BinOp::Mod, 6, 1),
        '&' => (BinOp::And, 8, 1),
        '|' => (BinOp::Or, 8, 1),
        '^' => (BinOp::Xor, 8, 1),
        _ => return None,
    })
}

fn parse_unary(parser: &mut Parser) -> Expr {
    if let Some(Tok::Punct(op @ ('!' | '-' | '+' | '~'))) = parser.peek() {
        let op = *op;
        parser.next();
        return Expr::Unary(op, Box::new(parse_unary(parser)));
    }
    parse_primary(parser)
}

fn parse_primary(parser: &mut Parser) -> Expr {
    match parser.peek().cloned() {
        Some(Tok::Num(text)) => {
            parser.next();
            Expr::Num(parse_int(&text))
        }
        Some(Tok::Ident(name)) => {
            parser.next();
            Expr::Name(name)
        }
        Some(Tok::Punct('(')) => {
            parser.next();
            let expr = parse_expr(parser);
            parser.eat_punct(')');
            expr
        }
        _ => {
            parser.next();
            Expr::Num(0)
        }
    }
}

fn parse_int(text: &str) -> i64 {
    let text = text.replace('_', "");
    if let Some((_, rest)) = text.split_once('\'') {
        let (base, digits) = rest.split_at(rest.len().min(1));
        let radix = match base {
            "b" | "B" => 2,
            "o" | "O" => 8,
            "h" | "H" => 16,
            _ => 10,
        };
        return i64::from_str_radix(digits, radix).unwrap_or(0);
    }
    text.parse().unwrap_or(0)
}

/// Evaluate a constant expression; `None` when any name is unknown.
pub(crate) fn eval(expr: &Expr, env: &ParamEnv) -> Option<i64> {
    Some(match expr {
        Expr::Num(value) => *value,
        Expr::Name(name) => return env.get(name).copied(),
        Expr::Unary(op, inner) => {
            let value = eval(inner, env)?;
            match op {
                '!' => i64::from(value == 0),
                '-' => -value,
                '+' => value,
                '~' => !value,
                _ => return None,
            }
        }
        Expr::Binary(a, op, b) => match op {
            BinOp::LAnd => i64::from(eval(a, env)? != 0 && eval(b, env)? != 0),
            BinOp::LOr => i64::from(eval(a, env)? != 0 || eval(b, env)? != 0),
            BinOp::And => eval(a, env)? & eval(b, env)?,
            BinOp::Or => eval(a, env)? | eval(b, env)?,
            BinOp::Xor => eval(a, env)? ^ eval(b, env)?,
            _ => {
                let x = eval(a, env)?;
                let y = eval(b, env)?;
                match op {
                    BinOp::Add => x.checked_add(y)?,
                    BinOp::Sub => x.checked_sub(y)?,
                    BinOp::Mul => x.checked_mul(y)?,
                    BinOp::Div => {
                        if y == 0 {
                            0
                        } else {
                            x.checked_div(y)?
                        }
                    }
                    BinOp::Mod => {
                        if y == 0 {
                            0
                        } else {
                            x.checked_rem(y)?
                        }
                    }
                    BinOp::Shl | BinOp::Shr if !(0..64).contains(&y) => return None,
                    BinOp::Shl => x.checked_shl(y as u32)?,
                    BinOp::Shr => x.checked_shr(y as u32)?,
                    BinOp::Eq => i64::from(x == y),
                    BinOp::Ne => i64::from(x != y),
                    BinOp::Lt => i64::from(x < y),
                    BinOp::Le => i64::from(x <= y),
                    BinOp::Gt => i64::from(x > y),
                    BinOp::Ge => i64::from(x >= y),
                    BinOp::LAnd | BinOp::LOr | BinOp::And | BinOp::Or | BinOp::Xor => {
                        unreachable!()
                    }
                }
            }
        },
        Expr::Ternary(cond, yes, no) => {
            if eval(cond, env)? != 0 {
                eval(yes, env)?
            } else {
                eval(no, env)?
            }
        }
    })
}

impl Parser {
    fn eat_punct(&mut self, c: char) -> bool {
        if matches!(self.peek(), Some(Tok::Punct(p)) if *p == c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Skip one expression: stops before ,, ; or ) at depth 0.
    fn skip_expr(&mut self) {
        let mut depth = 0i32;
        while let Some(tok) = self.peek() {
            match tok {
                Tok::Punct('(' | '[' | '{') => {
                    depth += 1;
                    self.next();
                }
                Tok::Punct(')' | ']' | '}') => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    self.next();
                }
                Tok::Punct(',' | ';') if depth == 0 => return,
                _ => {
                    self.next();
                }
            }
        }
    }
}

/// Parse `input logic [7:0] a, b;` (or a comma separated header chunk).
fn parse_declarations(parser: &mut Parser, module: &mut ModuleDef, header: bool) {
    let mut direction: Option<String> = None;
    let mut kind: Option<String> = None;
    let mut range: Option<String> = None;

    loop {
        // Qualifiers before a name: direction / type / range.
        loop {
            match parser.peek() {
                Some(Tok::Ident(word)) => {
                    let word = word.clone();
                    match word.as_str() {
                        "input" | "output" | "inout" => {
                            direction = Some(word);
                            kind = None;
                            range = None;
                            parser.next();
                        }
                        "wire" | "reg" | "logic" | "bit" | "integer" | "real" | "time"
                        | "supply0" | "supply1" | "tri" | "triand" | "trior" | "wand" | "wor"
                        | "trireg" => {
                            kind.get_or_insert(word);
                            parser.next();
                        }
                        "signed" | "unsigned" | "var" | "const" | "static" | "automatic" => {
                            parser.next();
                        }
                        _ => break,
                    }
                }
                Some(Tok::Punct('[')) => {
                    range = Some(collect_range(parser));
                }
                _ => break,
            }
        }

        let Some((name, line)) = parser.take_ident() else {
            break;
        };
        // Skip unpacked dimensions.
        while matches!(parser.peek(), Some(Tok::Punct('['))) {
            parser.skip_balanced();
        }
        let decl = SignalDecl {
            name,
            kind: kind.clone().unwrap_or_else(|| "wire".to_string()),
            direction: direction.clone(),
            range: range.clone(),
            line,
        };
        if header {
            module.ports.push(decl.clone());
        }
        if !module.signals.iter().any(|s| s.name == decl.name) {
            module.signals.push(decl);
        }

        // Default value / port connection: skip just the expression.
        if matches!(parser.peek(), Some(Tok::Punct('=')) | Some(Tok::Punct('('))) {
            parser.skip_expr();
        }
        match parser.peek() {
            Some(Tok::Punct(',')) => {
                parser.next();
            }
            Some(Tok::Punct(';')) => {
                parser.next();
                break;
            }
            Some(Tok::Punct(')')) | None => break,
            _ => {
                parser.skip_expr();
                if matches!(parser.peek(), Some(Tok::Punct(';'))) {
                    parser.next();
                }
                break;
            }
        }
    }
}
fn collect_range(parser: &mut Parser) -> String {
    parser.next(); // '['
    let mut text = String::new();
    let mut depth = 1i32;
    while let Some(tok) = parser.next() {
        match tok {
            Tok::Punct('[') => {
                depth += 1;
                text.push('[');
            }
            Tok::Punct(']') => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                text.push(']');
            }
            Tok::Punct(c) => text.push(*c),
            Tok::Ident(name) => text.push_str(name),
            Tok::Num(text_num) => text.push_str(text_num),
            Tok::Str => {}
        }
    }
    text
}

fn parse_assign(parser: &mut Parser, module: &mut ModuleDef) {
    let line = parser.line();
    let mut lhs = String::new();
    let mut uses = Vec::new();
    let mut depth = 0i32;
    let mut after_eq = false;
    while let Some(tok) = parser.peek() {
        match tok {
            Tok::Punct('(' | '[' | '{') => {
                depth += 1;
                parser.next();
            }
            Tok::Punct(')' | ']' | '}') => {
                depth -= 1;
                parser.next();
            }
            Tok::Punct(';') if depth <= 0 => {
                parser.next();
                break;
            }
            Tok::Punct('=') if depth <= 0 && !after_eq => {
                after_eq = true;
                parser.next();
            }
            Tok::Ident(name) if depth == 0 && !after_eq && lhs.is_empty() => {
                lhs = name.clone();
                parser.next();
            }
            Tok::Ident(name) => {
                if after_eq || depth > 0 {
                    uses.push((name.clone(), parser.line()));
                }
                parser.next();
            }
            _ => {
                parser.next();
            }
        }
    }
    module.assigns.push(Assign { lhs, line, uses });
}

fn parse_process(parser: &mut Parser, module: &mut ModuleDef) {
    let start = parser.line();
    let mut sensitivity = Vec::new();
    if matches!(parser.peek(), Some(Tok::Punct('@'))) {
        parser.next();
        let paren = matches!(parser.peek(), Some(Tok::Punct('(')));
        if paren {
            parser.next();
        }
        let mut depth = if paren { 1 } else { 0 };
        loop {
            let line = parser.line();
            let Some(tok) = parser.next() else { break };
            match tok {
                Tok::Punct('(') => depth += 1,
                Tok::Punct(')') => {
                    depth -= 1;
                    if paren && depth == 0 {
                        break;
                    }
                }
                Tok::Ident(name) if !is_keyword(name) => {
                    sensitivity.push((name.clone(), line));
                }
                Tok::Punct(';') if !paren => break,
                _ => {}
            }
        }
    }
    let (drivers, uses, end) = scan_statement(parser);
    let mut all_uses = sensitivity.clone();
    all_uses.extend(uses);
    all_uses.retain(|entry| !drivers.contains(entry));
    module.always.push(AlwaysBlock {
        start,
        end,
        sensitivity,
        drivers,
        uses: all_uses,
    });
}

/// Scan one statement or `begin ... end` block, collecting assignments and
/// identifier uses. Returns `(drivers, uses, end_line)`.
fn scan_statement(parser: &mut Parser) -> (Idents, Idents, usize) {
    let mut drivers = Vec::new();
    let mut uses = Vec::new();
    let mut end = parser.line();
    let mut depth = 0i32;
    let mut cond_depth: Option<i32> = None;
    let mut last_cond = false;
    let mut last_punct: Option<char> = None;
    let mut pending: Option<(String, usize)> = None;

    while let Some(tok) = parser.peek().cloned() {
        let line = parser.line();
        match &tok {
            Tok::Ident(name) => {
                last_cond = matches!(
                    name.as_str(),
                    "if" | "while" | "for" | "case" | "casex" | "casez"
                );
                if !is_keyword(name) {
                    uses.push((name.clone(), line));
                    pending = Some((name.clone(), line));
                }
                last_punct = None;
                parser.next();
            }
            Tok::Punct(c) => {
                let c = *c;
                match c {
                    '(' => {
                        depth += 1;
                        if last_cond {
                            cond_depth = Some(depth);
                        }
                    }
                    ')' => {
                        if cond_depth == Some(depth) {
                            cond_depth = None;
                        }
                        depth -= 1;
                    }
                    ';' if depth <= 0 => {
                        end = line;
                        parser.next();
                        break;
                    }
                    '=' if cond_depth.is_none()
                        && last_punct != Some('=')
                        && last_punct != Some('!') =>
                    {
                        if let Some((name, driver_line)) = pending.take() {
                            drivers.push((name, driver_line));
                        }
                    }
                    _ => {}
                }
                last_punct = Some(c);
                last_cond = false;
                parser.next();
            }
            _ => {
                parser.next();
            }
        }
        end = line;
    }
    (drivers, uses, end)
}

fn parse_instance(parser: &mut Parser, module: &ModuleDef) -> Option<Instance> {
    let (module_type, line) = match (parser.peek(), parser.peek_at(1)) {
        (Some(Tok::Ident(name)), _) if !is_keyword(name) => (name.clone(), parser.line()),
        _ => return None,
    };
    // `type #(...) inst (...)` or `type inst (...)`.
    let mut offset = 1;
    let mut has_overrides = false;
    if matches!(parser.peek_at(offset), Some(Tok::Punct('#'))) {
        offset += 1;
        if matches!(parser.peek_at(offset), Some(Tok::Punct('('))) {
            has_overrides = true;
            // Skip the balanced parameter override to find the instance name.
            let mut depth = 0i32;
            let mut i = offset;
            loop {
                match parser.peek_at(i) {
                    Some(Tok::Punct('(')) => depth += 1,
                    Some(Tok::Punct(')')) => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    Some(_) => {}
                    None => return None,
                }
                i += 1;
            }
            offset = i;
        }
    }
    let instance = match parser.peek_at(offset) {
        Some(Tok::Ident(name)) if !is_keyword(name) => name.clone(),
        _ => return None,
    };
    if !matches!(parser.peek_at(offset + 1), Some(Tok::Punct('('))) {
        return None;
    }
    if module.name == module_type {
        return None;
    }
    // Consume the type, parameter overrides, instance name and port list.
    parser.next(); // module type
    let mut overrides = Vec::new();
    if has_overrides {
        parser.next(); // '#'
        if parser.eat_punct('(') {
            parse_param_overrides(parser, &mut overrides);
        }
    }
    parser.take_ident(); // instance name
    parser.skip_balanced(); // port list
    if matches!(parser.peek(), Some(Tok::Punct(';'))) {
        parser.next();
    }
    Some(Instance {
        module: module_type,
        name: instance,
        line,
        overrides,
    })
}

/// `#(.WIDTH(8), .N(2))` parameter overrides.
fn parse_param_overrides(parser: &mut Parser, out: &mut Vec<(String, Expr)>) {
    loop {
        match parser.peek() {
            Some(Tok::Punct(')')) | None => {
                parser.eat_punct(')');
                return;
            }
            _ => {}
        }
        if parser.eat_punct('.') {
            let Some((name, _)) = parser.take_ident() else {
                return;
            };
            if parser.eat_punct('(') {
                let expr = parse_expr(parser);
                parser.eat_punct(')');
                out.push((name, expr));
            }
        } else {
            // Positional override: parse and drop.
            parse_expr(parser);
        }
        match parser.peek() {
            Some(Tok::Punct(',')) => {
                parser.next();
            }
            Some(Tok::Punct(')')) => {
                parser.next();
                return;
            }
            _ => return,
        }
    }
}

/// (identifier, line) pairs collected while scanning a process.
type Idents = Vec<(String, usize)>;

pub fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "begin"
            | "end"
            | "if"
            | "else"
            | "case"
            | "casex"
            | "casez"
            | "endcase"
            | "default"
            | "for"
            | "while"
            | "do"
            | "repeat"
            | "forever"
            | "fork"
            | "join"
            | "join_any"
            | "join_none"
            | "posedge"
            | "negedge"
            | "edge"
            | "or"
            | "and"
            | "not"
            | "xor"
            | "nand"
            | "nor"
            | "xnor"
            | "buf"
            | "signed"
            | "unsigned"
            | "logic"
            | "wire"
            | "reg"
            | "bit"
            | "integer"
            | "real"
            | "time"
            | "parameter"
            | "localparam"
            | "generate"
            | "endgenerate"
            | "genvar"
            | "always"
            | "always_ff"
            | "always_comb"
            | "always_latch"
            | "initial"
            | "final"
            | "assign"
            | "module"
            | "endmodule"
            | "input"
            | "output"
            | "inout"
            | "unique"
            | "priority"
            | "return"
            | "break"
            | "continue"
            | "typedef"
            | "struct"
            | "packed"
            | "enum"
            | "union"
            | "automatic"
            | "static"
            | "const"
            | "var"
            | "void"
            | "function"
            | "endfunction"
            | "task"
            | "endtask"
            | "disable"
            | "wait"
            | "assert"
            | "assume"
            | "cover"
            | "property"
            | "endproperty"
            | "sequence"
            | "endsequence"
            | "interface"
            | "endinterface"
            | "package"
            | "endpackage"
            | "import"
            | "export"
            | "defparam"
            | "specify"
            | "endspecify"
            | "table"
            | "endtable"
            | "primitive"
            | "endprimitive"
            | "macromodule"
            | "supply0"
            | "supply1"
            | "tri"
            | "triand"
            | "trior"
            | "wand"
            | "wor"
            | "trireg"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const COUNTER: &str = r#"
// A small counter used by the demo.
module counter #(parameter int WIDTH = 8) (
    input  logic clk,
    input  logic rst_n,
    input  logic en,
    output logic [WIDTH-1:0] count,
    output logic carry
);
    logic [WIDTH-1:0] next;

    assign carry = en && (count == {WIDTH{1'b1}});
    assign next = count + 1'b1;

    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) count <= '0;
        else if (en) count <= next;
    end

    /* a nested module mentioned in a comment must be ignored:
       module not_real; endmodule */
endmodule
"#;

    fn parse_counter() -> ModuleDef {
        parse_module_text(COUNTER, Path::new("counter.sv"))
            .into_iter()
            .next()
            .expect("module")
    }

    #[test]
    fn parses_module_header_and_declarations() {
        let m = parse_counter();
        assert_eq!(m.name, "counter");
        assert_eq!(m.start, 3);
        assert_eq!(m.ports.len(), 5);
        let count = m.signal("count").unwrap();
        assert_eq!(count.direction.as_deref(), Some("output"));
        assert_eq!(count.kind, "logic");
        assert_eq!(count.range.as_deref(), Some("WIDTH-1:0"));
        assert_eq!(
            m.signal("next").unwrap().range.as_deref(),
            Some("WIDTH-1:0")
        );
    }

    #[test]
    fn parses_assignments_and_always_blocks() {
        let m = parse_counter();
        assert_eq!(m.assigns.len(), 2);
        assert_eq!(m.assigns[0].lhs, "carry");
        assert!(m.assigns[0].uses.iter().any(|(name, _)| name == "count"));
        assert_eq!(m.always.len(), 1);
        let block = &m.always[0];
        assert!(block.sensitivity.iter().any(|(name, _)| name == "clk"));
        assert!(block.sensitivity.iter().any(|(name, _)| name == "rst_n"));
        assert!(block.drivers.iter().any(|(name, _)| name == "count"));
        // The `count == ...` comparison inside assign must not look like a load.
        assert_eq!(m.instances.len(), 0);
    }

    #[test]
    fn parses_instantiations() {
        let text = r#"
module top;
    logic clk, rst_n, en, carry;
    wire [7:0] count;
    counter #(.WIDTH(8)) u_dut (
        .clk(clk),
        .rst_n(rst_n),
        .en(en),
        .count(count),
        .carry(carry)
    );
    assign en = 1'b1;
endmodule
"#;
        let m = parse_module_text(text, Path::new("top.sv"))
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(m.instances.len(), 1);
        assert_eq!(m.instances[0].module, "counter");
        assert_eq!(m.instances[0].name, "u_dut");
        assert_eq!(m.instances[0].line, 5);
    }

    #[test]
    fn traces_signals_to_declaration_and_drivers() {
        let dir = std::env::temp_dir().join(format!("waverdi_scan_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("counter.sv");
        fs::write(&file, COUNTER).unwrap();
        let set = SourceSet::from_files(vec![file.clone()], "test");
        let db = RtlDb::parse_sources(&set);
        let trace = db.trace("counter", "count").expect("trace");
        assert_eq!(trace.decl.as_ref().unwrap().line, 7);
        assert!(trace.drivers.iter().any(|loc| loc.line == 16));
        assert!(trace.loads.iter().any(|loc| loc.line == 12));
        assert!(trace.loads.iter().any(|loc| loc.line == 13));
        assert!(db.module("counter").is_some());
    }

    #[test]
    fn elaborates_the_design_hierarchy_and_resolves_references() {
        let text = r#"
module counter(input logic clk, output logic [3:0] count);
    logic [3:0] next;
    assign next = count;
endmodule

module tb;
    logic clk;
    counter u_dut(.clk(clk));
    assign x = u_dut.count;
endmodule
"#;
        let dir = std::env::temp_dir().join(format!("waverdi_elab_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tb.sv");
        fs::write(&file, text).unwrap();
        let set = SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);
        assert_eq!(db.top_modules(), vec!["tb".to_string()]);
        let steps = |names: &[&str]| {
            names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(db.module_at_scope(&steps(&["tb"])).unwrap().name, "tb");
        assert_eq!(
            db.module_at_scope(&steps(&["tb", "u_dut"])).unwrap().name,
            "counter"
        );
        assert!(db.module_at_scope(&steps(&["nobody"])).is_none());
        // Plain references only resolve where the AST declares them.
        assert!(db.resolve_reference("tb", &[], "count").is_none());
        assert!(db.resolve_reference("counter", &[], "count").is_some());
        // Qualified references walk the instance hierarchy.
        let (path, signal) = db
            .resolve_reference("tb", &steps(&["u_dut"]), "count")
            .expect("qualified");
        assert_eq!(path, vec!["u_dut".to_string()]);
        assert_eq!(signal, "count");
        assert!(db.resolve_reference("tb", &[], "u_dut").is_none());
        // Elaborated placements cover the whole design, with top first.
        let placements = db.placements();
        assert!(placements.contains(&(steps(&["tb"]), "tb".to_string())));
        assert!(placements.contains(&(steps(&["tb", "u_dut"]), "counter".to_string())));
    }

    #[test]
    fn elaborates_generate_loops_and_branches() {
        let text = r#"
module leaf(input logic clk, output logic [3:0] q);
    logic [3:0] next;
    assign next = q;
endmodule

module mid #(parameter int N = 3, parameter bit FAST = 1'b1) (
    input logic clk
);
    genvar i;
    generate
        for (i = 0; i < N; i++) begin : gen_leaves
            if ((i % 2) == 0) begin : gen_even
                leaf u_even(.clk(clk), .q());
            end else begin : gen_odd
                leaf u_odd(.clk(clk), .q());
            end
        end
    endgenerate
    generate
        if (FAST) begin : gen_fast
            leaf u_fast(.clk(clk), .q());
        end else begin : gen_slow
            leaf u_slow(.clk(clk), .q());
        end
    endgenerate
endmodule

module top;
    logic clk;
    mid #(.N(4), .FAST(1'b0)) u_mid(.clk(clk));
    raw u_raw_top(.clk(clk));
endmodule

module raw(input logic clk);
    genvar r;
    generate
        for (r = 0; r < 2; r++) begin
            leaf u_raw(.clk(clk), .q());
        end
    endgenerate
endmodule
"#;
        let dir = std::env::temp_dir().join(format!("waverdi_gen_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gen.sv");
        fs::write(&file, text).unwrap();
        let set = SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);
        assert_eq!(db.top_modules(), vec!["top".to_string()]);

        let steps = |names: &[&str]| {
            names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        };
        // Parameters: default N=3, overridden to 4 through the instance.
        assert_eq!(db.module("mid").unwrap().params["N"], Some(3));
        // The loop is unrolled with the override; the else-branch is taken.
        let placements = db.placements();
        assert!(placements.contains(&(
            steps(&["top", "u_mid", "gen_leaves[0]", "gen_even", "u_even"]),
            "leaf".to_string()
        )));
        assert!(placements.contains(&(
            steps(&["top", "u_mid", "gen_leaves[3]", "gen_odd", "u_odd"]),
            "leaf".to_string()
        )));
        assert!(!placements
            .iter()
            .any(|(path, _)| path.contains(&"gen_leaves[4]".to_string())));
        assert!(placements.contains(&(
            steps(&["top", "u_mid", "gen_slow", "u_slow"]),
            "leaf".to_string()
        )));
        assert!(!placements
            .iter()
            .any(|(path, _)| path.contains(&"gen_fast".to_string())));
        // Unnamed generate loops use the tool-assigned genblk numbering.
        assert!(placements.contains(&(
            steps(&["top", "u_raw_top", "genblk1[0]", "u_raw"]),
            "leaf".to_string()
        )));
        assert!(placements.contains(&(
            steps(&["top", "u_raw_top", "genblk1[1]", "u_raw"]),
            "leaf".to_string()
        )));
        assert_eq!(placements.len(), 10);

        // Dump scope paths walk through loops and branches.
        assert_eq!(
            db.module_at_scope(&steps(&[
                "top",
                "u_mid",
                "gen_leaves[2]",
                "gen_even",
                "u_even"
            ]))
            .unwrap()
            .name,
            "leaf"
        );
        assert_eq!(
            db.module_at_scope(&steps(&["top", "u_mid", "gen_slow", "u_slow"]))
                .unwrap()
                .name,
            "leaf"
        );
        assert!(db
            .module_at_scope(&steps(&["top", "u_mid", "gen_fast", "u_fast"]))
            .is_none());
        assert_eq!(
            db.module_at_scope(&steps(&["top", "u_raw_top", "genblk1[1]", "u_raw"]))
                .unwrap()
                .name,
            "leaf"
        );
        // Unknown helper scopes are skipped.
        assert_eq!(
            db.module_at_scope(&steps(&[
                "top",
                "u_mid",
                "unnamed$$_0",
                "gen_leaves[1]",
                "gen_odd"
            ]))
            .unwrap()
            .name,
            "mid"
        );

        // References can walk into generated instances.
        let (path, signal) = db
            .resolve_reference(
                "top",
                &steps(&["u_mid", "gen_leaves[1]", "gen_odd", "u_odd"]),
                "q",
            )
            .expect("generated reference");
        assert_eq!(path, steps(&["u_mid", "gen_leaves[1]", "gen_odd", "u_odd"]));
        assert_eq!(signal, "q");
        // Plain instance names inside a loop resolve to the first iteration.
        let (path, _) = db
            .resolve_reference("mid", &steps(&["u_even"]), "q")
            .expect("loop instance");
        assert_eq!(path, steps(&["gen_leaves[0]", "gen_even", "u_even"]));
        // Unnamed loops use the genblk naming in returned paths too.
        let (path, _) = db
            .resolve_reference("raw", &steps(&["u_raw"]), "q")
            .expect("unnamed loop instance");
        assert_eq!(path, steps(&["genblk1[0]", "u_raw"]));
    }

    #[test]
    fn ignores_strings_and_directives() {
        let text = r#"
`timescale 1ns/1ps
module small;
    initial $display("module fake; endmodule");
endmodule
"#;
        let modules = parse_module_text(text, Path::new("small.sv"));
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "small");
    }
}
