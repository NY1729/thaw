//! Json values for the native `node:http` surface (`req.headers`, `res.getHeader`, `res.writeHead(code, headers)`).
//! Kept next to `json.rs` because `Value` and its constructors are private to that module tree.

use super::{leak, napi_undefined_value, utf8_or_wtf8, Value};

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn text_value(text: &str) -> Value {
    utf8_or_wtf8(text.as_bytes().to_vec())
}

fn object_of(fields: Vec<(String, Value)>) -> Value {
    let mut map = indexmap::IndexMap::new();
    for (name, value) in fields {
        map.insert(name.into_bytes(), value);
    }
    Value::shared_object(map)
}

/// A header value as Json: absent -> `undefined`, one value -> string, several -> array (`set-cookie`).
pub(crate) fn header_values_json(values: &[String], numeric: bool) -> *mut Value {
    leak(match values {
        [] => napi_undefined_value(),
        [one] if numeric => one
            .parse::<serde_json::Number>()
            .map_or_else(|_| text_value(one), Value::Number),
        [one] => text_value(one),
        many => Value::shared_array(many.iter().map(|text| text_value(text)).collect()),
    })
}

#[cfg(test)]
pub(crate) fn undefined_json() -> *mut Value {
    leak(napi_undefined_value())
}

pub(crate) fn string_array_json(items: &[String]) -> *mut Value {
    leak(Value::shared_array(
        items.iter().map(|text| text_value(text)).collect(),
    ))
}

pub(crate) fn empty_object_json() -> *mut Value {
    leak(object_of(Vec::new()))
}

/// `name -> value` object (`req.headers`): one entry per name, `set-cookie` always an array.
pub(crate) fn merged_headers_json(entries: &[(String, String)]) -> *mut Value {
    leak(object_of(
        entries
            .iter()
            .map(|(name, value)| {
                let json = if name == "set-cookie" {
                    Value::shared_array(value.split('\n').map(text_value).collect())
                } else {
                    text_value(value)
                };
                (name.clone(), json)
            })
            .collect(),
    ))
}

/// `name -> string[]` object (`req.headersDistinct`, `res.getHeaders()` style with arrays).
pub(crate) fn distinct_headers_json(entries: &[(String, Vec<String>)]) -> *mut Value {
    leak(object_of(
        entries
            .iter()
            .map(|(name, values)| {
                (
                    name.clone(),
                    Value::shared_array(values.iter().map(|text| text_value(text)).collect()),
                )
            })
            .collect(),
    ))
}

/// `name -> string | number | string[]` (`res.getHeaders()`); `numeric` lists the names last set from a number.
pub(crate) fn response_headers_json(
    entries: &[(String, Vec<String>)],
    numeric: &[String],
) -> *mut Value {
    leak(object_of(
        entries
            .iter()
            .map(|(name, values)| {
                let json = match values.as_slice() {
                    [one] if numeric.contains(name) => one
                        .parse::<serde_json::Number>()
                        .map_or_else(|_| text_value(one), Value::Number),
                    [one] => text_value(one),
                    many => Value::shared_array(many.iter().map(|text| text_value(text)).collect()),
                };
                (name.clone(), json)
            })
            .collect(),
    ))
}

/// Whether a `setHeader` argument was a JSON number (`getHeader` hands such a value back as a number, like Node).
///
/// # Safety
/// `value` must be null or a live Json value.
pub(crate) unsafe fn header_is_number(value: *const Value) -> bool {
    matches!(unsafe { value.as_ref() }, Some(Value::Number(_)))
}

/// One header value (scalar) or several (an array, or an engine-side array-like).
fn header_texts(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => super::shared_array_ref(items)
            .iter()
            .filter_map(scalar_text)
            .collect(),
        Value::Host(lease) => super::host_enumerable_entries(lease)
            .into_iter()
            .filter_map(|(_, item)| scalar_text(&item))
            .collect(),
        other => scalar_text(other).into_iter().collect(),
    }
}

/// A header argument (`setHeader(name, value)`): a scalar is one value, an array is several; anything else is none.
///
/// # Safety
/// `value` must be null or a live Json value.
pub(crate) unsafe fn header_values_of(value: *const Value) -> Vec<String> {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return Vec::new();
    };
    if super::is_json_undefined(value) {
        return Vec::new();
    }
    header_texts(value)
}

/// `writeHead`'s headers argument: an object (`{name: value | value[]}`) or a flat `[name, value, ...]` array.
///
/// # Safety
/// `value` must be null or a live Json value.
pub(crate) unsafe fn header_entries_of(value: *const Value) -> Vec<(String, Vec<String>)> {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return Vec::new();
    };
    // `writeHead(code)` / `writeHead(code, undefined)`: the absent headers arrive as the `undefined` sentinel object.
    if super::is_json_undefined(value) || matches!(value, Value::Null) {
        return Vec::new();
    }
    match value {
        Value::Object(fields) => super::shared_object_ref(fields)
            .iter()
            .filter(|(_, value)| !super::is_json_undefined(value))
            .map(|(name, value)| {
                (
                    String::from_utf8_lossy(name).into_owned(),
                    header_texts(value),
                )
            })
            .collect(),
        // An object built by the JS engine (`Object.fromEntries(response.headers)`): its own enumerable entries.
        Value::Host(lease) => super::host_enumerable_entries(lease)
            .into_iter()
            .filter(|(_, value)| !super::is_json_undefined(value))
            .map(|(name, value)| {
                (
                    String::from_utf8_lossy(&name).into_owned(),
                    header_texts(&value),
                )
            })
            .collect(),
        Value::Array(items) => super::shared_array_ref(items)
            .chunks(2)
            .filter_map(|pair| match pair {
                [name, value] => {
                    Some((scalar_text(name)?, scalar_text(value).into_iter().collect()))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// `request.socket`: the peer address fields real handlers read (`remoteAddress`, `remotePort`, `remoteFamily`).
pub(crate) fn socket_json(peer: Option<std::net::SocketAddr>) -> *mut Value {
    let mut fields = vec![("encrypted".to_string(), Value::Bool(false))];
    if let Some(peer) = peer {
        let ip = peer.ip().to_string();
        fields.push(("remoteAddress".into(), text_value(&ip)));
        fields.push(("remotePort".into(), Value::Number(peer.port().into())));
        fields.push((
            "remoteFamily".into(),
            text_value(if peer.is_ipv4() { "IPv4" } else { "IPv6" }),
        ));
    }
    leak(object_of(fields))
}
