// Real `Intl.Segmenter` (M10 of docs/design/intl-polyfill.md's "Real
// CLDR data via icu4x" plan), via `icu_segmenter` -- entirely new
// capability. Grapheme-cluster segmentation is locale-invariant per
// UAX #29 (`GraphemeClusterSegmenter::new()` takes no locale at all);
// word/sentence segmentation is real per-locale (Thai/Japanese/Chinese
// have no inter-word spaces, so a real word segmenter -- not a naive
// whitespace split -- is the entire point).
//
// The whole segment list for the given text is computed and returned
// in one call (as JSON), rather than trying to replicate icu4x's own
// lazy Rust iterator across the native/JS boundary -- real usage
// overwhelmingly iterates every segment anyway (`for (const s of
// segmenter.segment(text))`), so there's no benefit to a lazy protocol
// here worth the extra complexity. Byte offsets `icu_segmenter` reports
// (UTF-8, since it operates on `&str`) are converted to UTF-16 code
// -unit offsets here, matching real `Intl.Segmenter`'s `index` (JS
// strings are UTF-16).
//
// The text arrives as WTF-8 bytes (a lone UTF-16 surrogate is a 3-byte
// sequence). icu4x needs valid UTF-8, so for segmentation each lone
// surrogate is substituted with U+FFFD -- which is *also* 3 UTF-8 bytes,
// so every byte offset still lines up with the original WTF-8 text, and
// the emitted segment text is sliced from the original bytes (a lone
// surrogate survives into the JSON as a `\uXXXX` escape).

/// `__thaw_intl_segment(locale, granularity, text) -> String` (a JSON
/// array of `{"segment":..,"index":..,"isWordLike":bool|null}`, index
/// in UTF-16 code units). `granularity` is `"grapheme"`/`"word"`/
/// `"sentence"`. `text` is WTF-8. Falls back to one segment covering the
/// whole string if the word/sentence data provider fails to resolve.
fn intl_segment(locale_tag: &str, granularity: &str, text: &[u8]) -> String {
    // A valid-UTF-8 view for icu4x: each 3-byte WTF-8 lone surrogate
    // becomes 3-byte U+FFFD, so byte offsets are unchanged.
    let icu_text = wtf8_to_icu_text(text);

    // Byte offset -> UTF-16 code-unit offset, for every boundary in
    // `byte_offsets` (already sorted ascending, as every icu4x break
    // iterator yields boundaries in order).
    let utf16_offsets = |byte_offsets: &[usize]| -> Vec<usize> {
        let mut utf16_offset = 0usize;
        let mut last_byte = 0usize;
        byte_offsets
            .iter()
            .map(|&byte_offset| {
                utf16_offset += wtf8_utf16_len(&text[last_byte..byte_offset]);
                last_byte = byte_offset;
                utf16_offset
            })
            .collect()
    };

    let segments_json = |boundaries: Vec<usize>, word_like: Option<Vec<bool>>| -> String {
        let offsets = utf16_offsets(&boundaries);
        let mut json = String::from("[");
        let mut byte_cursor = 0usize;
        for (index, &byte_end) in boundaries.iter().enumerate() {
            if byte_end == 0 {
                continue;
            }
            let segment = &text[byte_cursor..byte_end];
            let start_offset = if index == 0 { 0 } else { offsets[index - 1] };
            if index > 0 {
                json.push(',');
            }
            let is_word_like = word_like
                .as_ref()
                .map(|values| values[index].to_string())
                .unwrap_or_else(|| "null".to_string());
            json.push_str(&format!(
                r#"{{"segment":{},"index":{},"isWordLike":{}}}"#,
                wtf8_json_string(segment),
                start_offset,
                is_word_like,
            ));
            byte_cursor = byte_end;
        }
        json.push(']');
        json
    };

    // `locale_tag` is accepted (and real `Intl.Segmenter` does resolve/
    // report a locale) but doesn't change segmentation logic itself:
    // both `WordSegmenter`'s and `SentenceSegmenter`'s "auto" data-
    // provider constructors take invariant options, not a locale --
    // real word/sentence boundary detection is *script*-driven (read
    // from the text's own content, e.g. a Thai/Chinese dictionary
    // lookup), not selected by the surrounding locale tag. Grapheme
    // clusters need neither a locale nor a provider at all (`UAX #29`
    // is fully locale-invariant), confirmed via `GraphemeClusterSegmenter
    // ::new()`'s own doc comment.
    let _ = locale_tag;

    if granularity == "grapheme" {
        let Ok(segmenter) =
            icu_segmenter::GraphemeClusterSegmenter::try_new_unstable(&thaw_icu_data::ThawIcuDataProvider)
        else {
            return format!(
                r#"[{{"segment":{},"index":0,"isWordLike":null}}]"#,
                wtf8_json_string(text)
            );
        };
        let boundaries: Vec<usize> = segmenter
            .as_borrowed()
            .segment_str(&icu_text)
            .skip(1)
            .collect();
        return segments_json(boundaries, None);
    }

    if granularity == "word" {
        let Ok(segmenter) = icu_segmenter::WordSegmenter::try_new_auto_unstable(
            &thaw_icu_data::ThawIcuDataProvider,
            Default::default(),
        ) else {
            return format!(
                r#"[{{"segment":{},"index":0,"isWordLike":null}}]"#,
                wtf8_json_string(text)
            );
        };
        let (boundaries, word_like): (Vec<usize>, Vec<bool>) = segmenter
            .as_borrowed()
            .segment_str(&icu_text)
            .iter_with_word_type()
            .skip(1)
            .map(|(offset, word_type)| (offset, word_type.is_word_like()))
            .unzip();
        return segments_json(boundaries, Some(word_like));
    }

    // "sentence" (the only remaining real ECMA-402 granularity).
    let Ok(segmenter) = icu_segmenter::SentenceSegmenter::try_new_unstable(
        &thaw_icu_data::ThawIcuDataProvider,
        Default::default(),
    ) else {
        return format!(
            r#"[{{"segment":{},"index":0,"isWordLike":null}}]"#,
            wtf8_json_string(text)
        );
    };
    let boundaries: Vec<usize> = segmenter
        .as_borrowed()
        .segment_str(&icu_text)
        .skip(1)
        .collect();
    segments_json(boundaries, None)
}

