use super::{Change, SigKind, Signal, Ticks, Value, Waveform};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;

const MAX_WIDTH: usize = 4096;

#[derive(Clone, Debug)]
enum Expr {
    Literal(Vec<u8>),
    Signal(usize),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Concat(Vec<Expr>),
    Select(Box<Expr>, usize, usize),
}

#[derive(Clone, Copy, Debug)]
enum UnOp {
    Not,
    BitNot,
    Neg,
    Plus,
    And,
    Nand,
    Or,
    Nor,
    Xor,
    Xnor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BinOp {
    And,
    Nand,
    Or,
    Nor,
    Xor,
    Xnor,
    LogicalAnd,
    LogicalOr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Shl,
    Shr,
    Sar,
}

#[derive(Clone)]
pub(crate) struct LogicProgram {
    expr: Expr,
    pub signals: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Literal(Vec<u8>),
    Ref(String),
    Op(&'static str),
}

pub(crate) fn quote_signal(name: &str) -> String {
    format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(crate) fn parse_logic(text: &str, wf: &Waveform) -> Result<LogicProgram, String> {
    let tokens = lex(text)?;
    let mut parser = Parser { tokens, pos: 0, wf };
    let expr = parser.ternary()?;
    if parser.pos != parser.tokens.len() {
        return Err(format!("unexpected token {:?}", parser.tokens[parser.pos]));
    }
    let mut signals = Vec::new();
    collect_signals(&expr, &mut signals);
    signals.sort_unstable();
    signals.dedup();
    Ok(LogicProgram { expr, signals })
}

fn collect_signals(expr: &Expr, out: &mut Vec<usize>) {
    match expr {
        Expr::Literal(_) => {}
        Expr::Signal(index) => out.push(*index),
        Expr::Unary(_, inner) | Expr::Select(inner, _, _) => collect_signals(inner, out),
        Expr::Binary(_, left, right) => {
            collect_signals(left, out);
            collect_signals(right, out);
        }
        Expr::Ternary(condition, left, right) => {
            collect_signals(condition, out);
            collect_signals(left, out);
            collect_signals(right, out);
        }
        Expr::Concat(items) => {
            for item in items {
                collect_signals(item, out);
            }
        }
    }
}

fn lex(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    const OPS: &[&str] = &[
        "<<<", ">>>", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "**", "~&", "~|", "~^", "+",
        "-", "*", "/", "%", "&", "|", "^", "!", "~", "<", ">", "(", ")", "{", "}", "[", "]", ",",
        ":", "?",
    ];
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        if chars[i] == '"' {
            i += 1;
            let mut name = String::new();
            let mut closed = false;
            while i < chars.len() {
                if chars[i] == '"' {
                    i += 1;
                    closed = true;
                    break;
                }
                if chars[i] == '\\' {
                    i += 1;
                    if i == chars.len() {
                        break;
                    }
                }
                name.push(chars[i]);
                i += 1;
            }
            if !closed {
                return Err("unterminated quoted signal name".to_string());
            }
            out.push(Token::Ref(name));
            continue;
        }
        if chars[i].is_ascii_digit()
            || (chars[i] == '\'' && chars.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic()))
        {
            let start = i;
            while i < chars.len()
                && !chars[i].is_whitespace()
                && !matches!(
                    chars[i],
                    '(' | ')'
                        | '{'
                        | '}'
                        | '['
                        | ']'
                        | ','
                        | ':'
                        | '?'
                        | '+'
                        | '-'
                        | '*'
                        | '/'
                        | '%'
                        | '&'
                        | '|'
                        | '^'
                        | '!'
                        | '~'
                        | '<'
                        | '>'
                        | '='
                )
            {
                i += 1;
            }
            let literal: String = chars[start..i].iter().collect();
            out.push(Token::Literal(parse_literal(&literal)?));
            continue;
        }
        if chars[i].is_alphabetic() || matches!(chars[i], '_' | '$') {
            let start = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '$' | '.' | '/'))
            {
                i += 1;
            }
            out.push(Token::Ref(chars[start..i].iter().collect()));
            continue;
        }
        let Some(op) = OPS
            .iter()
            .find(|op| chars[i..].starts_with(op.chars().collect::<Vec<_>>().as_slice()))
        else {
            return Err(format!("unexpected character '{}'", chars[i]));
        };
        out.push(Token::Op(op));
        i += op.chars().count();
    }
    if out.is_empty() {
        return Err("expression is empty".to_string());
    }
    Ok(out)
}

