use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::super::SourceSet;
use super::model::{DefKind, ModuleDef, ParamEnv, TypeDef};
use super::parser::{parse_module_text, width_range};

/// Memoized `scope_info` result. It owns everything the returned
/// [`ScopeMatch`] needs, so the cache can live behind a `RefCell` while the
/// match itself borrows the `ModuleDef` out of `RtlDb::modules`.
#[derive(Clone, Debug)]
pub(super) struct CachedScope {
    pub(super) module: String,
    pub(super) env: Arc<ParamEnv>,
    pub(super) instance_scope: Vec<String>,
    pub(super) line: Option<usize>,
}

/// All modules found in the RTL sources.
#[derive(Clone, Debug, Default)]
pub struct RtlDb {
    pub modules: BTreeMap<String, ModuleDef>,
    pub files: Vec<PathBuf>,
    /// Evaluated parameters of every `package`, by package name. Package
    /// parameters are visible to modules as `pkg::NAME` (and via `import`).
    pub package_params: BTreeMap<String, BTreeMap<String, i64>>,
    /// Lazily filled caches. `parse_sources` starts from `RtlDb::default`,
    /// so a new database naturally starts with empty caches.
    top_modules: OnceCell<Vec<String>>,
    pub(super) default_envs: RefCell<HashMap<String, Arc<ParamEnv>>>,
    pub(super) scope_cache: RefCell<HashMap<Vec<String>, CachedScope>>,
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

    /// Cache fill state, used by the memoization regression test.
    #[cfg(test)]
    pub(super) fn cache_sizes(&self) -> (bool, usize, usize) {
        (
            self.top_modules.get().is_some(),
            self.default_envs.borrow().len(),
            self.scope_cache.borrow().len(),
        )
    }
    /// Modules that no other parsed module instantiates: the roots of the
    /// elaborated design hierarchy. Computed once; every hierarchy lookup
    /// starts from this list.
    pub fn top_modules(&self) -> Vec<String> {
        self.top_modules
            .get_or_init(|| {
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
            })
            .clone()
    }
}
fn is_verilog(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("v" | "sv")
    )
}
