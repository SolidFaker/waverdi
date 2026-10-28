// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Tok {
    Ident(String),
    Num(String),
    Str,
    Punct(char),
}

#[derive(Clone, Debug)]
pub(super) struct Token {
    pub(super) tok: Tok,
    pub(super) line: usize,
}

pub(super) fn lex(text: &str) -> Vec<Token> {
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
