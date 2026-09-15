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
use crate::waveform::Waveform;

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
    #[cfg_attr(not(test), allow(dead_code))]
    pub kind: String,
    #[cfg_attr(not(test), allow(dead_code))]
    pub direction: Option<String>,
    /// Raw range text as written, e.g. `7:0` (kept for bus slicing).
    #[cfg_attr(not(test), allow(dead_code))]
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
    #[cfg_attr(not(test), allow(dead_code))]
    pub module: String,
    #[cfg_attr(not(test), allow(dead_code))]
    pub name: String,
    #[cfg_attr(not(test), allow(dead_code))]
    pub line: usize,
    /// `#(.NAME(expr))` parameter overrides, in source order.
    pub overrides: Vec<(String, Expr)>,
    /// Named port connections (`.port(...)`): port name and source line.
    pub ports: Vec<(String, usize)>,
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
    #[cfg_attr(not(test), allow(dead_code))]
    pub line: usize,
}

/// One `begin : label ... end` generate block (also the body of a for loop).
#[derive(Clone, Debug)]
pub struct GenBlock {
    pub label: Option<String>,
    pub items: Vec<Body>,
    pub line: usize,
    /// Line just after the block's last token; an inactive block dims
    /// `line..end` in the Source pane.
    pub end: usize,
}

/// `generate if` / `else if` / `else`, one entry per branch.
#[derive(Clone, Debug)]
pub struct GenIf {
    /// `(condition, block)`; the condition is `None` for the final `else`.
    pub branches: Vec<(Option<Expr>, GenBlock)>,
    /// Line of the `if` keyword; an inactive first branch dims from here.
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

/// Bounded search state for the generate fallbacks of `walk_chain` and
/// `walk_scope`. Without it, nested generate loops are re-walked for every
/// iteration and every unresolved identifier, which is exponential.
struct GenSearch {
    visited: HashSet<usize>,
    budget: usize,
}

impl GenSearch {
    const BUDGET: usize = 200_000;

    fn new() -> Self {
        Self {
            visited: HashSet::new(),
            budget: Self::BUDGET,
        }
    }

    fn tick(&mut self) -> bool {
        if self.budget == 0 {
            return false;
        }
        self.budget -= 1;
        true
    }

