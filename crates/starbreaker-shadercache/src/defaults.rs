//! Shader parameter defaults from decoded shader source ([`param_defaults`]).
//!
//! The decoded effect source (`hlsl::decode` of a family's `.cfxb` and its
//! `.cfib` includes) declares every public parameter as
//! `type Name [;] < annotations > = default;` (the decoder may join `>` `=`
//! into `>=`; a stray `;` before `<` occurs and is tolerated, as the engine
//! does). Defaults differ per family for the same name and `#define` / `#if`
//! can change them, so they are resolved by walking the family file and its
//! `#include`s in order through a small preprocessor:
//! - `#if` / `#elif` integer expressions with C-preprocessor semantics:
//!   `%FLAG` is 1 when in the material's flags or `#define`d (`%_RT_*`
//!   runtime flags are 0 unless defined), a macro is its integer value, an
//!   undefined name is 0; literals, `defined(X)`, `! - * / + - < <= > >= ==
//!   != && ||` and parentheses;
//! - `#ifdef` / `#ifndef` / `#else` / `#endif` / `#define` / `#undef`;
//! - `#include Name [code…]`: each stem is walked once, at its first include.
//!
//! The last definition on the active path wins. An expression it cannot
//! evaluate is an error naming the file and line, never a guess; a value it
//! cannot parse (a macro, an expression) leaves that parameter out.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::error::ShaderCacheError;

/// One parameter's declared default.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDefault {
    /// Declared type (`float`, `float4`, `int`, `bool`, …).
    pub ty: String,
    /// The default's lanes (bools are 0 / 1).
    pub values: Vec<f32>,
    /// The decoded file (stem) holding the winning definition.
    pub file: String,
}

const TYPES: &[&str] = &[
    "float", "float2", "float3", "float4", "half", "half2", "half3", "half4", "int", "uint", "bool", "enum",
];

fn error(file: &str, line: usize, why: impl std::fmt::Display) -> ShaderCacheError {
    ShaderCacheError::Defaults(format!("{file} line {line}: {why}"))
}

struct Walker<'a> {
    sources: &'a dyn Fn(&str) -> Option<String>,
    flags: &'a BTreeSet<String>,
    defines: HashMap<String, String>,
    visited: HashSet<String>,
    out: BTreeMap<String, ParamDefault>,
}

/// `#if` stack frame.
struct Frame {
    parent: bool,
    taken: bool,
    active: bool,
}

