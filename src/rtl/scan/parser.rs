use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use super::lexer::{lex, Tok, Token};
use super::model::{
    AlwaysBlock, Assign, BinOp, Body, DefKind, Expr, GenBlock, GenFor, GenIf, Instance, ModuleDef,
    ParamEnv, SignalDecl, TypeDef,
};
use super::resolve::eval;

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    signal_names: HashSet<String>,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            pos: 0,
            signal_names: HashSet::new(),
        }
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

pub(super) fn parse_module_text(text: &str, path: &Path) -> Vec<ModuleDef> {
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
    parser.signal_names.clear();
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

pub(super) fn parse_int(text: &str) -> i64 {
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
        if parser.signal_names.insert(decl.name.clone()) {
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

pub(super) fn width_range(width: u32) -> String {
    format!("{}:0", width.saturating_sub(1))
}

/// Width of a packed range like `7:0` or `WIDTH-1:0`.
pub(super) fn range_width(range: &str, module: &ModuleDef) -> Option<u32> {
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
