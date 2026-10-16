//! Expressions the debugger evaluates, as IntelliJ's Evaluate Expression,
//! Watches and breakpoint conditions take them: a Java / Kotlin subset of
//! literals, names (`x`, `this`), fields and properties (`p.x`,
//! `list.size`), array elements (`a[i]`), method calls (`s.length()`,
//! `Math.max(a, b)`), arithmetic, comparisons, `&&` `||` `!`, string
//! concatenation and parentheses.

/// A parsed expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Int(i64),
    /// An `L`-suffixed or too-large integer.
    Long(i64),
    Double(f64),
    Bool(bool),
    Char(char),
    Str(String),
    Null,
    This,
    /// A local variable, a field of `this`, or a class (`Math`).
    Name(String),
    /// `target.name`: a field, a Kotlin property, a class in a package.
    Field(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    /// `target.name(args)`; no target calls a method of `this`.
    Call(Option<Box<Expr>>, String, Vec<Expr>),
    Unary(UnaryOp, Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Int(i64, bool),
    Double(f64),
    Str(String),
    Char(char),
    Ident(String),
    Op(&'static str),
}

const OPS: &[&str] = &["&&", "||", "==", "!=", "<=", ">=", "+", "-", "*", "/", "%", "<", ">", "!", "(", ")", "[", "]", ".", ",", "?"];

fn lex(text: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_') {
                // `1.toString()` isn't a number's dot.
                if chars[i] == '.' && !chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
                    break;
                }
                i += 1;
            }
            let word: String = chars[start..i].iter().filter(|c| **c != '_').collect();
            let lower = word.to_ascii_lowercase();
            if let Some(hex) = lower.strip_prefix("0x") {
                let (digits, long) = match hex.strip_suffix('l') {
                    Some(d) => (d, true),
                    None => (hex, false),
                };
                out.push(Tok::Int(i64::from_str_radix(digits, 16).map_err(|_| format!("Invalid number: {word}"))?, long));
            } else if lower.contains('.') || lower.ends_with('f') || lower.ends_with('d') || lower.contains('e') {
                let digits = lower.trim_end_matches(['f', 'd']);
                out.push(Tok::Double(digits.parse().map_err(|_| format!("Invalid number: {word}"))?));
            } else {
                let (digits, long) = match lower.strip_suffix('l') {
                    Some(d) => (d, true),
                    None => (lower.as_str(), false),
                };
                out.push(Tok::Int(digits.parse().map_err(|_| format!("Invalid number: {word}"))?, long));
            }
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
        } else if c == '"' || c == '\'' {
            i += 1;
            let mut s = String::new();
            loop {
                let Some(&ch) = chars.get(i) else { return Err("Unterminated literal".into()) };
                i += 1;
                if ch == c {
                    break;
                }
                if ch == '\\' {
                    let Some(&e) = chars.get(i) else { return Err("Unterminated literal".into()) };
                    i += 1;
                    s.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '0' => '\0',
                        other => other,
                    });
                } else {
                    s.push(ch);
                }
            }
            if c == '"' {
                out.push(Tok::Str(s));
            } else {
                let mut it = s.chars();
                match (it.next(), it.next()) {
                    (Some(ch), None) => out.push(Tok::Char(ch)),
                    _ => return Err("Invalid character literal".into()),
                }
            }
        } else if let Some(op) = OPS.iter().find(|op| chars[i..].iter().take(op.len()).collect::<String>() == **op) {
            out.push(Tok::Op(op));
            i += op.len();
        } else {
            return Err(format!("Unexpected '{c}'"));
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }

    fn eat(&mut self, op: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Op(o)) if *o == op) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, op: &str) -> Result<(), String> {
        if self.eat(op) { Ok(()) } else { Err(format!("'{op}' expected")) }
    }

    fn binary(&mut self, level: usize) -> Result<Expr, String> {
        // Lowest first.
        const LEVELS: &[&[(&str, BinaryOp)]] = &[
            &[("||", BinaryOp::Or)],
            &[("&&", BinaryOp::And)],
            &[("==", BinaryOp::Eq), ("!=", BinaryOp::Ne)],
            &[("<=", BinaryOp::Le), (">=", BinaryOp::Ge), ("<", BinaryOp::Lt), (">", BinaryOp::Gt)],
            &[("+", BinaryOp::Add), ("-", BinaryOp::Sub)],
            &[("*", BinaryOp::Mul), ("/", BinaryOp::Div), ("%", BinaryOp::Rem)],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        'more: loop {
            for (op, kind) in LEVELS[level] {
                if self.eat(op) {
                    let right = self.binary(level + 1)?;
                    left = Expr::Binary(*kind, Box::new(left), Box::new(right));
                    continue 'more;
                }
            }
            return Ok(left);
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat("-") {
            return Ok(match self.unary()? {
                Expr::Int(n) => Expr::Int(-n),
                Expr::Long(n) => Expr::Long(-n),
                Expr::Double(f) => Expr::Double(-f),
                e => Expr::Unary(UnaryOp::Neg, Box::new(e)),
            });
        }
        if self.eat("!") {
            return Ok(Expr::Unary(UnaryOp::Not, Box::new(self.unary()?)));
        }
        self.postfix()
    }

    fn args(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        if self.eat(")") {
            return Ok(args);
        }
        loop {
            args.push(self.binary(0)?);
            if self.eat(")") {
                return Ok(args);
            }
            self.expect(",")?;
        }
    }

    fn postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.primary()?;
        loop {
            // Kotlin's safe call reads as a plain one here.
            if self.eat("?") {
                self.expect(".")?;
            } else if !self.eat(".") {
                if self.eat("[") {
                    let index = self.binary(0)?;
                    self.expect("]")?;
                    e = Expr::Index(Box::new(e), Box::new(index));
                    continue;
                }
                return Ok(e);
            }
            let Some(Tok::Ident(name)) = self.peek().cloned() else { return Err("Identifier expected".into()) };
            self.at += 1;
            if self.eat("(") {
                e = Expr::Call(Some(Box::new(e)), name, self.args()?);
            } else {
                e = Expr::Field(Box::new(e), name);
            }
        }
    }

    fn primary(&mut self) -> Result<Expr, String> {
        let Some(tok) = self.peek().cloned() else { return Err("Expression expected".into()) };
        self.at += 1;
        Ok(match tok {
            Tok::Int(n, long) => {
                if long || i32::try_from(n).is_err() {
                    Expr::Long(n)
                } else {
                    Expr::Int(n)
                }
            }
            Tok::Double(f) => Expr::Double(f),
            Tok::Str(s) => Expr::Str(s),
            Tok::Char(c) => Expr::Char(c),
            Tok::Ident(word) => match word.as_str() {
                "true" => Expr::Bool(true),
                "false" => Expr::Bool(false),
                "null" => Expr::Null,
                "this" => Expr::This,
                _ if self.eat("(") => Expr::Call(None, word, self.args()?),
                _ => Expr::Name(word),
            },
            Tok::Op("(") => {
                let e = self.binary(0)?;
                self.expect(")")?;
                e
            }
            Tok::Op(op) => return Err(format!("Unexpected '{op}'")),
        })
    }
}