fn parse_literal(text: &str) -> Result<Vec<u8>, String> {
    let (width, base, digits) = match text.split_once('\'') {
        Some((width, rest)) => {
            let width = if width.is_empty() {
                32
            } else {
                width
                    .parse::<usize>()
                    .map_err(|_| format!("invalid literal {text}"))?
            };
            let base = rest
                .chars()
                .next()
                .ok_or_else(|| format!("invalid literal {text}"))?;
            (width, base, rest[1..].replace('_', ""))
        }
        None => {
            let value = text
                .parse::<u128>()
                .map_err(|_| format!("invalid literal {text}"))?;
            return Ok((0..32).map(|bit| ((value >> bit) & 1) as u8).collect());
        }
    };
    if width == 0 || width > MAX_WIDTH {
        return Err(format!("literal width is outside 1..{MAX_WIDTH}"));
    }
    if digits.is_empty() {
        return Err(format!("invalid literal {text}"));
    }
    if matches!(base, 'd' | 'D') && digits.chars().all(|c| matches!(c, 'x' | 'X' | 'z' | 'Z')) {
        return Ok(vec![if digits.contains(['x', 'X']) { 2 } else { 3 }; width]);
    }
    let group = match base {
        'b' | 'B' | 'd' | 'D' => 1,
        'o' | 'O' => 3,
        'h' | 'H' => 4,
        _ => return Err(format!("invalid base in literal {text}")),
    };
    if matches!(base, 'd' | 'D') {
        let value = digits
            .parse::<u128>()
            .map_err(|_| format!("invalid decimal literal {text}"))?;
        return Ok((0..width).map(|bit| ((value >> bit) & 1) as u8).collect());
    }
    let mut groups = Vec::new();
    for c in digits.chars() {
        let value = match c {
            '0'..='9' => c as u8 - b'0',
            'a'..='f' => c as u8 - b'a' + 10,
            'A'..='F' => c as u8 - b'A' + 10,
            'x' | 'X' => 2,
            'z' | 'Z' => 3,
            _ => return Err(format!("invalid digit '{c}' in {text}")),
        };
        let unknown = matches!(c, 'x' | 'X' | 'z' | 'Z');
        if !unknown && value >= (1 << group) {
            return Err(format!("invalid digit '{c}' in {text}"));
        }
        let mut group_bits = Vec::with_capacity(group);
        for bit in 0..group {
            if unknown {
                group_bits.push(value);
            } else {
                group_bits.push((value >> bit) & 1);
            }
        }
        groups.push(group_bits);
    }
    let mut bits: Vec<u8> = groups.into_iter().rev().flatten().collect();
    let fill = bits.last().copied().filter(|&bit| bit >= 2).unwrap_or(0);
    bits.resize(width, fill);
    Ok(bits)
}

struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    wf: &'a Waveform,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn eat(&mut self, expected: &str) -> bool {
        if matches!(self.peek(), Some(Token::Op(text)) if *text == expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: &str) -> Result<(), String> {
        if self.eat(expected) {
            Ok(())
        } else {
            Err(format!("expected '{expected}' at token {}", self.pos + 1))
        }
    }

    fn ternary(&mut self) -> Result<Expr, String> {
        let condition = self.binary(1)?;
        if self.eat("?") {
            let left = self.ternary()?;
            self.expect(":")?;
            let right = self.ternary()?;
            Ok(Expr::Ternary(
                Box::new(condition),
                Box::new(left),
                Box::new(right),
            ))
        } else {
            Ok(condition)
        }
    }

    fn binary(&mut self, min_precedence: u8) -> Result<Expr, String> {
        let mut left = self.unary()?;
        while let Some(Token::Op(text)) = self.peek() {
            let Some((op, precedence)) = binary_operator(text) else {
                break;
            };
            if precedence < min_precedence {
                break;
            }
            self.pos += 1;
            let right = self.binary(if op == BinOp::Pow {
                precedence
            } else {
                precedence + 1
            })?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if let Some(Token::Op(text)) = self.peek() {
            let op = match *text {
                "!" => Some(UnOp::Not),
                "~" => Some(UnOp::BitNot),
                "-" => Some(UnOp::Neg),
                "+" => Some(UnOp::Plus),
                "&" => Some(UnOp::And),
                "~&" => Some(UnOp::Nand),
                "|" => Some(UnOp::Or),
                "~|" => Some(UnOp::Nor),
                "^" => Some(UnOp::Xor),
                "~^" => Some(UnOp::Xnor),
                _ => None,
            };
            if let Some(op) = op {
                self.pos += 1;
                return Ok(Expr::Unary(op, Box::new(self.unary()?)));
            }
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, String> {
        let mut expr = match self.peek().cloned() {
            Some(Token::Literal(bits)) => {
                self.pos += 1;
                Expr::Literal(bits)
            }
            Some(Token::Ref(name)) => {
                self.pos += 1;
                let path = name.strip_prefix('/').unwrap_or(&name);
                let index = resolve_signal(self.wf, path)
                    .ok_or_else(|| format!("signal not found: {path}"))?;
                Expr::Signal(index)
            }
            Some(Token::Op("(")) => {
                self.pos += 1;
                let expr = self.ternary()?;
                self.expect(")")?;
                expr
            }
            Some(Token::Op("{")) => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    if self.eat("}") {
                        break;
                    }
                    items.push(self.ternary()?);
                    if self.eat("}") {
                        break;
                    }
                    self.expect(",")?;
                }
                if items.is_empty() {
                    return Err("empty concatenation".to_string());
                }
                Expr::Concat(items)
            }
            other => return Err(format!("expected a value or signal, got {other:?}")),
        };
        while self.eat("[") {
            let high = self.selector_number()?;
            let low = if self.eat(":") {
                self.selector_number()?
            } else {
                high
            };
            self.expect("]")?;
            if high < low {
                return Err("descending bit range is required".to_string());
            }
            expr = Expr::Select(Box::new(expr), high, low);
        }
        Ok(expr)
    }

    fn selector_number(&mut self) -> Result<usize, String> {
        let Some(Token::Literal(bits)) = self.peek().cloned() else {
            return Err("bit selectors must be decimal constants".to_string());
        };
        self.pos += 1;
        bits_to_usize(&bits).ok_or_else(|| "invalid bit selector".to_string())
    }
}

fn resolve_signal(wf: &Waveform, path: &str) -> Option<usize> {
    let (scope_text, leaf) = path.rsplit_once('.').unwrap_or(("", path));
    let scope: Vec<String> = scope_text
        .split('.')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect();
    wf.signals.iter().position(|signal| {
        signal.scope == scope && crate::rtl::scan::signal_name_matches(&signal.name, leaf)
    })
}

fn binary_operator(text: &str) -> Option<(BinOp, u8)> {
    Some(match text {
        "||" => (BinOp::LogicalOr, 1),
        "&&" => (BinOp::LogicalAnd, 2),
        "|" => (BinOp::Or, 3),
        "~|" => (BinOp::Nor, 3),
        "^" => (BinOp::Xor, 4),
        "~^" => (BinOp::Xnor, 4),
        "&" => (BinOp::And, 5),
        "~&" => (BinOp::Nand, 5),
        "==" => (BinOp::Eq, 6),
        "!=" => (BinOp::Ne, 6),
        "<" => (BinOp::Lt, 7),
        "<=" => (BinOp::Le, 7),
        ">" => (BinOp::Gt, 7),
        ">=" => (BinOp::Ge, 7),
        "<<" | "<<<" => (BinOp::Shl, 8),
        ">>" => (BinOp::Shr, 8),
        ">>>" => (BinOp::Sar, 8),
        "+" => (BinOp::Add, 9),
        "-" => (BinOp::Sub, 9),
        "*" => (BinOp::Mul, 10),
        "/" => (BinOp::Div, 10),
        "%" => (BinOp::Mod, 10),
        "**" => (BinOp::Pow, 11),
        _ => return None,
    })
}

struct EvalSignal<'a> {
    signal: &'a Signal,
    next: usize,
}

