use std::collections::BTreeMap;
use std::path::PathBuf;

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
/// Identifier without a packed range: `count[7:0]` -> `count`.
pub(super) fn base_name(text: &str) -> &str {
    text.split('[').next().unwrap_or(text)
}
/// Declaration, drivers and loads of one signal inside a module.
#[derive(Clone, Debug, Default)]
pub struct SignalTrace {
    pub decl: Option<Location>,
    pub drivers: Vec<Location>,
    pub loads: Vec<Location>,
}
/// Known parameter/genvar values during elaboration.
pub(crate) type ParamEnv = BTreeMap<String, i64>;