/// Parses an expression; IntelliJ-like messages for what it can't read.
pub fn parse(text: &str) -> Result<Expr, String> {
    let toks = lex(text)?;
    if toks.is_empty() {
        return Err("Expression expected".into());
    }
    let mut parser = Parser { toks, at: 0 };
    let e = parser.binary(0)?;
    match parser.peek() {
        None => Ok(e),
        Some(Tok::Op(op)) => Err(format!("Unexpected '{op}'")),
        Some(_) => Err("';' expected".into()),
    }
}

/// What Evaluate Expression (Alt+F8) starts with when nothing is
/// selected, as IntelliJ's `getExpressionRangeAtOffset`: the reference at
/// the caret with its qualifiers (`point.x` on `x`, `a.b` on `b`), a call
/// with its arguments (`list.get(i)` on `get`), an element with its index.
/// None off a name, or on a keyword other than `this`.
pub fn expression_at(text: &str, offset: usize) -> Option<std::ops::Range<usize>> {
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let offset = offset.min(text.len());
    // The word at the caret, or the one just before it.
    let mut start = offset;
    while let Some(c) = text[..start].chars().next_back().filter(|c| word(*c)) {
        start -= c.len_utf8();
    }
    let mut end = offset;
    while let Some(c) = text[end..].chars().next().filter(|c| word(*c)) {
        end += c.len_utf8();
    }
    let name = &text[start..end];
    const KEYWORDS: &[&str] = &[
        "if", "else", "for", "while", "do", "when", "return", "val", "var", "fun", "class", "new", "true", "false", "null", "is", "in", "as", "try", "catch",
        "throw", "public", "private", "protected", "static", "final", "void", "int", "long", "boolean", "double", "float", "char", "byte", "short",
    ];
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) || KEYWORDS.contains(&name) {
        return None;
    }
    // Qualifiers: `a.`, `a?.`, `f(x).`, `a[i].` before the name.
    loop {
        let before = text[..start].trim_end_matches([' ', '\t']);
        let Some(dot) = before.strip_suffix('.') else { break };
        let dot = dot.strip_suffix('?').unwrap_or(dot);
        let mut q = dot.trim_end_matches([' ', '\t']).len();
        // Balanced groups after the qualifier's name.
        while let Some(close) = text[..q].chars().next_back().filter(|c| *c == ')' || *c == ']') {
            let open = if close == ')' { '(' } else { '[' };
            let mut depth = 0;
            let mut at = None;
            for (j, c) in text[..q].char_indices().rev() {
                if c == close {
                    depth += 1;
                } else if c == open {
                    depth -= 1;
                    if depth == 0 {
                        at = Some(j);
                        break;
                    }
                }
            }
            q = at?;
        }
        let mut s = q;
        while let Some(c) = text[..s].chars().next_back().filter(|c| word(*c)) {
            s -= c.len_utf8();
        }
        if s == q || text[s..q].starts_with(|c: char| c.is_ascii_digit()) {
            break;
        }
        start = s;
    }
    // A call's arguments, an element's index after the name.
    while let Some(open) = text[end..].chars().next().filter(|c| *c == '(' || *c == '[') {
        let close = if open == '(' { ')' } else { ']' };
        let mut depth = 0;
        let mut at = None;
        for (j, c) in text[end..].char_indices() {
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    at = Some(end + j + 1);
                    break;
                }
            }
            if c == '\n' && depth > 0 && j > 200 {
                break;
            }
        }
        match at {
            Some(e) => end = e,
            None => break,
        }
    }
    Some(start..end)
}