impl Walker<'_> {
    fn walk(&mut self, stem: &str) -> Result<(), ShaderCacheError> {
        if !self.visited.insert(stem.to_owned()) {
            return Ok(());
        }
        let text = (self.sources)(stem)
            .ok_or_else(|| ShaderCacheError::Defaults(format!("decoded source for {stem} is not available")))?;
        let mut stack: Vec<Frame> = Vec::new();
        let mut code = String::new();
        let active = |stack: &[Frame]| stack.last().is_none_or(|f| f.active);
        for (i, line) in text.split('\n').enumerate() {
            let n = i + 1;
            let t = line.trim_start();
            if !t.starts_with('#') {
                if active(&stack) {
                    code.push_str(line);
                    code.push('\n');
                }
                continue;
            }
            let (directive, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
            let rest = rest.trim();
            // The decoder can join code onto a directive line; `tail` is that
            // code, kept when the directive leaves the path active.
            let mut tail = "";
            match directive {
                "#if" | "#ifdef" | "#ifndef" => {
                    let parent = active(&stack);
                    let cond = match directive {
                        "#if" => {
                            let (c, t) = self.eval(rest, stem, n)?;
                            tail = t;
                            c
                        }
                        "#ifdef" => self.is_defined(first_word(rest)),
                        _ => !self.is_defined(first_word(rest)),
                    };
                    if directive != "#if" {
                        tail = after_first_word(rest);
                    }
                    stack.push(Frame { parent, taken: cond, active: parent && cond });
                }
                "#elif" => {
                    let taken = stack.last().ok_or_else(|| error(stem, n, "#elif without #if"))?.taken;
                    let cond = if taken {
                        false
                    } else {
                        let (c, t) = self.eval(rest, stem, n)?;
                        tail = t;
                        c
                    };
                    let f = stack.last_mut().expect("checked");
                    f.active = f.parent && cond;
                    f.taken |= cond;
                }
                "#else" => {
                    let f = stack.last_mut().ok_or_else(|| error(stem, n, "#else without #if"))?;
                    f.active = f.parent && !f.taken;
                    f.taken = true;
                    tail = rest;
                }
                "#endif" => {
                    stack.pop().ok_or_else(|| error(stem, n, "#endif without #if"))?;
                    tail = rest;
                }
                "#define" if active(&stack) => {
                    let (name, value) = rest.split_once(char::is_whitespace).unwrap_or((rest, "1"));
                    self.defines.insert(name.to_owned(), value.trim().to_owned());
                }
                "#undef" if active(&stack) => {
                    self.defines.remove(first_word(rest));
                }
                "#include" if active(&stack) => {
                    let (name, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                    self.scan(&std::mem::take(&mut code), stem);
                    self.walk(name)?;
                    code.push_str(tail);
                    code.push('\n');
                }
                _ => {}
            }
            if !tail.is_empty() && active(&stack) {
                code.push_str(tail);
                code.push('\n');
            }
        }
        if !stack.is_empty() {
            return Err(error(stem, text.split('\n').count(), "unterminated #if"));
        }
        self.scan(&code, stem);
        Ok(())
    }

    fn is_defined(&self, name: &str) -> bool {
        self.defines.contains_key(name)
    }

    /// The integer value of a name in a condition: `%FLAG` is 1 when in the
    /// material's flags or defined; a defined macro is its integer value
    /// (an empty definition is 1); anything undefined is 0, as in C. A macro
    /// whose value is not an integer cannot be evaluated.
    fn value(&self, name: &str) -> Option<i64> {
        if let Some(flag) = name.strip_prefix('%')
            && self.flags.contains(flag)
        {
            return Some(1);
        }
        match self.defines.get(name) {
            None => Some(0),
            Some(v) if v.is_empty() => Some(1),
            Some(v) => {
                let (toks, stop) = tokenize(v);
                let toks: Vec<Tok> = toks.into_iter().map(|(t, _)| t).collect();
                let mut p = Parser { tokens: &toks, at: 0, w: self };
                let n = p.expr()?;
                (p.at == toks.len() && stop == v.len()).then_some(n)
            }
        }
    }

    /// The longest valid expression at the start of `expr`, and the text
    /// after it (code the decoder joined onto the line).
    fn eval<'e>(&self, expr: &'e str, file: &str, line: usize) -> Result<(bool, &'e str), ShaderCacheError> {
        let (spanned, stop) = tokenize(expr);
        let tokens: Vec<Tok> = spanned.iter().map(|(t, _)| t.clone()).collect();
        let mut p = Parser { tokens: &tokens, at: 0, w: self };
        let bad = || error(file, line, format!("cannot evaluate `#if {expr}`"));
        let v = p.expr().ok_or_else(bad)?;
        let rest = spanned.get(p.at).map_or(stop, |(_, start)| *start);
        if p.at == 0 {
            return Err(bad());
        }
        Ok((v != 0, expr[rest..].trim()))
    }

    /// Record every annotated declaration in `code`.
    fn scan(&mut self, code: &str, file: &str) {
        let b = code.as_bytes();
        let mut i = 0;
        while let Some(off) = code[i..].find('<') {
            let lt = i + off;
            i = lt + 1;
            let Some((ty, name)) = declaration_before(code, lt) else { continue };
            let Some(gt) = matching_gt(b, lt) else { continue };
            let after = code[gt + 1..].trim_start();
            let Some(rest) = after.strip_prefix('=') else { continue };
            let Some(end) = rest.find(';') else { continue };
            if let Some(values) = parse_value(&rest[..end]) {
                self.out.insert(name.to_owned(), ParamDefault { ty: ty.to_owned(), values, file: file.to_owned() });
            }
            i = gt + 1;
        }
    }
}

fn first_word(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or("")
}