struct EvalContext<'a> {
    signals: HashMap<usize, EvalSignal<'a>>,
}

impl<'a> EvalContext<'a> {
    fn new(wf: &'a Waveform, indexes: &[usize]) -> Result<Self, String> {
        let mut signals = HashMap::new();
        for &index in indexes {
            let signal = wf
                .signals
                .get(index)
                .ok_or_else(|| "invalid signal index".to_string())?;
            if signal.kind != SigKind::Bits {
                return Err(format!("{} is not a bit signal", signal.full_name()));
            }
            signals.insert(index, EvalSignal { signal, next: 0 });
        }
        Ok(Self { signals })
    }

    fn value(&mut self, index: usize, time: Ticks) -> Vec<u8> {
        let state = self.signals.get_mut(&index).unwrap();
        while state
            .signal
            .changes
            .get(state.next)
            .is_some_and(|change| change.t <= time)
        {
            state.next += 1;
        }
        let Some(change) = state
            .next
            .checked_sub(1)
            .map(|index| &state.signal.changes[index])
        else {
            return vec![2; state.signal.bits.max(1) as usize];
        };
        value_bits(&change.v, state.signal.bits as usize)
    }
}

pub(crate) fn evaluate_logic(
    program: &LogicProgram,
    wf: &Waveform,
    start: Ticks,
    end: Ticks,
) -> Result<Arc<Vec<Change>>, String> {
    if start > end {
        return Err("invalid expression time range".to_string());
    }
    let mut context = EvalContext::new(wf, &program.signals)?;
    let mut heap = BinaryHeap::new();
    for (order, &index) in program.signals.iter().enumerate() {
        if !wf.signals[index].changes.is_empty() {
            heap.push(Reverse((wf.signals[index].changes[0].t, order, 0usize)));
        }
    }
    let mut out = Vec::new();
    let mut time = start;
    loop {
        let value = Value::compact(eval(&program.expr, &mut context, time)?);
        if out
            .last()
            .map(|change: &Change| change.v != value)
            .unwrap_or(true)
        {
            out.push(Change { t: time, v: value });
        }
        let Some(Reverse((next, _, _))) = heap.peek().copied() else {
            break;
        };
        if next > end {
            break;
        }
        time = next;
        while heap
            .peek()
            .is_some_and(|Reverse((time, _, _))| *time == next)
        {
            let Reverse((_, order, change)) = heap.pop().unwrap();
            let index = program.signals[order];
            let next_change = change + 1;
            if let Some(change) = wf.signals[index].changes.get(next_change) {
                heap.push(Reverse((change.t, order, next_change)));
            }
        }
    }
    Ok(Arc::new(out))
}

