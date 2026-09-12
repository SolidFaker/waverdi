//! Lightweight SystemVerilog scanner for the RTL source view and tracing.
//!
//! This is intentionally not a full parser: it recognises modules, signal
//! declarations, continuous assignments, `always`/`initial` blocks and module
//! instantiations together with their source lines — enough to browse RTL and
//! to trace a signal to its declaration, drivers and loads.

use std::collections::BTreeMap;
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
}

#[derive(Clone, Debug)]
pub struct ModuleDef {
    pub name: String,
    pub file: PathBuf,
    pub start: usize,
    pub end: usize,
    pub ports: Vec<SignalDecl>,
    pub signals: Vec<SignalDecl>,
    pub assigns: Vec<Assign>,
    pub always: Vec<AlwaysBlock>,
    pub instances: Vec<Instance>,
}

impl ModuleDef {
    pub fn signal(&self, name: &str) -> Option<&SignalDecl> {
        self.signals.iter().find(|decl| decl.name == name)
    }
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
        ports: Vec::new(),
        signals: Vec::new(),
        assigns: Vec::new(),
        always: Vec::new(),
        instances: Vec::new(),
    };

    // Optional parameter list, then the port list (ANSI declarations inside).
    if matches!(parser.peek(), Some(Tok::Punct('#'))) {
        parser.next();
        parser.skip_balanced();
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

    while let Some(tok) = parser.peek().cloned() {
        match tok {
            Tok::Ident(word) => match word.as_str() {
                "endmodule" => {
                    module.end = parser.line();
                    parser.next();
                    break;
                }
                "input" | "output" | "inout" | "wire" | "reg" | "logic" | "bit" | "integer"
                | "real" | "time" | "genvar" | "supply0" | "supply1" | "tri" | "triand"
                | "trior" | "wand" | "wor" | "trireg" | "parameter" | "localparam" => {
                    if word == "parameter" || word == "localparam" {
                        parser.skip_statement();
                    } else {
                        parse_declarations(parser, &mut module, false);
                    }
                }
                "assign" => {
                    parser.next();
                    parse_assign(parser, &mut module);
                }
                "always" | "always_ff" | "always_comb" | "always_latch" | "initial" | "final" => {
                    parser.next();
                    parse_process(parser, &mut module);
                }
                "function" => parser.skip_to(&["endfunction"]),
                "task" => parser.skip_to(&["endtask"]),
                "generate" | "endgenerate" | "begin" | "end" => {
                    parser.next();
                }
                "defparam" | "specify" | "endspecify" => {
                    parser.skip_statement();
                }
                _ => {
                    if let Some(instance) = parse_instance(parser, &module) {
                        module.instances.push(instance);
                    } else {
                        parser.next();
                    }
                }
            },
            _ => {
                parser.next();
            }
        }
    }
    Some(module)
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
    if matches!(parser.peek_at(offset), Some(Tok::Punct('#'))) {
        offset += 1;
        if matches!(parser.peek_at(offset), Some(Tok::Punct('('))) {
            // Skip the balanced parameter override.
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
    // Consume up to and including the port list + `;`.
    parser.pos += offset + 1;
    parser.skip_balanced();
    if matches!(parser.peek(), Some(Tok::Punct(';'))) {
        parser.next();
    }
    Some(Instance {
        module: module_type,
        name: instance,
        line,
    })
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
pub(crate) fn parse_text_for_test(text: &str, path: &Path) -> Option<ModuleDef> {
    parse_module_text(text, path).into_iter().next()
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