    /// Enter the generate fallback for one body group once per search.
    fn enter_group(&mut self, items: &[Body]) -> bool {
        if !self.tick() {
            return false;
        }
        self.visited.insert(items.as_ptr() as usize)
    }
}

/// Where a dump scope path lands in the elaborated design.
pub(crate) struct ScopeMatch<'a> {
    pub module: &'a ModuleDef,
    /// Parameter/genvar bindings along the path, e.g. `i = 1`.
    pub env: ParamEnv,
    /// Dump scope of the instance that owns the module's signals, without
    /// generate-block segments (`tb.u_proc.genblk1[1]` -> `tb.u_proc`).
    pub instance_scope: Vec<String>,
    /// Source line of the deepest generate block in the path, if the path
    /// does not end at an instance boundary.
    pub line: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DefKind {
    Module,
    Interface,
    Program,
    Package,
}

/// A user-defined type (`typedef`) with its packed width when statically known.
#[derive(Clone, Debug, Default)]
pub struct TypeDef {
    pub width: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct ModuleDef {
    pub name: String,
    pub file: PathBuf,
    pub start: usize,
    pub end: usize,
    /// What kind of definition this is (`module`, `interface`, `package`, ...).
    pub kind: DefKind,
    /// Module parameter/localparam values (evaluated from defaults), if known.
    pub params: BTreeMap<String, Option<i64>>,
    /// Parameter initializers in declaration order, so they can be evaluated
    /// again once package parameters are known.
    pub param_exprs: Vec<(String, Expr)>,
    /// Local `typedef`s, used to recognise declarations of user types.
    pub typedefs: BTreeMap<String, TypeDef>,
    /// Packages named by `import pkg::*;` / `import pkg::name;`.
    pub imports: Vec<String>,
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

/// Declaration of `name` in a module (signal or port), matched by base name.
fn module_declaration<'a>(def: &'a ModuleDef, name: &str) -> Option<&'a SignalDecl> {
    def.signals
        .iter()
        .chain(def.ports.iter())
        .find(|decl| decl.name == name || base_name(&decl.name) == base_name(name))
}

/// Identifier without a packed range: `count[7:0]` -> `count`.
fn base_name(text: &str) -> &str {
    text.split('[').next().unwrap_or(text)
}

/// Whether a dumped signal name matches a source reference. FSDB stores
/// ranges (`count[7:0]`) and unpacked array elements (`data_chain[1][7:0]`),
/// so a trailing packed range is ignored when comparing.
pub(crate) fn signal_name_matches(dumped: &str, wanted: &str) -> bool {
    dumped == wanted || strip_range(dumped) == wanted
}

/// Remove a trailing packed range (`[7:0]`) from a dumped name.
fn strip_range(name: &str) -> &str {
    let Some(body) = name.strip_suffix(']') else {
        return name;
    };
    let Some(open) = body.rfind('[') else {
        return name;
    };
    if body[open + 1..].contains(':') {
        &body[..open]
    } else {
        name
    }
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

/// Line ranges of generate branches whose condition is known false. Loop
/// bodies are walked once with the enclosing bindings: a condition that
/// depends on a genvar stays unknown and is never marked inactive, so the
/// marking cannot hide active code.
fn collect_inactive(items: &[Body], env: &ParamEnv, out: &mut Vec<std::ops::Range<usize>>) {
    for item in items {
        match item {
            Body::If(gen_if) => {
                let taken = taken_branches(gen_if, env);
                for (index, (_, block)) in gen_if.branches.iter().enumerate() {
                    if !taken.iter().any(|taken| std::ptr::eq(*taken, block)) {
                        // The first branch's header is the `if` line itself;
                        // later branches start at their own `begin` line.
                        let start = if index == 0 { gen_if.line } else { block.line };
                        out.push(start..block.end);
                    }
                    collect_inactive(&block.items, env, out);
                }
            }
            Body::For(gen) => collect_inactive(&gen.items, env, out),
            Body::Block(block) => collect_inactive(&block.items, env, out),
            Body::Instance(_) => {}
        }
    }
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

/// Ordinal of an unnamed generate construct for [`block_dump_name`], or
/// `None` when it carries a label. The named/unnamed decision and the
/// ordinal numbering must stay in one place: the counter advances per
/// unnamed construct in source order.
fn unnamed_number(label: Option<&str>, unnamed: &mut usize) -> Option<usize> {
    if label.is_none() {
        Some(unnamed_ordinal(unnamed))
    } else {
        None
    }
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
    /// Evaluated parameters of every `package`, by package name. Package
    /// parameters are visible to modules as `pkg::NAME` (and via `import`).
    pub package_params: BTreeMap<String, BTreeMap<String, i64>>,
}

impl RtlDb {
    /// Parse every `.v`/`.sv` file of a source set.
    pub fn parse_sources(set: &SourceSet) -> RtlDb {
        RtlDb::parse_sources_with_progress(set, &mut |_, _| true)
    }

    /// Parse the source set, reporting file progress. Returning `false` from
    /// the callback stops after the current file.
    pub fn parse_sources_with_progress(
        set: &SourceSet,
        progress: &mut dyn FnMut(usize, usize) -> bool,
    ) -> RtlDb {
        let mut db = RtlDb::default();
        let total = set.files.len();
        for (done, path) in set.files.iter().enumerate() {
            if is_verilog(path) {
                if let Ok(text) = fs::read_to_string(path) {
                    db.files.push(path.clone());
                    for module in parse_module_text(&text, path) {
                        db.modules.entry(module.name.clone()).or_insert(module);
                    }
                }
            }
            if !progress(done + 1, total) {
                break;
            }
        }
        for module in db.modules.values() {
            if module.kind == DefKind::Package {
                let params = module
                    .params
                    .iter()
                    .filter_map(|(name, value)| value.map(|value| (name.clone(), value)))
                    .collect();
                db.package_params.insert(module.name.clone(), params);
            }
        }
        // User-typed declarations may live in a package parsed after the
        // module that uses them: fill in the packed width once all packages
        // are known.
        let mut global_typedefs: BTreeMap<String, TypeDef> = BTreeMap::new();
        for module in db.modules.values() {
            if module.kind == DefKind::Package {
                for (name, def) in &module.typedefs {
                    global_typedefs
                        .entry(format!("{}::{name}", module.name))
                        .or_insert_with(|| def.clone());
                    global_typedefs
                        .entry(name.clone())
                        .or_insert_with(|| def.clone());
                }
            }
        }
        for module in db.modules.values_mut() {
            if module.kind == DefKind::Package {
                continue;
            }
            for signal in &mut module.signals {
                if signal.range.is_none() {
                    if let Some(width) = module
                        .typedefs
                        .get(&signal.kind)
                        .and_then(|def| def.width)
                        .or_else(|| global_typedefs.get(&signal.kind).and_then(|def| def.width))
                    {
                        signal.range = Some(width_range(width));
                    }
                }
            }
        }
        db
    }

    pub fn module(&self, name: &str) -> Option<&ModuleDef> {
        self.modules.get(name)
    }

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
        let mut env = ParamEnv::new();
        // `pkg::NAME` is visible everywhere; `import pkg::*` also binds the
        // plain name (module-local parameters win).
        for (package, params) in &self.package_params {
            for (name, value) in params {
                env.insert(format!("{package}::{name}"), *value);
                if def.imports.iter().any(|import| import == package) {
                    env.entry(name.clone()).or_insert(*value);
                }
            }
        }
        for (name, expr) in &def.param_exprs {
            if let Some(value) = eval(expr, &env) {
                env.insert(name.clone(), value);
            }
        }
        for (name, value) in &def.params {
            if let Some(value) = value {
                env.entry(name.clone()).or_insert(*value);
            }
        }
        env
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
        self.scope_info(steps).map(|found| found.module)
    }

    /// Like [`module_at_scope`](Self::module_at_scope) but also resolves
    /// scopes that are values rather than hierarchy: a struct variable or an
    /// interface port (`tb.u_dut.clk_fetch`) reports the module that declares
    /// `clk_fetch`, so its source can be shown.
    pub fn module_of_scope(&self, steps: &[String]) -> Option<&ModuleDef> {
        if let Some(def) = self.module_at_scope(steps) {
            return Some(def);
        }
        for k in (1..steps.len()).rev() {
            if let Some(def) = self.module_at_scope(&steps[..k]) {
                if module_declaration(def, &steps[k]).is_some() {
                    return Some(def);
                }
            }
        }
        None
    }

    /// Line of the declaration that a value scope names, e.g. the `clk_fetch`
    /// struct variable in the module reached by the path prefix. `None` for
    /// plain hierarchy paths and unknown names.
    pub(crate) fn scope_declaration_line(&self, steps: &[String]) -> Option<usize> {
        if self.module_at_scope(steps).is_some() {
            return None;
        }
        for k in (1..steps.len()).rev() {
            if let Some(def) = self.module_at_scope(&steps[..k]) {
                if let Some(decl) = module_declaration(def, &steps[k]) {
                    return Some(decl.line);
                }
            }
        }
        None
    }

    /// Line ranges (start inclusive, end exclusive) of generate branches that
    /// are known not to be instantiated, e.g. `generate if (0) ...`; the
    /// Source pane dims them.
    pub fn inactive_lines(&self, module: &str) -> Vec<std::ops::Range<usize>> {
        let Some(def) = self.modules.get(module) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        collect_inactive(&def.body, &self.default_env(def), &mut out);
        out.sort_by_key(|range| range.start);
        out.dedup();
        out
    }

    /// Like [`module_at_scope`](Self::module_at_scope) but also reports the
    /// parameter/genvar bindings along the path and the dump scope of the
    /// instance that owns the module's signals (trailing generate-block
    /// segments removed), e.g. `tb.u_proc.genblk1[1]` -> `tb.u_proc` with
    /// `i = 1`.
    pub(crate) fn scope_info(&self, steps: &[String]) -> Option<ScopeMatch<'_>> {
        if steps.is_empty() {
            return None;
        }
        // One budget per lookup: callers like `merge_existing_scopes` run a
        // lookup per hierarchy node, so the budget must restart here instead
        // of accumulating across nodes.
        let mut search = GenSearch::new();
        for top in self.top_modules() {
            let Some(def) = self.modules.get(&top) else {
                continue;
            };
            let Some(start) = steps.iter().position(|step| step == &top) else {
                continue;
            };
            let env = self.default_env(def);
            let base = vec![top.clone()];
            if let Some(found) = self.walk_scope(
                def,
                &env,
                &def.body,
                &base,
                &base,
                &steps[start + 1..],
                &mut search,
            ) {
                return Some(found);
            }
        }
        None
    }

    /// Walk a dump scope path through a module body (instances, generate
    /// loops, named and unnamed blocks) and return the module that owns it.
    ///
    /// The search is budgeted like `walk_chain`: a `for` block whose dump
    /// name carries no index recurses into every value and every nesting
    /// level, so a path whose tail does not match would otherwise explore
    /// `4096^depth` combinations - once for every hierarchy node, because
    /// `merge_existing_scopes` calls this for each node. When the budget runs
    /// out the path is reported as unresolved.
    #[allow(clippy::too_many_arguments)]
    fn walk_scope<'a>(
        &'a self,
        def: &'a ModuleDef,
        env: &ParamEnv,
        items: &[Body],
        path: &[String],
        instance_scope: &[String],
        steps: &[String],
        search: &mut GenSearch,
    ) -> Option<ScopeMatch<'a>> {
        if !search.tick() {
            return None;
        }
        if steps.is_empty() {
            return Some(ScopeMatch {
                module: def,
                env: env.clone(),
                instance_scope: instance_scope.to_vec(),
                line: None,
            });
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
                    let mut path = path.to_vec();
                    path.push(instance.name.clone());
                    if let Some(found) =
                        self.walk_scope(child, &env, &child.body, &path, &path, rest, search)
                    {
                        return Some(found);
                    }
                }
                Body::For(gen) => {
                    let number = unnamed_number(gen.label.as_deref(), &mut unnamed);
                    let Some(index) = block_step_matches(step, gen.label.as_deref(), number) else {
                        continue;
                    };
                    for value in gen_values(gen, env) {
                        if !search.tick() {
                            return None;
                        }
                        if index.is_some_and(|wanted| wanted != value) {
                            continue;
                        }
                        let mut env = env.clone();
                        env.insert(gen.var.clone(), value);
                        let mut path = path.to_vec();
                        path.push(block_dump_name(gen.label.as_deref(), number, Some(value)));
                        if let Some(mut found) = self.walk_scope(
                            def,
                            &env,
                            &gen.items,
                            &path,
                            instance_scope,
                            rest,
                            search,
                        ) {
                            found.line = found.line.or(Some(gen.line));
                            return Some(found);
                        }
                    }
                }
                Body::If(gen_if) => {
                    for block in taken_branches(gen_if, env) {
                        let number = unnamed_number(block.label.as_deref(), &mut unnamed);
                        let Some(index) = block_step_matches(step, block.label.as_deref(), number)
                        else {
                            continue;
                        };
                        if index.is_some() {
                            continue;
                        }
                        let mut path = path.to_vec();
                        path.push(block_dump_name(block.label.as_deref(), number, None));
                        if let Some(mut found) = self.walk_scope(
                            def,
                            env,
                            &block.items,
                            &path,
                            instance_scope,
                            rest,
                            search,
                        ) {
                            found.line = found.line.or(Some(block.line));
                            return Some(found);
                        }
                    }
                }
                Body::Block(block) => {
                    let number = unnamed_number(block.label.as_deref(), &mut unnamed);
                    let Some(index) = block_step_matches(step, block.label.as_deref(), number)
                    else {
                        continue;
                    };
                    if index.is_some() {
                        continue;
                    }
                    let mut path = path.to_vec();
                    path.push(block_dump_name(block.label.as_deref(), number, None));
                    if let Some(mut found) =
                        self.walk_scope(def, env, &block.items, &path, instance_scope, rest, search)
                    {
                        found.line = found.line.or(Some(block.line));
                        return Some(found);
                    }
                }
            }
        }
        // Unnamed generate blocks and tool-generated helper scopes carry no
        // meaning for the source mapping: skip them.
        if generated_scope_name(step) {
            let mut path = path.to_vec();
            path.push(step.clone());
            return self.walk_scope(def, env, items, &path, instance_scope, rest, search);
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
        self.resolve_reference_with(module, chain, name, &ParamEnv::new())
    }

    /// Like [`resolve_reference`](Self::resolve_reference), with extra
    /// bindings (genvars of the selected generated scope) so that plain
    /// instance names inside the matching loop iteration resolve correctly.
    pub fn resolve_reference_with(
        &self,
        module: &str,
        chain: &[String],
        name: &str,
        bindings: &ParamEnv,
    ) -> Option<(Vec<String>, String)> {
        let def = self.module(module)?;
        let mut env = self.default_env(def);
        for (key, value) in bindings {
            env.insert(key.clone(), *value);
        }
        // Whole instance (`dprx_if`, `u_dut`): resolved to the aggregate of
        // its dump scope by the caller.
        if chain.is_empty() && def.instances.iter().any(|inst| inst.name == name) {
            return Some((Vec::new(), name.to_string()));
        }
        // Struct/interface variable member (`clk_fetch.run`): the dump stores
        // struct members in a scope of the variable's name, so the chain walks
        // scopes here instead of instances.
        if let Some((head, rest)) = chain.split_first() {
            if def.declares(head) && !def.instances.iter().any(|inst| inst.name == *head) {
                let mut path = vec![head.clone()];
                path.extend(rest.iter().cloned());
                return Some((path, name.to_string()));
            }
        }
        let mut search = GenSearch::new();
        let (path, target) = self.walk_chain(def, &env, &def.body, chain, &mut search)?;
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
        search: &mut GenSearch,
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
                    let (rest, target) =
                        self.walk_chain(child, &env, &child.body, &chain[1..], search)?;
                    let mut path = vec![instance.name.clone()];
                    path.extend(rest);
                    return Some((path, target));
                }
                Body::For(gen) => {
                    let number = unnamed_number(gen.label.as_deref(), &mut unnamed);
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
                            self.walk_chain(def, &env, &gen.items, &chain[1..], search)
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
                        let number = unnamed_number(block.label.as_deref(), &mut unnamed);
                        let Some(index) = block_step_matches(step, block.label.as_deref(), number)
                        else {
                            continue;
                        };
                        if index.is_some() {
                            continue;
                        }
                        if let Some((rest, target)) =
                            self.walk_chain(def, env, &block.items, &chain[1..], search)
                        {
                            let mut path =
                                vec![block_dump_name(block.label.as_deref(), number, None)];
                            path.extend(rest);
                            return Some((path, target));
                        }
                    }
                }
                Body::Block(block) => {
                    let number = unnamed_number(block.label.as_deref(), &mut unnamed);
                    let Some(index) = block_step_matches(step, block.label.as_deref(), number)
                    else {
                        continue;
                    };
                    if index.is_some() {
                        continue;
                    }
                    if let Some((rest, target)) =
                        self.walk_chain(def, env, &block.items, &chain[1..], search)
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
        // Each body group is visited once and the search is bounded, otherwise
        // nested generate loops are re-walked exponentially for identifiers
        // that do not resolve (comments, ports of other modules, ...).
        if !search.enter_group(items) {
            return None;
        }
        let mut unnamed = 0usize;
        for item in items {
            match item {
                Body::For(gen) => {
                    let number = unnamed_number(gen.label.as_deref(), &mut unnamed);
                    // A genvar bound by the selected scope pins the iteration.
                    let bound = env.get(&gen.var).copied();
                    for value in gen_values(gen, env) {
                        if bound.is_some_and(|wanted| wanted != value) {
                            continue;
                        }
                        if !search.tick() {
                            return None;
                        }
                        let mut env = env.clone();
                        env.insert(gen.var.clone(), value);
                        if let Some((rest, target)) =
                            self.walk_chain(def, &env, &gen.items, chain, search)
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
                        let number = unnamed_number(block.label.as_deref(), &mut unnamed);
                        if !search.tick() {
                            return None;
                        }
                        if let Some((rest, target)) =
                            self.walk_chain(def, env, &block.items, chain, search)
                        {
                            let mut path =
                                vec![block_dump_name(block.label.as_deref(), number, None)];
                            path.extend(rest);
                            return Some((path, target));
                        }
                    }
                }
                Body::Block(block) => {
                    let number = unnamed_number(block.label.as_deref(), &mut unnamed);
                    if !search.tick() {
                        return None;
                    }
                    if let Some((rest, target)) =
                        self.walk_chain(def, env, &block.items, chain, search)
                    {
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
                    let number = unnamed_number(gen.label.as_deref(), &mut unnamed);
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
                        let number = unnamed_number(block.label.as_deref(), &mut unnamed);
                        path.push(block_dump_name(block.label.as_deref(), number, None));
                        self.walk_body_placements(env, &block.items, path, out, stack);
                        path.pop();
                    }
                }
                Body::Block(block) => {
                    let number = unnamed_number(block.label.as_deref(), &mut unnamed);
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
        let driver_lines: HashSet<usize> = trace.drivers.iter().map(|loc| loc.line).collect();
        trace.loads.retain(|loc| !driver_lines.contains(&loc.line));
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
            '\'' => {
                // Unsized/sized literal without width: 'd125, 'h0, '0, 'x.
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
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
    let mut package_typedefs: BTreeMap<String, TypeDef> = BTreeMap::new();
    while parser.pos < parser.tokens.len() {
        let kind = match parser.peek() {
            Some(Tok::Ident(word)) => match word.as_str() {
                "module" | "macromodule" => Some(DefKind::Module),
                "interface" => Some(DefKind::Interface),
                "program" => Some(DefKind::Program),
                "package" => Some(DefKind::Package),
                _ => None,
            },
            _ => None,
        };
        if let Some(kind) = kind {
            parser.next();
            if let Some(module) = parse_definition(&mut parser, path, kind, &package_typedefs) {
                if module.kind == DefKind::Package {
                    for (name, def) in &module.typedefs {
                        package_typedefs
                            .entry(name.clone())
                            .or_insert_with(|| def.clone());
                    }
                }
                modules.push(module);
            }
        } else {
            parser.next();
        }
    }
    modules
}

fn def_end_keyword(kind: DefKind) -> &'static str {
    match kind {
        DefKind::Module => "endmodule",
        DefKind::Interface => "endinterface",
        DefKind::Program => "endprogram",
        DefKind::Package => "endpackage",
    }
}

fn parse_definition(
    parser: &mut Parser,
    path: &Path,
    kind: DefKind,
    inherited: &BTreeMap<String, TypeDef>,
) -> Option<ModuleDef> {
    let start = parser.line();
    let (name, _) = parser.take_ident()?;
    let mut module = ModuleDef {
        name,
        file: path.to_path_buf(),
        start,
        end: start,
        kind,
        params: BTreeMap::new(),
        param_exprs: Vec::new(),
        typedefs: inherited.clone(),
        imports: Vec::new(),
        ports: Vec::new(),
        signals: Vec::new(),
        assigns: Vec::new(),
        always: Vec::new(),
        instances: Vec::new(),
        body: Vec::new(),
    };
    if kind == DefKind::Package {
        parse_package_body(parser, &mut module);
    } else {
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
    }
    module.end = parser.line();
    if parser.at_ident(def_end_keyword(kind)) {
        parser.next();
    }
    Some(module)
}

/// `package pkg;` body: parameters, typedefs and imports are collected; the
/// rest (functions, tasks, classes) is skipped to `endpackage`.
fn parse_package_body(parser: &mut Parser, module: &mut ModuleDef) {
    loop {
        match parser.peek() {
            None => break,
            Some(Tok::Ident(word)) if word == "endpackage" || word == "end" => break,
            Some(Tok::Ident(word)) => match word.as_str() {
                "parameter" | "localparam" => parse_param_decls(parser, module),
                "typedef" => parse_typedef(parser, module),
                "import" | "export" => parse_import(parser, module),
                "function" => parser.skip_to(&["endfunction"]),
                "task" => parser.skip_to(&["endtask"]),
                "class" => parser.skip_to(&["endclass"]),
                "covergroup" => parser.skip_to(&["endcovergroup"]),
                "property" => parser.skip_to(&["endproperty"]),
                "sequence" => parser.skip_to(&["endsequence"]),
                _ => parser.skip_statement(),
            },
            Some(_) => {
                parser.next();
            }
        }
    }
}

/// Parse module-body items until `end`/`endmodule` (not consumed).
fn parse_body(parser: &mut Parser, module: &mut ModuleDef) -> Vec<Body> {
    let mut items = Vec::new();
    loop {
        match parser.peek() {
            None => break,
            Some(Tok::Ident(word))
                if matches!(
                    word.as_str(),
                    "end" | "endmodule" | "endinterface" | "endprogram" | "endpackage" | "endclass"
                ) =>
            {
                break;
            }
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
        "input" | "output" | "inout" | "wire" | "reg" | "logic" | "bit" | "int" | "integer"
        | "real" | "time" | "genvar" | "supply0" | "supply1" | "tri" | "triand" | "trior"
        | "wand" | "wor" | "trireg" | "byte" | "shortint" | "longint" | "struct" | "enum"
        | "union" => {
            parse_declarations(parser, module, false);
            None
        }
        "parameter" | "localparam" => {
            parse_param_decls(parser, module);
            None
        }
        "typedef" => {
            parse_typedef(parser, module);
            None
        }
        "import" | "export" => {
            parse_import(parser, module);
            None
        }
        "modport" => {
            parser.skip_statement();
            None
        }
        "virtual" => {
            parser.skip_statement();
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
        "class" => {
            parser.skip_to(&["endclass"]);
            None
        }
        "covergroup" => {
            parser.skip_to(&["endcovergroup"]);
            None
        }
        "property" => {
            parser.skip_to(&["endproperty"]);
            None
        }
        "sequence" => {
            parser.skip_to(&["endsequence"]);
            None
        }
        "clocking" => {
            parser.skip_to(&["endclocking"]);
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
        "end" | "endmodule" | "endinterface" | "endprogram" | "endpackage" => None,
        _ => {
            // A declaration of a user type (`state_t s;`, `pkg::t x, y;`,
            // `if_t.master bus;`) is not an instance.
            if looks_like_user_decl(parser, module) {
                parse_declarations(parser, module, false);
                None
            } else if let Some(instance) = parse_instance(parser, module) {
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
    if parser.at_ident("genvar") {
        parser.next();
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
        let end = parser.line();
        GenBlock {
            label,
            items,
            line,
            end,
        }
    } else {
        let items = parse_body_item(parser, module).into_iter().collect();
        let end = parser.line();
        GenBlock {
            label: None,
            items,
            line,
            end,
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
                    _ => {
                        // User type: `pkg::t P = ...` / `type_t P = ...`.
                        if parser.peek_at(1) == Some(&Tok::Punct(':')) {
                            parser.next();
                            parser.next();
                            parser.next();
                            parser.take_ident();
                        } else if parser.peek_at(1) == Some(&Tok::Punct('.')) {
                            parser.next();
                            parser.next();
                            parser.take_ident();
                        } else {
                            break;
                        }
                    }
                },
                Some(Tok::Punct('[')) => parser.skip_balanced(),
                _ => break,
            }
        }
        let Some((name, _)) = parser.take_ident() else {
            // Malformed entry: stop the list instead of skipping ahead into
            // the port list.
            return;
        };
        let mut value = None;
        if parser.eat_punct('=') {
            let expr = parse_expr(parser);
            value = eval(&expr, &module_env(module));
            module.param_exprs.push((name.clone(), expr));
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

/// Parse a constant expression from source text, e.g. an index `[i]`.
pub(crate) fn parse_expr_text(text: &str) -> Option<Expr> {
    let mut parser = Parser::new(lex(text));
    if parser.tokens.is_empty() {
        return None;
    }
    let expr = parse_expr(&mut parser);
    (parser.pos == parser.tokens.len()).then_some(expr)
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
            // Package-qualified constant: `pkg::NAME`.
            if parser.peek() == Some(&Tok::Punct(':'))
                && parser.peek_at(1) == Some(&Tok::Punct(':'))
            {
                parser.next();
                parser.next();
                if let Some((member, _)) = parser.take_ident() {
                    return Expr::Name(format!("{name}::{member}"));
                }
            }
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
                        return None;
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
                        "wire" | "reg" | "logic" | "bit" | "int" | "integer" | "real" | "time"
                        | "supply0" | "supply1" | "tri" | "triand" | "trior" | "wand" | "wor"
                        | "trireg" | "byte" | "shortint" | "longint" => {
                            kind.get_or_insert(word);
                            parser.next();
                        }
                        "struct" | "enum" | "union" => {
                            kind.get_or_insert(word.clone());
                            if let Some(width) = parse_anonymous_type(parser, module) {
                                range = Some(width_range(width));
                            }
                        }
                        "signed" | "unsigned" | "var" | "const" | "static" | "automatic"
                        | "packed" => {
                            parser.next();
                        }
                        _ => {
                            // A new type starts only when the identifier is
                            // qualified (`pkg::t`, `if_t.master`) or followed
                            // by another identifier; otherwise it is the
                            // declared name of the previous type.
                            let qualified = matches!(
                                parser.peek_at(1),
                                Some(Tok::Punct('.')) | Some(Tok::Punct(':'))
                            );
                            let two_idents = matches!(
                                parser.peek_at(1),
                                Some(Tok::Ident(next)) if !is_keyword(next)
                            );
                            if !qualified && !two_idents {
                                break;
                            }
                            // `input wire a, if_t.master b` starts a fresh
                            // declaration with its own type.
                            direction = None;
                            kind = None;
                            range = None;
                            let mut type_name = word.clone();
                            parser.next();
                            if parser.peek() == Some(&Tok::Punct(':'))
                                && parser.peek_at(1) == Some(&Tok::Punct(':'))
                            {
                                parser.next();
                                parser.next();
                                if let Some((second, _)) = parser.take_ident() {
                                    type_name = format!("{word}::{second}");
                                }
                            } else if parser.peek() == Some(&Tok::Punct('.')) {
                                parser.next();
                                if let Some((modport, _)) = parser.take_ident() {
                                    type_name = format!("{word}.{modport}");
                                }
                            }
                            if let Some(width) =
                                module.typedefs.get(&type_name).and_then(|t| t.width)
                            {
                                range = Some(width_range(width));
                            }
                            kind.get_or_insert(type_name);
                        }
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

fn width_range(width: u32) -> String {
    format!("{}:0", width.saturating_sub(1))
}

/// Width of a packed range like `7:0` or `WIDTH-1:0`.
fn range_width(range: &str, module: &ModuleDef) -> Option<u32> {
    let (hi, lo) = range.split_once(':')?;
    let env = module_env(module);
    // `base +: W` / `base -: W`: the part after the colon is the width, while
    // the part before it is an offset rather than a bound.
    if range.contains("+:") || range.contains("-:") {
        return eval(&parse_expr_text(lo.trim())?, &env).map(|width| width.unsigned_abs() as u32);
    }
    let hi = eval(&parse_expr_text(hi.trim())?, &env)?;
    let lo = eval(&parse_expr_text(lo.trim())?, &env)?;
    Some((hi - lo).unsigned_abs() as u32 + 1)
}

/// Consume one type (keywords, packed ranges, user types, anonymous
/// `struct`/`union`/`enum`) and report its packed width when known.
fn consume_type_width(parser: &mut Parser, module: &ModuleDef) -> Option<u32> {
    let mut width = match parser.peek().cloned() {
        Some(Tok::Ident(word)) => match word.as_str() {
            "byte" => {
                parser.next();
                Some(8)
            }
            "shortint" => {
                parser.next();
                Some(16)
            }
            "int" | "integer" => {
                parser.next();
                Some(32)
            }
            "longint" | "time" => {
                parser.next();
                Some(64)
            }
            "struct" | "union" | "enum" => parse_anonymous_type(parser, module),
            "wire" | "reg" | "logic" | "bit" | "tri" | "triand" | "trior" | "wand" | "wor"
            | "trireg" | "supply0" | "supply1" => {
                parser.next();
                Some(1)
            }
            "real" => {
                parser.next();
                None
            }
            _ => {
                let mut name = word.clone();
                parser.next();
                if parser.peek() == Some(&Tok::Punct(':'))
                    && parser.peek_at(1) == Some(&Tok::Punct(':'))
                {
                    parser.next();
                    parser.next();
                    if let Some((member, _)) = parser.take_ident() {
                        name = format!("{word}::{member}");
                    }
                } else if parser.peek() == Some(&Tok::Punct('.')) {
                    parser.next();
                    if let Some((modport, _)) = parser.take_ident() {
                        name = format!("{word}.{modport}");
                    }
                }
                module.typedefs.get(&name).and_then(|t| t.width)
            }
        },
        Some(Tok::Punct('[')) => None,
        _ => None,
    };
    while matches!(parser.peek(), Some(Tok::Punct('['))) {
        let range = collect_range(parser);
        if let Some(w) = range_width(&range, module) {
            width = Some(w);
        }
    }
    width
}

/// Consume an anonymous `struct`/`union`/`enum` body and report its packed
/// width (`struct`: sum of fields, `union`: widest, `enum`: base type or the
/// bits needed by the largest member).
fn parse_anonymous_type(parser: &mut Parser, module: &ModuleDef) -> Option<u32> {
    let is_union = matches!(parser.peek(), Some(Tok::Ident(word)) if word == "union");
    let is_enum = matches!(parser.peek(), Some(Tok::Ident(word)) if word == "enum");
    if !is_union && !is_enum && !matches!(parser.peek(), Some(Tok::Ident(word)) if word == "struct")
    {
        return None;
    }
    parser.next();
    loop {
        match parser.peek() {
            Some(Tok::Ident(word)) if matches!(word.as_str(), "packed" | "signed" | "unsigned") => {
                parser.next();
            }
            _ => break,
        }
    }
    if is_enum {
        let base = if matches!(parser.peek(), Some(Tok::Punct('{'))) {
            None
        } else {
            consume_type_width(parser, module)
        };
        if !parser.eat_punct('{') {
            parser.skip_statement();
            return base;
        }
        let mut next = 0i64;
        let mut max_value = 0i64;
        loop {
            match parser.peek() {
                Some(Tok::Punct('}')) | None => {
                    parser.eat_punct('}');
                    break;
                }
                Some(Tok::Punct(',')) => {
                    parser.next();
                }
                Some(Tok::Punct('[')) => parser.skip_balanced(),
                Some(Tok::Ident(_)) => {
                    parser.next();
                    if parser.eat_punct('=') {
                        let expr = parse_expr(parser);
                        if let Some(value) = eval(&expr, &module_env(module)) {
                            next = value;
                        }
                    }
                    max_value = max_value.max(next);
                    next += 1;
                }
                _ => {
                    parser.next();
                }
            }
        }
        if let Some(base) = base {
            return Some(base);
        }
        let bits = 64 - (max_value.max(1) as u64).leading_zeros();
        return Some(bits.max(1));
    }
    if !parser.eat_punct('{') {
        parser.skip_statement();
        return None;
    }
    let mut total = 0u32;
    let mut widest = 0u32;
    loop {
        match parser.peek() {
            Some(Tok::Punct('}')) | None => {
                parser.eat_punct('}');
                break;
            }
            Some(Tok::Punct(';')) => {
                parser.next();
            }
            _ => {
                let Some(field) = consume_type_width(parser, module) else {
                    parser.skip_statement();
                    continue;
                };
                total = total.saturating_add(field);
                widest = widest.max(field);
                // Field names (and unpacked dimensions) up to `;`.
                while !matches!(
                    parser.peek(),
                    Some(Tok::Punct(';')) | Some(Tok::Punct('}')) | None
                ) {
                    if matches!(parser.peek(), Some(Tok::Punct('['))) {
                        parser.skip_balanced();
                    } else {
                        parser.next();
                    }
                }
                parser.eat_punct(';');
            }
        }
    }
    Some(if is_union { widest } else { total })
}

/// `typedef ... name;` — stores the type with its packed width when known.
fn parse_typedef(parser: &mut Parser, module: &mut ModuleDef) {
    parser.eat_ident("typedef");
    loop {
        match parser.peek() {
            Some(Tok::Ident(word)) if matches!(word.as_str(), "packed" | "signed" | "unsigned") => {
                parser.next();
            }
            _ => break,
        }
    }
    let width = match parser.peek() {
        Some(Tok::Ident(word)) if matches!(word.as_str(), "struct" | "union" | "enum") => {
            parse_anonymous_type(parser, module)
        }
        _ => consume_type_width(parser, module),
    };
    if let Some((name, _)) = parser.take_ident() {
        while matches!(parser.peek(), Some(Tok::Punct('['))) {
            parser.skip_balanced();
        }
        module.typedefs.insert(name, TypeDef { width });
    }
    while !matches!(
        parser.peek(),
        Some(Tok::Punct(';')) | Some(Tok::Punct('}')) | None
    ) {
        parser.next();
    }
    parser.eat_punct(';');
}

/// `import pkg::*;` / `import pkg::name;` — records the package names.
fn parse_import(parser: &mut Parser, module: &mut ModuleDef) {
    parser.next(); // import / export
    loop {
        match parser.peek() {
            Some(Tok::Punct(';')) | Some(Tok::Punct(')')) | None => {
                parser.eat_punct(';');
                return;
            }
            Some(Tok::Ident(name)) => {
                let package = name.clone();
                parser.next();
                if parser.eat_punct(':') {
                    parser.eat_punct(':');
                    if matches!(parser.peek(), Some(Tok::Punct('*'))) {
                        parser.next();
                    } else {
                        parser.take_ident();
                    }
                }
                if !module.imports.contains(&package) {
                    module.imports.push(package);
                }
            }
            _ => {
                parser.next();
            }
        }
    }
}

/// Tokens after `[`..`]` starting at index `i`.
fn skip_balanced_tokens(parser: &Parser, mut i: usize) -> Option<usize> {
    let mut depth = 0i32;
    loop {
        match parser.peek_at(i)? {
            Tok::Punct('[') => depth += 1,
            Tok::Punct(']') => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
}

/// Heuristic: does the current statement declare signals of a user type
/// (`state_t s;`, `pkg::t x, y;`, `if_t.master bus;`) rather than instantiate
/// a module (`my_mod u ( ... );`)?
fn looks_like_user_decl(parser: &Parser, module: &ModuleDef) -> bool {
    let Some(Tok::Ident(first)) = parser.peek() else {
        return false;
    };
    if is_keyword(first) {
        return false;
    }
    let mut i = 1;
    match (parser.peek_at(i), parser.peek_at(i + 1)) {
        (Some(Tok::Punct(':')), Some(Tok::Punct(':'))) => {
            i += 2;
            if matches!(parser.peek_at(i), Some(Tok::Ident(_))) {
                i += 1;
            } else {
                return false;
            }
        }
        (Some(Tok::Punct('.')), _) => {
            i += 1;
            if matches!(parser.peek_at(i), Some(Tok::Ident(_))) {
                i += 1;
            } else {
                return false;
            }
        }
        _ => {}
    }
    while matches!(parser.peek_at(i), Some(Tok::Punct('['))) {
        let Some(next) = skip_balanced_tokens(parser, i) else {
            return false;
        };
        i = next;
    }
    match parser.peek_at(i) {
        Some(Tok::Ident(name)) if !is_keyword(name) => {}
        _ => return false,
    }
    matches!(
        parser.peek_at(i + 1),
        Some(Tok::Punct(',' | ';' | '=')) | None
    ) || module.typedefs.contains_key(first)
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
    let driver_names: HashSet<&str> = drivers.iter().map(|(name, _)| name.as_str()).collect();
    let mut all_uses = sensitivity.clone();
    all_uses.extend(uses);
    all_uses.retain(|entry| !driver_names.contains(entry.0.as_str()));
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
///
/// A `begin ... end` body is consumed as a whole (nested blocks, `if/else`
/// chains and `case` statements included); a bare statement ends at its `;`.
fn scan_statement(parser: &mut Parser) -> (Idents, Idents, usize) {
    let mut drivers = Vec::new();
    let mut uses = Vec::new();
    let mut end = parser.line();
    let mut depth = 0i32;
    let mut paren_depth = 0i32;
    let mut block_depth = 0i32;
    let mut started_block = false;
    let mut cond_depth: Option<i32> = None;
    let mut last_cond = false;
    let mut last_punct: Option<char> = None;
    let mut pending: Option<(String, usize)> = None;
    // Base identifier of an assignment target (`scratch` in
    // `scratch[a][b] <= ...`), kept across index selectors.
    let mut lhs_base: Option<(String, usize)> = None;

    while let Some(tok) = parser.peek().cloned() {
        let line = parser.line();
        match &tok {
            Tok::Ident(name) => {
                match name.as_str() {
                    "begin" | "fork" => {
                        started_block = true;
                        block_depth += 1;
                    }
                    "end" | "join" | "join_any" | "join_none" => {
                        block_depth -= 1;
                        if started_block && block_depth <= 0 {
                            // `end else ...`: the alternative branch continues
                            // the same statement.
                            if matches!(parser.peek_at(1), Some(Tok::Ident(word)) if word == "else")
                            {
                                end = line;
                                parser.next();
                                continue;
                            }
                            end = line;
                            parser.next();
                            break;
                        }
                    }
                    _ => {}
                }
                last_cond = matches!(
                    name.as_str(),
                    "if" | "while" | "for" | "case" | "casex" | "casez"
                );
                if !is_keyword(name) {
                    uses.push((name.clone(), line));
                    if depth <= 0 && cond_depth.is_none() {
                        lhs_base = Some((name.clone(), line));
                    }
                    pending = Some((name.clone(), line));
                }
                last_punct = None;
                parser.next();
            }
            Tok::Punct(c) => {
                let c = *c;
                match c {
                    '(' | '[' | '{' => {
                        if c == '(' {
                            paren_depth += 1;
                            if last_cond {
                                cond_depth = Some(paren_depth);
                            }
                        }
                        depth += 1;
                    }
                    ')' | ']' | '}' => {
                        if c == ')' {
                            if cond_depth == Some(paren_depth) {
                                cond_depth = None;
                            }
                            paren_depth -= 1;
                        }
                        depth -= 1;
                    }
                    ';' if depth <= 0 && block_depth <= 0 => {
                        end = line;
                        parser.next();
                        break;
                    }
                    '=' if depth <= 0
                        && cond_depth.is_none()
                        && last_punct != Some('=')
                        && last_punct != Some('!') =>
                    {
                        if let Some((name, driver_line)) =
                            lhs_base.take().or_else(|| pending.take())
                        {
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
    if module.name == module_type || module.typedefs.contains_key(&module_type) {
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
    let mut ports = Vec::new();
    if parser.eat_punct('(') {
        loop {
            match parser.peek() {
                Some(Tok::Punct(')')) | None => {
                    parser.eat_punct(')');
                    break;
                }
                Some(Tok::Punct('.')) => {
                    parser.next();
                    let line = parser.line();
                    if let Some((name, _)) = parser.take_ident() {
                        ports.push((name, line));
                    }
                    if matches!(parser.peek(), Some(Tok::Punct('('))) {
                        parser.skip_balanced();
                    }
                }
                Some(Tok::Punct(',')) => {
                    parser.next();
                }
                Some(Tok::Punct('(' | '[' | '{')) => parser.skip_balanced(),
                Some(_) => {
                    parser.next();
                }
            }
        }
    }
    if matches!(parser.peek(), Some(Tok::Punct(';'))) {
        parser.next();
    }
    Some(Instance {
        module: module_type,
        name: instance,
        line,
        overrides,
        ports,
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
            | "int"
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
            | "program"
            | "endprogram"
            | "modport"
            | "virtual"
            | "class"
            | "endclass"
            | "covergroup"
            | "endcovergroup"
            | "clocking"
            | "endclocking"
            | "byte"
            | "shortint"
            | "longint"
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
        assert_eq!(m.instances[0].ports.len(), 5);
        assert!(m.instances[0]
            .ports
            .iter()
            .any(|(name, line)| name == "clk" && *line == 6));
        assert!(m.instances[0]
            .ports
            .iter()
            .any(|(name, line)| name == "carry" && *line == 10));
    }

    #[test]
    fn part_select_ranges_report_the_width_after_the_colon() {
        let module = parse_module_text("module m; endmodule\n", Path::new("m.sv"))
            .into_iter()
            .next()
            .expect("module");
        assert_eq!(range_width("4+:3", &module), Some(3));
        assert_eq!(range_width("4-:3", &module), Some(3));
        // Plain ranges keep their inclusive width.
        assert_eq!(range_width("7:0", &module), Some(8));
    }

    #[test]
    fn walk_scope_budget_bounds_nested_indexless_loops() {
        // Both loops are described by dump steps without an index, so the
        // search recurses into every value of every level: the missing tail
        // step would explore `4096 * 4096` paths without a budget.
        let text = r#"
module top;
    generate
        for (genvar i = 0; i < 4096; i++) begin : a
            for (genvar j = 0; j < 4096; j++) begin : b
                leaf u_leaf();
            end
        end
    endgenerate
endmodule

module leaf;
    logic y;
endmodule
"#;
        let dir = std::env::temp_dir().join(format!("waverdi_walk_budget_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gen.sv");
        fs::write(&file, text).unwrap();
        let set = SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);
        let steps = |names: &[&str]| {
            names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        };
        // The unresolvable tail must terminate through the budget, not spin
        // through the loop product.
        let start = std::time::Instant::now();
        assert!(db
            .module_at_scope(&steps(&["top", "a", "b", "missing"]))
            .is_none());
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "bounded search must return quickly"
        );
        // A matching path inside the same loops still resolves.
        assert_eq!(
            db.module_at_scope(&steps(&["top", "a", "b", "u_leaf"]))
                .unwrap()
                .name,
            "leaf"
        );
    }

    #[test]
    fn walk_scope_budget_keeps_shallow_matches() {
        let text = r#"
module top;
    generate
        for (genvar i = 0; i < 512; i++) begin : a
            leaf u_leaf();
        end
    endgenerate
endmodule

module leaf;
    logic y;
endmodule
"#;
        let dir = std::env::temp_dir().join(format!("waverdi_walk_shallow_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gen.sv");
        fs::write(&file, text).unwrap();
        let set = SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);
        let steps = |names: &[&str]| {
            names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        };
        let found = db
            .scope_info(&steps(&["top", "a", "u_leaf"]))
            .expect("loop instance inside an index-less generate block");
        assert_eq!(found.module.name, "leaf");
        assert_eq!(found.instance_scope, steps(&["top", "a[0]", "u_leaf"]));
    }

    #[test]
    fn simulation_types_declare_signals() {
        let text = "module m;\n\
                    \x20   int a;\n\
                    \x20   int unsigned b;\n\
                    \x20   int signed c = 0;\n\
                    \x20   integer d;\n\
                    \x20   real e;\n\
                    \x20   time f;\n\
                    \x20   shortint g;\n\
                    \x20   longint h;\n\
                    \x20   byte i;\n\
                    endmodule\n";
        let module = parse_module_text(text, Path::new("m.sv"))
            .into_iter()
            .next()
            .unwrap();
        for name in ["a", "b", "c", "d", "e", "f", "g", "h", "i"] {
            assert!(module.declares(name), "{name} not declared");
        }
    }

    #[test]
    fn inactive_generate_branches_report_their_lines() {
        let text = "\
module top;
    generate
        if (0) begin : off
            assign a = b;
        end else begin : on
            assign c = d;
        end
    endgenerate
endmodule

module pick;
    generate
        if (1) begin : on
            assign e = f;
        end else begin : off
            assign g = h;
        end
    endgenerate
endmodule

module unknown;
    generate
        if (MAYBE) begin : a
            assign i = j;
        end else begin : b
            assign k = l;
        end
    endgenerate
endmodule

module alone;
    generate
        if (0) begin : dead
            assign m = n;
        end
    endgenerate
endmodule
";
        let dir = std::env::temp_dir().join(format!("waverdi_inactive_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gen.sv");
        fs::write(&file, text).unwrap();
        let set = SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);
        // `if (0)`: the first branch is dead, the `else` is instantiated.
        assert_eq!(db.inactive_lines("top"), vec![3..5]);
        // `if (1)`: the `else` branch is dead.
        assert_eq!(db.inactive_lines("pick"), vec![15..18]);
        // An unknown condition cannot rule either branch out.
        assert!(db.inactive_lines("unknown").is_empty());
        // A dead branch without an `else` is the whole construct.
        assert_eq!(db.inactive_lines("alone"), vec![33..36]);
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
    fn resolves_struct_members_and_instance_aggregates() {
        let text = r#"
module m(input logic clk);
    typedef struct packed { logic run; logic [3:0] pc; } fetch_t;
    fetch_t clk_fetch;
    fetch_t clk_pc;
    child u_child(.clk(clk));
endmodule

module child(input logic clk);
endmodule

module top(input logic clk);
    if_t vif(.clk(clk));
    assign y = vif.valid;
endmodule

interface if_t(input logic clk);
    logic valid;
endinterface
"#;
        let dir = std::env::temp_dir().join(format!("waverdi_sv_member_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("m.sv");
        fs::write(&file, text).unwrap();
        let set = SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);

        // Struct variable member: the chain walks the dump scope of the
        // variable (`clk_fetch.run`).
        let (path, signal) = db
            .resolve_reference("m", &["clk_fetch".to_string()], "run")
            .expect("struct member");
        assert_eq!(path, vec!["clk_fetch".to_string()]);
        assert_eq!(signal, "run");
        // Whole struct variable.
        let (path, signal) = db
            .resolve_reference("m", &[], "clk_fetch")
            .expect("struct variable");
        assert!(path.is_empty());
        assert_eq!(signal, "clk_fetch");
        // Interface instance member resolves through the instance scope.
        let (path, signal) = db
            .resolve_reference("top", &["vif".to_string()], "valid")
            .expect("interface member");
        assert_eq!(path, vec!["vif".to_string()]);
        assert_eq!(signal, "valid");
        // A whole instance resolves to its scope aggregate.
        let (path, signal) = db.resolve_reference("top", &[], "vif").expect("instance");
        assert!(path.is_empty());
        assert_eq!(signal, "vif");
    }

    #[test]
    fn nested_generate_fallback_is_bounded() {
        let mut text = String::from("module top;\n    generate\n");
        for (var, label) in [("i", "a"), ("j", "b"), ("k", "c")] {
            text.push_str(&format!(
                "        for (genvar {var} = 0; {var} < 128; {var}++) begin : {label}\n"
            ));
        }
        text.push_str("            leaf u_leaf();\n");
        for _ in 0..3 {
            text.push_str("        end\n");
        }
        text.push_str("    endgenerate\nendmodule\n\nmodule leaf;\n    logic y;\nendmodule\n");
        let dir = std::env::temp_dir().join(format!("waverdi_gen_bound_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gen.sv");
        fs::write(&file, text).unwrap();
        let set = SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);
        // An unresolved identifier inside nested loops must terminate quickly.
        assert!(db
            .resolve_reference("top", &["missing".to_string()], "x")
            .is_none());
        // A real instance inside the loops still resolves to its first path.
        let (path, signal) = db
            .resolve_reference("top", &["u_leaf".to_string()], "y")
            .expect("nested instance");
        assert_eq!(
            path,
            vec![
                "a[0]".to_string(),
                "b[0]".to_string(),
                "c[0]".to_string(),
                "u_leaf".to_string()
            ]
        );
        assert_eq!(signal, "y");
    }

    #[test]
    fn real_world_header_with_params_and_interface_ports() {
        let text = r#"`default_nettype none

module prt_dp_pm_top
#
(
    parameter                           P_VENDOR        = "none",    // Vendor - "AMD", "ALTERA" or "LSC" 
    parameter                           P_BEAT          = 'd125,     // Beat value
    parameter                           P_HW_VER_MAJOR  = 1,         // Hardware version major
    parameter                           P_HW_VER_MINOR  = 0,         // Hardware version minor
    parameter                           P_CFG           = "tx",      // Configuration TX / RX
    parameter                           P_SIM           = 0,
    parameter                           P_ROM_INIT_FILE = "none",
    parameter                           P_RAM_INIT_FILE = "none",
    parameter                           P_PIO_IN_WIDTH  = 8,
    parameter                           P_PIO_OUT_WIDTH = 8,
    parameter                           P_SPL = 2,                   // Symbols per lane
    parameter                           P_MST = 0                    // MST
)
(
    // Reset and clock
    input wire                          RST_IN,
    input wire                          CLK_IN,

    // Interrupt
    input wire [1:0]                    IRQ_IN,

    // PIO
    input wire [P_PIO_IN_WIDTH-1:0]     PIO_IN,
    output wire [P_PIO_OUT_WIDTH-1:0]   PIO_OUT,

    // Host
    prt_dp_lb_if.lb_in                  HOST_IF,
    output wire                         HOST_IRQ_OUT, 

    // HPD
    input wire                          HPD_IN,
    output wire                         HPD_OUT,

    // AUX
    output wire                         AUX_EN_OUT,
    output wire                         AUX_TX_OUT,
    input wire                          AUX_RX_IN,

    // Message 
    prt_dp_msg_if.src                   MSG_SRC_IF,
    prt_dp_msg_if.snk                   MSG_SNK_IF
);
endmodule
"#;
        let modules = parse_module_text(text, Path::new("prt_dp_pm_top.sv"));
        assert_eq!(modules.len(), 1);
        let m = &modules[0];
        assert_eq!(m.params.len(), 12);
        let ports: Vec<&str> = m.ports.iter().map(|decl| decl.name.as_str()).collect();
        assert_eq!(
            ports,
            vec![
                "RST_IN",
                "CLK_IN",
                "IRQ_IN",
                "PIO_IN",
                "PIO_OUT",
                "HOST_IF",
                "HOST_IRQ_OUT",
                "HPD_IN",
                "HPD_OUT",
                "AUX_EN_OUT",
                "AUX_TX_OUT",
                "AUX_RX_IN",
                "MSG_SRC_IF",
                "MSG_SNK_IF"
            ]
        );
    }

    #[test]
    fn header_ports_with_interface_types() {
        let text = r#"
module prt_dp_pm_top
#
(
    parameter P_VENDOR = "none",
    parameter P_CFG = "tx",
    parameter P_PIO_IN_WIDTH = 8,
    parameter P_MST = 0
)
(
    input wire RST_IN,
    input wire [1:0] IRQ_IN,
    input wire [P_PIO_IN_WIDTH-1:0] PIO_IN,
    prt_dp_lb_if.lb_in HOST_IF,
    output wire HOST_IRQ_OUT,
    prt_dp_msg_if.src MSG_SRC_IF
);
endmodule
"#;
        let modules = parse_module_text(text, Path::new("t.sv"));
        let m = &modules[0];
        let ports: Vec<&str> = m.ports.iter().map(|decl| decl.name.as_str()).collect();
        assert_eq!(
            ports,
            vec![
                "RST_IN",
                "IRQ_IN",
                "PIO_IN",
                "HOST_IF",
                "HOST_IRQ_OUT",
                "MSG_SRC_IF"
            ]
        );
    }

    #[test]
    fn scope_info_reports_generate_block_lines() {
        let text = r#"
module top;
    generate
        for (genvar i = 0; i < 2; i++) begin : gen_a
            child u_child();
        end
    endgenerate
    if (1) begin : gen_b
        child u_b();
    end
endmodule

module child;
endmodule
"#;
        let dir = std::env::temp_dir().join(format!("waverdi_gen_line_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("top.sv");
        fs::write(&file, text).unwrap();
        let set = SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);
        // `begin : gen_a` is on line 4 of the file.
        let found = db
            .scope_info(&["top".to_string(), "gen_a[1]".to_string()])
            .expect("generate block");
        assert_eq!(found.line, Some(4));
        // Nested if-block: deepest block line wins.
        let found = db
            .scope_info(&["top".to_string(), "gen_b".to_string()])
            .expect("if block");
        assert_eq!(found.line, Some(8));
        // Paths that end at an instance report the instance boundary instead.
        let found = db
            .scope_info(&[
                "top".to_string(),
                "gen_a[0]".to_string(),
                "u_child".to_string(),
            ])
            .expect("instance inside generate");
        assert_eq!(found.instance_scope.len(), 3);
    }

    #[test]
    fn missing_generate_scopes_are_merged_into_the_tree() {
        let text = r#"
module top #(parameter int N = 3) (input logic clk);
    generate
        for (genvar i = 0; i < N; i++) begin : gen_x
            logic flag;
        end
    endgenerate
    generate
        if (N > 1) begin : gen_y
            logic y;
        end
    endgenerate
endmodule
"#;
        let dir = std::env::temp_dir().join(format!("waverdi_gen_merge_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("top.sv");
        fs::write(&file, text).unwrap();
        let set = SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);
        let mut wf = Waveform {
            ts: crate::waveform::TimeScale::default(),
            start: 0,
            end: 0,
            signals: vec![],
            tree: crate::waveform::ScopeTree::new(),
        };
        let top = wf
            .tree
            .add_scope(wf.tree.root, "top".to_string(), "top".to_string());
        // One iteration already exists in the dump; it must be reused.
        wf.tree
            .add_scope(top, "gen_x[1]".to_string(), String::new());
        db.merge_generate_scopes(&mut wf);
        let names: Vec<&str> = wf.tree.nodes[top]
            .children
            .iter()
            .map(|&child| wf.tree.nodes[child].name.as_str())
            .collect();
        assert_eq!(names, vec!["gen_x[1]", "gen_x[0]", "gen_x[2]", "gen_y"]);
        assert_eq!(names.iter().filter(|name| **name == "gen_x[1]").count(), 1);
    }

    #[test]
    fn large_generated_case_tables_parse_without_quadratic_blowup() {
        // Generated video overlay tables are single always blocks with
        // hundreds of thousands of case items; use/trace filtering must not
        // be quadratic in the number of items.
        let items = 20_000;
        let mut text = String::from("module big(input logic clk, output logic data);\n");
        text.push_str("    always_ff @(posedge clk) begin\n        case (clk)\n");
        for i in 0..items {
            text.push_str(&format!("            32'd{i} : data <= 1'b0;\n"));
        }
        text.push_str("        endcase\n    end\n");
        for i in 0..items {
            text.push_str(&format!("    assign use_{i} = data;\n"));
        }
        text.push_str("endmodule\n");

        let dir = std::env::temp_dir().join(format!("waverdi_big_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("big.sv");
        fs::write(&file, text).unwrap();
        let set = SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);

        let m = db.module("big").expect("module");
        assert_eq!(m.always.len(), 1);
        assert_eq!(m.always[0].drivers.len(), items);
        assert_eq!(m.assigns.len(), items);
        let trace = db.trace("big", "data").expect("trace");
        assert_eq!(trace.drivers.len(), items);
        assert_eq!(trace.loads.len(), items);
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
        // An instance name resolves to the aggregate of its dump scope.
        let (path, signal) = db
            .resolve_reference("tb", &[], "u_dut")
            .expect("instance aggregate");
        assert!(path.is_empty());
        assert_eq!(signal, "u_dut");
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

        // scope_info reports the bindings and instance scope behind a
        // generate block scope.
        let found = db
            .scope_info(&steps(&["top", "u_mid", "gen_leaves[1]"]))
            .expect("genblk scope");
        assert_eq!(found.module.name, "mid");
        assert_eq!(found.instance_scope, steps(&["top", "u_mid"]));
        assert_eq!(found.env.get("i"), Some(&1));
        // A genvar bound by the selected scope pins plain instance references.
        let (path, _) = db
            .resolve_reference_with("mid", &steps(&["u_odd"]), "q", &found.env)
            .expect("pinned iteration");
        assert_eq!(path, steps(&["gen_leaves[1]", "gen_odd", "u_odd"]));
        // Index selectors are constant expressions over those bindings.
        assert_eq!(eval(&parse_expr_text("i").unwrap(), &found.env), Some(1));
    }

    #[test]
    fn always_blocks_with_begin_end_do_not_end_the_module() {
        let text = r#"
module pipe_reg(input logic clk, input logic rst_n, output logic q);
    logic r;
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            r <= 1'b0;
        end else begin
            r <= ~r;
        end
    end
    assign q = r;
endmodule

module fifo(input logic clk, output logic [3:0] count);
    logic [3:0] c;
    always_ff @(posedge clk) begin
        c <= c + 1'b1;
    end
    assign count = c;
endmodule
"#;
        let modules = parse_module_text(text, Path::new("common.sv"));
        assert_eq!(modules.len(), 2);
        let pipe = &modules[0];
        assert_eq!(pipe.name, "pipe_reg");
        assert_eq!(pipe.end, 12); // the module ends at its `endmodule`
        assert!(pipe.signal("r").is_some());
        assert!(pipe.assigns.iter().any(|assign| assign.lhs == "q"));
        assert!(pipe.always[0]
            .drivers
            .iter()
            .any(|(name, line)| name == "r" && *line == 6));
        let fifo = &modules[1];
        assert_eq!(fifo.name, "fifo");
        assert_eq!(fifo.start, 14);
        assert_eq!(fifo.end, 20);
        assert!(fifo.signal("c").is_some());
    }

    #[test]
    fn indexed_assignments_are_driven_by_their_base_signal() {
        let text = r#"
module m(input logic clk);
    logic [7:0] mem [2][2];
    always_ff @(posedge clk) begin
        mem[a][b] <= 8'h01;
    end
endmodule
"#;
        let modules = parse_module_text(text, Path::new("m.sv"));
        let block = &modules[0].always[0];
        assert!(block
            .drivers
            .iter()
            .any(|(name, line)| name == "mem" && *line == 5));
        assert!(!block.drivers.iter().any(|(name, _)| name == "b"));
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

    const SV_TYPES: &str = r#"
package cfg_pkg;
    parameter int WIDTH = 16;
    localparam int DEPTH = 4;
    typedef enum logic [1:0] {S_IDLE, S_RUN} state_e;
    typedef struct packed { logic [WIDTH-1:0] data; logic last; } beat_t;
endpackage

interface lb_if (input logic clk);
    logic valid;
    modport master (output valid, input clk);
endinterface

module top #(parameter int N = cfg_pkg::WIDTH) (input logic clk);
    import cfg_pkg::*;
    typedef logic [7:0] byte_t;
    state_e state;
    beat_t beat;
    byte_t a, b;
    lb_if u_if (.clk(clk));
    lb_if.master bus;
    generate
        for (genvar i = 0; i < DEPTH; i++) begin : g
            sub u_sub (.clk(clk));
        end
    endgenerate
endmodule

module sub(input logic clk);
    logic flag;
endmodule
"#;

    #[test]
    fn parses_interfaces_packages_and_typedefs() {
        let modules = parse_module_text(SV_TYPES, Path::new("sv.sv"));
        assert_eq!(modules.len(), 4);
        let package = &modules[0];
        assert_eq!(package.name, "cfg_pkg");
        assert_eq!(package.kind, DefKind::Package);
        assert_eq!(package.params.get("WIDTH"), Some(&Some(16)));
        assert_eq!(package.params.get("DEPTH"), Some(&Some(4)));
        assert_eq!(package.typedefs.get("state_e").unwrap().width, Some(2));

        let interface = &modules[1];
        assert_eq!(interface.kind, DefKind::Interface);
        assert!(interface.signal("valid").is_some());

        let top = &modules[2];
        assert_eq!(top.kind, DefKind::Module);
        assert!(top.imports.contains(&"cfg_pkg".to_string()));
        // User-typed declarations become signals, not instances.
        assert!(top.signal("state").is_some());
        assert_eq!(top.signal("state").unwrap().range.as_deref(), Some("1:0"));
        assert!(top.signal("beat").is_some());
        assert_eq!(top.signal("beat").unwrap().range.as_deref(), Some("16:0"));
        assert_eq!(top.signal("a").unwrap().range.as_deref(), Some("7:0"));
        assert!(top.signal("b").is_some());
        // Interface instances are instances; interface ports are signals.
        assert!(top
            .instances
            .iter()
            .any(|inst| inst.module == "lb_if" && inst.name == "u_if"));
        assert!(top.signal("bus").is_some());
        assert!(!top.instances.iter().any(|inst| inst.name == "bus"));
        // `for (genvar i = 0; i < DEPTH; ...)` with an imported parameter.
        assert_eq!(top.body.len(), 2);
    }

    #[test]
    fn package_parameters_and_imports_elaborate() {
        let dir = std::env::temp_dir().join(format!("waverdi_sv_pkg_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sv.sv");
        fs::write(&file, SV_TYPES).unwrap();
        let set = SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);
        // `pkg::NAME` is visible everywhere, `import pkg::*` binds plain names.
        let top = db.module("top").unwrap();
        let env = db.default_env(top);
        assert_eq!(env.get("cfg_pkg::WIDTH"), Some(&16));
        assert_eq!(env.get("WIDTH"), Some(&16));
        assert_eq!(env.get("DEPTH"), Some(&4));
        assert_eq!(env.get("N"), Some(&16));
        // The generate loop elaborates into g[0..3].u_sub.
        let placements = db.placements();
        let paths: Vec<String> = placements
            .iter()
            .map(|(steps, _)| steps.join("."))
            .collect();
        for index in 0..4 {
            assert!(
                paths
                    .iter()
                    .any(|path| path.ends_with(&format!("g[{index}].u_sub"))),
                "missing g[{index}].u_sub in {paths:?}"
            );
        }
    }

    /// Malformed inputs must terminate and keep whatever parsed before the
    /// damage; the assertions pin the current partial-result behavior.
    #[test]
    fn malformed_modules_terminate_with_partial_results() {
        let path = Path::new("broken.sv");
        // Empty input and `endmodule` without a module: nothing to report.
        assert!(parse_module_text("", path).is_empty());
        assert!(parse_module_text("endmodule\n", path).is_empty());
        // A module truncated at EOF keeps its name and declarations.
        let mods = parse_module_text("module top;\n  logic a;\n", path);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "top");
        assert!(mods[0].signal("a").is_some());
        // `begin` without a matching `end` still yields the always block.
        let mods = parse_module_text("module top;\n  always @(*) begin\n    a = b;\n", path);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "top");
        assert_eq!(mods[0].always.len(), 1);
        // An unbalanced `(` does not lose the assignment.
        let mods = parse_module_text("module top;\n  assign y = (a + (b;\nendmodule\n", path);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].assigns.len(), 1);
        assert_eq!(mods[0].assigns[0].lhs, "y");
    }

    #[test]
    fn lexical_hazards_do_not_panic() {
        let path = Path::new("hazard.sv");
        // Garbage before a valid module is skipped by the lexer.
        let mods = parse_module_text("%%%\u{0}\u{ff}\u{1} module m(); endmodule\n", path);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "m");
        // An unclosed block comment swallows the rest of the file, but not
        // the module header seen before it.
        let mods = parse_module_text("module top;\n  /* never closed\n  logic a;\n", path);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "top");
        assert!(mods[0].signals.is_empty());
        // An unclosed string runs to EOF; the module still parses.
        let mods = parse_module_text("module top;\n  initial $display(\"abc\n", path);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "top");
    }

    #[test]
    fn lexer_literals_and_escaped_identifiers() {
        assert_eq!(parse_int("8'hFF"), 255);
        assert_eq!(parse_int("'d125"), 125);
        // `'x` is an unsized all-x literal; without a width to expand it to,
        // the scanner falls back to 0.
        assert_eq!(parse_int("'x"), 0);
        // One `z` digit makes the whole literal unparsable, so it falls back
        // to 0 as well.
        assert_eq!(parse_int("10'b1z0"), 0);
        // An escaped identifier is one token; the leading backslash stays in
        // the name (the parser strips it where it matters).
        let tokens = lex("\\bus+clk ");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].tok, Tok::Ident("\\bus+clk".to_string()));
    }

    #[test]
    fn eval_edge_cases() {
        let env = ParamEnv::new();
        let eval_text = |text: &str| parse_expr_text(text).and_then(|expr| eval(&expr, &env));

        // Division and modulo by zero are guarded to 0 instead of failing or
        // panicking.
        assert_eq!(eval_text("1/0"), Some(0));
        assert_eq!(eval_text("1%0"), Some(0));
        // Checked arithmetic: overflow refuses to wrap.
        assert_eq!(eval_text("9223372036854775807 + 1"), None);
        assert_eq!(eval_text("3037000500 * 3037000500"), None);
        // Ternary, shifts (an out-of-range shift is rejected), unary minus.
        assert_eq!(eval_text("5 ? 7 : 9"), Some(7));
        assert_eq!(eval_text("0 ? 7 : 9"), Some(9));
        assert_eq!(eval_text("1 << 3"), Some(8));
        assert_eq!(eval_text("8 >> 1"), Some(4));
        assert_eq!(eval_text("1 << 64"), None);
        assert_eq!(eval_text("-5"), Some(-5));
        // Unknown names evaluate to None.
        assert_eq!(eval_text("unknown_name"), None);
        assert_eq!(eval_text("unknown_name + 1"), None);
    }
}
