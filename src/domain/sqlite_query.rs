//! The Database Inspector's queries, run on the read-only database file
//! without SQLite. A `SELECT` over one table or several joined (`JOIN …
//! ON` / `USING`, `LEFT JOIN`, `CROSS JOIN`, commas) with expressions
//! (arithmetic, `||`, comparisons, `LIKE`, `IN`, `BETWEEN`, `IS [NOT]
//! NULL`, `CASE`, `CAST`, common scalar functions), aggregates (`COUNT`,
//! `SUM`, `AVG`, `MIN`, `MAX`, `TOTAL`, `GROUP_CONCAT`, with `DISTINCT`),
//! `GROUP BY`, `HAVING`, `DISTINCT`, `ORDER BY` and `LIMIT` / `OFFSET`.
//! Subqueries, compound selects and every statement that would change the
//! database are refused with a reason.

use std::cmp::Ordering;
use std::collections::HashMap;

use super::sqlite_file::{Database, Table, Value};

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    /// A quoted identifier: never a keyword.
    Name(String),
    Str(String),
    Num(f64, bool),
    Sym(&'static str),
}

fn tokens(sql: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '\'' {
            let mut s = String::new();
            i += 1;
            loop {
                match chars.get(i) {
                    None => return Err("A string isn't closed".into()),
                    Some('\'') if chars.get(i + 1) == Some(&'\'') => {
                        s.push('\'');
                        i += 2;
                    }
                    Some('\'') => {
                        i += 1;
                        break;
                    }
                    Some(&ch) => {
                        s.push(ch);
                        i += 1;
                    }
                }
            }
            out.push(Token::Str(s));
        } else if matches!(c, '"' | '`' | '[') {
            let close = if c == '[' { ']' } else { c };
            let end = chars[i + 1..].iter().position(|&ch| ch == close).ok_or("A quoted name isn't closed")?;
            out.push(Token::Name(chars[i + 1..i + 1 + end].iter().collect()));
            i += end + 2;
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || (matches!(chars[i], '+' | '-') && matches!(chars[i - 1], 'e' | 'E'))) {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let integer = !text.contains(['.', 'e', 'E']);
            out.push(Token::Num(text.parse().map_err(|_| format!("Not a number: {text}"))?, integer));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                i += 1;
            }
            out.push(Token::Word(chars[start..i].iter().collect()));
        } else {
            let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
            if let Some(s) = ["<=", ">=", "<>", "!=", "==", "||"].into_iter().find(|s| *s == two) {
                out.push(Token::Sym(s));
                i += 2;
            } else {
                let one = ["(", ")", ",", "*", "=", "<", ">", ";", "-", "+", "/", "%", "."].into_iter().find(|s| s.starts_with(c)).ok_or_else(|| format!("Unexpected '{c}'"))?;
                out.push(Token::Sym(one));
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Tokens back as text, for a result column's name (`COUNT(*)`, `age + 1`).
fn render(tokens: &[Token]) -> String {
    let mut out = String::new();
    for (i, t) in tokens.iter().enumerate() {
        let text = match t {
            Token::Word(w) | Token::Name(w) => w.clone(),
            Token::Str(s) => format!("'{}'", s.replace('\'', "''")),
            Token::Num(n, true) => (*n as i64).to_string(),
            Token::Num(n, false) => n.to_string(),
            Token::Sym(s) => (*s).to_owned(),
        };
        let prev = i.checked_sub(1).map(|p| &tokens[p]);
        let tight = matches!(t, Token::Sym(")" | "," | ".")) || matches!(prev, Some(Token::Sym("(" | ".")) | None) || (matches!(t, Token::Sym("(")) && matches!(prev, Some(Token::Word(_))));
        if !tight {
            out.push(' ');
        }
        out.push_str(&text);
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Column(usize),
    Literal(Value),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    /// `+ - * / % || = != < <= > >= AND OR`.
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Like(Box<Expr>, Box<Expr>, bool),
    In(Box<Expr>, Vec<Expr>, bool),
    IsNull(Box<Expr>, bool),
    /// A scalar function, by its lower-case name.
    Func(String, Vec<Expr>),
    /// The value of an aggregate, by its index in the query's list.
    Aggregate(usize),
    Case(Option<Box<Expr>>, Vec<(Expr, Expr)>, Option<Box<Expr>>),
    Cast(Box<Expr>, String),
}

#[derive(Clone, Debug)]
struct Aggregate {
    func: String,
    /// `None` for `COUNT(*)`.
    arg: Option<Expr>,
    distinct: bool,
    separator: Option<Expr>,
}

const AGGREGATES: [&str; 7] = ["count", "sum", "avg", "min", "max", "total", "group_concat"];
const FUNCTIONS: [&str; 22] = [
    "lower", "upper", "length", "abs", "coalesce", "ifnull", "nullif", "substr", "substring", "trim", "ltrim", "rtrim", "replace", "instr", "round", "typeof", "max", "min", "hex", "iif", "sign", "char",
];
/// Words that end a table's alias.
const CLAUSES: [&str; 20] =
    ["WHERE", "JOIN", "LEFT", "INNER", "CROSS", "OUTER", "NATURAL", "RIGHT", "FULL", "ON", "USING", "GROUP", "HAVING", "ORDER", "LIMIT", "UNION", "INTERSECT", "EXCEPT", "WINDOW", "OFFSET"];

struct Source {
    alias: String,
    table: Table,
    offset: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum JoinKind {
    Inner,
    Left,
}

struct Join {
    kind: JoinKind,
    source: usize,
    on: Option<Expr>,
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    sources: Vec<Source>,
    aggregates: Vec<Aggregate>,
    /// Where aggregates may appear (not in WHERE, ON or GROUP BY), and
    /// whether one is being read (they don't nest).
    allow_aggregates: bool,
    in_aggregate: bool,
    /// The select list's aliases, which later clauses may use as names.
    aliases: Vec<(String, Expr)>,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn peek_word(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Token::Word(w)) if w.eq_ignore_ascii_case(word))
    }

    fn keyword(&mut self, word: &str) -> bool {
        if self.peek_word(word) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn symbol(&mut self, sym: &str) -> bool {
        if matches!(self.peek(), Some(Token::Sym(s)) if *s == sym) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, sym: &str) -> Result<(), String> {
        if self.symbol(sym) { Ok(()) } else { Err(format!("Expected '{sym}'")) }
    }

    fn expect_word(&mut self, word: &str) -> Result<(), String> {
        if self.keyword(word) { Ok(()) } else { Err(format!("Expected {word}")) }
    }

    fn name(&mut self) -> Result<String, String> {
        match self.peek().cloned() {
            Some(Token::Word(w) | Token::Name(w)) => {
                self.at += 1;
                Ok(w)
            }
            _ => Err("Expected a name".into()),
        }
    }

    fn width(&self) -> usize {
        self.sources.last().map(|s| s.offset + s.table.columns.len()).unwrap_or(0)
    }

    /// A column of the tables so far, optionally qualified by a table's name or alias.
    fn column(&self, qualifier: Option<&str>, name: &str) -> Result<usize, String> {
        let mut found = None;
        for s in &self.sources {
            if qualifier.is_some_and(|q| !q.eq_ignore_ascii_case(&s.alias)) {
                continue;
            }
            let at = s.table.columns.iter().position(|c| c.eq_ignore_ascii_case(name)).or_else(|| {
                // `rowid` names the INTEGER PRIMARY KEY column when there is one.
                ["rowid", "_rowid_", "oid"].iter().any(|r| r.eq_ignore_ascii_case(name)).then_some(s.table.rowid_column).flatten()
            });
            if let Some(at) = at {
                if found.is_some() {
                    return Err(format!("Column {name} is in more than one table: write table.{name}"));
                }
                found = Some(s.offset + at);
            }
        }
        found.ok_or_else(|| match (qualifier, self.sources.as_slice()) {
            (Some(q), _) if !self.sources.iter().any(|s| s.alias.eq_ignore_ascii_case(q)) => format!("No table named {q} in the query"),
            (_, [only]) => format!("No column named {name} in {}", only.table.name),
            _ => format!("No column named {name}"),
        })
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut left = self.and()?;
        while self.keyword("OR") {
            left = Expr::Binary("OR", Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut left = self.not()?;
        while self.keyword("AND") {
            left = Expr::Binary("AND", Box::new(left), Box::new(self.not()?));
        }
        Ok(left)
    }

    fn not(&mut self) -> Result<Expr, String> {
        if self.keyword("NOT") {
            return Ok(Expr::Not(Box::new(self.not()?)));
        }
        self.predicate()
    }

    fn predicate(&mut self) -> Result<Expr, String> {
        let left = self.additive()?;
        if self.keyword("IS") {
            let negated = self.keyword("NOT");
            if !self.keyword("NULL") {
                return Err("Expected NULL after IS".into());
            }
            return Ok(Expr::IsNull(Box::new(left), negated));
        }
        if self.keyword("ISNULL") {
            return Ok(Expr::IsNull(Box::new(left), false));
        }
        if self.keyword("NOTNULL") {
            return Ok(Expr::IsNull(Box::new(left), true));
        }
        let negated = self.keyword("NOT");
        if self.keyword("LIKE") {
            return Ok(Expr::Like(Box::new(left), Box::new(self.additive()?), negated));
        }
        if self.keyword("IN") {
            self.expect("(")?;
            if self.peek_word("SELECT") {
                return Err("Subqueries aren't supported here".into());
            }
            let mut list = vec![self.or()?];
            while self.symbol(",") {
                list.push(self.or()?);
            }
            self.expect(")")?;
            return Ok(Expr::In(Box::new(left), list, negated));
        }
        if self.keyword("BETWEEN") {
            let low = self.additive()?;
            self.expect_word("AND")?;
            let high = self.additive()?;
            let inside = Expr::Binary("AND", Box::new(Expr::Binary(">=", Box::new(left.clone()), Box::new(low))), Box::new(Expr::Binary("<=", Box::new(left), Box::new(high))));
            return Ok(if negated { Expr::Not(Box::new(inside)) } else { inside });
        }
        if negated {
            return Err("Expected LIKE, IN or BETWEEN after NOT".into());
        }
        for op in ["=", "==", "!=", "<>", "<=", ">=", "<", ">"] {
            if self.symbol(op) {
                let op = match op {
                    "==" => "=",
                    "<>" => "!=",
                    o => o,
                };
                return Ok(Expr::Binary(op, Box::new(left), Box::new(self.additive()?)));
            }
        }
        Ok(left)
    }

    fn additive(&mut self) -> Result<Expr, String> {
        let mut left = self.multiplicative()?;
        loop {
            let op = if self.symbol("+") {
                "+"
            } else if self.symbol("-") {
                "-"
            } else {
                return Ok(left);
            };
            left = Expr::Binary(op, Box::new(left), Box::new(self.multiplicative()?));
        }
    }

    fn multiplicative(&mut self) -> Result<Expr, String> {
        let mut left = self.concat()?;
        loop {
            let op = if self.symbol("*") {
                "*"
            } else if self.symbol("/") {
                "/"
            } else if self.symbol("%") {
                "%"
            } else {
                return Ok(left);
            };
            left = Expr::Binary(op, Box::new(left), Box::new(self.concat()?));
        }
    }

    fn concat(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        while self.symbol("||") {
            left = Expr::Binary("||", Box::new(left), Box::new(self.unary()?));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.symbol("-") {
            return Ok(match self.unary()? {
                Expr::Literal(Value::Integer(i)) => Expr::Literal(Value::Integer(-i)),
                Expr::Literal(Value::Real(r)) => Expr::Literal(Value::Real(-r)),
                e => Expr::Neg(Box::new(e)),
            });
        }
        if self.symbol("+") {
            return self.unary();
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, String> {
        if self.symbol("(") {
            if self.peek_word("SELECT") {
                return Err("Subqueries aren't supported here".into());
            }
            let e = self.or()?;
            self.expect(")")?;
            return Ok(e);
        }
        match self.peek().cloned() {
            Some(Token::Num(n, integer)) => {
                self.at += 1;
                Ok(Expr::Literal(if integer && n.abs() < 9.2e18 { Value::Integer(n as i64) } else { Value::Real(n) }))
            }
            Some(Token::Str(s)) => {
                self.at += 1;
                Ok(Expr::Literal(Value::Text(s)))
            }
            Some(Token::Word(w)) if w.eq_ignore_ascii_case("NULL") => {
                self.at += 1;
                Ok(Expr::Literal(Value::Null))
            }
            Some(Token::Word(w)) if w.eq_ignore_ascii_case("TRUE") || w.eq_ignore_ascii_case("FALSE") => {
                self.at += 1;
                Ok(Expr::Literal(Value::Integer(w.eq_ignore_ascii_case("TRUE") as i64)))
            }
            Some(Token::Word(w)) if w.eq_ignore_ascii_case("CASE") => {
                self.at += 1;
                let operand = if self.peek_word("WHEN") { None } else { Some(Box::new(self.or()?)) };
                let mut arms = Vec::new();
                while self.keyword("WHEN") {
                    let when = self.or()?;
                    self.expect_word("THEN")?;
                    arms.push((when, self.or()?));
                }
                if arms.is_empty() {
                    return Err("Expected WHEN in CASE".into());
                }
                let otherwise = if self.keyword("ELSE") { Some(Box::new(self.or()?)) } else { None };
                self.expect_word("END")?;
                Ok(Expr::Case(operand, arms, otherwise))
            }
            Some(Token::Word(w)) if w.eq_ignore_ascii_case("CAST") && matches!(self.tokens.get(self.at + 1), Some(Token::Sym("("))) => {
                self.at += 2;
                let e = self.or()?;
                self.expect_word("AS")?;
                let kind = self.name()?.to_ascii_uppercase();
                self.expect(")")?;
                Ok(Expr::Cast(Box::new(e), kind))
            }
            Some(Token::Word(w)) if matches!(self.tokens.get(self.at + 1), Some(Token::Sym("("))) => {
                self.at += 2;
                self.call(&w.to_ascii_lowercase())
            }
            Some(Token::Word(_) | Token::Name(_)) => {
                let first = self.name()?;
                if self.symbol(".") {
                    let column = self.name()?;
                    return Ok(Expr::Column(self.column(Some(&first), &column)?));
                }
                match self.column(None, &first) {
                    Ok(c) => Ok(Expr::Column(c)),
                    Err(e) => match self.aliases.iter().find(|(a, _)| a.eq_ignore_ascii_case(&first)) {
                        // An aggregate's alias only where aggregates may be.
                        Some((_, Expr::Aggregate(_))) if !self.allow_aggregates => Err(format!("{first} is an aggregate, which can't be used here")),
                        Some((_, expr)) => Ok(expr.clone()),
                        None => Err(e),
                    },
                }
            }
            Some(Token::Sym(s)) => Err(format!("Unexpected '{s}'")),
            None => Err("The query ends early".into()),
        }
    }

    /// A function call, after its `(`.
    fn call(&mut self, name: &str) -> Result<Expr, String> {
        let star = self.symbol("*");
        let distinct = !star && self.keyword("DISTINCT");
        let mut args = Vec::new();
        if star {
            if name != "count" {
                return Err(format!("{}(*) isn't a function", name.to_uppercase()));
            }
        } else if !matches!(self.peek(), Some(Token::Sym(")"))) {
            // Arguments are read as if inside an aggregate when this is one.
            let aggregate = AGGREGATES.contains(&name);
            let was = self.in_aggregate;
            if aggregate {
                self.in_aggregate = true;
            }
            loop {
                args.push(self.or()?);
                if !self.symbol(",") {
                    break;
                }
            }
            self.in_aggregate = was;
        }
        self.expect(")")?;
        // `min(a, b)` and `max(a, b)` are scalar; with one argument, aggregates.
        let aggregate = AGGREGATES.contains(&name) && !(matches!(name, "min" | "max") && args.len() > 1);
        if aggregate {
            if !self.allow_aggregates {
                return Err(format!("{}() can't be used here", name.to_uppercase()));
            }
            if self.in_aggregate {
                return Err("Aggregates can't be nested".into());
            }
            let expected = match name {
                "count" => star || args.len() == 1,
                "group_concat" => (1..=2).contains(&args.len()),
                _ => args.len() == 1,
            };
            if !expected {
                return Err(format!("Wrong number of arguments to {}()", name.to_uppercase()));
            }
            let mut args = args.into_iter();
            self.aggregates.push(Aggregate { func: name.to_owned(), arg: args.next(), distinct, separator: args.next() });
            return Ok(Expr::Aggregate(self.aggregates.len() - 1));
        }
        if !FUNCTIONS.contains(&name) {
            return Err(format!("Unknown function {}()", name.to_uppercase()));
        }
        if distinct {
            return Err(format!("DISTINCT only goes in aggregates, not {}()", name.to_uppercase()));
        }
        Ok(Expr::Func(name.to_owned(), args))
    }

    /// `table [AS alias]`.
    fn source(&mut self, tables: &[Table]) -> Result<usize, String> {
        if matches!(self.peek(), Some(Token::Sym("("))) {
            return Err("Subqueries aren't supported here".into());
        }
        let name = self.name().map_err(|_| "Expected a table name".to_owned())?;
        let table = tables.iter().find(|t| t.name.eq_ignore_ascii_case(&name)).ok_or_else(|| format!("No table named {name}"))?;
        let alias = if self.keyword("AS") {
            self.name()?
        } else {
            match self.peek() {
                Some(Token::Name(n)) => {
                    let n = n.clone();
                    self.at += 1;
                    n
                }
                Some(Token::Word(w)) if !CLAUSES.iter().any(|c| c.eq_ignore_ascii_case(w)) => {
                    let w = w.clone();
                    self.at += 1;
                    w
                }
                _ => table.name.clone(),
            }
        };
        if self.sources.iter().any(|s| s.alias.eq_ignore_ascii_case(&alias)) {
            return Err(format!("{alias} is used twice: give one of them another name (FROM {name} AS other)"));
        }
        let offset = self.width();
        self.sources.push(Source { alias, table: table.clone(), offset });
        Ok(self.sources.len() - 1)
    }

    /// The FROM clause's tables and joins.
    fn from(&mut self, tables: &[Table]) -> Result<Vec<Join>, String> {
        self.source(tables)?;
        let mut joins = Vec::new();
        loop {
            let kind = if self.symbol(",") {
                let source = self.source(tables)?;
                joins.push(Join { kind: JoinKind::Inner, source, on: None });
                continue;
            } else if self.peek_word("NATURAL") || self.peek_word("RIGHT") || self.peek_word("FULL") {
                return Err(format!("Not supported here: {} JOIN", self.name()?.to_uppercase()));
            } else if self.keyword("LEFT") {
                self.keyword("OUTER");
                JoinKind::Left
            } else if self.keyword("INNER") || self.keyword("CROSS") {
                JoinKind::Inner
            } else if self.peek_word("JOIN") {
                JoinKind::Inner
            } else {
                return Ok(joins);
            };
            self.expect_word("JOIN")?;
            let source = self.source(tables)?;
            let on = if self.keyword("ON") {
                Some(self.or()?)
            } else if self.keyword("USING") {
                self.expect("(")?;
                let mut on: Option<Expr> = None;
                loop {
                    let name = self.name()?;
                    let right = &self.sources[source];
                    let r = right.table.columns.iter().position(|c| c.eq_ignore_ascii_case(&name)).ok_or_else(|| format!("No column named {name} in {}", right.table.name))? + right.offset;
                    let l = self.sources[..source]
                        .iter()
                        .find_map(|s| s.table.columns.iter().position(|c| c.eq_ignore_ascii_case(&name)).map(|i| s.offset + i))
                        .ok_or_else(|| format!("No column named {name} on the left of USING"))?;
                    let eq = Expr::Binary("=", Box::new(Expr::Column(l)), Box::new(Expr::Column(r)));
                    on = Some(match on {
                        Some(prev) => Expr::Binary("AND", Box::new(prev), Box::new(eq)),
                        None => eq,
                    });
                    if !self.symbol(",") {
                        break;
                    }
                }
                self.expect(")")?;
                on
            } else {
                None
            };
            joins.push(Join { kind, source, on });
        }
    }
}

/// SQLite's order between values of different kinds: NULL, numbers, text, blobs.
fn compare(a: &Value, b: &Value) -> Ordering {
    fn rank(v: &Value) -> u8 {
        match v {
            Value::Null => 0,
            Value::Integer(_) | Value::Real(_) => 1,
            Value::Text(_) => 2,
            Value::Blob(_) => 3,
        }
    }
    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
        (Value::Integer(_) | Value::Real(_), Value::Integer(_) | Value::Real(_)) => float(a).partial_cmp(&float(b)).unwrap_or(Ordering::Equal),
        (Value::Text(x), Value::Text(y)) => x.cmp(y),
        (Value::Blob(x), Value::Blob(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

fn float(v: &Value) -> f64 {
    match numeric(v) {
        Value::Integer(i) => i as f64,
        Value::Real(r) => r,
        _ => 0.,
    }
}

/// A value as a number, as SQLite reads text in arithmetic: its longest
/// numeric prefix, else 0. NULL stays NULL.
fn numeric(v: &Value) -> Value {
    match v {
        Value::Integer(_) | Value::Real(_) | Value::Null => v.clone(),
        Value::Blob(_) => Value::Integer(0),
        Value::Text(t) => {
            let t = t.trim_start();
            let end = (1..=t.len()).rev().filter(|&n| t.is_char_boundary(n)).find(|&n| t[..n].parse::<f64>().is_ok() && !t[..n].ends_with(['e', 'E', '+', '-'])).unwrap_or(0);
            let prefix = &t[..end];
            prefix.parse::<i64>().map(Value::Integer).or_else(|_| prefix.parse::<f64>().map(Value::Real)).unwrap_or(Value::Integer(0))
        }
    }
}

/// A literal compared with a column takes the column's kind, as SQLite's
/// type affinity does for `age = '30'`.
fn coerce(literal: &Value, like: &Value) -> Value {
    match (literal, like) {
        (Value::Text(t), Value::Integer(_) | Value::Real(_)) => match t.trim().parse::<i64>() {
            Ok(i) => Value::Integer(i),
            Err(_) => t.trim().parse::<f64>().map(Value::Real).unwrap_or_else(|_| literal.clone()),
        },
        (Value::Integer(_) | Value::Real(_), Value::Text(_)) => Value::Text(text(literal).unwrap_or_default()),
        _ => literal.clone(),
    }
}

/// `LIKE`: `%` any run, `_` one character, ASCII letters in either case.
fn like(text: &str, pattern: &str) -> bool {
    fn go(t: &[char], p: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('%') => (0..=t.len()).any(|i| go(&t[i..], &p[1..])),
            Some('_') => !t.is_empty() && go(&t[1..], &p[1..]),
            Some(c) => t.first().is_some_and(|x| x.eq_ignore_ascii_case(c)) && go(&t[1..], &p[1..]),
        }
    }
    let (t, p): (Vec<char>, Vec<char>) = (text.chars().collect(), pattern.chars().collect());
    go(&t, &p)
}

/// A value as SQLite's text: `3.0` keeps its `.0`; NULL has none.
fn text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Text(t) => Some(t.clone()),
        Value::Real(r) if r.fract() == 0. && r.abs() < 1e15 => Some(format!("{r:.1}")),
        Value::Blob(b) => Some(String::from_utf8_lossy(b).into_owned()),
        v => Some(v.to_string()),
    }
}

fn truth(v: &Value) -> Option<bool> {
    match v {
        Value::Null => None,
        v => Some(float(v) != 0.),
    }
}

fn boolean(b: Option<bool>) -> Value {
    b.map(|b| Value::Integer(b as i64)).unwrap_or(Value::Null)
}

fn arithmetic(op: &str, a: &Value, b: &Value) -> Value {
    let (a, b) = (numeric(a), numeric(b));
    if a == Value::Null || b == Value::Null {
        return Value::Null;
    }
    if let (Value::Integer(x), Value::Integer(y)) = (&a, &b) {
        let (x, y) = (*x, *y);
        let exact = match op {
            "+" => x.checked_add(y),
            "-" => x.checked_sub(y),
            "*" => x.checked_mul(y),
            "/" if y == 0 => return Value::Null,
            "/" => x.checked_div(y),
            _ if y == 0 => return Value::Null,
            _ => x.checked_rem(y),
        };
        if let Some(v) = exact {
            return Value::Integer(v);
        }
    }
    let (x, y) = (float(&a), float(&b));
    Value::Real(match op {
        "+" => x + y,
        "-" => x - y,
        "*" => x * y,
        "/" if y == 0. => return Value::Null,
        "/" => x / y,
        _ if y as i64 == 0 => return Value::Null,
        _ => ((x as i64) % (y as i64)) as f64,
    })
}

/// A row's values and, for a group, its aggregates' values.
struct Ctx<'a> {
    row: &'a [Value],
    aggregates: &'a [Value],
}

fn eval(e: &Expr, cx: &Ctx) -> Value {
    // Comparing a column with a literal: the literal takes the column's kind.
    let pair = |l: &Expr, r: &Expr| {
        let (a, b) = (eval(l, cx), eval(r, cx));
        match (l, r) {
            (Expr::Column(_), Expr::Literal(_)) => {
                let b = coerce(&b, &a);
                (a, b)
            }
            (Expr::Literal(_), Expr::Column(_)) => (coerce(&a, &b), b),
            _ => (a, b),
        }
    };
    match e {
        Expr::Column(i) => cx.row.get(*i).cloned().unwrap_or(Value::Null),
        Expr::Literal(v) => v.clone(),
        Expr::Aggregate(i) => cx.aggregates.get(*i).cloned().unwrap_or(Value::Null),
        Expr::Neg(inner) => arithmetic("-", &Value::Integer(0), &eval(inner, cx)),
        Expr::Not(inner) => boolean(truth(&eval(inner, cx)).map(|b| !b)),
        Expr::Binary("AND", l, r) => boolean(match (truth(&eval(l, cx)), truth(&eval(r, cx))) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        }),
        Expr::Binary("OR", l, r) => boolean(match (truth(&eval(l, cx)), truth(&eval(r, cx))) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        }),
        Expr::Binary("||", l, r) => match (text(&eval(l, cx)), text(&eval(r, cx))) {
            (Some(a), Some(b)) => Value::Text(a + &b),
            _ => Value::Null,
        },
        Expr::Binary(op @ ("+" | "-" | "*" | "/" | "%"), l, r) => arithmetic(op, &eval(l, cx), &eval(r, cx)),
        Expr::Binary(op, l, r) => {
            let (a, b) = pair(l, r);
            if a == Value::Null || b == Value::Null {
                return Value::Null;
            }
            let o = compare(&a, &b);
            boolean(Some(match *op {
                "=" => o == Ordering::Equal,
                "!=" => o != Ordering::Equal,
                "<" => o == Ordering::Less,
                "<=" => o != Ordering::Greater,
                ">" => o == Ordering::Greater,
                _ => o != Ordering::Less,
            }))
        }
        Expr::Like(l, p, negated) => match (text(&eval(l, cx)), text(&eval(p, cx))) {
            (Some(t), Some(p)) => boolean(Some(like(&t, &p) != *negated)),
            _ => Value::Null,
        },
        Expr::In(l, list, negated) => {
            let mut unknown = false;
            for item in list {
                let (a, b) = pair(l, item);
                if a == Value::Null || b == Value::Null {
                    unknown = true;
                } else if compare(&a, &b) == Ordering::Equal {
                    return boolean(Some(!negated));
                }
            }
            if unknown { Value::Null } else { boolean(Some(*negated)) }
        }
        Expr::IsNull(l, negated) => boolean(Some((eval(l, cx) == Value::Null) != *negated)),
        Expr::Case(operand, arms, otherwise) => {
            let subject = operand.as_ref().map(|o| eval(o, cx));
            for (when, then) in arms {
                let hit = match &subject {
                    Some(s) => {
                        let w = eval(when, cx);
                        *s != Value::Null && w != Value::Null && compare(s, &w) == Ordering::Equal
                    }
                    None => truth(&eval(when, cx)) == Some(true),
                };
                if hit {
                    return eval(then, cx);
                }
            }
            otherwise.as_ref().map(|o| eval(o, cx)).unwrap_or(Value::Null)
        }
        Expr::Cast(inner, kind) => {
            let v = eval(inner, cx);
            if v == Value::Null {
                return v;
            }
            match kind.as_str() {
                k if k.contains("INT") => Value::Integer(match numeric(&v) {
                    Value::Real(r) => r as i64,
                    Value::Integer(i) => i,
                    _ => 0,
                }),
                k if k.contains("CHAR") || k.contains("CLOB") || k.contains("TEXT") => Value::Text(text(&v).unwrap_or_default()),
                k if k.contains("REAL") || k.contains("FLOA") || k.contains("DOUB") => Value::Real(float(&v)),
                "BLOB" => match v {
                    Value::Blob(b) => Value::Blob(b),
                    v => Value::Blob(text(&v).unwrap_or_default().into_bytes()),
                },
                _ => numeric(&v),
            }
        }
        Expr::Func(name, args) => {
            let values: Vec<Value> = args.iter().map(|a| eval(a, cx)).collect();
            function(name, &values).unwrap_or(Value::Null)
        }
    }
}

/// A scalar function's value; `None` is NULL.
fn function(name: &str, args: &[Value]) -> Option<Value> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Null);
    let str_arg = |i: usize| text(&arg(i));
    let int_arg = |i: usize| match numeric(&arg(i)) {
        Value::Integer(n) => Some(n),
        Value::Real(r) => Some(r as i64),
        _ => None,
    };
    Some(match name {
        "lower" => Value::Text(str_arg(0)?.to_ascii_lowercase()),
        "upper" => Value::Text(str_arg(0)?.to_ascii_uppercase()),
        "length" => Value::Integer(match arg(0) {
            Value::Null => return None,
            Value::Blob(b) => b.len() as i64,
            v => text(&v)?.chars().count() as i64,
        }),
        "abs" => match numeric(&arg(0)) {
            Value::Integer(i) => Value::Integer(i.checked_abs().unwrap_or(i64::MAX)),
            Value::Real(r) => Value::Real(r.abs()),
            _ => return None,
        },
        "sign" => match numeric(&arg(0)) {
            Value::Null => return None,
            v => Value::Integer(float(&v).partial_cmp(&0.).map(|o| o as i64).unwrap_or(0)),
        },
        "coalesce" | "ifnull" => args.iter().find(|v| **v != Value::Null).cloned()?,
        "nullif" => {
            if compare(&arg(0), &arg(1)) == Ordering::Equal {
                return None;
            }
            arg(0)
        }
        "iif" => {
            if truth(&arg(0)) == Some(true) {
                arg(1)
            } else {
                arg(2)
            }
        }
        "substr" | "substring" => {
            let chars: Vec<char> = str_arg(0)?.chars().collect();
            let len = chars.len() as i64;
            let start = int_arg(1)?;
            let count = if args.len() > 2 { int_arg(2)? } else { len };
            // 1-based; 0 counts as just before the first; negative from the end.
            let first = if start > 0 { start - 1 } else if start < 0 { len + start } else { -1 };
            let (from, to) = if count >= 0 { (first, first + count) } else { (first + count, first) };
            let (from, to) = (from.clamp(0, len) as usize, to.clamp(0, len) as usize);
            Value::Text(chars[from..to.max(from)].iter().collect())
        }
        "trim" | "ltrim" | "rtrim" => {
            let s = str_arg(0)?;
            let set: Vec<char> = if args.len() > 1 { str_arg(1)?.chars().collect() } else { vec![' '] };
            let s = match name {
                "ltrim" => s.trim_start_matches(set.as_slice()),
                "rtrim" => s.trim_end_matches(set.as_slice()),
                _ => s.trim_matches(set.as_slice()),
            };
            Value::Text(s.to_owned())
        }
        "replace" => {
            let (s, from, to) = (str_arg(0)?, str_arg(1)?, str_arg(2)?);
            Value::Text(if from.is_empty() { s } else { s.replace(&from, &to) })
        }
        "instr" => {
            let (s, sub) = (str_arg(0)?, str_arg(1)?);
            Value::Integer(s.find(&sub).map(|b| s[..b].chars().count() as i64 + 1).unwrap_or(0))
        }
        "round" => {
            let x = match numeric(&arg(0)) {
                Value::Null => return None,
                v => float(&v),
            };
            let digits = if args.len() > 1 { int_arg(1)?.clamp(0, 15) } else { 0 } as i32;
            let scale = 10f64.powi(digits);
            Value::Real((x * scale).round() / scale)
        }
        "typeof" => Value::Text(
            match arg(0) {
                Value::Null => "null",
                Value::Integer(_) => "integer",
                Value::Real(_) => "real",
                Value::Text(_) => "text",
                Value::Blob(_) => "blob",
            }
            .into(),
        ),
        "max" | "min" => {
            if args.iter().any(|v| *v == Value::Null) {
                return None;
            }
            let pick = if name == "max" { Ordering::Greater } else { Ordering::Less };
            args.iter().cloned().reduce(|a, b| if compare(&b, &a) == pick { b } else { a })?
        }
        "hex" => {
            let bytes = match arg(0) {
                Value::Blob(b) => b,
                v => text(&v).unwrap_or_default().into_bytes(),
            };
            Value::Text(bytes.iter().map(|b| format!("{b:02X}")).collect())
        }
        "char" => Value::Text(args.iter().filter_map(|v| int_arg_of(v).and_then(|c| char::from_u32(c as u32))).collect()),
        _ => return None,
    })
}

fn int_arg_of(v: &Value) -> Option<i64> {
    match numeric(v) {
        Value::Integer(n) => Some(n),
        Value::Real(r) => Some(r as i64),
        _ => None,
    }
}

/// A value as a grouping or hashing key: `1` and `1.0` are the same.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Null,
    Int(i64),
    Real(u64),
    Text(String),
    Blob(Vec<u8>),
}

fn key(v: &Value) -> Key {
    match v {
        Value::Null => Key::Null,
        Value::Integer(i) => Key::Int(*i),
        Value::Real(r) if r.fract() == 0. && r.abs() < 9.2e18 => Key::Int(*r as i64),
        Value::Real(r) => Key::Real(r.to_bits()),
        Value::Text(t) => Key::Text(t.clone()),
        Value::Blob(b) => Key::Blob(b.clone()),
    }
}

fn aggregate(a: &Aggregate, rows: &[&Vec<Value>]) -> Value {
    let no_aggregates: [Value; 0] = [];
    let mut values: Vec<Value> = match &a.arg {
        None => return Value::Integer(rows.len() as i64),
        Some(arg) => rows.iter().map(|row| eval(arg, &Ctx { row, aggregates: &no_aggregates })).filter(|v| *v != Value::Null).collect(),
    };
    if a.distinct {
        let mut seen = std::collections::HashSet::new();
        values.retain(|v| seen.insert(key(v)));
    }
    let sum = |values: &[Value]| -> Value {
        if values.iter().all(|v| matches!(numeric(v), Value::Integer(_))) {
            let mut total: i64 = 0;
            for v in values {
                let Value::Integer(i) = numeric(v) else { unreachable!() };
                match total.checked_add(i) {
                    Some(t) => total = t,
                    None => return Value::Real(values.iter().map(float).sum()),
                }
            }
            Value::Integer(total)
        } else {
            Value::Real(values.iter().map(float).sum())
        }
    };
    match a.func.as_str() {
        "count" => Value::Integer(values.len() as i64),
        "sum" if values.is_empty() => Value::Null,
        "sum" => sum(&values),
        "total" => Value::Real(values.iter().map(float).sum()),
        "avg" if values.is_empty() => Value::Null,
        "avg" => Value::Real(values.iter().map(float).sum::<f64>() / values.len() as f64),
        "min" => values.into_iter().reduce(|a, b| if compare(&b, &a) == Ordering::Less { b } else { a }).unwrap_or(Value::Null),
        "max" => values.into_iter().reduce(|a, b| if compare(&b, &a) == Ordering::Greater { b } else { a }).unwrap_or(Value::Null),
        _ => {
            if values.is_empty() {
                return Value::Null;
            }
            let separator = match &a.separator {
                Some(s) => rows.first().and_then(|row| text(&eval(s, &Ctx { row, aggregates: &no_aggregates }))).unwrap_or_default(),
                None => ",".to_owned(),
            };
            Value::Text(values.iter().filter_map(text).collect::<Vec<_>>().join(&separator))
        }
    }
}

/// A query's answer: its column names and rows.
#[derive(Debug, PartialEq)]
pub struct QueryResult {
    pub table: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

/// The most row pairs a join without an equality condition compares.
const MAX_PAIRS: usize = 20_000_000;

/// The rows of the FROM clause's tables, joined.
fn join(db: &Database, sources: &[Source], joins: &[Join], width: usize) -> Result<Vec<Vec<Value>>, String> {
    let Some(first) = sources.first() else { return Ok(vec![Vec::new()]) };
    let mut rows: Vec<Vec<Value>> = db
        .rows(&first.table, usize::MAX)?
        .into_iter()
        .map(|mut r| {
            r.resize(first.table.columns.len(), Value::Null);
            r.resize(width, Value::Null);
            r
        })
        .collect();
    let no_aggregates: [Value; 0] = [];
    for j in joins {
        let source = &sources[j.source];
        let (start, end) = (source.offset, source.offset + source.table.columns.len());
        let right = db.rows(&source.table, usize::MAX)?;
        // `left.x = right.y`: a hash join instead of comparing every pair.
        let equality = match &j.on {
            Some(Expr::Binary("=", a, b)) => match (a.as_ref(), b.as_ref()) {
                (Expr::Column(l), Expr::Column(r)) if *l < start && (start..end).contains(r) => Some((*l, *r - start)),
                (Expr::Column(r), Expr::Column(l)) if *l < start && (start..end).contains(r) => Some((*l, *r - start)),
                _ => None,
            },
            _ => None,
        };
        let mut index: HashMap<Key, Vec<usize>> = HashMap::new();
        if let Some((_, r)) = equality {
            for (i, row) in right.iter().enumerate() {
                let v = row.get(r).cloned().unwrap_or(Value::Null);
                if v != Value::Null {
                    index.entry(key(&v)).or_default().push(i);
                }
            }
        } else if rows.len().saturating_mul(right.len()) > MAX_PAIRS {
            return Err(format!("Joining {} with {} rows compares too many pairs here: add ON left.column = right.column", rows.len(), right.len()));
        }
        let mut out = Vec::new();
        for left in rows {
            let candidates: Vec<usize> = match equality {
                Some((l, _)) => index.get(&key(&left[l])).cloned().unwrap_or_default(),
                None => (0..right.len()).collect(),
            };
            let mut matched = false;
            for i in candidates {
                let mut row = left.clone();
                for (c, v) in right[i].iter().enumerate().take(end - start) {
                    row[start + c] = v.clone();
                }
                if j.on.as_ref().is_none_or(|on| truth(&eval(on, &Ctx { row: &row, aggregates: &no_aggregates })) == Some(true)) {
                    matched = true;
                    out.push(row);
                }
            }
            if !matched && j.kind == JoinKind::Left {
                out.push(left);
            }
        }
        rows = out;
    }
    Ok(rows)
}

/// An output column: an expression with its name.
struct Output {
    expr: Expr,
    name: String,
}

enum OrderBy {
    /// An output column, by position or name.
    Output(usize),
    Expr(Expr),
}

/// Runs `sql` on the database; `limit` caps the rows kept (after ORDER BY).
pub fn run(db: &Database, tables: &[Table], sql: &str, limit: usize) -> Result<QueryResult, String> {
    let mut toks = tokens(sql)?;
    while matches!(toks.last(), Some(Token::Sym(";"))) {
        toks.pop();
    }
    match toks.first() {
        Some(Token::Word(w)) if w.eq_ignore_ascii_case("SELECT") => {}
        Some(Token::Word(w)) if w.eq_ignore_ascii_case("WITH") => return Err("WITH isn't supported here".into()),
        Some(Token::Word(w)) => return Err(format!("Only SELECT runs here: the database is read from a copy, so {} can't change the app's data", w.to_uppercase())),
        _ => return Err("Type a SELECT statement".into()),
    }
    if toks.iter().any(|t| matches!(t, Token::Sym(";"))) {
        return Err("Run one statement at a time".into());
    }
    // The tables first, so names in the select list can be resolved.
    let mut depth = 0i32;
    let from = toks.iter().position(|t| {
        match t {
            Token::Sym("(") => depth += 1,
            Token::Sym(")") => depth -= 1,
            _ => {}
        }
        depth == 0 && matches!(t, Token::Word(w) if w.eq_ignore_ascii_case("FROM"))
    });
    let mut p = Parser { tokens: toks, at: 0, sources: Vec::new(), aggregates: Vec::new(), allow_aggregates: false, in_aggregate: false, aliases: Vec::new() };
    let mut joins = Vec::new();
    let mut after_from = p.tokens.len();
    if let Some(from) = from {
        p.at = from + 1;
        joins = p.from(tables)?;
        after_from = p.at;
    }
    let width = p.width();
    let select_end = from.unwrap_or(p.tokens.len());
    p.at = 1;
    p.keyword("ALL");
    let distinct = p.keyword("DISTINCT");
    p.allow_aggregates = true;
    let mut outputs = Vec::new();
    loop {
        if p.symbol("*") {
            if p.sources.is_empty() {
                return Err("SELECT * needs a FROM".into());
            }
            for s in &p.sources {
                outputs.extend(s.table.columns.iter().enumerate().map(|(i, c)| Output { expr: Expr::Column(s.offset + i), name: c.clone() }));
            }
        } else if matches!((p.peek(), p.tokens.get(p.at + 1), p.tokens.get(p.at + 2)), (Some(Token::Word(_) | Token::Name(_)), Some(Token::Sym(".")), Some(Token::Sym("*")))) {
            let alias = p.name()?;
            p.at += 2;
            let s = p.sources.iter().find(|s| s.alias.eq_ignore_ascii_case(&alias)).ok_or_else(|| format!("No table named {alias} in the query"))?;
            outputs.extend(s.table.columns.iter().enumerate().map(|(i, c)| Output { expr: Expr::Column(s.offset + i), name: c.clone() }));
        } else {
            let start = p.at;
            let expr = p.or()?;
            let end = p.at;
            let alias = if p.keyword("AS") {
                Some(p.name()?)
            } else {
                match p.peek() {
                    Some(Token::Name(n)) => {
                        let n = n.clone();
                        p.at += 1;
                        Some(n)
                    }
                    Some(Token::Word(w)) if p.at < select_end && !w.eq_ignore_ascii_case("FROM") => {
                        let w = w.clone();
                        p.at += 1;
                        Some(w)
                    }
                    _ => None,
                }
            };
            let name = alias.unwrap_or_else(|| match &expr {
                // A column keeps its own name, without the table's.
                Expr::Column(i) => p.sources.iter().find(|s| (s.offset..s.offset + s.table.columns.len()).contains(i)).map(|s| s.table.columns[i - s.offset].clone()).unwrap_or_default(),
                _ => render(&p.tokens[start..end]),
            });
            outputs.push(Output { expr, name });
        }
        if !p.symbol(",") {
            break;
        }
    }
    if p.at != select_end {
        return Err(match p.peek() {
            Some(Token::Word(w) | Token::Name(w)) => format!("Not supported here: {w}"),
            Some(Token::Sym(s)) => format!("Unexpected '{s}'"),
            _ => "Expected FROM".into(),
        });
    }
    p.aliases = outputs.iter().map(|o| (o.name.clone(), o.expr.clone())).collect();
    p.at = after_from;
    p.allow_aggregates = false;
    let filter = if p.keyword("WHERE") { Some(p.or()?) } else { None };
    // GROUP BY and ORDER BY can name an output column by its alias or position.
    let output_ref = |p: &mut Parser, outputs: &[Output]| -> Option<usize> {
        match p.peek() {
            Some(Token::Num(n, true)) if !matches!(p.tokens.get(p.at + 1), Some(Token::Sym(s)) if *s != ",") => {
                let n = *n as usize;
                (1..=outputs.len()).contains(&n).then(|| {
                    p.at += 1;
                    n - 1
                })
            }
            Some(Token::Word(w) | Token::Name(w)) if !matches!(p.tokens.get(p.at + 1), Some(Token::Sym("." | "("))) => {
                let found = outputs.iter().position(|o| o.name.eq_ignore_ascii_case(w) && !matches!(o.expr, Expr::Column(_)));
                if found.is_some() {
                    p.at += 1;
                }
                found
            }
            _ => None,
        }
    };
    let mut group_by = Vec::new();
    if p.keyword("GROUP") {
        p.expect_word("BY")?;
        loop {
            match output_ref(&mut p, &outputs) {
                Some(i) if matches!(outputs[i].expr, Expr::Aggregate(_)) => return Err("GROUP BY can't use an aggregate".into()),
                Some(i) => group_by.push(outputs[i].expr.clone()),
                None => group_by.push(p.or()?),
            }
            if !p.symbol(",") {
                break;
            }
        }
    }
    p.allow_aggregates = true;
    let having = if p.keyword("HAVING") { Some(p.or()?) } else { None };
    let mut order = Vec::new();
    if p.keyword("ORDER") {
        p.expect_word("BY")?;
        loop {
            let term = match output_ref(&mut p, &outputs) {
                Some(i) => OrderBy::Output(i),
                None => OrderBy::Expr(p.or()?),
            };
            p.keyword("COLLATE").then(|| p.name()).transpose()?;
            let desc = p.keyword("DESC");
            if !desc {
                p.keyword("ASC");
            }
            order.push((term, desc));
            if !p.symbol(",") {
                break;
            }
        }
    }
    let (mut take, mut skip) = (usize::MAX, 0usize);
    let number = |p: &mut Parser| match p.peek() {
        Some(Token::Num(n, true)) if *n >= 0. => {
            let n = *n as usize;
            p.at += 1;
            Ok(n)
        }
        _ => Err::<usize, String>("Expected a whole number".into()),
    };
    if p.keyword("LIMIT") {
        take = number(&mut p)?;
        if p.keyword("OFFSET") {
            skip = number(&mut p)?;
        } else if p.symbol(",") {
            // `LIMIT skip, take`
            skip = take;
            take = number(&mut p)?;
        }
    }
    if let Some(t) = p.peek() {
        return Err(format!("Not supported here: {}", match t {
            Token::Word(w) | Token::Name(w) => w.to_uppercase(),
            Token::Str(s) => format!("'{s}'"),
            Token::Num(n, _) => n.to_string(),
            Token::Sym(s) => (*s).to_owned(),
        }));
    }

    let rows = join(db, &p.sources, &joins, width)?;
    let no_aggregates: [Value; 0] = [];
    let rows: Vec<Vec<Value>> = rows.into_iter().filter(|row| filter.as_ref().is_none_or(|f| truth(&eval(f, &Ctx { row, aggregates: &no_aggregates })) == Some(true))).collect();
    // Each result row with its sort keys.
    let mut results: Vec<(Vec<Value>, Vec<Value>)> = Vec::new();
    let mut emit = |row: &[Value], aggregates: &[Value]| {
        let cx = Ctx { row, aggregates };
        let out: Vec<Value> = outputs.iter().map(|o| eval(&o.expr, &cx)).collect();
        let keys = order
            .iter()
            .map(|(term, _)| match term {
                OrderBy::Output(i) => out[*i].clone(),
                OrderBy::Expr(e) => eval(e, &cx),
            })
            .collect();
        results.push((keys, out));
    };
    if group_by.is_empty() && p.aggregates.is_empty() {
        if having.is_some() {
            return Err("HAVING needs GROUP BY or an aggregate".into());
        }
        for row in &rows {
            emit(row, &[]);
        }
    } else {
        let mut groups: Vec<Vec<&Vec<Value>>> = Vec::new();
        let mut by_key: HashMap<Vec<Key>, usize> = HashMap::new();
        for row in &rows {
            let k: Vec<Key> = group_by.iter().map(|e| key(&eval(e, &Ctx { row, aggregates: &no_aggregates }))).collect();
            let at = *by_key.entry(k).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            groups[at].push(row);
        }
        // Aggregates over no rows still make one row (`COUNT(*)` is 0).
        if groups.is_empty() && group_by.is_empty() {
            groups.push(Vec::new());
        }
        let empty = vec![Value::Null; width];
        for group in groups {
            let values: Vec<Value> = p.aggregates.iter().map(|a| aggregate(a, &group)).collect();
            // Bare columns take the group's last row, as SQLite's do.
            let row = group.last().copied().unwrap_or(&empty);
            if having.as_ref().is_some_and(|h| truth(&eval(h, &Ctx { row, aggregates: &values })) != Some(true)) {
                continue;
            }
            emit(row, &values);
        }
    }
    if distinct {
        let mut seen = std::collections::HashSet::new();
        results.retain(|(_, out)| seen.insert(out.iter().map(key).collect::<Vec<_>>()));
    }
    if !order.is_empty() {
        results.sort_by(|(a, _), (b, _)| {
            a.iter().zip(b).zip(&order).map(|((x, y), (_, desc))| if *desc { compare(x, y).reverse() } else { compare(x, y) }).find(|o| *o != Ordering::Equal).unwrap_or(Ordering::Equal)
        });
    }
    Ok(QueryResult {
        table: p.sources.first().map(|s| s.table.name.clone()).unwrap_or_default(),
        columns: outputs.into_iter().map(|o| o.name).collect(),
        rows: results.into_iter().skip(skip).take(take.min(limit)).map(|(_, out)| out).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (Database, Vec<Table>) {
        let db = Database::open(include_bytes!("testdata/sqlite/base.db").to_vec(), None).unwrap();
        let tables = db.tables();
        (db, tables)
    }

    fn shop() -> (Database, Vec<Table>) {
        let db = Database::open(include_bytes!("testdata/sqlite/shop.db").to_vec(), None).unwrap();
        let tables = db.tables();
        (db, tables)
    }

    fn ids(r: &QueryResult) -> Vec<i64> {
        r.rows.iter().map(|row| match row[0] {
            Value::Integer(i) => i,
            _ => -1,
        }).collect()
    }

    /// Rows as SQLite's Python module prints them, to compare with it.
    fn show(r: &QueryResult) -> Vec<String> {
        r.rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|v| match v {
                        Value::Null => "None".to_owned(),
                        Value::Text(t) => format!("'{t}'"),
                        Value::Real(_) => text(v).unwrap(),
                        v => v.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .collect()
    }

    #[test]
    fn selects_filters_and_orders() {
        let (db, tables) = db();
        let r = run(&db, &tables, "select id, name from users where age >= 68 and name like 'USER2%' order by id desc;", 100).unwrap();
        assert_eq!(r.columns, ["id", "name"]);
        assert_eq!(ids(&r), [299, 298, 249, 248]);
        assert_eq!(r.rows[0][1], Value::Text("user299".into()));
        let r = run(&db, &tables, "SELECT * FROM \"users\" WHERE id IN (3, 1, 500) OR (score > 447 AND NOT id = 300)", 100).unwrap();
        assert_eq!(ids(&r), [1, 3, 299]);
        assert_eq!(r.columns.len(), 5);
        let r = run(&db, &tables, "SELECT id FROM users WHERE id BETWEEN 10 AND 12 LIMIT 2", 100).unwrap();
        assert_eq!(ids(&r), [10, 11]);
        let r = run(&db, &tables, "SELECT id FROM users ORDER BY score DESC LIMIT 2 OFFSET 1", 100).unwrap();
        assert_eq!(ids(&r), [299, 298]);
        let r = run(&db, &tables, "SELECT COUNT(*) FROM users WHERE age = '25' OR avatar IS NULL", 100).unwrap();
        assert_eq!((r.columns.clone(), r.rows.clone()), (vec!["COUNT(*)".to_owned()], vec![vec![Value::Integer(6)]]));
        let r = run(&db, &tables, "SELECT rowid FROM users WHERE id < 3", 1).unwrap();
        assert_eq!(ids(&r), [1]);
    }

    // The expected rows below are what Python's sqlite3 prints for the same queries.
    #[test]
    fn expressions_and_functions() {
        let (db, tables) = shop();
        let q = |sql: &str| show(&run(&db, &tables, sql, 100).unwrap());
        assert_eq!(
            q("SELECT id, upper(name) || '/' || city AS label, length(name), id * 10 + 1, 7 / 2, 7 / 2.0, 7 % 3, -id FROM customers ORDER BY id LIMIT 2"),
            ["1, 'ADA/London', 3, 11, 3, 3.5, 1, -1", "2, 'BOB/Paris', 3, 21, 3, 3.5, 1, -2"]
        );
        assert_eq!(
            q("SELECT id, CASE WHEN amount >= 100 THEN 'big' WHEN amount IS NULL THEN 'none' ELSE 'small' END, coalesce(note, '-'), CAST(amount AS INTEGER), typeof(amount), round(amount * 1.1, 1) FROM orders ORDER BY id"),
            [
                "1, 'small', 'first', 50, 'real', 55.6",
                "2, 'big', '-', 120, 'real', 132.0",
                "3, 'small', '-', 30, 'integer', 33.0",
                "4, 'none', 'gift', None, 'null', None",
                "5, 'big', '-', 200, 'real', 220.0",
                "6, 'small', '-', 75, 'real', 83.1",
            ]
        );
        assert_eq!(q("SELECT substr(name, 2), substr(name, -2, 1), instr(city, 'o'), replace(city, 'o', '0'), trim('  x  '), nullif(id, 1) FROM customers WHERE id <= 2"), [
            "'da', 'd', 2, 'L0nd0n', 'x', None",
            "'ob', 'o', 0, 'Paris', 'x', 2"
        ]);
        assert_eq!(q("SELECT 1 + 1, 'a' || 2, 10 / 0, max(3, 9, 4), min(1, NULL)"), ["2, 'a2', None, 9, None"]);
    }

    #[test]
    fn groups_and_aggregates() {
        let (db, tables) = shop();
        let q = |sql: &str| show(&run(&db, &tables, sql, 100).unwrap());
        assert_eq!(
            q("SELECT status, COUNT(*), COUNT(amount), SUM(amount), AVG(amount), MIN(amount), MAX(amount), TOTAL(amount) FROM orders GROUP BY status ORDER BY status"),
            ["'open', 3, 2, 105.5, 52.75, 30, 75.5, 105.5", "'paid', 3, 3, 370.5, 123.5, 50.5, 200.0, 370.5"]
        );
        assert_eq!(q("SELECT customer_id, COUNT(*) AS n FROM orders GROUP BY 1 HAVING n > 1 ORDER BY n DESC, customer_id"), ["1, 3", "2, 2"]);
        assert_eq!(q("SELECT group_concat(id, '-'), COUNT(DISTINCT status), SUM(id) FROM orders"), ["'1-2-3-4-5-6', 2, 21"]);
        assert_eq!(q("SELECT COUNT(*), SUM(amount), MAX(id) FROM orders WHERE id > 100"), ["0, None, None"]);
        assert_eq!(q("SELECT DISTINCT status FROM orders ORDER BY 1"), ["'open'", "'paid'"]);
    }

    #[test]
    fn joins() {
        let (db, tables) = shop();
        let q = |sql: &str| show(&run(&db, &tables, sql, 100).unwrap());
        assert_eq!(
            q("SELECT c.name, o.id, o.amount FROM customers c JOIN orders o ON o.customer_id = c.id WHERE o.status = 'paid' ORDER BY o.id"),
            ["'Ada', 1, 50.5", "'Bob', 2, 120.0", "'Ada', 5, 200.0"]
        );
        assert_eq!(
            q("SELECT c.name, COUNT(o.id), SUM(o.amount) FROM customers AS c LEFT JOIN orders AS o ON c.id = o.customer_id GROUP BY c.id ORDER BY c.id"),
            ["'Ada', 3, 280.5", "'Bob', 2, 120.0", "'Cy', 0, None", "'Dee', 1, 75.5"]
        );
        assert_eq!(q("SELECT customers.name, orders.id FROM customers, orders WHERE customers.id = orders.customer_id AND orders.amount > 100 ORDER BY 2"), ["'Bob', 2", "'Ada', 5"]);
        assert_eq!(q("SELECT COUNT(*) FROM customers CROSS JOIN orders"), ["24"]);
        assert_eq!(q("SELECT a.name, b.name FROM customers a JOIN customers b ON a.city = b.city AND a.id < b.id"), ["'Ada', 'Dee'"]);
        let r = run(&db, &tables, "SELECT * FROM customers c JOIN orders o ON o.customer_id = c.id WHERE o.id = 1", 10).unwrap();
        assert_eq!(r.columns, ["id", "name", "city", "id", "customer_id", "amount", "status", "note"]);
    }

    #[test]
    fn refuses_what_it_cannot_run() {
        let (db, tables) = shop();
        let err = |sql: &str| run(&db, &tables, sql, 10).unwrap_err();
        assert!(err("DELETE FROM orders").starts_with("Only SELECT runs here"));
        assert_eq!(err("SELECT * FROM nope"), "No table named nope");
        assert_eq!(err("SELECT height FROM orders"), "No column named height in orders");
        assert_eq!(err("SELECT id FROM customers JOIN orders ON customer_id = customers.id"), "Column id is in more than one table: write table.id");
        assert_eq!(err("SELECT * FROM orders WHERE id IN (SELECT id FROM customers)"), "Subqueries aren't supported here");
        assert_eq!(err("SELECT id FROM orders UNION SELECT id FROM customers"), "Not supported here: UNION");
        assert_eq!(err("SELECT * FROM orders WHERE COUNT(*) > 1"), "COUNT() can't be used here");
        assert_eq!(err("SELECT SUM(MAX(id)) FROM orders"), "Aggregates can't be nested");
        assert_eq!(err("SELECT median(id) FROM orders"), "Unknown function MEDIAN()");
        assert_eq!(err("SELECT * FROM orders WHERE note = 'x"), "A string isn't closed");
    }

    #[test]
    fn like_and_nulls() {
        assert!(like("Hello", "h_LL%"));
        assert!(!like("Hello", "h_l"));
        let cx = Ctx { row: &[], aggregates: &[] };
        let null = Expr::Binary("=", Box::new(Expr::Literal(Value::Null)), Box::new(Expr::Literal(Value::Integer(1))));
        assert_eq!(eval(&null, &cx), Value::Null);
        assert_eq!(eval(&Expr::Not(Box::new(null.clone())), &cx), Value::Null);
        assert_eq!(eval(&Expr::Binary("OR", Box::new(null), Box::new(Expr::Literal(Value::Integer(1)))), &cx), Value::Integer(1));
        assert_eq!(numeric(&Value::Text(" 12abc".into())), Value::Integer(12));
        assert_eq!(numeric(&Value::Text("1.5e3x".into())), Value::Real(1500.));
    }
}
