//! V8's `JSON.parse` error messages (Node 22). Run only after the real parser has
//! rejected the text: a recursive-descent scan that mirrors `JsonParser` in V8
//! (`src/json/json-parser.cc`) and stops at the first offence with the same wording.

use std::ffi::{c_char, CStr};

const MAX_CONTEXT: usize = 10;

struct Scan<'a> {
    text: &'a [u16],
    at: usize,
}

type Failure = String;

impl Scan<'_> {
    fn peek(&self) -> Option<u16> {
        self.text.get(self.at).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(0x20 | 0x09 | 0x0A | 0x0D)) {
            self.at += 1;
        }
    }

    fn location(&self, pos: usize) -> String {
        let (mut line, mut start, mut i) = (1, 0, 0);
        while i < pos.min(self.text.len()) {
            let c = self.text[i];
            if c == 0x0D && self.text.get(i + 1) == Some(&0x0A) {
                i += 1;
            }
            if c == 0x0D || c == 0x0A {
                line += 1;
                start = i + 1;
            }
            i += 1;
        }
        format!("(line {line} column {})", pos - start + 1)
    }

    /// V8's `ReportUnexpectedToken`: `explicit` is the call-site message, else the
    /// wording follows the token under the cursor.
    fn fail(&self, pos: usize, explicit: Option<&str>) -> Failure {
        let at = |what: &str| {
            let tail = if what.ends_with("after JSON") { "" } else { " in JSON" };
            format!("{what}{tail} at position {pos} {}", self.location(pos))
        };
        if let Some(message) = explicit {
            return at(message);
        }
        match self.text.get(pos).copied() {
            None => "Unexpected end of JSON input".to_string(),
            Some(0x22) => at("Unexpected string"),
            Some(0x2D | 0x30..=0x39) => at("Unexpected number"),
            Some(unit) => {
                let whole = String::from_utf16_lossy(self.text);
                if matches!(
                    whole.as_str(),
                    "NaN" | "Infinity" | "undefined" | "[object Object]"
                ) {
                    return format!("\"{whole}\" is not valid JSON");
                }
                let token = String::from_utf16_lossy(&[unit]);
                let len = self.text.len();
                if len < 2 * MAX_CONTEXT + 1 {
                    return format!("Unexpected token '{token}', \"{whole}\" is not valid JSON");
                }
                let slice = |from: usize, to: usize| String::from_utf16_lossy(&self.text[from..to]);
                if pos < MAX_CONTEXT {
                    let context = slice(0, pos + MAX_CONTEXT);
                    format!("Unexpected token '{token}', \"{context}\"... is not valid JSON")
                } else if pos < len - MAX_CONTEXT {
                    let context = slice(pos - MAX_CONTEXT, pos + MAX_CONTEXT);
                    format!("Unexpected token '{token}', ...\"{context}\"... is not valid JSON")
                } else {
                    let context = slice(pos - MAX_CONTEXT, len);
                    format!("Unexpected token '{token}', ...\"{context}\" is not valid JSON")
                }
            }
        }
    }

    fn value(&mut self) -> Result<(), Failure> {
        self.skip_ws();
        match self.peek() {
            Some(0x22) => self.string(),
            Some(0x7B) => self.object(),
            Some(0x5B) => self.array(),
            Some(0x2D | 0x30..=0x39) => self.number(),
            Some(0x74) => self.literal("true"),
            Some(0x66) => self.literal("false"),
            Some(0x6E) => self.literal("null"),
            _ => Err(self.fail(self.at, None)),
        }
    }

    fn literal(&mut self, word: &str) -> Result<(), Failure> {
        for expected in word.encode_utf16() {
            if self.peek() != Some(expected) {
                return Err(self.fail(self.at, None));
            }
            self.at += 1;
        }
        Ok(())
    }

    fn number(&mut self) -> Result<(), Failure> {
        let digit = |unit: Option<u16>| matches!(unit, Some(0x30..=0x39));
        if self.peek() == Some(0x2D) {
            self.at += 1;
            if !digit(self.peek()) {
                return Err(self.fail(self.at, Some("No number after minus sign")));
            }
        }
        if self.peek() == Some(0x30) {
            self.at += 1;
            if digit(self.peek()) {
                return Err(self.fail(self.at, None));
            }
        } else {
            while digit(self.peek()) {
                self.at += 1;
            }
        }
        if self.peek() == Some(0x2E) {
            self.at += 1;
            if !digit(self.peek()) {
                return Err(self.fail(self.at, Some("Unterminated fractional number")));
            }
            while digit(self.peek()) {
                self.at += 1;
            }
        }
        if matches!(self.peek(), Some(0x65 | 0x45)) {
            self.at += 1;
            if matches!(self.peek(), Some(0x2B | 0x2D)) {
                self.at += 1;
            }
            if !digit(self.peek()) {
                return Err(self.fail(self.at, Some("Exponent part is missing a number")));
            }
            while digit(self.peek()) {
                self.at += 1;
            }
        }
        Ok(())
    }

    fn string(&mut self) -> Result<(), Failure> {
        self.at += 1;
        loop {
            match self.peek() {
                None => return Err(self.fail(self.at, Some("Unterminated string"))),
                Some(0x22) => {
                    self.at += 1;
                    return Ok(());
                }
                Some(0..=0x1F) => {
                    return Err(self.fail(self.at, Some("Bad control character in string literal")))
                }
                Some(0x5C) => {
                    self.at += 1;
                    match self.peek() {
                        None => return Err(self.fail(self.at, None)),
                        Some(0x22 | 0x5C | 0x2F | 0x62 | 0x66 | 0x6E | 0x72 | 0x74) => self.at += 1,
                        Some(0x75) => {
                            self.at += 1;
                            for _ in 0..4 {
                                let hex = self
                                    .peek()
                                    .and_then(|unit| char::from_u32(u32::from(unit)))
                                    .is_some_and(|c| c.is_ascii_hexdigit());
                                if !hex {
                                    return Err(self.fail(self.at, Some("Bad Unicode escape")));
                                }
                                self.at += 1;
                            }
                        }
                        _ => return Err(self.fail(self.at, Some("Bad escaped character"))),
                    }
                }
                Some(_) => self.at += 1,
            }
        }
    }

    fn expect(&mut self, unit: u16, message: &str) -> Result<(), Failure> {
        self.skip_ws();
        if self.peek() == Some(unit) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.fail(self.at, Some(message)))
        }
    }

    fn property(&mut self, name_message: &str) -> Result<(), Failure> {
        self.skip_ws();
        if self.peek() != Some(0x22) {
            return Err(self.fail(self.at, Some(name_message)));
        }
        self.string()?;
        self.expect(0x3A, "Expected ':' after property name")
    }

    fn object(&mut self) -> Result<(), Failure> {
        self.at += 1;
        self.skip_ws();
        if self.peek() == Some(0x7D) {
            self.at += 1;
            return Ok(());
        }
        self.property("Expected property name or '}'")?;
        loop {
            self.value()?;
            self.skip_ws();
            match self.peek() {
                Some(0x2C) => {
                    self.at += 1;
                    self.property("Expected double-quoted property name")?;
                }
                Some(0x7D) => {
                    self.at += 1;
                    return Ok(());
                }
                _ => return Err(self.fail(self.at, Some("Expected ',' or '}' after property value"))),
            }
        }
    }

    fn array(&mut self) -> Result<(), Failure> {
        self.at += 1;
        self.skip_ws();
        if self.peek() == Some(0x5D) {
            self.at += 1;
            return Ok(());
        }
        loop {
            self.value()?;
            self.skip_ws();
            if self.peek() == Some(0x2C) {
                self.at += 1;
            } else {
                return self.expect(0x5D, "Expected ',' or ']' after array element");
            }
        }
    }
}