#[cfg(test)]
mod tests {

    #[test]
    fn finds_the_expression_at_the_caret() {
        let at = |text: &str, marker: &str| {
            let offset = text.find(marker).unwrap();
            expression_at(text, offset).map(|r| text[r].to_owned())
        };
        let text = "val d = point.x + list.get(i).size * a[2].length";
        assert_eq!(at(text, "x +").as_deref(), Some("point.x"));
        assert_eq!(at(text, "point").as_deref(), Some("point"));
        assert_eq!(at(text, "get").as_deref(), Some("list.get(i)"));
        assert_eq!(at(text, "size").as_deref(), Some("list.get(i).size"));
        assert_eq!(at(text, "length").as_deref(), Some("a[2].length"));
        assert_eq!(at("user?.name ?: \"\"", "name").as_deref(), Some("user?.name"));
        assert_eq!(at("this.total += n", "total").as_deref(), Some("this.total"));
        // Just after a name; on a keyword or a number.
        assert_eq!(expression_at("count;", 5).map(|r| r.end), Some(5));
        assert_eq!(at("return x", "return"), None);
        assert_eq!(at("1.5 + y", "5"), None);
    }
    use super::*;

    fn name(n: &str) -> Box<Expr> {
        Box::new(Expr::Name(n.into()))
    }

    #[test]
    fn parses_precedence() {
        assert_eq!(
            parse("a + b * 2 > 3 && !done").unwrap(),
            Expr::Binary(
                BinaryOp::And,
                Box::new(Expr::Binary(BinaryOp::Gt, Box::new(Expr::Binary(BinaryOp::Add, name("a"), Box::new(Expr::Binary(BinaryOp::Mul, name("b"), Box::new(Expr::Int(2)))))), Box::new(Expr::Int(3)))),
                Box::new(Expr::Unary(UnaryOp::Not, name("done")))
            )
        );
        assert_eq!(parse("(1 - 2) - 3").unwrap(), Expr::Binary(BinaryOp::Sub, Box::new(Expr::Binary(BinaryOp::Sub, Box::new(Expr::Int(1)), Box::new(Expr::Int(2)))), Box::new(Expr::Int(3))));
    }

    #[test]
    fn parses_access_and_calls() {
        assert_eq!(
            parse("this.items.get(i + 1).name").unwrap(),
            Expr::Field(
                Box::new(Expr::Call(Some(Box::new(Expr::Field(Box::new(Expr::This), "items".into()))), "get".into(), vec![Expr::Binary(BinaryOp::Add, name("i"), Box::new(Expr::Int(1)))])),
                "name".into()
            )
        );
        assert_eq!(parse("arr[0].length").unwrap(), Expr::Field(Box::new(Expr::Index(name("arr"), Box::new(Expr::Int(0)))), "length".into()));
        assert_eq!(parse("Math.max(a, -2)").unwrap(), Expr::Call(Some(name("Math")), "max".into(), vec![Expr::Name("a".into()), Expr::Int(-2)]));
        assert_eq!(parse("helper()").unwrap(), Expr::Call(None, "helper".into(), vec![]));
        assert_eq!(parse("user?.name").unwrap(), Expr::Field(name("user"), "name".into()));
    }

    #[test]
    fn parses_literals() {
        assert_eq!(parse("\"a\\\"b\\n\"").unwrap(), Expr::Str("a\"b\n".into()));
        assert_eq!(parse("'x'").unwrap(), Expr::Char('x'));
        assert_eq!(parse("10L").unwrap(), Expr::Long(10));
        assert_eq!(parse("3000000000").unwrap(), Expr::Long(3_000_000_000));
        assert_eq!(parse("0xff").unwrap(), Expr::Int(255));
        assert_eq!(parse("1.5f").unwrap(), Expr::Double(1.5));
        assert_eq!(parse("1_000").unwrap(), Expr::Int(1000));
        assert_eq!(parse("null").unwrap(), Expr::Null);
    }

    #[test]
    fn reports_errors() {
        assert_eq!(parse(""), Err("Expression expected".into()));
        assert_eq!(parse("a +"), Err("Expression expected".into()));
        assert_eq!(parse("(a"), Err("')' expected".into()));
        assert_eq!(parse("a b"), Err("';' expected".into()));
        assert_eq!(parse("\"open"), Err("Unterminated literal".into()));
        assert_eq!(parse("a # b"), Err("Unexpected '#'".into()));
    }
}