fn after_first_word(s: &str) -> &str {
    s.split_once(char::is_whitespace).map_or("", |(_, rest)| rest.trim())
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `(type, name)` when `code[..lt]` ends with `type name [;]` and whitespace.
fn declaration_before(code: &str, lt: usize) -> Option<(&str, &str)> {
    let s = code[..lt].trim_end();
    let s = s.strip_suffix(';').map_or(s, str::trim_end);
    let name_start = s.rfind(|c: char| !is_ident(c)).map_or(0, |p| p + 1);
    let name = &s[name_start..];
    let s = s[..name_start].trim_end();
    let ty_start = s.rfind(|c: char| !is_ident(c)).map_or(0, |p| p + 1);
    let ty = &s[ty_start..];
    (!name.is_empty() && name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') && TYPES.contains(&ty))
        .then_some((ty, name))
}

/// The `>` closing the annotation block opened at `lt`, skipping quoted text.
fn matching_gt(b: &[u8], lt: usize) -> Option<usize> {
    let (mut quoted, mut depth) = (false, 0usize);
    for (k, &c) in b.iter().enumerate().skip(lt) {
        match c {
            b'"' => quoted = !quoted,
            b'<' if !quoted => depth += 1,
            b'>' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_number(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = s.strip_suffix(['f', 'F']).unwrap_or(s);
    match s {
        "true" => Some(1.0),
        "false" => Some(0.0),
        _ => s.parse().ok(),
    }
}

fn parse_value(v: &str) -> Option<Vec<f32>> {
    let v = v.trim();
    let inner = if let Some(body) = v.strip_prefix('{').and_then(|b| b.strip_suffix('}')) {
        body
    } else if let Some(open) = v.find('(').filter(|_| v.ends_with(')')) {
        let ty = v[..open].trim();
        if !TYPES.contains(&ty) {
            return None;
        }
        &v[open + 1..v.len() - 1]
    } else {
        return parse_number(v).map(|x| vec![x]);
    };
    inner.split(',').map(parse_number).collect()
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Open,
    Close,
    /// `!` `&&` `||` `==` `!=` `<` `<=` `>` `>=` `+` `-` `*` `/` `%`
    Op(&'static str),
    Num(i64),
    Word(String),
}

const OPS: &[&str] = &["&&", "||", "==", "!=", "<=", ">=", "<", ">", "!", "+", "-", "*", "/"];

/// Expression tokens with their byte offsets, up to the first character an
/// expression cannot contain; also returns that stop offset.
fn tokenize(s: &str) -> (Vec<(Tok, usize)>, usize) {
    let b = s.as_bytes();
    let (mut i, mut out) = (0, Vec::new());
    'next: while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == '(' || c == ')' {
            out.push((if c == '(' { Tok::Open } else { Tok::Close }, i));
            i += 1;
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < b.len() && (b[i] as char).is_ascii_alphanumeric() {
                i += 1;
            }
            let text = s[start..i].trim_end_matches(['u', 'U', 'l', 'L']);
            let n = if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
                i64::from_str_radix(hex, 16).ok()
            } else {
                text.parse().ok()
            };
            match n {
                Some(n) => out.push((Tok::Num(n), start)),
                None => return (out, start),
            }
            continue;
        }
        if c == '%' || is_ident(c) {
            let start = i;
            i += 1;
            while i < b.len() && is_ident(b[i] as char) {
                i += 1;
            }
            out.push((Tok::Word(s[start..i].to_owned()), start));
            continue;
        }
        for op in OPS {
            if s[i..].starts_with(op) {
                out.push((Tok::Op(op), i));
                i += op.len();
                continue 'next;
            }
        }
        return (out, i);
    }
    (out, b.len())
}

struct Parser<'t, 'w, 'a> {
    tokens: &'t [Tok],
    at: usize,
    w: &'w Walker<'a>,
}

/// Binary operator levels, loosest first (C precedence).
const LEVELS: &[&[&str]] = &[&["||"], &["&&"], &["==", "!="], &["<", "<=", ">", ">="], &["+", "-"], &["*", "/"]];

impl Parser<'_, '_, '_> {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.at)
    }

    fn expr(&mut self) -> Option<i64> {
        self.level(0)
    }

    fn level(&mut self, k: usize) -> Option<i64> {
        if k == LEVELS.len() {
            return self.unary();
        }
        let mut v = self.level(k + 1)?;
        while let Some(Tok::Op(op)) = self.peek().cloned() {
            if !LEVELS[k].contains(&op) {
                break;
            }
            self.at += 1;
            let r = self.level(k + 1)?;
            v = match op {
                "||" => i64::from(v != 0 || r != 0),
                "&&" => i64::from(v != 0 && r != 0),
                "==" => i64::from(v == r),
                "!=" => i64::from(v != r),
                "<" => i64::from(v < r),
                "<=" => i64::from(v <= r),
                ">" => i64::from(v > r),
                ">=" => i64::from(v >= r),
                "+" => v.checked_add(r)?,
                "-" => v.checked_sub(r)?,
                "*" => v.checked_mul(r)?,
                _ => v.checked_div(r)?,
            };
        }
        Some(v)
    }

    fn unary(&mut self) -> Option<i64> {
        match self.tokens.get(self.at)?.clone() {
            Tok::Op("!") => {
                self.at += 1;
                Some(i64::from(self.unary()? == 0))
            }
            Tok::Op("-") => {
                self.at += 1;
                self.unary()?.checked_neg()
            }
            Tok::Open => {
                self.at += 1;
                let v = self.expr()?;
                (self.peek() == Some(&Tok::Close)).then(|| self.at += 1)?;
                Some(v)
            }
            Tok::Num(n) => {
                self.at += 1;
                Some(n)
            }
            Tok::Word(w) if w == "defined" => {
                self.at += 1;
                let paren = self.peek() == Some(&Tok::Open);
                if paren {
                    self.at += 1;
                }
                let Some(Tok::Word(name)) = self.tokens.get(self.at).cloned() else { return None };
                self.at += 1;
                if paren {
                    (self.peek() == Some(&Tok::Close)).then(|| self.at += 1)?;
                }
                Some(i64::from(self.w.is_defined(&name)))
            }
            Tok::Word(w) => {
                self.at += 1;
                self.w.value(&w)
            }
            _ => None,
        }
    }
}

/// Every public parameter default of `family`, with the material's `flags`.
/// `sources(stem)` returns a decoded file (`hlsl::decode` of its bin).
pub fn param_defaults(
    family: &str,
    sources: &dyn Fn(&str) -> Option<String>,
    flags: &BTreeSet<String>,
) -> Result<BTreeMap<String, ParamDefault>, ShaderCacheError> {
    let mut w = Walker { sources, flags, defines: HashMap::new(), visited: HashSet::new(), out: BTreeMap::new() };
    w.walk(family)?;
    Ok(w.out)
}
