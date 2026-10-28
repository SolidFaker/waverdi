//! Lightweight SystemVerilog scanner for the RTL source view and tracing.
//!
//! This is intentionally not a full parser: it recognises modules, signal
//! declarations, continuous assignments, `always`/`initial` blocks and module
//! instantiations together with their source lines — enough to browse RTL and
//! to trace a signal to its declaration, drivers and loads.

mod db;
mod lexer;
mod model;
mod parser;
mod resolve;
mod trace;
mod waveform;

#[cfg(test)]
mod tests;

pub use db::RtlDb;
#[allow(unused_imports)]
pub use model::{
    AlwaysBlock, Assign, BinOp, Body, DefKind, Expr, GenBlock, GenFor, GenIf, Instance, Location,
    ModuleDef, SignalDecl, SignalTrace, TypeDef,
};
pub use parser::is_keyword;

#[cfg(test)]
pub(crate) use model::ParamEnv;
#[allow(unused_imports)]
pub(crate) use model::ScopeMatch;
pub(crate) use parser::parse_expr_text;
pub(crate) use resolve::{eval, signal_name_matches};
