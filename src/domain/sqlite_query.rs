//! The Database Inspector's queries, run on the read-only database file
//! without SQLite: one table's `SELECT` with `WHERE` (comparisons, `LIKE`,
//! `IN`, `IS [NOT] NULL`, `AND`, `OR`, `NOT`, parentheses), `ORDER BY`,
//! `LIMIT` / `OFFSET` and `COUNT(*)`. Anything else, and every statement
//! that would change the database, is refused with a reason.

use std::cmp::Ordering;

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
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
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
            let sym = ["<=", ">=", "<>", "!=", "=="].into_iter().find(|s| *s == two);
            if let Some(s) = sym {
                out.push(Token::Sym(s));
                i += 2;
            } else {
                let one = ["(", ")", ",", "*", "=", "<", ">", ";", "-", "+"].into_iter().find(|s| s.starts_with(c)).ok_or_else(|| format!("Unexpected '{c}'"))?;
                out.push(Token::Sym(one));
                i += 1;
            }
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Column(usize),
    Literal(Value),
    Compare(Box<Expr>, &'static str, Box<Expr>),
    Like(Box<Expr>, Box<Expr>, bool),
    In(Box<Expr>, Vec<Expr>, bool),
    IsNull(Box<Expr>, bool),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
}

enum Output {
    Columns(Vec<usize>),
    Count,
}

