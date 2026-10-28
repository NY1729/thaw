//! JSDoc `@param {type} name` tags as a contract for the function that follows the comment. Like a
//! `.d.ts` it is trusted (the function gets an entry guard, `TRUST_JSDOC`), but only parameters are
//! used: `@returns` is ignored, and anything that is not a primitive / array / union of those is `any`.

use crate::dts::DT;

/// Parameter types of one `/** .. */` block (by name), and the offset just after the comment.
pub struct Block {
    pub end: usize,
    pub params: Vec<(String, DT)>,
}

fn split_top(s: &str, sep: char) -> Vec<&str> {
    let (mut depth, mut start, mut out) = (0i32, 0, Vec::new());
    for (i, c) in s.char_indices() {
        match c {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' | ')' | ']' | '}' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// A JSDoc type expression; `None` = optional (`T=`).
fn ty(t: &str) -> DT {
    let t = t.trim();
    if let Some(r) = t.strip_prefix('?') {
        return DT::Union(vec![ty(r), DT::Null]);
    }
    if let Some(r) = t.strip_prefix('!') {
        return ty(r);
    }
    if let Some(r) = t.strip_suffix('=') {
        return DT::Union(vec![ty(r), DT::Undef]);
    }
    if let Some(r) = t.strip_suffix("[]") {
        return DT::Arr(Box::new(ty(r)));
    }
    if t.starts_with('(') && t.ends_with(')') {
        return ty(&t[1..t.len() - 1]);
    }
    let parts = split_top(t, '|');
    if parts.len() > 1 {
        let ms: Vec<DT> = parts.iter().map(|p| ty(p)).collect();
        return if ms.contains(&DT::Any) {
            DT::Any
        } else {
            DT::Union(ms)
        };
    }
    if let Some(inner) = t
        .strip_prefix("Array<")
        .or_else(|| t.strip_prefix("Array.<"))
        .and_then(|r| r.strip_suffix('>'))
    {
        return DT::Arr(Box::new(ty(inner)));
    }
    match t {
        "number" | "Number" => DT::Num,
        "string" | "String" => DT::Str,
        "boolean" | "Boolean" => DT::Bool,
        "undefined" | "void" => DT::Undef,
        "null" => DT::Null,
        _ => DT::Any,
    }
}

fn parse_block(body: &str) -> Vec<(String, DT)> {
    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.trim().trim_start_matches('*').trim();
        let Some(rest) = line
            .strip_prefix("@param")
            .or_else(|| line.strip_prefix("@arg"))
            .or_else(|| line.strip_prefix("@argument"))
        else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('{') else {
            continue;
        };
        // matching brace
        let (mut depth, mut close) = (1, None);
        for (i, c) in rest.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else { continue };
        let mut t = ty(&rest[..close]);
        let name = rest[close + 1..].trim_start();
        let (name, optional) = match name.strip_prefix('[') {
            Some(n) => (n.split([']', '=']).next().unwrap_or(""), true),
            None => (name.split(char::is_whitespace).next().unwrap_or(""), false),
        };
        if optional {
            t = DT::Union(vec![t, DT::Undef]);
        }
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        {
            continue;
        }
        out.push((name.to_string(), t));
    }
    out
}

/// Every `/** .. */` block of `src` that has `@param {..}` tags.
pub fn blocks(src: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = src[from..].find("/**") {
        let start = from + i;
        let Some(j) = src[start + 3..].find("*/") else {
            break;
        };
        let end = start + 3 + j + 2;
        let params = parse_block(&src[start + 3..end - 2]);
        if !params.is_empty() {
            out.push(Block { end, params });
        }
        from = end;
    }
    out
}

/// Whether the text between the end of a comment and the start of a function is only the declaration
/// head (`export`, `var x =`, `exports.f =`, `key:`, `async`).
pub fn gap_ok(gap: &str) -> bool {
    gap.len() < 96
        && !gap.contains([';', '{', '}', '(', ')', '/', '"', '\'', '`', ','])
        && !gap.contains("function ")
}
