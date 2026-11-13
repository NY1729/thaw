//! Turns a module's own (defined) linear memory into an import.
//!
//! wasmi owns memories it creates for a module, so their bytes can never be
//! shared with the embedder. Rewriting the module so that its single memory is
//! imported from a hidden module name lets the embedder create the memory
//! itself (see `Store::instantiate_inner`). Only the single-memory case is
//! rewritten. Anything else keeps wasmi-owned memory.
//!
//! The rewrite: drop the memory section entry (index 0 stays the same memory,
//! now an import), append one memory import under [`HIDDEN_MODULE`] /
//! [`HIDDEN_FIELD`], and leave exports, data segments and start alone (they
//! refer to memory index 0, which is unchanged).

/// Import module name of the hidden memory import. Modules that already
/// import from this name are not rewritten, so it cannot collide.
pub(crate) const HIDDEN_MODULE: &str = "\u{0}thaw";
pub(crate) const HIDDEN_FIELD: &str = "memory0";

pub(crate) struct Externalized {
    pub bytes: Vec<u8>,
    pub min: u32,
    pub max: Option<u32>,
}

/// `Some` when `bytes` defines exactly one memory, imports none, uses no
/// reserved import name, and has plain (flag 0/1) limits.
pub(crate) fn externalize(bytes: &[u8], compiled: &wasmi::Module) -> Option<Externalized> {
    let mut imports = compiled.imports();
    if imports.any(|import| {
        import.module() == HIDDEN_MODULE || matches!(import.ty(), wasmi::ExternType::Memory(_))
    }) {
        return None;
    }
    let sections = split_sections(bytes)?;
    let memory_index = sections.iter().position(|s| s.id == 5)?;
    let body = sections[memory_index].body;
    let (count, after) = read_uleb(body, 0)?;
    if count != 1 {
        return None;
    }
    let limits = body.get(after..)?;
    let (min, max, end) = parse_limits(limits)?;
    if end != limits.len() {
        return None;
    }

    let import_entry = [
        name(HIDDEN_MODULE),
        name(HIDDEN_FIELD),
        vec![0x02],
        limits.to_vec(),
    ]
    .concat();
    let import_index = sections.iter().position(|s| s.id == 2);
    let mut out = bytes[..8].to_vec();
    let mut pending_import = Some(import_entry.as_slice());
    for (index, section) in sections.iter().enumerate() {
        if index == memory_index {
            continue;
        }
        if section.id == 2 {
            // Existing import section: bump the count, append our entry at the end.
            let (count, after) = read_uleb(section.body, 0)?;
            let body = [
                uleb(count + 1),
                section.body[after..].to_vec(),
                pending_import.take()?.to_vec(),
            ]
            .concat();
            push_section(&mut out, 2, &body);
            continue;
        }
        // Insert a new import section right before the first non-custom section
        // that must come after imports (ids 3..=12 in the order the spec gives).
        if import_index.is_none() && section.id >= 3 && pending_import.is_some() {
            let body = [uleb(1), pending_import.take()?.to_vec()].concat();
            push_section(&mut out, 2, &body);
        }
        push_section(&mut out, section.id, section.body);
    }
    if let Some(entry) = pending_import.take() {
        // No section after the memory section needs it, so the import goes last.
        let body = [uleb(1), entry.to_vec()].concat();
        push_section(&mut out, 2, &body);
    }
    Some(Externalized {
        bytes: out,
        min,
        max,
    })
}

struct Section<'a> {
    id: u8,
    body: &'a [u8],
}

fn split_sections(bytes: &[u8]) -> Option<Vec<Section<'_>>> {
    if bytes.len() < 8 {
        return None;
    }
    let mut pos = 8;
    let mut out = Vec::new();
    while pos < bytes.len() {
        let id = bytes[pos];
        let (size, body_start) = read_uleb(bytes, pos + 1)?;
        let end = body_start.checked_add(usize::try_from(size).ok()?)?;
        out.push(Section {
            id,
            body: bytes.get(body_start..end)?,
        });
        pos = end;
    }
    Some(out)
}

/// Limits as encoded in a memory type: flag 0 (min) or 1 (min, max).
/// Returns `(min, max, bytes consumed)`.
fn parse_limits(bytes: &[u8]) -> Option<(u32, Option<u32>, usize)> {
    match *bytes.first()? {
        0x00 => {
            let (min, after) = read_uleb(bytes, 1)?;
            Some((u32::try_from(min).ok()?, None, after))
        }
        0x01 => {
            let (min, after) = read_uleb(bytes, 1)?;
            let (max, after) = read_uleb(bytes, after)?;
            Some((
                u32::try_from(min).ok()?,
                Some(u32::try_from(max).ok()?),
                after,
            ))
        }
        _ => None,
    }
}

fn read_uleb(bytes: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let (mut value, mut shift) = (0u64, 0u32);
    loop {
        let byte = *bytes.get(pos)?;
        pos += 1;
        value |= u64::from(byte & 0x7f).checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return Some((value, pos));
        }
        shift += 7;
    }
}

fn uleb(value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    let mut value = value;
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

fn name(text: &str) -> Vec<u8> {
    [uleb(text.len() as u64), text.as_bytes().to_vec()].concat()
}

fn push_section(out: &mut Vec<u8>, id: u8, body: &[u8]) {
    out.push(id);
    out.extend(uleb(body.len() as u64));
    out.extend_from_slice(body);
}
