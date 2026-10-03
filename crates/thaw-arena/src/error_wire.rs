//! Versioned length-framed message segment for the tagged error string ABI.
//! Legacy unframed tags remain readable by their existing consumers.

const HEADER: &[u8] = b"\x1eE1:";

pub struct ErrorFrame<'a> {
    pub chain: &'a [u8],
    pub display: &'a [u8],
    pub original: Option<&'a [u8]>,
    pub suffix: &'a [u8],
}

fn encoded_width(bytes: &[u8], index: usize) -> Option<(usize, usize)> {
    let first = *bytes.get(index)?;
    if first < 0x80 { return Some((1, 1)); }
    let (width, units) = match first {
        0xC2..=0xDF => (2, 1),
        0xE0..=0xEF => (3, 1),
        0xF0..=0xF4 => (4, 2),
        _ => return None,
    };
    let tail = bytes.get(index + 1..index + width)?;
    if tail.iter().any(|byte| byte & 0xC0 != 0x80) { return None; }
    if width == 3 && first == 0xE0 && tail[0] < 0xA0 { return None; }
    if width == 4 && ((first == 0xF0 && tail[0] < 0x90)
        || (first == 0xF4 && tail[0] > 0x8F)) { return None; }
    Some((width, units))
}

fn utf16_units(bytes: &[u8]) -> Option<usize> {
    let mut index = 0;
    let mut units: usize = 0;
    while index < bytes.len() {
        let (width, count) = encoded_width(bytes, index)?;
        index += width;
        units = units.checked_add(count)?;
    }
    Some(units)
}

fn boundary(bytes: &[u8], expected: usize) -> Option<usize> {
    let mut index = 0;
    let mut units = 0usize;
    while units < expected {
        let (width, count) = encoded_width(bytes, index)?;
        units = units.checked_add(count)?;
        if units > expected { return None; }
        index += width;
    }
    Some(index)
}

fn decimal<'a>(bytes: &'a [u8]) -> Option<(usize, &'a [u8])> {
    let end = bytes.iter().position(|byte| *byte == b':')?;
    if end == 0 || !bytes[..end].iter().all(u8::is_ascii_digit) { return None; }
    let mut value = 0usize;
    for byte in &bytes[..end] {
        value = value.checked_mul(10)?.checked_add((*byte - b'0') as usize)?;
    }
    Some((value, &bytes[end + 1..]))
}

pub fn encode_tagged(chain: &[u8], display: &[u8], original: Option<&[u8]>) -> Option<Vec<u8>> {
    let display_units = utf16_units(display)?;
    let original_units = match original { Some(bytes) => Some(utf16_units(bytes)?), None => None };
    let mut output = Vec::with_capacity(chain.len() + display.len()
        + original.map_or(0, |bytes| bytes.len()) + 40);
    output.push(1);
    output.extend_from_slice(chain);
    output.push(1);
    output.extend_from_slice(HEADER);
    output.extend_from_slice(display_units.to_string().as_bytes());
    output.push(b':');
    match original_units {
        Some(units) => output.extend_from_slice(units.to_string().as_bytes()),
        None => output.push(b'-'),
    }
    output.push(b':');
    output.extend_from_slice(display);
    if let Some(original) = original { output.extend_from_slice(original); }
    Some(output)
}

pub fn parse_tagged(bytes: &[u8]) -> Option<ErrorFrame<'_>> {
    let rest = bytes.strip_prefix(&[1])?;
    let end = rest.iter().position(|byte| *byte == 1)?;
    let chain = &rest[..end];
    let framed = rest[end + 1..].strip_prefix(HEADER)?;
    let (display_units, framed) = decimal(framed)?;
    let (original_units, framed) = if let Some(framed) = framed.strip_prefix(b"-:") {
        (None, framed)
    } else {
        let (units, framed) = decimal(framed)?;
        (Some(units), framed)
    };
    let display_end = boundary(framed, display_units)?;
    let display = &framed[..display_end];
    let framed = &framed[display_end..];
    let (original, suffix) = if let Some(units) = original_units {
        let end = boundary(framed, units)?;
        (Some(&framed[..end]), &framed[end..])
    } else { (None, framed) };
    Some(ErrorFrame { chain, display, original, suffix })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_bounds_messages_before_marker_suffixes() {
        let display = b"A\x02B\x03C\x04D\x05E\x06F\0\xed\xa0\x80\xf0\x9f\x98\x80";
        let original = b"raw\x05property";
        let mut wire = encode_tagged(b"TypeError", display, Some(original)).unwrap();
        wire.extend_from_slice(b"\x02real cause\x03REAL_CODE\x05{\"status\":418}");
        let frame = parse_tagged(&wire).unwrap();
        assert_eq!(frame.chain, b"TypeError");
        assert_eq!(frame.display, display);
        assert_eq!(frame.original, Some(&original[..]));
        assert_eq!(frame.suffix, b"\x02real cause\x03REAL_CODE\x05{\"status\":418}");
    }

    #[test]
    fn malformed_or_partial_utf16_frame_is_not_accepted() {
        assert!(parse_tagged(b"\x01Error\x01\x1eE1:1:-:\xf0\x9f\x98\x80").is_none());
        assert!(parse_tagged(b"\x01Error\x01\x1eE1:999999999999999999999999:-:x").is_none());
        assert!(parse_tagged(b"\x01Error\x01legacy\x02text").is_none());
    }
}