/// Replaces each 3-byte WTF-8 lone surrogate with 3-byte U+FFFD, yielding
/// a valid UTF-8 string with identical byte offsets.
fn wtf8_to_icu_text(bytes: &[u8]) -> String {
    let mut fixed = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0xED
            && index + 2 < bytes.len()
            && (0xA0..=0xBF).contains(&bytes[index + 1])
            && bytes[index + 2] & 0xC0 == 0x80
        {
            fixed.extend_from_slice("\u{FFFD}".as_bytes());
            index += 3;
        } else {
            fixed.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(fixed).unwrap_or_default()
}

/// Number of UTF-16 code units in a WTF-8 byte slice (an astral code point
/// counts as two, a lone surrogate as one).
fn wtf8_utf16_len(bytes: &[u8]) -> usize {
    let mut count = 0;
    let mut index = 0;
    while index < bytes.len() {
        let first = bytes[index];
        if first < 0x80 {
            count += 1;
            index += 1;
        } else if (0xC2..=0xDF).contains(&first) {
            count += 1;
            index += 2;
        } else if (0xE0..=0xEF).contains(&first) {
            count += 1;
            index += 3;
        } else if (0xF0..=0xF4).contains(&first) {
            count += 2;
            index += 4;
        } else {
            count += 1;
            index += 1;
        }
    }
    count
}

/// JSON-encodes a WTF-8 byte slice as a string literal, escaping a lone
/// surrogate as `\uXXXX` (which `serde_json` cannot, since it only
/// handles valid UTF-8).
fn wtf8_json_string(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
    }
    // Has a lone surrogate: decode UTF-16 units and escape each.
    let units = wtf8_decode_utf16(bytes);
    let mut out = String::from("\"");
    let mut index = 0;
    while index < units.len() {
        let unit = units[index];
        match unit {
            0x22 => out.push_str("\\\""),
            0x5C => out.push_str("\\\\"),
            0x08 => out.push_str("\\b"),
            0x0C => out.push_str("\\f"),
            0x0A => out.push_str("\\n"),
            0x0D => out.push_str("\\r"),
            0x09 => out.push_str("\\t"),
            0x00..=0x1F => out.push_str(&format!("\\u{unit:04x}")),
            0xD800..=0xDBFF
                if index + 1 < units.len() && (0xDC00..=0xDFFF).contains(&units[index + 1]) =>
            {
                let code = 0x10000
                    + ((u32::from(unit) - 0xD800) << 10)
                    + (u32::from(units[index + 1]) - 0xDC00);
                out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                index += 2;
                continue;
            }
            0xD800..=0xDFFF => out.push_str(&format!("\\u{unit:04x}")),
            _ => out.push(char::from_u32(u32::from(unit)).unwrap_or('\u{FFFD}')),
        }
        index += 1;
    }
    out.push('"');
    out
}

/// Decodes WTF-8 bytes to UTF-16 code units, preserving lone surrogates.
fn wtf8_decode_utf16(bytes: &[u8]) -> Vec<u16> {
    let continuation = |byte: u8| byte & 0xC0 == 0x80;
    let mut units = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let first = bytes[index];
        let (width, code) = if first < 0x80 {
            (1, u32::from(first))
        } else if (0xC2..=0xDF).contains(&first)
            && index + 1 < bytes.len()
            && continuation(bytes[index + 1])
        {
            (2, ((u32::from(first) & 0x1F) << 6) | (u32::from(bytes[index + 1]) & 0x3F))
        } else if (0xE0..=0xEF).contains(&first)
            && index + 2 < bytes.len()
            && continuation(bytes[index + 1])
            && continuation(bytes[index + 2])
        {
            (
                3,
                ((u32::from(first) & 0x0F) << 12)
                    | ((u32::from(bytes[index + 1]) & 0x3F) << 6)
                    | (u32::from(bytes[index + 2]) & 0x3F),
            )
        } else if (0xF0..=0xF4).contains(&first)
            && index + 3 < bytes.len()
            && continuation(bytes[index + 1])
            && continuation(bytes[index + 2])
            && continuation(bytes[index + 3])
        {
            (
                4,
                ((u32::from(first) & 0x07) << 18)
                    | ((u32::from(bytes[index + 1]) & 0x3F) << 12)
                    | ((u32::from(bytes[index + 2]) & 0x3F) << 6)
                    | (u32::from(bytes[index + 3]) & 0x3F),
            )
        } else {
            units.push(0xFFFD);
            index += 1;
            continue;
        };
        if width == 4 && (0x10000..=0x10FFFF).contains(&code) {
            let code = code - 0x10000;
            units.push(0xD800 + (code >> 10) as u16);
            units.push(0xDC00 + (code & 0x3FF) as u16);
        } else {
            units.push(code as u16);
        }
        index += width;
    }
    units
}