struct Parser<'a> {
    tokens: Vec<Token>,
    at: usize,
    table: &'a Table,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn keyword(&mut self, word: &str) -> bool {
        if matches!(self.peek(), Some(Token::Word(w)) if w.eq_ignore_ascii_case(word)) {
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

    fn column(&self, name: &str) -> Result<usize, String> {
        if let Some(i) = self.table.columns.iter().position(|c| c.eq_ignore_ascii_case(name)) {
            return Ok(i);
        }
        // `rowid` names the INTEGER PRIMARY KEY column when there is one.
        if ["rowid", "_rowid_", "oid"].iter().any(|r| r.eq_ignore_ascii_case(name)) {
            if let Some(i) = self.table.rowid_column {
                return Ok(i);
            }
        }
        Err(format!("No column named {name} in {}", self.table.name))
    }

    fn name(&mut self) -> Result<String, String> {
        match self.peek().cloned() {
            Some(Token::Word(w) | Token::Name(w)) => {
                self.at += 1;
                Ok(w)
            }
            _ => Err("Expected a column name".into()),
        }
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut left = self.and()?;
        while self.keyword("OR") {
            left = Expr::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut left = self.not()?;
        while self.keyword("AND") {
            left = Expr::And(Box::new(left), Box::new(self.not()?));
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
        let left = self.operand()?;
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
            return Ok(Expr::Like(Box::new(left), Box::new(self.operand()?), negated));
        }
        if self.keyword("IN") {
            self.expect("(")?;
            let mut list = vec![self.operand()?];
            while self.symbol(",") {
                list.push(self.operand()?);
            }
            self.expect(")")?;
            return Ok(Expr::In(Box::new(left), list, negated));
        }
        if self.keyword("BETWEEN") {
            let low = self.operand()?;
            if !self.keyword("AND") {
                return Err("Expected AND in BETWEEN".into());
            }
            let high = self.operand()?;
            let inside = Expr::And(Box::new(Expr::Compare(Box::new(left.clone()), ">=", Box::new(low))), Box::new(Expr::Compare(Box::new(left), "<=", Box::new(high))));
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
                return Ok(Expr::Compare(Box::new(left), op, Box::new(self.operand()?)));
            }
        }
        Ok(left)
    }

    fn operand(&mut self) -> Result<Expr, String> {
        if self.symbol("(") {
            let e = self.or()?;
            self.expect(")")?;
            return Ok(e);
        }
        let negative = self.symbol("-");
        if !negative {
            self.symbol("+");
        }
        match self.peek().cloned() {
            Some(Token::Num(n, integer)) => {
                self.at += 1;
                let n = if negative { -n } else { n };
                Ok(Expr::Literal(if integer && n.abs() < 9.2e18 { Value::Integer(n as i64) } else { Value::Real(n) }))
            }
            _ if negative => Err("Expected a number after '-'".into()),
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
            Some(Token::Word(_) | Token::Name(_)) => {
                let name = self.name()?;
                Ok(Expr::Column(self.column(&name)?))
            }
            _ => Err("Expected a column, a number or a string".into()),
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
        (Value::Integer(_) | Value::Real(_), Value::Integer(_) | Value::Real(_)) => number(a).partial_cmp(&number(b)).unwrap_or(Ordering::Equal),
        (Value::Text(x), Value::Text(y)) => x.cmp(y),
        (Value::Blob(x), Value::Blob(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

fn number(v: &Value) -> f64 {
    match v {
        Value::Integer(i) => *i as f64,
        Value::Real(r) => *r,
        _ => 0.,
    }
}

/// A column compared with a literal takes the column's kind, as SQLite's
/// type affinity does for `age = '30'`.
fn coerce(literal: &Value, like: &Value) -> Value {
    match (literal, like) {
        (Value::Text(t), Value::Integer(_) | Value::Real(_)) => match t.trim().parse::<i64>() {
            Ok(i) => Value::Integer(i),
            Err(_) => t.trim().parse::<f64>().map(Value::Real).unwrap_or_else(|_| literal.clone()),
        },
        (Value::Integer(i), Value::Text(_)) => Value::Text(i.to_string()),
        (Value::Real(r), Value::Text(_)) => Value::Text(r.to_string()),
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

fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Text(t) => Some(t.clone()),
        v => Some(v.to_string()),
    }
}

/// `None` is SQL's unknown (a comparison with NULL).
fn eval(e: &Expr, row: &[Value]) -> Option<bool> {
    let value = |e: &Expr| -> Value {
        match e {
            Expr::Column(i) => row.get(*i).cloned().unwrap_or(Value::Null),
            Expr::Literal(v) => v.clone(),
            other => match eval(other, row) {
                Some(b) => Value::Integer(b as i64),
                None => Value::Null,
            },
        }
    };
    let pair = |l: &Expr, r: &Expr| {
        let (a, b) = (value(l), value(r));
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
        Expr::Compare(l, op, r) => {
            let (a, b) = pair(l, r);
            if a == Value::Null || b == Value::Null {
                return None;
            }
            let o = compare(&a, &b);
            Some(match *op {
                "=" => o == Ordering::Equal,
                "!=" => o != Ordering::Equal,
                "<" => o == Ordering::Less,
                "<=" => o != Ordering::Greater,
                ">" => o == Ordering::Greater,
                _ => o != Ordering::Less,
            })
        }
        Expr::Like(l, p, negated) => {
            let (t, p) = (text_of(&value(l))?, text_of(&value(p))?);
            Some(like(&t, &p) != *negated)
        }
        Expr::In(l, list, negated) => {
            let mut unknown = false;
            for item in list {
                let (a, b) = pair(l, item);
                if a == Value::Null || b == Value::Null {
                    unknown = true;
                } else if compare(&a, &b) == Ordering::Equal {
                    return Some(!negated);
                }
            }
            if unknown { None } else { Some(*negated) }
        }
        Expr::IsNull(l, negated) => Some((value(l) == Value::Null) != *negated),
        Expr::And(l, r) => match (eval(l, row), eval(r, row)) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        },
        Expr::Or(l, r) => match (eval(l, row), eval(r, row)) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        },
        Expr::Not(inner) => eval(inner, row).map(|b| !b),
        Expr::Column(_) | Expr::Literal(_) => match value(e) {
            Value::Null => None,
            v => Some(number(&v) != 0. || matches!(v, Value::Text(ref t) if t.trim().parse::<f64>().is_ok_and(|n| n != 0.))),
        },
    }
}

/// A query's answer: its column names and rows.
#[derive(Debug, PartialEq)]
pub struct QueryResult {
    pub table: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

/// Runs `sql` on the database; `limit` caps the rows kept (after ORDER BY).
pub fn run(db: &Database, tables: &[Table], sql: &str, limit: usize) -> Result<QueryResult, String> {
    let mut toks = tokens(sql)?;
    while matches!(toks.last(), Some(Token::Sym(";"))) {
        toks.pop();
    }
    match toks.first() {
        Some(Token::Word(w)) if w.eq_ignore_ascii_case("SELECT") => {}
        Some(Token::Word(w)) => return Err(format!("Only SELECT runs here: the database is read from a copy, so {} can't change the app's data", w.to_uppercase())),
        _ => return Err("Type a SELECT statement".into()),
    }
    // The table first, so column names can be checked as they are read.
    let from = toks.iter().position(|t| matches!(t, Token::Word(w) if w.eq_ignore_ascii_case("FROM"))).ok_or("Expected FROM")?;
    let table_name = match toks.get(from + 1) {
        Some(Token::Word(w) | Token::Name(w)) => w.clone(),
        _ => return Err("Expected a table name after FROM".into()),
    };
    let table = tables.iter().find(|t| t.name.eq_ignore_ascii_case(&table_name)).ok_or_else(|| format!("No table named {table_name}"))?;
    let mut p = Parser { tokens: toks, at: 1, table };
    p.keyword("ALL");
    if p.keyword("DISTINCT") {
        return Err("DISTINCT isn't supported here".into());
    }
    let output = if p.symbol("*") {
        Output::Columns((0..table.columns.len()).collect())
    } else if p.keyword("COUNT") {
        p.expect("(")?;
        p.expect("*")?;
        p.expect(")")?;
        Output::Count
    } else {
        let mut cols = vec![{
            let n = p.name()?;
            p.column(&n)?
        }];
        while p.symbol(",") {
            let n = p.name()?;
            cols.push(p.column(&n)?);
        }
        Output::Columns(cols)
    };
    if p.at != from {
        return Err("Only column names, * or COUNT(*) can be selected here".into());
    }
    p.at = from + 2;
    let filter = if p.keyword("WHERE") { Some(p.or()?) } else { None };
    let mut order = Vec::new();
    if p.keyword("ORDER") {
        if !p.keyword("BY") {
            return Err("Expected BY after ORDER".into());
        }
        loop {
            let n = p.name()?;
            let c = p.column(&n)?;
            let desc = p.keyword("DESC");
            if !desc {
                p.keyword("ASC");
            }
            order.push((c, desc));
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
            Token::Word(w) | Token::Name(w) => w.clone(),
            Token::Str(s) => format!("'{s}'"),
            Token::Num(n, _) => n.to_string(),
            Token::Sym(s) => (*s).to_owned(),
        }));
    }
    let mut rows: Vec<Vec<Value>> = db.rows(table, usize::MAX)?.into_iter().filter(|r| filter.as_ref().is_none_or(|f| eval(f, r) == Some(true))).collect();
    if let Output::Count = output {
        return Ok(QueryResult { table: table.name.clone(), columns: vec!["COUNT(*)".into()], rows: vec![vec![Value::Integer(rows.len() as i64)]] });
    }
    if !order.is_empty() {
        rows.sort_by(|a, b| {
            order.iter().map(|&(c, desc)| {
                let o = compare(a.get(c).unwrap_or(&Value::Null), b.get(c).unwrap_or(&Value::Null));
                if desc { o.reverse() } else { o }
            }).find(|o| *o != Ordering::Equal).unwrap_or(Ordering::Equal)
        });
    }
    let Output::Columns(cols) = output else { unreachable!() };
    let rows = rows.into_iter().skip(skip).take(take.min(limit)).map(|r| cols.iter().map(|&c| r.get(c).cloned().unwrap_or(Value::Null)).collect()).collect();
    Ok(QueryResult { table: table.name.clone(), columns: cols.iter().map(|&c| table.columns[c].clone()).collect(), rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (Database, Vec<Table>) {
        let db = Database::open(include_bytes!("testdata/sqlite/base.db").to_vec(), None).unwrap();
        let tables = db.tables();
        (db, tables)
    }

    fn ids(r: &QueryResult) -> Vec<i64> {
        r.rows.iter().map(|row| match row[0] {
            Value::Integer(i) => i,
            _ => -1,
        }).collect()
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
        assert_eq!(r.rows, [[Value::Integer(6)]]);
        let r = run(&db, &tables, "SELECT rowid FROM users WHERE id < 3", 1).unwrap();
        assert_eq!(ids(&r), [1]);
    }

    #[test]
    fn refuses_what_it_cannot_run() {
        let (db, tables) = db();
        let err = |sql: &str| run(&db, &tables, sql, 10).unwrap_err();
        assert!(err("DELETE FROM users").starts_with("Only SELECT runs here"));
        assert_eq!(err("SELECT * FROM nope"), "No table named nope");
        assert_eq!(err("SELECT height FROM users"), "No column named height in users");
        assert_eq!(err("SELECT * FROM users GROUP BY age"), "Not supported here: GROUP");
        assert_eq!(err("SELECT age + 1 FROM users"), "Only column names, * or COUNT(*) can be selected here");
        assert_eq!(err("SELECT * FROM users WHERE name = 'x"), "A string isn't closed");
    }

    #[test]
    fn like_and_nulls() {
        assert!(like("Hello", "h_LL%"));
        assert!(!like("Hello", "h_l"));
        let null = Expr::Compare(Box::new(Expr::Literal(Value::Null)), "=", Box::new(Expr::Literal(Value::Integer(1))));
        assert_eq!(eval(&null, &[]), None);
        assert_eq!(eval(&Expr::Not(Box::new(null.clone())), &[]), None);
        assert_eq!(eval(&Expr::Or(Box::new(null), Box::new(Expr::Literal(Value::Integer(1)))), &[]), Some(true));
    }
}