fn eval(expr: &Expr, context: &mut EvalContext, time: Ticks) -> Result<Vec<u8>, String> {
    match expr {
        Expr::Literal(bits) => Ok(bits.clone()),
        Expr::Signal(index) => Ok(context.value(*index, time)),
        Expr::Unary(op, inner) => {
            let value = eval(inner, context, time)?;
            match op {
                UnOp::Not => Ok(vec![match truth(&value) {
                    Some(value) => u8::from(!value),
                    None => 2,
                }]),
                UnOp::BitNot => Ok(value
                    .into_iter()
                    .map(|bit| match bit {
                        0 => 1,
                        1 => 0,
                        other => other,
                    })
                    .collect()),
                UnOp::Neg => Ok(sub(&vec![0; value.len()], &value)),
                UnOp::Plus => Ok(value),
                UnOp::And | UnOp::Nand | UnOp::Or | UnOp::Nor | UnOp::Xor | UnOp::Xnor => {
                    if has_unknown(&value) {
                        return Ok(vec![2]);
                    }
                    let (one, invert) = match op {
                        UnOp::And => (value.iter().all(|&bit| bit == 1), false),
                        UnOp::Nand => (value.iter().all(|&bit| bit == 1), true),
                        UnOp::Or => (value.contains(&1), false),
                        UnOp::Nor => (value.contains(&1), true),
                        UnOp::Xor => (
                            value.iter().filter(|&&bit| bit == 1).count() % 2 == 1,
                            false,
                        ),
                        UnOp::Xnor => {
                            (value.iter().filter(|&&bit| bit == 1).count() % 2 == 1, true)
                        }
                        _ => unreachable!(),
                    };
                    Ok(vec![u8::from(if invert { !one } else { one })])
                }
            }
        }
        Expr::Binary(op, left, right) => {
            let left = eval(left, context, time)?;
            let right = eval(right, context, time)?;
            binary(*op, &left, &right)
        }
        Expr::Ternary(condition, left, right) => {
            let condition = eval(condition, context, time)?;
            match truth(&condition) {
                Some(true) => eval(left, context, time),
                Some(false) => eval(right, context, time),
                None => {
                    let left = eval(left, context, time)?;
                    let right = eval(right, context, time)?;
                    let width = checked_width(left.len().max(right.len()))?;
                    let left = resize(&left, width);
                    let right = resize(&right, width);
                    Ok((0..width)
                        .map(|bit| {
                            if left[bit] == right[bit] {
                                left[bit]
                            } else {
                                2
                            }
                        })
                        .collect())
                }
            }
        }
        Expr::Concat(items) => {
            let mut out = Vec::new();
            for item in items.iter().rev() {
                out.extend(eval(item, context, time)?);
            }
            checked_width(out.len())?;
            Ok(out)
        }
        Expr::Select(inner, high, low) => {
            let value = eval(inner, context, time)?;
            Ok((*low..=*high)
                .map(|bit| value.get(bit).copied().unwrap_or(2))
                .collect())
        }
    }
}

fn value_bits(value: &Value, width: usize) -> Vec<u8> {
    match value {
        Value::Small(packed, size) => (0..*size as usize)
            .map(|bit| ((packed >> (2 * bit)) & 3) as u8)
            .collect(),
        Value::Bits(bits) => bits.clone(),
        _ => vec![2; width.max(1)],
    }
}

fn checked_width(width: usize) -> Result<usize, String> {
    if width > MAX_WIDTH {
        Err(format!("expression result exceeds {MAX_WIDTH} bits"))
    } else {
        Ok(width)
    }
}

fn resize(bits: &[u8], width: usize) -> Vec<u8> {
    let mut out = vec![0; width];
    let count = width.min(bits.len());
    out[..count].copy_from_slice(&bits[..count]);
    out
}

fn has_unknown(bits: &[u8]) -> bool {
    bits.iter().any(|&bit| bit >= 2)
}

fn truth(bits: &[u8]) -> Option<bool> {
    if has_unknown(bits) {
        None
    } else {
        Some(bits.contains(&1))
    }
}

