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

/// `__thaw_intl_segment(locale, granularity, text) -> String` (a JSON
/// array of `{"segment":..,"index":..,"isWordLike":bool|null}`, index
/// in UTF-16 code units). `granularity` is `"grapheme"`/`"word"`/
/// `"sentence"`. Falls back to one segment covering the whole string if
/// the word/sentence data provider fails to resolve.
fn intl_segment(locale_tag: &str, granularity: &str, text: &str) -> String {
    // UTF-8 byte offset -> UTF-16 code-unit offset, for every boundary
    // in `byte_offsets` (already sorted ascending, as every icu4x
    // break iterator yields boundaries in order).
    let utf16_offsets = |byte_offsets: &[usize]| -> Vec<usize> {
        let mut utf16_offset = 0usize;
        let mut last_byte = 0usize;
        byte_offsets
            .iter()
            .map(|&byte_offset| {
                utf16_offset += text[last_byte..byte_offset].encode_utf16().count();
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
                serde_json::to_string(segment).unwrap_or_else(|_| "\"\"".to_string()),
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
                serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
            );
        };
        let boundaries: Vec<usize> = segmenter.as_borrowed().segment_str(text).skip(1).collect();
        return segments_json(boundaries, None);
    }

    if granularity == "word" {
        let Ok(segmenter) = icu_segmenter::WordSegmenter::try_new_auto_unstable(
            &thaw_icu_data::ThawIcuDataProvider,
            Default::default(),
        ) else {
            return format!(
                r#"[{{"segment":{},"index":0,"isWordLike":null}}]"#,
                serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
            );
        };
        let (boundaries, word_like): (Vec<usize>, Vec<bool>) = segmenter
            .as_borrowed()
            .segment_str(text)
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
            serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
        );
    };
    let boundaries: Vec<usize> = segmenter.as_borrowed().segment_str(text).skip(1).collect();
    segments_json(boundaries, None)
}
