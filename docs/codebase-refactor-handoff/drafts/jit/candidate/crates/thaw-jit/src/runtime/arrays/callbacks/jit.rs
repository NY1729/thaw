#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_map(value: f64, operation: impl Fn(f64, f64) -> f64) -> f64 {
    let _source_root = array_callback_root(value);
    let Some((_, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(mut output) = mapped_slots(length) else { return 0.0; };
    for index in 0..length {
        let Some((array, current_length)) = (unsafe { array_data(value) }) else {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        };
        if index >= current_length || !array_index_present(value, index) { continue; }
        let element = unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() };
        let mapped = operation(element, index as f64);
        if CALL_ERROR.with(|error| !error.get().is_null()) { return 0.0; }
        output[index] = Some(mapped.to_bits());
    }
    mapped_array_result(&output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
type JitCallback = extern "C" fn(*const f64) -> f64;

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
#[derive(Clone, Copy, PartialEq, Eq)]
enum JitCallbackResultKind {
    Number,
    Boolean,
    String,
    Dynamic,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
#[derive(Clone, Copy)]
enum JitCallbackPresence {
    Present,
    Undefined,
    Null,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
#[derive(Clone, Copy)]
struct JitCallbackResult {
    word: f64,
    presence: JitCallbackPresence,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
struct CapturedArguments {
    words: Vec<f64>,
    presence: Vec<u8>,
    _source_root: Option<thaw_arena::ArenaRoot>,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
impl CapturedArguments {
    fn empty() -> Self {
        Self {
            words: Vec::new(),
            presence: Vec::new(),
            _source_root: None,
        }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn proven_callback_result_kind(
    proven: u8,
    declared: Option<JitCallbackResultKind>,
) -> Option<JitCallbackResultKind> {
    let kind = match proven {
        0 => JitCallbackResultKind::Number,
        1 => JitCallbackResultKind::Boolean,
        2 => JitCallbackResultKind::String,
        3 => JitCallbackResultKind::Dynamic,
        _ => return None,
    };
    declared.is_none_or(|declared| declared == kind).then_some(kind)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn jit_callback_truthy(result: JitCallbackResult, kind: JitCallbackResultKind) -> bool {
    if !matches!(result.presence, JitCallbackPresence::Present) {
        return false;
    }
    let value = result.word;
    match kind {
        JitCallbackResultKind::Number => value != 0.0 && !value.is_nan(),
        JitCallbackResultKind::Boolean => value != 0.0,
        JitCallbackResultKind::String => {
            let pointer = value.to_bits() as usize as *const std::ffi::c_char;
            !pointer.is_null() && unsafe { *pointer != 0 }
        }
        JitCallbackResultKind::Dynamic => dynamic_to_boolean(value) != 0.0,
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn normalize_jit_callback_result(result: JitCallbackResult, kind: JitCallbackResultKind, target: u8) -> u64 {
    if !matches!(result.presence, JitCallbackPresence::Present) {
        return match target {
            1 => 0,
            0 => match result.presence {
                JitCallbackPresence::Undefined => f64::NAN.to_bits(),
                JitCallbackPresence::Null => 0.0_f64.to_bits(),
                JitCallbackPresence::Present => unreachable!(),
            },
            2 => arena_string(match result.presence {
                JitCallbackPresence::Undefined => "undefined",
                JitCallbackPresence::Null => "null",
                JitCallbackPresence::Present => unreachable!(),
            }.to_owned()).to_bits(),
            _ => 0,
        };
    }
    let value = result.word;
    // StringToNumber and DynamicToString can allocate or re-enter through
    // runtime helpers. Keep a callback-owned pointer live through conversion.
    let _input_root = matches!(kind, JitCallbackResultKind::String | JitCallbackResultKind::Dynamic)
        .then(|| thaw_arena::ArenaRoot::new(value.to_bits() as usize));
    if target == 1 {
        return u64::from(jit_callback_truthy(result, kind));
    }
    if target == 0 {
        return match kind {
            JitCallbackResultKind::Number => value.to_bits(),
            JitCallbackResultKind::Boolean => {
                (if value != 0.0 { 1.0_f64 } else { 0.0_f64 }).to_bits()
            }
            JitCallbackResultKind::String => string_to_number(value).to_bits(),
            JitCallbackResultKind::Dynamic => dynamic_to_number(value).to_bits(),
        };
    }
    match kind {
        JitCallbackResultKind::Number => number_to_string(value).to_bits(),
        JitCallbackResultKind::Boolean => boolean_to_string(value).to_bits(),
        JitCallbackResultKind::String => value.to_bits(),
        JitCallbackResultKind::Dynamic => dynamic_to_string(value).to_bits(),
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn compile_jit_callback(callback: f64) -> Option<(JitCallback, usize, bool, JitCallbackResultKind)> {
    let Some(callback) = (unsafe { string_argument(callback) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    let (expression, explicit_kind) = match callback.rsplit_once(',') {
        Some((expression, "cbresultnumber")) => (expression, Some(JitCallbackResultKind::Number)),
        Some((expression, "cbresultboolean")) => (expression, Some(JitCallbackResultKind::Boolean)),
        Some((expression, "cbresultstring")) => (expression, Some(JitCallbackResultKind::String)),
        Some((expression, "cbresultdynamic")) => (expression, Some(JitCallbackResultKind::Dynamic)),
        Some((_, marker)) if marker.starts_with("cbresult") => {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return None;
        }
        _ => (callback.as_str(), None),
    };
    let symbol = format!("expr:{expression}:array-callback");
    let Some(program) = NumericProgram::parse(&symbol) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    let Some(proven_kind) = program.callback_result_kind_code() else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    let Some(result_kind) = proven_callback_result_kind(proven_kind, explicit_kind) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    let required_args = program.required_args();
    let tagged_arg0 = program.0.iter().any(|value| matches!(value, NumericValue::DynamicArgument(0)));
    if required_args > 16 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    }
    let code = match compile(&symbol, &program) {
        Ok((code, _)) => code,
        Err(error) => {
            CALL_ERROR.with(|slot| slot.set(error));
            return None;
        }
    };
    Some((
        unsafe { std::mem::transmute::<*mut libc::c_void, JitCallback>(code) },
        required_args,
        tagged_arg0,
        result_kind,
    ))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn capture_arguments(value: f64, builtins: usize, required: usize) -> Option<CapturedArguments> {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    if builtins + length > 16 || required > builtins + length {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    }
    let mut words = Vec::with_capacity(length);
    let mut presence = Vec::with_capacity(length);
    for index in 0..length {
        let state = array_index_state(value, index);
        words.push(if state == 1 {
            unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() }
        } else {
            0.0
        });
        // Captured holes and sparse undefined entries are callback undefined.
        presence.push(u8::from(state != 1));
    }
    Some(CapturedArguments {
        words,
        presence,
        _source_root: Some(array_callback_root(value)),
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn call_jit_callback(callback: JitCallback, builtins: &[f64], captures: &CapturedArguments) -> JitCallbackResult {
    call_jit_callback_with_presence(callback, builtins, captures, &[])
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn call_jit_callback_with_presence(
    callback: JitCallback,
    builtins: &[f64],
    captures: &CapturedArguments,
    builtin_presence: &[u8],
) -> JitCallbackResult {
    let mut arguments = [0.0; 16];
    arguments[..builtins.len()].copy_from_slice(builtins);
    arguments[builtins.len()..builtins.len() + captures.words.len()].copy_from_slice(&captures.words);
    let mut input_presence = [0u8; 16];
    let presence_count = builtin_presence.len().min(builtins.len());
    input_presence[..presence_count].copy_from_slice(&builtin_presence[..presence_count]);
    input_presence[builtins.len()..builtins.len() + captures.presence.len()]
        .copy_from_slice(&captures.presence);
    let prior_input = RECUR_INPUT_PRESENCE.with(|slot| {
        slot.replace((input_presence.as_ptr(), builtins.len() + captures.words.len()))
    });
    let prior_present = CALL_PRESENT.with(|present| present.replace(true));
    let prior_absence = CALL_ABSENCE.with(|absence| absence.replace(1));
    let word = callback(arguments.as_ptr());
    let present = CALL_PRESENT.with(Cell::get);
    let absence = CALL_ABSENCE.with(Cell::get);
    CALL_PRESENT.with(|slot| slot.set(prior_present));
    CALL_ABSENCE.with(|slot| slot.set(prior_absence));
    RECUR_INPUT_PRESENCE.with(|slot| slot.set(prior_input));
    JitCallbackResult {
        word,
        presence: if present { JitCallbackPresence::Present } else if absence == 2 {
            JitCallbackPresence::Null
        } else {
            JitCallbackPresence::Undefined
        },
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn primitive_array_jit_map_impl(
    value: f64,
    callback: f64,
    encoded: f64,
    captures: Option<f64>,
) -> f64 {
    let _source_root = array_callback_root(value);
    let encoded = encoded as u8;
    let source = encoded / 4;
    let target = encoded % 4;
    if source > 2 || target > 2 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required, tagged_arg0, result_kind)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let builtins = if tagged_arg0 { 4 } else { 3 };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, builtins, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > builtins {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        CapturedArguments::empty()
    };
    let Some((_, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(mut output) = mapped_slots(length) else { return 0.0; };
    let mut output_states = Vec::new();
    if output_states.try_reserve_exact(length).is_err() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    output_states.resize(length, 0);
    let mut mapped_strings = if target == 2 {
        let Some(roots) = mapped_string_roots(length) else { return 0.0; };
        Some(roots)
    } else { None };
    for index in 0..length {
        let Some((array, current_length)) = (unsafe { array_data(value) }) else {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        };
        if index >= current_length { continue; }
        let state = array_index_state(value, index);
        if state == 0 { continue; }
        let element = unsafe { array_element(array, index, source) };
        let _element_root = (source == 2 && state == 1)
            .then(|| thaw_arena::ArenaRoot::new(element.to_bits() as usize));
        let mapped = if tagged_arg0 {
            let tag = if state == 2 { DYNAMIC_UNDEFINED_TAG } else {
                [DYNAMIC_NUMBER_TAG, DYNAMIC_BOOLEAN_TAG, DYNAMIC_STRING_TAG][source as usize]
            };
            call_jit_callback(
                callback,
                &[tag as f64, if state == 2 { 0.0 } else { element }, index as f64, value],
                &captures,
            )
        } else {
            call_jit_callback_with_presence(
                callback,
                &[element, index as f64, value],
                &captures,
                &[if state == 2 { 1 } else { 0 }, 0, 0],
            )
        };
        if CALL_ERROR.with(|error| !error.get().is_null()) { return 0.0; }
        let undefined = matches!(mapped.presence, JitCallbackPresence::Undefined);
        let mapped = if undefined { 0 } else { normalize_jit_callback_result(mapped, result_kind, target) };
        if CALL_ERROR.with(|error| !error.get().is_null()) { return 0.0; }
        output[index] = Some(mapped);
        output_states[index] = if undefined { 2 } else { 1 };
        if !undefined {
            if let Some(roots) = mapped_strings.as_mut() {
                roots.push(thaw_arena::ArenaRoot::new(mapped as usize));
            }
        }
    }
    mapped_array_result_with_states(&output, &output_states)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_map(value: f64, callback: f64, encoded: f64) -> f64 {
    primitive_array_jit_map_impl(value, callback, encoded, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_map_captured(
    value: f64,
    callback: f64,
    captures: f64,
    encoded: f64,
) -> f64 {
    primitive_array_jit_map_impl(value, callback, encoded, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_map_impl(
    value: f64,
    callback: f64,
    target: f64,
    captures: Option<f64>,
) -> f64 {
    let target = target as u8;
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let _dynamic_root = thaw_arena::ArenaRoot::new(value.to_bits() as usize);
    let (source, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    if target > 2 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required, _tagged_arg0, result_kind)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 5, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 5 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        CapturedArguments::empty()
    };
    let source_value = f64::from_bits(array.payload);
    let _source_root = array_callback_root(source_value);
    let Some((_, length)) = (unsafe { array_data(source_value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(mut output) = mapped_slots(length) else { return 0.0; };
    let mut output_states = Vec::new();
    if output_states.try_reserve_exact(length).is_err() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    output_states.resize(length, 0);
    let mut mapped_strings = if target == 2 {
        let Some(roots) = mapped_string_roots(length) else { return 0.0; };
        Some(roots)
    } else { None };
    for index in 0..length {
        let Some((data, current_length)) = (unsafe { array_data(source_value) }) else {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        };
        if index >= current_length { continue; }
        let state = array_index_state(source_value, index);
        if state == 0 { continue; }
        let element = unsafe { array_element(data, index, source) };
        let _element_root = (source == 2 && state == 1)
            .then(|| thaw_arena::ArenaRoot::new(element.to_bits() as usize));
        let mapped = call_jit_callback(
            callback,
            &[
                (if state == 2 { DYNAMIC_UNDEFINED_TAG } else { element_tag }) as f64,
                if state == 2 { 0.0 } else { element },
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
        if CALL_ERROR.with(|error| !error.get().is_null()) { return 0.0; }
        let undefined = matches!(mapped.presence, JitCallbackPresence::Undefined);
        let mapped = if undefined { 0 } else { normalize_jit_callback_result(mapped, result_kind, target) };
        if CALL_ERROR.with(|error| !error.get().is_null()) { return 0.0; }
        output[index] = Some(mapped);
        output_states[index] = if undefined { 2 } else { 1 };
        if !undefined {
            if let Some(roots) = mapped_strings.as_mut() {
                roots.push(thaw_arena::ArenaRoot::new(mapped as usize));
            }
        }
    }
    mapped_array_result_with_states(&output, &output_states)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_map(value: f64, callback: f64, target: f64) -> f64 {
    dynamic_array_jit_map_impl(value, callback, target, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_map_captured(
    value: f64,
    callback: f64,
    captures: f64,
    target: f64,
) -> f64 {
    dynamic_array_jit_map_impl(value, callback, target, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_scan_impl(value: f64, callback: f64, mode: f64, captures: Option<f64>) -> f64 {
    let mode = mode as u8;
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let _dynamic_root = thaw_arena::ArenaRoot::new(value.to_bits() as usize);
    let (kind, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    if mode > 6 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required, _tagged_arg0, result_kind)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 5, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 5 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        CapturedArguments::empty()
    };
    let source_value = f64::from_bits(array.payload);
    let result = primitive_array_scan(source_value, kind, mode, |element, index, state| {
        let (tag, element) = if element.is_none() || state == 2 {
            (DYNAMIC_UNDEFINED_TAG, 0.0)
        } else {
            (element_tag, element.unwrap_or(0.0))
        };
        let result = call_jit_callback(
            callback,
            &[
                tag as f64,
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
        jit_callback_truthy(result, result_kind)
    });
    dynamic_array_scan_result(array.tag, mode, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_scan(value: f64, callback: f64, mode: f64) -> f64 {
    dynamic_array_jit_scan_impl(value, callback, mode, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_scan_captured(
    value: f64,
    callback: f64,
    captures: f64,
    mode: f64,
) -> f64 {
    dynamic_array_jit_scan_impl(value, callback, mode, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_scan(value: f64, callback: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let kind = encoded / 8;
    let mode = encoded % 8;
    if kind > 2 || mode > 6 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required, tagged_arg0, result_kind)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let builtins = if tagged_arg0 { 4 } else { 3 };
    if required > builtins {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    primitive_array_scan(value, kind, mode, |element, index, state| {
        let result = if tagged_arg0 {
            let tag = if state == 0 || state == 2 {
                DYNAMIC_UNDEFINED_TAG
            } else {
                [DYNAMIC_NUMBER_TAG, DYNAMIC_BOOLEAN_TAG, DYNAMIC_STRING_TAG][kind as usize]
            };
            let payload = if tag == DYNAMIC_UNDEFINED_TAG { 0.0 } else { element.unwrap_or(0.0) };
            call_jit_callback(
                callback,
                &[tag as f64, payload, index as f64, value],
                &CapturedArguments::empty(),
            )
        } else {
            call_jit_callback_with_presence(
                callback,
                &[element.unwrap_or(0.0), index as f64, value],
                &CapturedArguments::empty(),
                &[u8::from(state != 1), 0, 0],
            )
        };
        jit_callback_truthy(result, result_kind)
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_scan_captured(
    value: f64,
    callback: f64,
    captures: f64,
    encoded: f64,
) -> f64 {
    let encoded = encoded as u8;
    let kind = encoded / 8;
    let mode = encoded % 8;
    if kind > 2 || mode > 6 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required, tagged_arg0, result_kind)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let builtins = if tagged_arg0 { 4 } else { 3 };
    let Some(captures) = capture_arguments(captures, builtins, required) else {
        return 0.0;
    };
    primitive_array_scan(value, kind, mode, |element, index, state| {
        let result = if tagged_arg0 {
            let tag = if state == 0 || state == 2 {
                DYNAMIC_UNDEFINED_TAG
            } else {
                [DYNAMIC_NUMBER_TAG, DYNAMIC_BOOLEAN_TAG, DYNAMIC_STRING_TAG][kind as usize]
            };
            let payload = if tag == DYNAMIC_UNDEFINED_TAG { 0.0 } else { element.unwrap_or(0.0) };
            call_jit_callback(callback, &[tag as f64, payload, index as f64, value], &captures)
        } else {
            call_jit_callback_with_presence(
                callback,
                &[element.unwrap_or(0.0), index as f64, value],
                &captures,
                &[u8::from(state != 1), 0, 0],
            )
        };
        jit_callback_truthy(result, result_kind)
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_jit_reduce(
    value: f64,
    initial: f64,
    callback: f64,
    from_right: bool,
    has_initial: bool,
    captures: Option<f64>,
) -> f64 {
    let initial_presence = read_call_presence() as u8;
    let _source_root = array_callback_root(value);
    let Some((callback, required, _tagged_arg0, result_kind)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    if result_kind != JitCallbackResultKind::Number {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 4, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 4 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        CapturedArguments::empty()
    };
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let first = if has_initial { None } else {
        let first = (0..length)
            .map(|offset| if from_right { length - 1 - offset } else { offset })
            .find(|index| array_index_present(value, *index));
        if first.is_none() {
            CALL_ERROR.with(|error| error.set(EMPTY_REDUCE.as_ptr().cast()));
            return 0.0;
        }
        first
    };
    let mut accumulator = if let Some(index) = first {
        unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() }
    } else { initial };
    let mut accumulator_presence = first.map_or_else(
        || initial_presence,
        |index| u8::from(array_index_state(value, index) == 2),
    );
    let mut apply = |index| {
        if CALL_ERROR.with(|error| !error.get().is_null()) { return; }
        let Some((current, current_length)) = (unsafe { array_data(value) }) else {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return;
        };
        if index >= current_length || !array_index_present(value, index) { return; }
        let element_presence = u8::from(array_index_state(value, index) == 2);
        let element = unsafe { current.add(8 + index * 8).cast::<f64>().read_unaligned() };
        let result = call_jit_callback_with_presence(
            callback,
            &[accumulator, element, index as f64, value],
            &captures,
            &[accumulator_presence, element_presence, 0, 0],
        );
        accumulator = result.word;
        accumulator_presence = match result.presence {
            JitCallbackPresence::Present => 0,
            JitCallbackPresence::Undefined => 1,
            JitCallbackPresence::Null => 2,
        };
    };
    if from_right {
        for index in (0..first.unwrap_or(length)).rev() {
            apply(index);
        }
    } else {
        for index in first.map_or(0, |index| index + 1)..length {
            apply(index);
        }
    }
    publish_call_presence(u64::from(accumulator_presence));
    accumulator
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! jit_reduce_fn {
    ($name:ident, $from_right:expr, $has_initial:expr) => {
        extern "C" fn $name(value: f64, initial: f64, callback: f64) -> f64 {
            number_array_jit_reduce(value, initial, callback, $from_right, $has_initial, None)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_initial, false, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_first, false, false);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_right_initial, true, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_right_last, true, false);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! jit_reduce_captured_fn {
    ($name:ident, $from_right:expr, $has_initial:expr) => {
        extern "C" fn $name(value: f64, initial: f64, callback: f64, captures: f64) -> f64 {
            number_array_jit_reduce(
                value,
                initial,
                callback,
                $from_right,
                $has_initial,
                Some(captures),
            )
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_initial_captured, false, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_first_captured, false, false);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_right_initial_captured, true, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_right_last_captured, true, false);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_reduce(
    value: f64,
    initial: f64,
    callback: f64,
    from_right: bool,
    captures: Option<f64>,
) -> f64 {
    let initial_presence = read_call_presence() as u8;
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let _dynamic_root = thaw_arena::ArenaRoot::new(value.to_bits() as usize);
    let (kind, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let source_value = f64::from_bits(array.payload);
    let _source_root = array_callback_root(source_value);
    let Some((callback, required, _tagged_arg0, result_kind)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    if result_kind != JitCallbackResultKind::Number {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 6, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 6 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        CapturedArguments::empty()
    };
    let Some((data, length)) = (unsafe { array_data(f64::from_bits(array.payload)) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let mut accumulator = initial;
    let mut accumulator_presence = initial_presence;
    let mut apply = |index| {
        if CALL_ERROR.with(|error| !error.get().is_null()) { return; }
        let Some((current, current_length)) =
            (unsafe { array_data(f64::from_bits(array.payload)) }) else {
                CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
                return;
            };
        if index >= current_length { return; }
        let state = array_index_state(f64::from_bits(array.payload), index);
        if state == 0 { return; }
        let element = if state == 1 { unsafe { array_element(current, index, kind) } } else { 0.0 };
        let _element_root = (state == 1 && kind == 2)
            .then(|| thaw_arena::ArenaRoot::new(element.to_bits() as usize));
        let result = call_jit_callback_with_presence(
            callback,
            &[
                accumulator,
                if state == 2 { DYNAMIC_UNDEFINED_TAG as f64 } else { element_tag as f64 },
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
            &[accumulator_presence, 0, u8::from(state == 2), 0, 0, 0],
        );
        accumulator = result.word;
        accumulator_presence = match result.presence {
            JitCallbackPresence::Present => 0,
            JitCallbackPresence::Undefined => 1,
            JitCallbackPresence::Null => 2,
        };
    };
    if from_right {
        for index in (0..length).rev() {
            apply(index);
        }
    } else {
        for index in 0..length {
            apply(index);
        }
    }
    publish_call_presence(u64::from(accumulator_presence));
    accumulator
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_reduce_unseeded(
    value: f64,
    callback: f64,
    from_right: bool,
    captures: Option<f64>,
) -> f64 {
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let _dynamic_root = thaw_arena::ArenaRoot::new(value.to_bits() as usize);
    let (kind, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let source_value = f64::from_bits(array.payload);
    let _source_root = array_callback_root(source_value);
    let Some((callback, required, _tagged_arg0, result_kind)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    if result_kind != JitCallbackResultKind::Dynamic {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 7, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 7 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        CapturedArguments::empty()
    };
    let Some((data, length)) = (unsafe { array_data(f64::from_bits(array.payload)) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let first = (0..length)
        .map(|offset| if from_right { length - 1 - offset } else { offset })
        .find(|index| array_index_present(f64::from_bits(array.payload), *index));
    let Some(first) = first else {
        CALL_ERROR.with(|error| error.set(EMPTY_REDUCE.as_ptr().cast()));
        return 0.0;
    };
    let first_state = array_index_state(f64::from_bits(array.payload), first);
    let first_payload = if first_state == 1 {
        unsafe { array_element(data, first, kind) }
    } else { 0.0 };
    let mut accumulator = DynamicPrimitive {
        tag: if first_state == 2 { DYNAMIC_UNDEFINED_TAG } else { element_tag },
        payload: if element_tag == DYNAMIC_BOOLEAN_TAG {
            u64::from(first_payload != 0.0)
        } else {
            first_payload.to_bits()
        },
    };
    let mut accumulator_root = (accumulator.tag == DYNAMIC_STRING_TAG)
        .then(|| thaw_arena::ArenaRoot::new(accumulator.payload as usize));
    let mut apply = |index| -> Option<()> {
        if CALL_ERROR.with(|error| !error.get().is_null()) { return None; }
        let Some((current, current_length)) =
            (unsafe { array_data(f64::from_bits(array.payload)) }) else {
                CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
                return None;
            };
        if index >= current_length { return Some(()); }
        let state = array_index_state(f64::from_bits(array.payload), index);
        if state == 0 { return Some(()); }
        let element = if state == 1 { unsafe { array_element(current, index, kind) } } else { 0.0 };
        let _element_root = (state == 1 && kind == 2)
            .then(|| thaw_arena::ArenaRoot::new(element.to_bits() as usize));
        let callback_result = call_jit_callback(
            callback,
            &[
                accumulator.tag as f64,
                f64::from_bits(accumulator.payload),
                if state == 2 { DYNAMIC_UNDEFINED_TAG as f64 } else { element_tag as f64 },
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
        if CALL_ERROR.with(|error| !error.get().is_null()) { return None; }
        let result = match callback_result.presence {
            JitCallbackPresence::Present => dynamic_primitive(callback_result.word, None)?,
            JitCallbackPresence::Undefined => DynamicPrimitive { tag: DYNAMIC_UNDEFINED_TAG, payload: 0 },
            JitCallbackPresence::Null => DynamicPrimitive { tag: DYNAMIC_NULL_TAG, payload: 0 },
        };
        accumulator = DynamicPrimitive {
            tag: result.tag,
            payload: result.payload,
        };
        accumulator_root = (accumulator.tag == DYNAMIC_STRING_TAG)
            .then(|| thaw_arena::ArenaRoot::new(accumulator.payload as usize));
        Some(())
    };
    if from_right {
        for index in (0..first).rev() {
            if apply(index).is_none() {
                CALL_ERROR.with(|error| {
                    if error.get().is_null() {
                        error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast());
                    }
                });
                return 0.0;
            }
        }
    } else {
        for index in first + 1..length {
            if apply(index).is_none() {
                CALL_ERROR.with(|error| {
                    if error.get().is_null() {
                        error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast());
                    }
                });
                return 0.0;
            }
        }
    }
    arena_dynamic(accumulator.tag, accumulator.payload)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left(value: f64, initial: f64, callback: f64) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, false, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right(value: f64, initial: f64, callback: f64) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, true, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left_captured(
    value: f64,
    initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, false, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right_captured(
    value: f64,
    initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, true, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left_unseeded(
    value: f64,
    _initial: f64,
    callback: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, false, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right_unseeded(
    value: f64,
    _initial: f64,
    callback: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, true, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left_unseeded_captured(
    value: f64,
    _initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, false, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right_unseeded_captured(
    value: f64,
    _initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, true, Some(captures))
}