fn binary(op: BinOp, left: &[u8], right: &[u8]) -> Result<Vec<u8>, String> {
    let width = checked_width(left.len().max(right.len()))?;
    match op {
        BinOp::And | BinOp::Nand | BinOp::Or | BinOp::Nor | BinOp::Xor | BinOp::Xnor => {
            let left = resize(left, width);
            let right = resize(right, width);
            let mut out = Vec::with_capacity(width);
            for bit in 0..width {
                let value = match (left[bit], right[bit]) {
                    (0, 0) => match op {
                        BinOp::And | BinOp::Or | BinOp::Xor => 0,
                        _ => 1,
                    },
                    (1, 1) => match op {
                        BinOp::And | BinOp::Xor => 1,
                        _ => 0,
                    },
                    (0, 1) | (1, 0) => match op {
                        BinOp::And => 0,
                        BinOp::Nand => 1,
                        BinOp::Or | BinOp::Xor => 1,
                        BinOp::Nor | BinOp::Xnor => 0,
                        _ => unreachable!(),
                    },
                    (0, _) => match op {
                        BinOp::And => 0,
                        BinOp::Nand => 1,
                        BinOp::Or => 2,
                        BinOp::Nor => 2,
                        _ => 2,
                    },
                    (_, 0) => match op {
                        BinOp::And => 0,
                        BinOp::Nand => 1,
                        BinOp::Or => 2,
                        BinOp::Nor => 2,
                        _ => 2,
                    },
                    (1, _) => match op {
                        BinOp::And => 2,
                        BinOp::Nand => 2,
                        BinOp::Or => 1,
                        BinOp::Nor | BinOp::Xor | BinOp::Xnor => 2,
                        _ => unreachable!(),
                    },
                    (_, 1) => match op {
                        BinOp::And => 2,
                        BinOp::Nand => 2,
                        BinOp::Or => 1,
                        BinOp::Nor | BinOp::Xor | BinOp::Xnor => 2,
                        _ => unreachable!(),
                    },
                    _ => 2,
                };
                out.push(value);
            }
            Ok(out)
        }
        BinOp::LogicalAnd => Ok(vec![match (truth(left), truth(right)) {
            (Some(false), _) | (_, Some(false)) => 0,
            (Some(true), Some(true)) => 1,
            _ => 2,
        }]),
        BinOp::LogicalOr => Ok(vec![match (truth(left), truth(right)) {
            (Some(true), _) | (_, Some(true)) => 1,
            (Some(false), Some(false)) => 0,
            _ => 2,
        }]),
        BinOp::Eq | BinOp::Ne => {
            if has_unknown(left) || has_unknown(right) {
                return Ok(vec![2]);
            }
            let equal = resize(left, width) == resize(right, width);
            Ok(vec![u8::from(if op == BinOp::Eq { equal } else { !equal })])
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            if has_unknown(left) || has_unknown(right) {
                return Ok(vec![2]);
            }
            let left = resize(left, width);
            let right = resize(right, width);
            let ordering = left.iter().rev().cmp(right.iter().rev());
            let value = match op {
                BinOp::Lt => ordering.is_lt(),
                BinOp::Le => ordering.is_le(),
                BinOp::Gt => ordering.is_gt(),
                BinOp::Ge => ordering.is_ge(),
                _ => unreachable!(),
            };
            Ok(vec![u8::from(value)])
        }
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
            if has_unknown(left) || has_unknown(right) {
                return Ok(vec![2; width]);
            }
            let left = resize(left, width);
            let right = resize(right, width);
            match op {
                BinOp::Add => Ok(add(&left, &right)),
                BinOp::Sub => Ok(sub(&left, &right)),
                BinOp::Mul => Ok(mul(&left, &right)),
                BinOp::Div | BinOp::Mod => {
                    if right.iter().all(|&bit| bit == 0) {
                        return Ok(vec![2; width]);
                    }
                    let (quotient, remainder) = divmod(&left, &right);
                    Ok(if op == BinOp::Div {
                        quotient
                    } else {
                        remainder
                    })
                }
                BinOp::Pow => {
                    let Some(exponent) = bits_to_usize(&right) else {
                        return Ok(vec![2; width]);
                    };
                    if exponent > 64 {
                        return Ok(vec![2; width]);
                    }
                    let mut result = vec![1];
                    let mut base = left;
                    let mut exponent = exponent;
                    while exponent > 0 {
                        if exponent & 1 == 1 {
                            result = resize(&mul(&result, &base), width);
                        }
                        exponent >>= 1;
                        if exponent > 0 {
                            base = resize(&mul(&base, &base), width);
                        }
                    }
                    Ok(resize(&result, width))
                }
                _ => unreachable!(),
            }
        }
        BinOp::Shl | BinOp::Shr | BinOp::Sar => {
            if has_unknown(right) {
                return Ok(vec![2; left.len()]);
            }
            let Some(amount) = bits_to_usize(right) else {
                return Ok(vec![2; left.len()]);
            };
            if op == BinOp::Shl {
                let mut out = vec![0; left.len()];
                if amount < left.len() {
                    out[amount..].copy_from_slice(&left[..left.len() - amount]);
                }
                Ok(out)
            } else {
                let sign = if op == BinOp::Sar {
                    left.last().copied().unwrap_or(0)
                } else {
                    0
                };
                let mut out = vec![sign; left.len()];
                let start = amount.min(left.len());
                let count = left.len().saturating_sub(start);
                out[..count].copy_from_slice(&left[start..start + count]);
                Ok(out)
            }
        }
    }
}

fn add(left: &[u8], right: &[u8]) -> Vec<u8> {
    let width = left.len().max(right.len());
    let left = resize(left, width);
    let right = resize(right, width);
    let mut out = Vec::with_capacity(width);
    let mut carry = 0u8;
    for (&left, &right) in left.iter().zip(right.iter()) {
        let sum = left + right + carry;
        out.push(sum & 1);
        carry = sum >> 1;
    }
    out
}

