use std::collections::HashSet;
use std::sync::Arc;

use super::db::{CachedScope, RtlDb};
use super::model::{
    base_name, BinOp, Body, Expr, GenBlock, GenFor, GenIf, Instance, ModuleDef, ParamEnv,
    ScopeMatch, SignalDecl,
};

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
/// Declaration of `name` in a module (signal or port), matched by base name.
fn module_declaration<'a>(def: &'a ModuleDef, name: &str) -> Option<&'a SignalDecl> {
    def.signals
        .iter()
        .chain(def.ports.iter())
        .find(|decl| decl.name == name || base_name(&decl.name) == base_name(name))
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
pub(super) fn gen_values(gen: &GenFor, env: &ParamEnv) -> Vec<i64> {
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
pub(super) fn taken_branches<'a>(gen_if: &'a GenIf, env: &ParamEnv) -> Vec<&'a GenBlock> {
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
pub(super) fn unnamed_number(label: Option<&str>, unnamed: &mut usize) -> Option<usize> {
    if label.is_none() {
        Some(unnamed_ordinal(unnamed))
    } else {
        None
    }
}

/// Dump scope name of a generate block: the label or the tool-assigned
/// `genblk<N>`, with the loop index appended for `for` loops.
pub(super) fn block_dump_name(
    label: Option<&str>,
    number: Option<usize>,
    index: Option<i64>,
) -> String {
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

impl RtlDb {
    /// Known parameter/genvar values of a module (default overrides).
    pub(crate) fn default_env(&self, def: &ModuleDef) -> ParamEnv {
        if let Some(env) = self.default_envs.borrow().get(&def.name) {
            return (**env).clone();
        }
        let env = self.compute_default_env(def);
        self.default_envs
            .borrow_mut()
            .insert(def.name.clone(), Arc::new(env.clone()));
        env
    }

    /// Uncached `default_env` body; called once per module.
    fn compute_default_env(&self, def: &ModuleDef) -> ParamEnv {
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
    /// `i = 1`. Results are memoized per scope path: the UI re-resolves the
    /// selected instance on every selection change, and the walk enumerates
    /// generate loops.
    pub(crate) fn scope_info(&self, steps: &[String]) -> Option<ScopeMatch<'_>> {
        if steps.is_empty() {
            return None;
        }
        if let Some(found) = self.cached_scope(steps) {
            return Some(found);
        }
        let found = self.compute_scope_info(steps)?;
        let module = found.module.name.clone();
        let env = Arc::new(found.env);
        let instance_scope = found.instance_scope;
        let line = found.line;
        self.scope_cache.borrow_mut().insert(
            steps.to_vec(),
            CachedScope {
                module: module.clone(),
                env: env.clone(),
                instance_scope: instance_scope.clone(),
                line,
            },
        );
        Some(ScopeMatch {
            module: self.modules.get(&module)?,
            env: (*env).clone(),
            instance_scope,
            line,
        })
    }

    /// Rebuild a memoized match, borrowing the module out of `self.modules`
    /// so the returned reference outlives the cache borrow.
    fn cached_scope(&self, steps: &[String]) -> Option<ScopeMatch<'_>> {
        let cache = self.scope_cache.borrow();
        let hit = cache.get(steps)?;
        Some(ScopeMatch {
            module: self.modules.get(&hit.module)?,
            env: (*hit.env).clone(),
            instance_scope: hit.instance_scope.clone(),
            line: hit.line,
        })
    }

    /// Uncached `scope_info` body.
    fn compute_scope_info(&self, steps: &[String]) -> Option<ScopeMatch<'_>> {
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
