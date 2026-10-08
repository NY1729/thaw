//! A port of Node v22's `util.inspect` (`lib/internal/util/inspect.js`) for a small
//! value tree. The goal is byte-identical output to `console.log` for the value
//! categories the tree can express; every algorithm below mirrors the Node function of
//! the same name, in UTF-16 code units where Node measures `.length`.
//!
//! Not modelled (see `Node`): symbols, getters/setters, proxies, boxed primitives,
//! typed arrays other than `Buffer`, class instances with extra own state, `showHidden`,
//! `colors`, `sorted`, `numericSeparator`, `compact: true/false`, custom inspect hooks.

use std::fmt::Write as _;

/// The subset of Node's `inspect` options that `console.log` can reach with defaults.
#[derive(Clone, Debug)]
pub(crate) struct InspectOptions {
    /// `None` is `depth: null` (unlimited).
    pub depth: Option<usize>,
    pub break_length: usize,
    /// Node's numeric `compact` mode (`3` by default); `true`/`false` are not ported.
    pub compact: usize,
    pub max_array_length: usize,
    pub max_string_length: usize,
    /// `showHidden`: only the array `[length]` slot is modelled (what `%o` shows).
    pub show_hidden: bool,
}

impl Default for InspectOptions {
    fn default() -> Self {
        Self {
            depth: Some(2),
            break_length: 80,
            compact: 3,
            max_array_length: 100,
            max_string_length: 10000,
            show_hidden: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FnKind {
    Function,
    Async,
    Generator,
    AsyncGenerator,
    /// `[class A extends B]`
    Class {
        extends: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Ctor {
    /// `Object` (no prefix).
    Plain,
    /// A named constructor other than `Object` (`Foo {}`).
    Named(String),
    /// `Object.create(null)` (`[Object: null prototype] {}`).
    NullProto,
}

/// The values `inspect` can describe. Container `id`s identify shared containers for
/// cycle detection (`0` = never referenced again); `Circular(id)` is a back reference
/// to a container that is currently being formatted.
#[derive(Clone, Debug)]
pub(crate) enum Node {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    BigInt(String),
    /// A JS string as UTF-16 code units (may hold lone surrogates).
    Str(Vec<u16>),
    /// `None` items are holes; `extra` are non-index own properties.
    Array {
        id: usize,
        items: Vec<Option<Node>>,
        extra: Vec<(Vec<u16>, Node)>,
    },
    Object {
        id: usize,
        ctor: Ctor,
        entries: Vec<(Vec<u16>, Node)>,
    },
    Map {
        id: usize,
        entries: Vec<(Node, Node)>,
    },
    Set {
        id: usize,
        items: Vec<Node>,
    },
    /// ISO string, or `None` for an invalid date (`Invalid Date`).
    Date(Option<String>),
    /// The `/source/flags` text.
    RegExp(String),
    Buffer(Vec<u8>),
    /// A typed array (`Uint8Array(3) [ 1, 2, 3 ]`) other than `Buffer`.
    Typed {
        name: String,
        items: Vec<Node>,
    },
    Function {
        name: String,
        kind: FnKind,
    },
    /// Text that something else already formatted (a live host value).
    Raw(String),
    Circular(usize),
}

fn len16(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

fn pad_start(text: &str, target: usize) -> String {
    let have = len16(text);
    if have >= target {
        text.to_string()
    } else {
        format!("{}{}", " ".repeat(target - have), text)
    }
}

fn pad_end(text: &str, target: usize) -> String {
    let have = len16(text);
    if have >= target {
        text.to_string()
    } else {
        format!("{}{}", text, " ".repeat(target - have))
    }
}

/// `Number.prototype.toString()` for finite non-negative-zero values (sign handled here).
pub(crate) fn js_number_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value == f64::INFINITY {
        return "Infinity".into();
    }
    if value == f64::NEG_INFINITY {
        return "-Infinity".into();
    }
    if value == 0.0 {
        return "0".into();
    }
    let sign = if value < 0.0 { "-" } else { "" };
    // `{:e}` yields the shortest round-tripping digits, like V8.
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').expect("exponent form");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exponent.parse::<i32>().expect("exponent") + 1;
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let sign_e = if e < 0 { '-' } else { '+' };
        if k == 1 {
            format!("{digits}e{sign_e}{}", e.abs())
        } else {
            format!("{}.{}e{sign_e}{}", &digits[..1], &digits[1..], e.abs())
        }
    };
    format!("{sign}{body}")
}

fn format_number(value: f64) -> String {
    if value == 0.0 && value.is_sign_negative() {
        "-0".into()
    } else {
        js_number_string(value)
    }
}

/// Node's `meta` table entry for a code unit below 160 that needs escaping.
fn meta(point: u16) -> String {
    match point {
        8 => "\\b".into(),
        9 => "\\t".into(),
        10 => "\\n".into(),
        12 => "\\f".into(),
        13 => "\\r".into(),
        39 => "\\'".into(),
        92 => "\\\\".into(),
        _ => format!("\\x{point:02X}"),
    }
}

/// `strEscape`: quote choice plus escaping of control characters, the backslash, the
/// chosen quote and lone surrogates.
pub(crate) fn str_escape(text: &[u16]) -> String {
    let has = |unit: u16| text.contains(&unit);
    let has_dollar_brace = text
        .windows(2)
        .any(|pair| pair == [b'$' as u16, b'{' as u16]);
    // 39 = single quotes (escaped), -1 = double quotes, -2 = backticks.
    let mut quote: i32 = 39;
    if has(39) {
        if !has(34) {
            quote = -1;
        } else if !has(96) && !has_dollar_brace {
            quote = -2;
        }
    }
    let mut out = String::with_capacity(text.len() + 2);
    out.push(match quote {
        -1 => '"',
        -2 => '`',
        _ => '\'',
    });
    let mut i = 0;
    while i < text.len() {
        let point = text[i];
        if (point == 39 && quote == 39) || point == 92 || point < 32 || (point > 126 && point < 160)
        {
            out.push_str(&meta(point));
        } else if (0xd800..=0xdfff).contains(&point) {
            if point <= 0xdbff && i + 1 < text.len() && (0xdc00..=0xdfff).contains(&text[i + 1]) {
                let pair = char::decode_utf16([point, text[i + 1]])
                    .next()
                    .unwrap()
                    .unwrap();
                out.push(pair);
                i += 2;
                continue;
            }
            let _ = write!(out, "\\u{point:x}");
        } else {
            out.push(char::from_u32(point as u32).expect("non-surrogate unit"));
        }
        i += 1;
    }
    out.push(match quote {
        -1 => '"',
        -2 => '`',
        _ => '\'',
    });
    out
}

fn is_identifier_key(key: &[u16]) -> bool {
    let valid = |unit: u16, first: bool| {
        unit < 128 && {
            let byte = unit as u8;
            byte == b'_' || byte.is_ascii_alphabetic() || (!first && byte.is_ascii_digit())
        }
    };
    !key.is_empty()
        && key
            .iter()
            .enumerate()
            .all(|(index, &unit)| valid(unit, index == 0))
}

/// ICU's column width of one code point, from tables generated out of Node itself
/// (`inspect-suite/gen_width.js`): 0 for control/combining/format characters, 2 for
/// East Asian wide and emoji-presentation characters, 1 otherwise.
fn char_width(code: u32) -> usize {
    use crate::inspect_width::{DOUBLE_WIDTH, OTHER_WIDTH, ZERO_WIDTH};
    let in_ranges = |ranges: &[(u32, u32)]| {
        ranges
            .binary_search_by(|&(start, end)| {
                if code < start {
                    std::cmp::Ordering::Greater
                } else if code > end {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    };
    if let Ok(index) = OTHER_WIDTH.binary_search_by_key(&code, |&(point, _)| point) {
        return OTHER_WIDTH[index].1;
    }
    if in_ranges(ZERO_WIDTH) {
        0
    } else if in_ranges(DOUBLE_WIDTH) {
        2
    } else {
        1
    }
}

/// Adjacent-pair canonical composition (Hangul algorithmically, the rest from a table
/// generated out of Node), so `string_width` measures the NFC form like Node does.
/// ponytail: no canonical-order reordering or blocking; exact for the common cases,
/// add a full NFC implementation if a script that needs it shows up.
fn compose_pairs(text: &str) -> Vec<u32> {
    use crate::inspect_width::COMPOSE;
    let mut out: Vec<u32> = Vec::with_capacity(text.len());
    for ch in text.chars().map(|c| c as u32) {
        if let Some(&prev) = out.last() {
            let hangul_lv = (0x1100..=0x1112).contains(&prev) && (0x1161..=0x1175).contains(&ch);
            let hangul_lvt = (0xac00..=0xd7a3).contains(&prev)
                && (prev - 0xac00) % 28 == 0
                && (0x11a8..=0x11c2).contains(&ch);
            let composed = if hangul_lv {
                Some(0xac00 + ((prev - 0x1100) * 21 + (ch - 0x1161)) * 28)
            } else if hangul_lvt {
                Some(prev + (ch - 0x11a7))
            } else {
                COMPOSE
                    .binary_search_by_key(&(prev, ch), |&(a, b, _)| (a, b))
                    .ok()
                    .map(|index| COMPOSE[index].2)
            };
            if let Some(composed) = composed {
                *out.last_mut().unwrap() = composed;
                continue;
            }
        }
        out.push(ch);
    }
    out
}

fn string_width(text: &str) -> usize {
    if text.is_ascii() {
        return text.bytes().filter(|byte| (32..127).contains(byte)).count();
    }
    compose_pairs(text).into_iter().map(char_width).sum()
}

struct Ctx<'a> {
    opts: &'a InspectOptions,
    seen: Vec<usize>,
    circular: Vec<(usize, usize)>,
    indentation_lvl: usize,
    current_depth: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Extras {
    Object,
    ArrayExtras,
}

pub(crate) fn inspect(node: &Node, opts: &InspectOptions) -> String {
    let mut ctx = Ctx {
        opts,
        seen: Vec::new(),
        circular: Vec::new(),
        indentation_lvl: 0,
        current_depth: 0,
    };
    ctx.format_value(node, 0)
}

impl Ctx<'_> {
    fn depth_exceeded(&self, recurse_times: usize) -> bool {
        self.opts.depth.is_some_and(|depth| recurse_times > depth)
    }

    fn format_value(&mut self, node: &Node, recurse_times: usize) -> String {
        match node {
            Node::Undefined
            | Node::Null
            | Node::Bool(_)
            | Node::Number(_)
            | Node::BigInt(_)
            | Node::Str(_) => self.format_primitive(node),
            Node::Raw(text) => text.clone(),
            Node::Date(iso) => iso.clone().unwrap_or_else(|| "Invalid Date".into()),
            Node::RegExp(text) => text.clone(),
            Node::Buffer(bytes) => format_buffer(bytes),
            Node::Function { name, kind } => format_function(name, kind),
            Node::Circular(id) => {
                let index = self.circular_index(*id);
                format!("[Circular *{index}]")
            }
            Node::Typed { .. } => self.format_raw(node, recurse_times),
            Node::Array { id, .. }
            | Node::Object { id, .. }
            | Node::Map { id, .. }
            | Node::Set { id, .. } => {
                if *id != 0 && self.seen.contains(id) {
                    let index = self.circular_index(*id);
                    return format!("[Circular *{index}]");
                }
                self.format_raw(node, recurse_times)
            }
        }
    }

    fn circular_index(&mut self, id: usize) -> usize {
        if let Some((_, index)) = self.circular.iter().find(|(known, _)| *known == id) {
            return *index;
        }
        let index = self.circular.len() + 1;
        self.circular.push((id, index));
        index
    }

    fn format_primitive(&self, node: &Node) -> String {
        match node {
            Node::Str(text) => {
                let mut text: &[u16] = text;
                let mut trailer = String::new();
                if text.len() > self.opts.max_string_length {
                    let remaining = text.len() - self.opts.max_string_length;
                    text = &text[..self.opts.max_string_length];
                    trailer = format!(
                        "... {remaining} more character{}",
                        if remaining > 1 { "s" } else { "" }
                    );
                }
                // `kMinLineLength` is 16: long strings split after each newline.
                if text.len() > 16 && text.len() + self.indentation_lvl + 4 > self.opts.break_length
                {
                    let mut pieces: Vec<&[u16]> = Vec::new();
                    let mut start = 0;
                    for (index, unit) in text.iter().enumerate() {
                        if *unit == 10 && index + 1 < text.len() {
                            pieces.push(&text[start..=index]);
                            start = index + 1;
                        }
                    }
                    pieces.push(&text[start..]);
                    let separator = format!(" +\n{}", " ".repeat(self.indentation_lvl + 2));
                    let joined = pieces
                        .iter()
                        .map(|piece| str_escape(piece))
                        .collect::<Vec<_>>()
                        .join(&separator);
                    return joined + &trailer;
                }
                str_escape(text) + &trailer
            }
            Node::Number(value) => format_number(*value),
            Node::BigInt(text) => format!("{text}n"),
            Node::Bool(value) => value.to_string(),
            Node::Undefined => "undefined".into(),
            Node::Null => "null".into(),
            _ => unreachable!("not a primitive"),
        }
    }

    fn format_raw(&mut self, node: &Node, mut recurse_times: usize) -> String {
        let mut base = String::new();
        let braces: [String; 2];
        let extras;
        enum Kind<'n> {
            Array(&'n [Option<Node>]),
            Object,
            Map(&'n [(Node, Node)]),
            Set(&'n [Node]),
        }
        let typed_items: Vec<Option<Node>>;
        let kind;
        let keys: &[(Vec<u16>, Node)];
        let ctx_style: String; // `getCtxStyle(...)` without the trailing space, for `[Object]`
        let mut null_proto = false;
        let id;
        match node {
            Node::Array {
                id: node_id,
                items,
                extra,
            } => {
                if items.is_empty() && extra.is_empty() && !self.opts.show_hidden {
                    return "[]".into();
                }
                id = *node_id;
                braces = ["[".into(), "]".into()];
                extras = Extras::ArrayExtras;
                kind = Kind::Array(items);
                keys = extra;
                ctx_style = "Array".into();
            }
            Node::Typed { name, items } => {
                if items.is_empty() {
                    return format!("{name}(0) []");
                }
                id = 0;
                braces = [format!("{name}({}) [", items.len()), "]".into()];
                extras = Extras::ArrayExtras;
                typed_items = items.iter().cloned().map(Some).collect();
                kind = Kind::Array(&typed_items);
                keys = &[];
                ctx_style = name.clone();
            }
            Node::Set { id: node_id, items } => {
                if items.is_empty() {
                    return "Set(0) {}".into();
                }
                id = *node_id;
                braces = [format!("Set({}) {{", items.len()), "}".into()];
                extras = Extras::Object;
                kind = Kind::Set(items);
                keys = &[];
                ctx_style = "Set".into();
            }
            Node::Map {
                id: node_id,
                entries,
            } => {
                if entries.is_empty() {
                    return "Map(0) {}".into();
                }
                id = *node_id;
                braces = [format!("Map({}) {{", entries.len()), "}".into()];
                extras = Extras::Object;
                kind = Kind::Map(entries);
                keys = &[];
                ctx_style = "Map".into();
            }
            Node::Object {
                id: node_id,
                ctor,
                entries,
            } => {
                let prefix = match ctor {
                    Ctor::Plain => String::new(),
                    Ctor::Named(name) => format!("{name} "),
                    Ctor::NullProto => {
                        null_proto = true;
                        "[Object: null prototype] ".to_string()
                    }
                };
                if entries.is_empty() {
                    return format!("{prefix}{{}}");
                }
                id = *node_id;
                braces = [format!("{prefix}{{"), "}".into()];
                extras = Extras::Object;
                kind = Kind::Object;
                keys = entries;
                ctx_style = match ctor {
                    Ctor::Plain => "Object".into(),
                    Ctor::Named(name) => name.clone(),
                    Ctor::NullProto => "[Object: null prototype]".into(),
                };
            }
            _ => unreachable!("not a container"),
        }
        if self.depth_exceeded(recurse_times) {
            return if null_proto {
                ctx_style
            } else {
                format!("[{ctx_style}]")
            };
        }
        recurse_times += 1;
        self.seen.push(id);
        self.current_depth = recurse_times;
        let mut output = match &kind {
            Kind::Array(items) => {
                let mut output = self.format_array(items, recurse_times);
                if self.opts.show_hidden {
                    output.push(format!("[length]: {}", items.len()));
                }
                output
            }
            Kind::Set(items) => self.format_set(items, recurse_times),
            Kind::Map(entries) => self.format_map(entries, recurse_times),
            Kind::Object => Vec::new(),
        };
        for (key, value) in keys {
            output.push(self.format_property(key, value, recurse_times));
        }
        if id != 0 {
            if let Some((_, index)) = self.circular.iter().find(|(known, _)| *known == id) {
                let reference = format!("<ref *{index}>");
                base = if base.is_empty() {
                    reference
                } else {
                    format!("{reference} {base}")
                };
            }
        }
        self.seen.pop();
        let items = match &kind {
            Kind::Array(items) => Some(*items),
            _ => None,
        };
        self.reduce_to_single_string(output, &base, &braces, extras, recurse_times, items)
    }

    fn format_array(&mut self, items: &[Option<Node>], recurse_times: usize) -> Vec<String> {
        let len = self.opts.max_array_length.min(items.len());
        let remaining = items.len() - len;
        let mut output = Vec::new();
        for index in 0..len {
            match &items[index] {
                None => return self.format_special_array(items, recurse_times, len, output, index),
                Some(item) => {
                    self.indentation_lvl += 2;
                    output.push(self.format_value(item, recurse_times));
                    self.indentation_lvl -= 2;
                }
            }
        }
        if remaining > 0 {
            output.push(remaining_text(remaining));
        }
        output
    }

    /// `formatSpecialArray`: a sparse array, driven by the list of defined indexes.
    fn format_special_array(
        &mut self,
        items: &[Option<Node>],
        recurse_times: usize,
        max_length: usize,
        mut output: Vec<String>,
        start: usize,
    ) -> Vec<String> {
        let defined: Vec<usize> = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| item.as_ref().map(|_| index))
            .collect();
        // `start` entries were already pushed; they correspond to the first `start` defined keys.
        let mut index = start;
        let mut key_slot = start;
        while key_slot < defined.len() && output.len() < max_length {
            let key = defined[key_slot];
            if index != key {
                let empty = key - index;
                output.push(format!(
                    "<{empty} empty item{}>",
                    if empty > 1 { "s" } else { "" }
                ));
                index = key;
                if output.len() == max_length {
                    break;
                }
            }
            self.indentation_lvl += 2;
            output.push(self.format_value(items[key].as_ref().expect("defined"), recurse_times));
            self.indentation_lvl -= 2;
            index += 1;
            key_slot += 1;
        }
        let remaining = items.len() - index;
        if output.len() != max_length {
            if remaining > 0 {
                output.push(format!(
                    "<{remaining} empty item{}>",
                    if remaining > 1 { "s" } else { "" }
                ));
            }
        } else if remaining > 0 {
            output.push(remaining_text(remaining));
        }
        output
    }

    fn format_set(&mut self, items: &[Node], recurse_times: usize) -> Vec<String> {
        let max_length = self.opts.max_array_length.min(items.len());
        let remaining = items.len() - max_length;
        let mut output = Vec::new();
        self.indentation_lvl += 2;
        for item in items.iter().take(max_length) {
            output.push(self.format_value(item, recurse_times));
        }
        if remaining > 0 {
            output.push(remaining_text(remaining));
        }
        self.indentation_lvl -= 2;
        output
    }

    fn format_map(&mut self, entries: &[(Node, Node)], recurse_times: usize) -> Vec<String> {
        let max_length = self.opts.max_array_length.min(entries.len());
        let remaining = entries.len() - max_length;
        let mut output = Vec::new();
        self.indentation_lvl += 2;
        for (key, value) in entries.iter().take(max_length) {
            let key = self.format_value(key, recurse_times);
            let value = self.format_value(value, recurse_times);
            output.push(format!("{key} => {value}"));
        }
        if remaining > 0 {
            output.push(remaining_text(remaining));
        }
        self.indentation_lvl -= 2;
        output
    }

    fn format_property(&mut self, key: &[u16], value: &Node, recurse_times: usize) -> String {
        self.indentation_lvl += 2;
        let text = self.format_value(value, recurse_times);
        self.indentation_lvl -= 2;
        let name = if key == "__proto__".encode_utf16().collect::<Vec<_>>().as_slice() {
            "['__proto__']".to_string()
        } else if key.first() == Some(&1) {
            // `\u{1}[errors]`: a non-enumerable property shown verbatim, like Node's `[errors]`.
            String::from_utf16_lossy(&key[1..])
        } else if is_identifier_key(key) {
            String::from_utf16_lossy(key)
        } else {
            str_escape(key)
        };
        format!("{name}: {text}")
    }

    fn is_below_break_length(&self, output: &[String], start: usize, base: &str) -> bool {
        let mut total = output.len() + start;
        if total + output.len() > self.opts.break_length {
            return false;
        }
        for entry in output {
            total += len16(entry);
            if total > self.opts.break_length {
                return false;
            }
        }
        base.is_empty() || !base.contains('\n')
    }

    fn reduce_to_single_string(
        &mut self,
        mut output: Vec<String>,
        base: &str,
        braces: &[String; 2],
        extras: Extras,
        recurse_times: usize,
        value: Option<&[Option<Node>]>,
    ) -> String {
        let entries = output.len();
        if extras == Extras::ArrayExtras && entries > 6 {
            output = self.group_array_elements(output, value);
        }
        if self.current_depth - recurse_times < self.opts.compact && entries == output.len() {
            let start = output.len() + self.indentation_lvl + len16(&braces[0]) + len16(base) + 10;
            if self.is_below_break_length(&output, start, base) {
                let joined = output.join(", ");
                if !joined.contains('\n') {
                    let prefix = if base.is_empty() {
                        String::new()
                    } else {
                        format!("{base} ")
                    };
                    return format!("{prefix}{} {joined} {}", braces[0], braces[1]);
                }
            }
        }
        let indentation = format!("\n{}", " ".repeat(self.indentation_lvl));
        let prefix = if base.is_empty() {
            String::new()
        } else {
            format!("{base} ")
        };
        format!(
            "{prefix}{}{indentation}  {}{indentation}{}",
            braces[0],
            output.join(&format!(",{indentation}  ")),
            braces[1]
        )
    }

    fn group_array_elements(
        &self,
        output: Vec<String>,
        value: Option<&[Option<Node>]>,
    ) -> Vec<String> {
        let mut total_length = 0usize;
        let mut max_length = 0usize;
        let mut output_length = output.len();
        if self.opts.max_array_length < output.len() {
            // The "... n more items" entry is not part of the grouping.
            output_length -= 1;
        }
        const SEPARATOR_SPACE: usize = 2;
        let mut data_len = Vec::with_capacity(output_length);
        for entry in output.iter().take(output_length) {
            let len = string_width(entry);
            data_len.push(len);
            total_length += len + SEPARATOR_SPACE;
            max_length = max_length.max(len);
        }
        let actual_max = max_length + SEPARATOR_SPACE;
        if actual_max * 3 + self.indentation_lvl < self.opts.break_length
            && (total_length as f64 / actual_max as f64 > 5.0 || max_length <= 6)
        {
            let approx_char_heights = 2.5;
            let average_bias =
                (actual_max as f64 - total_length as f64 / output.len() as f64).sqrt();
            let biased_max = (actual_max as f64 - 3.0 - average_bias).max(1.0);
            let columns = [
                ((approx_char_heights * biased_max * output_length as f64).sqrt() / biased_max)
                    .round(),
                ((self.opts.break_length as f64 - self.indentation_lvl as f64) / actual_max as f64)
                    .floor(),
                (self.opts.compact * 4) as f64,
                15.0,
            ]
            .into_iter()
            .fold(f64::INFINITY, f64::min) as usize;
            if columns <= 1 {
                return output;
            }
            let mut tmp = Vec::new();
            let mut max_line_length = Vec::new();
            for column in 0..columns {
                let mut line_max = 0;
                let mut j = column;
                while j < output.len() {
                    if let Some(&len) = data_len.get(j) {
                        line_max = line_max.max(len);
                    }
                    j += columns;
                }
                max_line_length.push(line_max + SEPARATOR_SPACE);
            }
            let mut pad_numbers = true;
            if let Some(items) = value {
                for index in 0..output.len() {
                    let numeric = matches!(
                        items.get(index),
                        Some(Some(Node::Number(_) | Node::BigInt(_)))
                    );
                    if !numeric {
                        pad_numbers = false;
                        break;
                    }
                }
            }
            let mut i = 0;
            while i < output_length {
                let max = (i + columns).min(output_length);
                let mut line = String::new();
                let mut j = i;
                while j + 1 < max {
                    let padding = max_line_length[j - i] + len16(&output[j]) - data_len[j];
                    let cell = format!("{}, ", output[j]);
                    line += &if pad_numbers {
                        pad_start(&cell, padding)
                    } else {
                        pad_end(&cell, padding)
                    };
                    j += 1;
                }
                if pad_numbers {
                    let padding = (max_line_length[j - i] + len16(&output[j]))
                        .saturating_sub(data_len[j] + SEPARATOR_SPACE);
                    line += &pad_start(&output[j], padding);
                } else {
                    line += &output[j];
                }
                tmp.push(line);
                i += columns;
            }
            if self.opts.max_array_length < output.len() {
                tmp.push(output[output_length].clone());
            }
            return tmp;
        }
        output
    }
}

fn remaining_text(remaining: usize) -> String {
    format!(
        "... {remaining} more item{}",
        if remaining > 1 { "s" } else { "" }
    )
}

/// `Buffer.prototype[util.inspect.custom]` (up to `INSPECT_MAX_BYTES` = 50 bytes).
fn format_buffer(bytes: &[u8]) -> String {
    const MAX: usize = 50;
    let shown = MAX.min(bytes.len());
    let mut text = bytes[..shown]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    if bytes.len() > MAX {
        let remaining = bytes.len() - MAX;
        let _ = write!(
            text,
            " ... {remaining} more byte{}",
            if remaining > 1 { "s" } else { "" }
        );
    }
    format!("<Buffer {text}>")
}

/// `getFunctionBase` for functions without own properties.
fn format_function(name: &str, kind: &FnKind) -> String {
    if let FnKind::Class { extends } = kind {
        let mut base = String::from("[class");
        if name.is_empty() {
            base.push_str(" (anonymous)");
        } else {
            let _ = write!(base, " {name}");
        }
        if let Some(parent) = extends {
            let _ = write!(base, " extends {parent}");
        }
        base.push(']');
        return base;
    }
    let kind_name = match kind {
        FnKind::Function => "Function",
        FnKind::Async => "AsyncFunction",
        FnKind::Generator => "GeneratorFunction",
        FnKind::AsyncGenerator => "AsyncGeneratorFunction",
        FnKind::Class { .. } => unreachable!(),
    };
    if name.is_empty() {
        format!("[{kind_name} (anonymous)]")
    } else {
        format!("[{kind_name}: {name}]")
    }
}

#[cfg(test)]
mod corpus_tests {
    use super::*;
    use serde_json::Value as J;

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn pairs(value: &J) -> Vec<(Vec<u16>, Node)> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|pair| (units(pair[0].as_str().unwrap()), decode(&pair[1])))
            .collect()
    }

    /// Decodes the corpus tree description (see `inspect-suite/gen.js`).
    fn decode(value: &J) -> Node {
        match value {
            J::Null => Node::Null,
            J::Bool(flag) => Node::Bool(*flag),
            J::String(text) => Node::Str(units(text)),
            J::Object(map) => {
                let id = map.get("id").and_then(J::as_u64).unwrap_or(0) as usize;
                if map.contains_key("u") {
                    Node::Undefined
                } else if let Some(number) = map.get("n") {
                    Node::Number(match number.as_str().unwrap() {
                        "-0" => -0.0,
                        "NaN" => f64::NAN,
                        "Infinity" => f64::INFINITY,
                        "-Infinity" => f64::NEG_INFINITY,
                        other => other.parse().unwrap(),
                    })
                } else if let Some(big) = map.get("b") {
                    Node::BigInt(big.as_str().unwrap().into())
                } else if let Some(raw) = map.get("s") {
                    Node::Str(
                        raw.as_array()
                            .unwrap()
                            .iter()
                            .map(|unit| unit.as_u64().unwrap() as u16)
                            .collect(),
                    )
                } else if let Some(items) = map.get("a") {
                    let holes: Vec<usize> = map["h"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|hole| hole.as_u64().unwrap() as usize)
                        .collect();
                    let items = items
                        .as_array()
                        .unwrap()
                        .iter()
                        .enumerate()
                        .map(|(index, item)| {
                            if holes.contains(&index) {
                                None
                            } else {
                                Some(decode(item))
                            }
                        })
                        .collect();
                    Node::Array {
                        id,
                        items,
                        extra: pairs(&map["x"]),
                    }
                } else if let Some(entries) = map.get("o") {
                    let ctor = match map["c"].as_str() {
                        None => Ctor::Plain,
                        Some("null") => Ctor::NullProto,
                        Some(name) => Ctor::Named(name.into()),
                    };
                    Node::Object {
                        id,
                        ctor,
                        entries: pairs(entries),
                    }
                } else if let Some(entries) = map.get("m") {
                    let entries = entries
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|pair| (decode(&pair[0]), decode(&pair[1])))
                        .collect();
                    Node::Map { id, entries }
                } else if let Some(items) = map.get("e") {
                    Node::Set {
                        id,
                        items: items.as_array().unwrap().iter().map(decode).collect(),
                    }
                } else if let Some(iso) = map.get("d") {
                    Node::Date(iso.as_str().map(str::to_string))
                } else if let Some(source) = map.get("r") {
                    Node::RegExp(source.as_str().unwrap().into())
                } else if let Some(bytes) = map.get("bf") {
                    Node::Buffer(
                        bytes
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|byte| byte.as_u64().unwrap() as u8)
                            .collect(),
                    )
                } else if let Some(name) = map.get("f") {
                    let kind = match map["k"].as_str().unwrap() {
                        "async" => FnKind::Async,
                        "generator" => FnKind::Generator,
                        "asyncgenerator" => FnKind::AsyncGenerator,
                        "class" => FnKind::Class {
                            extends: map.get("x").and_then(J::as_str).map(str::to_string),
                        },
                        _ => FnKind::Function,
                    };
                    Node::Function {
                        name: name.as_str().unwrap().into(),
                        kind,
                    }
                } else if let Some(target) = map.get("cir") {
                    Node::Circular(target.as_u64().unwrap() as usize)
                } else {
                    panic!("unknown corpus node {value}")
                }
            }
            other => panic!("unknown corpus node {other}"),
        }
    }

    /// Counts of corpus cases whose expected output exercises each hard feature, so the
    /// replay cannot pass vacuously if the generator regresses.
    fn assert_corpus_is_not_vacuous(corpus: &[J]) {
        let count = |needle: &str| {
            corpus
                .iter()
                .filter(|case| case["e"].as_str().unwrap().contains(needle))
                .count()
        };
        for (needle, minimum) in [
            ("\n", 1000),
            ("<ref *", 40),
            ("[Circular", 40),
            ("empty item", 80),
            ("more item", 100),
            ("Map(", 100),
            ("Set(", 100),
            ("<Buffer", 60),
            ("[Object]", 30),
            ("[Array]", 30),
            (" +\n", 100),
            ("\\ud", 200),
            ("[class", 20),
            ("null prototype", 50),
            ("Invalid Date", 20),
            ("e+21", 40),
        ] {
            assert!(
                count(needle) >= minimum,
                "corpus has too few cases containing {needle:?}"
            );
        }
        assert!(corpus.len() >= 3000);
    }

    #[test]
    fn replay_node_corpus() {
        // `THAW_INSPECT_CORPUS=/path/other.json` replays a freshly generated (held-out) corpus.
        let (text, committed) = match std::env::var("THAW_INSPECT_CORPUS") {
            Ok(path) => (std::fs::read_to_string(path).unwrap(), false),
            Err(_) => (include_str!("inspect_corpus.json").to_string(), true),
        };
        let corpus: Vec<J> = serde_json::from_str(&text).unwrap();
        if committed {
            assert_corpus_is_not_vacuous(&corpus);
        }
        let options = InspectOptions::default();
        let mut mismatches = Vec::new();
        let mut per_category: std::collections::BTreeMap<String, (usize, usize)> =
            Default::default();
        for (index, case) in corpus.iter().enumerate() {
            let category = case["c"].as_str().unwrap().to_string();
            let actual = inspect(&decode(&case["v"]), &options);
            let expected = case["e"].as_str().unwrap();
            let entry = per_category.entry(category.clone()).or_default();
            entry.0 += 1;
            if actual != expected {
                entry.1 += 1;
                if mismatches.len() < 8 {
                    mismatches.push(format!("case {index} [{category}]\n--- expected\n{expected}\n--- actual\n{actual}\n"));
                }
            }
        }
        let failed: usize = per_category.values().map(|(_, bad)| bad).sum();
        assert!(
            failed == 0,
            "{failed}/{} corpus cases differ from Node (per category total/bad: {per_category:?}); first mismatches:\n{}",
            corpus.len(),
            mismatches.join("\n")
        );
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    fn s(text: &str) -> Node {
        Node::Str(text.encode_utf16().collect())
    }

    fn num(value: f64) -> Node {
        Node::Number(value)
    }

    fn arr(items: Vec<Node>) -> Node {
        Node::Array {
            id: 0,
            items: items.into_iter().map(Some).collect(),
            extra: Vec::new(),
        }
    }

    fn obj(entries: Vec<(&str, Node)>) -> Node {
        Node::Object {
            id: 0,
            ctor: Ctor::Plain,
            entries: entries
                .into_iter()
                .map(|(key, value)| (key.encode_utf16().collect(), value))
                .collect(),
        }
    }

    fn show(node: &Node) -> String {
        inspect(node, &InspectOptions::default())
    }

    #[test]
    fn strings_choose_their_quotes_like_node() {
        assert_eq!(show(&s("plain")), "'plain'");
        assert_eq!(show(&s("it's")), "\"it's\"");
        assert_eq!(show(&s("say \"hi\" it's")), "`say \"hi\" it's`");
        assert_eq!(show(&s("all ' \" ` three")), "'all \\' \" ` three'");
        assert_eq!(show(&s("${x}'\"")), "'${x}\\'\"'");
        assert_eq!(show(&s("a\nb\t\u{7f}\u{85}\\")), "'a\\nb\\t\\x7F\\x85\\\\'");
        assert_eq!(show(&Node::Str(vec![0xd83d])), "'\\ud83d'");
        assert_eq!(show(&Node::Str(vec![0xdc00, 0x78])), "'\\udc00x'");
    }

    #[test]
    fn keys_quote_unless_identifier_shaped() {
        let node = obj(vec![
            ("ok_1", num(1.0)),
            ("with-dash", num(2.0)),
            ("1", num(3.0)),
            ("$x", num(4.0)),
            ("", num(5.0)),
            ("__proto__", num(6.0)),
        ]);
        // Entry order is the caller's (the Json converter sorts integer keys first, like JS).
        assert_eq!(
            show(&node),
            "{ ok_1: 1, 'with-dash': 2, '1': 3, '$x': 4, '': 5, ['__proto__']: 6 }"
        );
    }

    #[test]
    fn numbers_follow_number_prototype_to_string() {
        for (value, text) in [
            (0.0, "0"),
            (-0.0, "-0"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1.5e-7, "1.5e-7"),
            (1e-6, "0.000001"),
            (123456789.125, "123456789.125"),
            (-1e-7, "-1e-7"),
            (f64::NAN, "NaN"),
            (f64::NEG_INFINITY, "-Infinity"),
            (5e-324, "5e-324"),
            (0.1 + 0.2, "0.30000000000000004"),
        ] {
            assert_eq!(show(&num(value)), text, "{value:?}");
        }
        assert_eq!(show(&Node::BigInt("-12".into())), "-12n");
    }

    #[test]
    fn short_arrays_stay_on_one_line_and_long_ones_group_into_columns() {
        assert_eq!(
            show(&arr((1..=3).map(|n| num(n as f64)).collect())),
            "[ 1, 2, 3 ]"
        );
        let seven = show(&arr((1..=7).map(|n| num(n as f64)).collect()));
        assert_eq!(seven, "[\n  1, 2, 3, 4,\n  5, 6, 7\n]");
        let twenty = show(&arr((0..30).map(|n| s(&format!("item{n}"))).collect()));
        assert!(
            twenty.starts_with("[\n  'item0',  'item1',  'item2',\n"),
            "{twenty}"
        );
    }

    #[test]
    fn arrays_cap_at_one_hundred_and_report_holes() {
        let long = show(&arr((0..105).map(|n| num(n as f64)).collect()));
        assert!(long.ends_with("... 5 more items\n]"), "{long}");
        let sparse = Node::Array {
            id: 0,
            items: vec![Some(num(1.0)), None, None, Some(num(2.0)), None],
            extra: Vec::new(),
        };
        assert_eq!(show(&sparse), "[ 1, <2 empty items>, 2, <1 empty item> ]");
    }

    #[test]
    fn depth_limit_and_empty_containers() {
        let deep = obj(vec![(
            "a",
            obj(vec![("b", obj(vec![("c", obj(vec![("d", num(1.0))]))]))]),
        )]);
        assert_eq!(show(&deep), "{ a: { b: { c: [Object] } } }");
        let empty_deep = obj(vec![("a", obj(vec![("b", obj(vec![("c", obj(vec![]))]))]))]);
        assert_eq!(show(&empty_deep), "{ a: { b: { c: {} } } }");
    }

    #[test]
    fn long_objects_break_lines_at_the_width_limit() {
        let entries: Vec<(String, Node)> = (0..6)
            .map(|n| (format!("key{n}"), s(&"v".repeat(12))))
            .collect();
        let node = obj(entries
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect());
        let text = show(&node);
        assert!(
            text.contains('\n') && text.lines().all(|line| len16(line) <= 80),
            "{text}"
        );
    }

    #[test]
    fn circular_references_get_a_ref_marker() {
        let node = Node::Object {
            id: 1,
            ctor: Ctor::Plain,
            entries: vec![("self".encode_utf16().collect(), Node::Circular(1))],
        };
        assert_eq!(show(&node), "<ref *1> { self: [Circular *1] }");
    }

    #[test]
    fn maps_sets_buffers_and_functions() {
        let map = Node::Map {
            id: 0,
            entries: vec![(s("a"), num(1.0))],
        };
        assert_eq!(show(&map), "Map(1) { 'a' => 1 }");
        assert_eq!(
            show(&Node::Set {
                id: 0,
                items: vec![num(1.0), s("x")]
            }),
            "Set(2) { 1, 'x' }"
        );
        assert_eq!(show(&Node::Buffer(vec![1, 2, 255])), "<Buffer 01 02 ff>");
        let typed = |name: &str, items: Vec<f64>| Node::Typed {
            name: name.into(),
            items: items.into_iter().map(Node::Number).collect(),
        };
        assert_eq!(
            show(&typed("Uint8Array", vec![1.0, 2.0, 3.0])),
            "Uint8Array(3) [ 1, 2, 3 ]"
        );
        assert_eq!(show(&typed("Int32Array", vec![])), "Int32Array(0) []");
        assert_eq!(
            show(&typed("Float64Array", vec![1.5; 8])),
            "Float64Array(8) [\n  1.5, 1.5, 1.5,\n  1.5, 1.5, 1.5,\n  1.5, 1.5\n]"
        );
        let hidden = InspectOptions {
            show_hidden: true,
            ..InspectOptions::default()
        };
        let pair = Node::Array {
            id: 0,
            items: vec![Some(Node::Number(1.0)), Some(Node::Number(2.0))],
            extra: Vec::new(),
        };
        assert_eq!(inspect(&pair, &hidden), "[ 1, 2, [length]: 2 ]");
        assert_eq!(
            inspect(
                &Node::Array {
                    id: 0,
                    items: vec![],
                    extra: Vec::new()
                },
                &hidden
            ),
            "[ [length]: 0 ]"
        );
        assert!(show(&Node::Buffer(vec![0; 52])).ends_with("... 2 more bytes>"));
        assert_eq!(
            show(&Node::Function {
                name: "".into(),
                kind: FnKind::Async
            }),
            "[AsyncFunction (anonymous)]"
        );
        assert_eq!(
            show(&Node::Function {
                name: "B".into(),
                kind: FnKind::Class {
                    extends: Some("A".into())
                }
            }),
            "[class B extends A]"
        );
    }

    #[test]
    fn column_widths_follow_icu() {
        assert_eq!(string_width("abc"), 3);
        assert_eq!(string_width("日本"), 4);
        assert_eq!(string_width("e\u{301}"), 1);
        assert_eq!(string_width("\u{1100}\u{1161}"), 2); // composes to a Hangul syllable
        assert_eq!(string_width("😀"), 2);
    }
}