fn sub(left: &[u8], right: &[u8]) -> Vec<u8> {
    let width = left.len().max(right.len());
    let left = resize(left, width);
    let mut borrow = 0u8;
    let mut out = Vec::with_capacity(width);
    for (bit, &a) in left.iter().enumerate().take(width) {
        let b = right.get(bit).copied().unwrap_or(0);
        out.push(a ^ b ^ borrow);
        borrow = u8::from((a == 0 && (b == 1 || borrow == 1)) || (b == 1 && borrow == 1));
    }
    out
}

fn mul(left: &[u8], right: &[u8]) -> Vec<u8> {
    let width = left.len().max(right.len());
    let mut out = vec![0; width];
    for (shift, &bit) in right.iter().enumerate().take(width) {
        if bit == 0 {
            continue;
        }
        let mut shifted = vec![0; width];
        let count = width.min(left.len() + shift) - shift;
        shifted[shift..shift + count].copy_from_slice(&left[..count]);
        out = add(&out, &shifted);
    }
    out
}

fn divmod(left: &[u8], right: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let width = left.len();
    if right.len() > width {
        return (vec![0; width], left.to_vec());
    }
    let divisor_width = right
        .iter()
        .rposition(|&bit| bit != 0)
        .map(|index| index + 1)
        .unwrap_or(0);
    if divisor_width == 0 {
        return (vec![0; width], left.to_vec());
    }
    let mut quotient = vec![0; width];
    let mut remainder = left.to_vec();
    for shift in (0..=width - divisor_width).rev() {
        let mut divisor = vec![0; width];
        divisor[shift..shift + divisor_width].copy_from_slice(&right[..divisor_width]);
        if divisor.iter().rev().cmp(remainder.iter().rev()).is_le() {
            remainder = sub(&remainder, &divisor);
            quotient[shift] = 1;
        }
    }
    (quotient, remainder)
}