/// The `SyntaxError` message V8 gives `JSON.parse(source)`.
pub(crate) fn parse_error_message(source: &str) -> String {
    let text: Vec<u16> = source.encode_utf16().collect();
    let mut scan = Scan { text: &text, at: 0 };
    let outcome = scan.value().and_then(|()| {
        scan.skip_ws();
        match scan.peek() {
            None => Ok(()),
            Some(_) => Err(scan.fail(scan.at, Some("Unexpected non-whitespace character after JSON"))),
        }
    });
    outcome.err().unwrap_or_else(|| "Unexpected token in JSON".to_string())
}

/// # Safety
/// `source` must be a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_parse_error_message(source: *const c_char) -> *const c_char {
    let source = String::from_utf8_lossy(unsafe { CStr::from_ptr(source) }.to_bytes());
    thaw_arena::owned_string(parse_error_message(&source))
}

#[cfg(test)]
mod tests {
    use super::parse_error_message;

    /// `[source, message]` pairs recorded from Node v22 (`json_parse_error_corpus.json`).
    #[test]
    fn messages_match_node() {
        let corpus: Vec<(String, String)> =
            serde_json::from_str(include_str!("json_parse_error_corpus.json")).unwrap();
        assert!(corpus.len() > 300);
        let mut wrong = Vec::new();
        for (source, expected) in &corpus {
            let got = parse_error_message(source);
            if &got != expected {
                wrong.push(format!("{source:?}\n  node: {expected}\n  here: {got}"));
            }
        }
        assert!(wrong.is_empty(), "{} mismatches:\n{}", wrong.len(), wrong.join("\n"));
    }
}