fn bits_to_usize(bits: &[u8]) -> Option<usize> {
    if has_unknown(bits) || bits.len() > usize::BITS as usize {
        return None;
    }
    bits.iter().rev().try_fold(0usize, |value, &bit| {
        value.checked_shl(1)?.checked_add(bit as usize)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcd;

    const VCD: &str = "$timescale 1ns $end\n\
        $scope module top $end\n\
        $var wire 1 ! a $end\n\
        $var wire 1 \" b $end\n\
        $var wire 4 # data [3:0] $end\n\
        $upscope $end\n$enddefinitions $end\n\
        #0\n0!\n0\"\nb0101 #\n\
        #10\n1!\nb1010 #\n\
        #20\n0\"\nb0011 #\n";

    fn waveform() -> Waveform {
        let mut wf = vcd::parse_bytes(VCD.as_bytes()).unwrap().wf;
        wf.signals[2].name = "data[3:0]".to_string();
        wf
    }

    fn evaluate(text: &str, time: u64) -> Vec<u8> {
        let wf = waveform();
        let program = parse_logic(text, &wf).unwrap();
        let mut context = EvalContext::new(&wf, &program.signals).unwrap();
        eval(&program.expr, &mut context, time).unwrap()
    }

    #[test]
    fn precedence_paths_and_concatenation_work() {
        assert_eq!(evaluate("top.a | top.b & 1'b1", 10), vec![1]);
        assert_eq!(
            evaluate("{top.a, top.b, top.data}", 10),
            vec![0, 1, 0, 1, 0, 1]
        );
        assert_eq!(evaluate("top.data[3:2]", 10), vec![0, 1]);
        assert_eq!(
            evaluate("top.data ? 4'b0011 : 4'b1000", 10),
            vec![1, 1, 0, 0]
        );
        assert_eq!(evaluate("top.a ? 4'b0011 : 4'b1000", 0), vec![0, 0, 0, 1]);
    }

    #[test]
    fn arithmetic_shifts_and_comparisons_work() {
        assert_eq!(
            binary(BinOp::Div, &[1, 0, 1, 0], &[0, 1]).unwrap(),
            vec![0, 1, 0, 0]
        );
        assert_eq!(
            divmod(&[1, 0, 1, 0], &[0, 1]),
            (vec![0, 1, 0, 0], vec![1, 0, 0, 0])
        );
        assert_eq!(evaluate("4'b0010 / 4'b0010", 0), vec![1, 0, 0, 0]);
        assert_eq!(evaluate("top.data + 4'b0001", 0), vec![0, 1, 1, 0]);
        assert_eq!(evaluate("top.data * 4'b0010", 0), vec![0, 1, 0, 1]);
        assert_eq!(evaluate("top.data / 4'b0010", 0), vec![0, 1, 0, 0]);
        assert_eq!(evaluate("top.data % 4'b0010", 0), vec![1, 0, 0, 0]);
        assert_eq!(evaluate("4'b0010 ** 4'b0011", 0), vec![0, 0, 0, 1]);
        assert_eq!(evaluate("top.data << 2", 0), vec![0, 0, 1, 0]);
        assert_eq!(evaluate("top.data >> 2", 0), vec![1, 0, 0, 0]);
        assert_eq!(evaluate("top.data > 4'b0100", 0), vec![1]);
        assert_eq!(evaluate("top.data != 4'b0101", 0), vec![0]);
    }

    #[test]
    fn verilog_literals_support_unsized_and_sized_forms() {
        let expected = (0..32)
            .map(|bit| ((0x5844u32 >> bit) & 1) as u8)
            .collect::<Vec<_>>();
        assert_eq!(evaluate("'h5844", 0), expected);
        assert_eq!(
            evaluate("32'h01", 0),
            std::iter::once(1)
                .chain(std::iter::repeat_n(0, 31))
                .collect::<Vec<_>>()
        );
        assert_eq!(evaluate("4'd1", 0), vec![1, 0, 0, 0]);
        assert_eq!(evaluate("4'd2", 0), vec![0, 1, 0, 0]);
        assert_eq!(evaluate("4'd12", 0), vec![0, 0, 1, 1]);
        assert!(parse_logic("4'b2", &waveform()).is_err());
        assert_eq!(
            evaluate("'d1", 0),
            std::iter::once(1)
                .chain(std::iter::repeat_n(0, 31))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            evaluate("'b10", 0),
            std::iter::once(0)
                .chain(std::iter::once(1))
                .chain(std::iter::repeat_n(0, 30))
                .collect::<Vec<_>>()
        );
        assert_eq!(evaluate("'hX", 0), vec![2; 32]);
        assert_eq!(evaluate("'hZ", 0), vec![3; 32]);
        assert!(parse_logic("'hG", &waveform()).is_err());
    }

    #[test]
    fn unknown_values_follow_logic_rules() {
        assert_eq!(
            binary(BinOp::Or, &[2], &[0, 1, 0, 1]).unwrap(),
            vec![2, 1, 0, 1]
        );
        assert_eq!(evaluate("1'bx", 0), vec![2]);
        assert_eq!(evaluate("1'b1 ~& 1'bx", 0), vec![2]);
        assert_eq!(evaluate("1'b0 ~| 1'bx", 0), vec![2]);
        assert_eq!(evaluate("top.a & top.data", 10), vec![0, 0, 0, 0]);
        assert_eq!(evaluate("4'bxxxx | top.data", 0), vec![1, 2, 1, 2]);
        assert_eq!(evaluate("top.a && top.data", 10), vec![1]);
        assert_eq!(evaluate("top.a == 1'bx", 0), vec![2]);
        assert_eq!(evaluate("1'bx ? top.data : top.b", 0), vec![2, 0, 2, 0]);
    }

    #[test]
    fn full_program_merges_changes_without_duplicates() {
        let wf = waveform();
        let program = parse_logic("top.a ^ top.b", &wf).unwrap();
        let changes = evaluate_logic(&program, &wf, 0, 20).unwrap();
        assert_eq!(
            changes.iter().map(|change| change.t).collect::<Vec<_>>(),
            vec![0, 10]
        );
        assert_eq!(changes[0].v, Value::compact(vec![0]));
        assert_eq!(changes[1].v, Value::compact(vec![1]));
    }
}
