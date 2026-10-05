#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
struct LoopPatch {
    start: usize,
    continue_target: Option<usize>,
    base_depth: u8,
    condition_exits: Vec<usize>,
    continues: Vec<usize>,
    breaks: Vec<usize>,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
struct SwitchPatch {
    base_depth: u8,
    next_case: Vec<usize>,
    fallthrough: Option<usize>,
    breaks: Vec<usize>,
    has_case: bool,
    has_default: bool,
    default_body: Option<usize>,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
struct TryPatch {
    base_depth: u8,
    throws: Vec<usize>,
    tagged: bool,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
struct ResultPatch {
    base_depth: u8,
    result_depth: Option<u8>,
    exits: Vec<usize>,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn take_call_error() -> f64 {
    let error = CALL_ERROR.with(|error| error.replace(ptr::null()));
    f64::from_bits(error as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn has_call_error() -> u64 {
    u64::from(!CALL_ERROR.with(Cell::get).is_null())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_return_if_call_error(code: &mut Vec<u8>, depth: u8, frame_size: u32) -> Option<()> {
    emit_spill(code, depth);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&(has_call_error as *const () as u64).to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_restore(code, depth);
    code.extend_from_slice(&[0x48, 0x85, 0xc0]);
    let no_error = emit_near_jump(code, 0x84);
    code.extend_from_slice(&[0x66, 0x0f, 0xef, 0xc0]);
    emit_presence_epilogue(code, frame_size);
    patch_near_jump(code, no_error)
}

// Each XMM value slot has a parallel byte in the permanent metadata frame.
// 0=present, 1=undefined, 2=null. The helpers only run at steady RSP.
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_presence_set(code: &mut Vec<u8>, slot: u8, state: u8) {
    code.extend_from_slice(&[0xc6, 0x44, 0x24, slot, state]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_presence_copy(code: &mut Vec<u8>, destination: u8, source: u8) {
    if destination != source {
        code.extend_from_slice(&[0x8a, 0x44, 0x24, source, 0x88, 0x44, 0x24, destination]);
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_presence_copy_scratch(code: &mut Vec<u8>, slot: u8, save: bool) {
    let scratch = 8 + slot;
    let (destination, source) = if save { (scratch, slot) } else { (slot, scratch) };
    code.extend_from_slice(&[
        0x8a, 0x44, 0x24, source, 0x88, 0x44, 0x24, destination,
    ]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_presence_restore_scratch(code: &mut Vec<u8>, destination: u8, source: u8) {
    code.extend_from_slice(&[
        0x8a,
        0x44,
        0x24,
        8 + source,
        0x88,
        0x44,
        0x24,
        destination,
    ]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_presence_save_context(code: &mut Vec<u8>, source: u8, context: u32) {
    code.extend_from_slice(&[0x8a, 0x44, 0x24, source, 0x88, 0x84, 0x24]);
    code.extend_from_slice(&context.to_le_bytes());
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_presence_restore_context(code: &mut Vec<u8>, destination: u8, context: u32) {
    code.extend_from_slice(&[0x8a, 0x84, 0x24]);
    code.extend_from_slice(&context.to_le_bytes());
    code.extend_from_slice(&[0x88, 0x44, 0x24, destination]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_dynamic_tag_with_presence(code: &mut Vec<u8>, slot: u8, kind: u8) {
    emit_spill(code, slot);
    emit_move(code, 0, slot);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&f64::from(kind).to_bits().to_le_bytes());
    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc8]); // movq xmm1, rax
    code.extend_from_slice(&[0x0f, 0xb6, 0x44, 0x24, 72 + slot]); // movzx eax, [rsp+72+slot]
    code.extend_from_slice(&[0xf2, 0x48, 0x0f, 0x2a, 0xd0]); // cvtsi2sd xmm2, rax
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&(tag_primitive_with_presence as *const () as u64).to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, slot, 0);
    emit_restore(code, slot);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_strict_presence_mismatch(code: &mut Vec<u8>, left: u8, right: u8) {
    emit_spill(code, left);
    code.extend_from_slice(&[0x0f, 0xb6, 0xbc, 0x24]);
    code.extend_from_slice(&(72u32 + u32::from(left)).to_le_bytes());
    code.extend_from_slice(&[0x0f, 0xb6, 0xb4, 0x24]);
    code.extend_from_slice(&(72u32 + u32::from(right)).to_le_bytes());
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&(strict_presence_mismatch as *const () as u64).to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, left, 0);
    emit_restore(code, left);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_reference_same_value_with_presence(code: &mut Vec<u8>, left: u8, right: u8) {
    emit_spill(code, left);
    emit_move(code, 0, left);
    emit_move(code, 1, right);
    code.extend_from_slice(&[0x0f, 0xb6, 0xbc, 0x24]);
    code.extend_from_slice(&(72u32 + u32::from(left)).to_le_bytes());
    code.extend_from_slice(&[0x0f, 0xb6, 0xb4, 0x24]);
    code.extend_from_slice(&(72u32 + u32::from(right)).to_le_bytes());
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&(reference_same_value_with_presence as *const () as u64).to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, left, 0);
    emit_restore(code, left);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_presence_test(code: &mut Vec<u8>, slot: u8, state: u8, condition: u8) -> usize {
    code.extend_from_slice(&[0x80, 0x7c, 0x24, slot, state]);
    emit_near_jump(code, condition)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_boolean_constant(code: &mut Vec<u8>, slot: u8, value: bool) {
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&f64::from(u8::from(value)).to_bits().to_le_bytes());
    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (slot << 3)]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_numeric_presence_materialization(code: &mut Vec<u8>, slot: u8) -> Option<()> {
    // Optional Number results use an unboxed zero placeholder. Materialize
    // undefined as NaN and null as +0 before an ordinary numeric operation.
    code.extend_from_slice(&[0x80, 0x7c, 0x24, slot, 1]);
    let undefined = emit_near_jump(code, 0x84);
    code.extend_from_slice(&[0x80, 0x7c, 0x24, slot, 2]);
    let is_null = emit_near_jump(code, 0x84);
    let done = emit_unconditional_jump(code);
    patch_near_jump(code, undefined)?;
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&0x7ff8_0000_0000_0000u64.to_le_bytes());
    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (slot << 3)]);
    let undefined_done = emit_unconditional_jump(code);
    patch_near_jump(code, is_null)?;
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&0u64.to_le_bytes());
    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (slot << 3)]);
    patch_near_jump(code, undefined_done)?;
    patch_near_jump(code, done)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_presence_epilogue(code: &mut Vec<u8>, frame_size: u32) {
    code.extend_from_slice(&[0x48, 0x81, 0xc4]);
    code.extend_from_slice(&frame_size.to_le_bytes());
    code.push(0xc3);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_presence_publish(code: &mut Vec<u8>, depth: u8, slot: u8) {
    // The temporary helper frame starts below the permanent metadata frame.
    emit_spill(code, depth);
    code.extend_from_slice(&[0x0f, 0xb6, 0x7c, 0x24, 72 + slot]); // movzx edi, byte [rsp+72+slot]
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&(publish_call_presence as *const () as u64).to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_restore(code, depth);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_presence_reset_call(code: &mut Vec<u8>, depth: u8) {
    emit_spill(code, depth);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&(reset_call_presence as *const () as u64).to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_restore(code, depth);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_presence_capture_call(code: &mut Vec<u8>, depth: u8, slot: u8) {
    emit_spill(code, depth);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&(read_call_presence as *const () as u64).to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0, 0x88, 0x44, 0x24, 72 + slot]);
    emit_restore(code, depth);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_number_boolean_with_presence(code: &mut Vec<u8>, slot: u8, negate: bool) {
    emit_spill(code, slot);
    emit_move(code, 0, slot);
    code.extend_from_slice(&[0x0f, 0xb6, 0x7c, 0x24, 72 + slot]);
    code.extend_from_slice(&[0xbe, u8::from(negate), 0, 0, 0]);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&(number_boolean_with_presence as *const () as u64).to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, slot, 0);
    emit_restore(code, slot);
    emit_presence_set(code, slot, 0);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_presence_import_argument(code: &mut Vec<u8>, slot: u8, index: u8) {
    if index >= 16 {
        emit_presence_set(code, slot, 0);
        return;
    }
    let source = 16 + index;
    code.extend_from_slice(&[0x8a, 0x44, 0x24, source, 0x88, 0x44, 0x24, slot]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn uncaught_string_throw(value: f64) -> f64 {
    CALL_ERROR.with(|error| error.set(value.to_bits() as usize as *const c_char));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn uncaught_number_throw(value: f64) -> f64 {
    let error = NUMBER_TO_STRING
        .with(Cell::get)
        .map(|format| unsafe { format(value) })
        .unwrap_or_else(|| INVALID_SYMBOL.as_ptr().cast());
    CALL_ERROR.with(|slot| slot.set(error));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn uncaught_boolean_throw(value: f64) -> f64 {
    let error = if value == 0.0 {
        FALSE_THROW.as_ptr()
    } else {
        TRUE_THROW.as_ptr()
    };
    CALL_ERROR.with(|slot| slot.set(error.cast()));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn uncaught_array_throw(value: f64, operation: u8) -> f64 {
    let error = array_format(
        operation,
        value,
        f64::from_bits(COMMA.as_ptr() as usize as u64),
    );
    if error != 0.0 {
        CALL_ERROR.with(|slot| slot.set(error.to_bits() as usize as *const c_char));
    }
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn uncaught_number_array_throw(value: f64) -> f64 {
    uncaught_array_throw(value, 0)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn uncaught_string_array_throw(value: f64) -> f64 {
    uncaught_array_throw(value, 1)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn uncaught_boolean_array_throw(value: f64) -> f64 {
    uncaught_array_throw(value, 2)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn uncaught_dictionary_throw(_: f64) -> f64 {
    CALL_ERROR.with(|slot| slot.set(OBJECT_THROW.as_ptr().cast()));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_move(code: &mut Vec<u8>, destination: u8, source: u8) {
    code.extend_from_slice(&[0x66, 0x0f, 0x28, 0xc0 | (destination << 3) | source]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_recursive_call(code: &mut Vec<u8>, base: u8, arity: u8) -> Option<()> {
    const FRAME_SIZE: u32 = 168;
    code.extend_from_slice(&[0x48, 0x81, 0xec]);
    code.extend_from_slice(&FRAME_SIZE.to_le_bytes());
    code.extend_from_slice(&[0x48, 0x89, 0xbc, 0x24, 0x80, 0, 0, 0]);
    for register in 0..base {
        code.extend_from_slice(&[
            0xf2,
            0x0f,
            0x11,
            0x44 | (register << 3),
            0x24,
            64 + register * 8,
        ]);
    }
    for index in 0..arity {
        let register = base + index;
        code.extend_from_slice(&[0xf2, 0x0f, 0x11, 0x44 | (register << 3), 0x24, index * 8]);
        // The packed recursive argument array has parallel bytes at +136.
        code.extend_from_slice(&[0x8a, 0x84, 0x24]);
        code.extend_from_slice(&(FRAME_SIZE as i32 + i32::from(register)).to_le_bytes());
        code.extend_from_slice(&[0x88, 0x84, 0x24]);
        code.extend_from_slice(&(136 + i32::from(index)).to_le_bytes());
    }
    // Preserve an enclosing recursive invocation's input metadata scope.
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&(get_recur_input_pointer as *const () as u64).to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0, 0x48, 0x89, 0x84, 0x24, 144, 0, 0, 0]);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&(get_recur_input_length as *const () as u64).to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0, 0x48, 0x89, 0x84, 0x24, 152, 0, 0, 0]);
    code.extend_from_slice(&[0x48, 0x8d, 0xbc, 0x24, 136, 0, 0, 0]);
    code.extend_from_slice(&[0x48, 0xc7, 0xc6]);
    code.extend_from_slice(&i32::from(arity).to_le_bytes());
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&(set_recur_input_presence as *const () as u64).to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    code.extend_from_slice(&[0x48, 0x89, 0xe7]);
    code.push(0xe8);
    let next = code.len().checked_add(4)?;
    code.extend_from_slice(&i32::try_from(next).ok()?.wrapping_neg().to_le_bytes());
    // Keep the returned XMM0 value while restoring the outer metadata scope.
    code.extend_from_slice(&[0xf2, 0x0f, 0x11, 0x44, 0x24, 64 + base * 8]);
    code.extend_from_slice(&[0x48, 0x8b, 0xbc, 0x24, 144, 0, 0, 0]);
    code.extend_from_slice(&[0x48, 0x8b, 0xb4, 0x24, 152, 0, 0, 0]);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&(set_recur_input_presence as *const () as u64).to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    code.extend_from_slice(&[0xf2, 0x0f, 0x10, 0x44, 0x24, 64 + base * 8]);
    emit_move(code, base, 0);
    for register in 0..base {
        code.extend_from_slice(&[
            0xf2,
            0x0f,
            0x10,
            0x44 | (register << 3),
            0x24,
            64 + register * 8,
        ]);
    }
    code.extend_from_slice(&[0x48, 0x8b, 0xbc, 0x24, 0x80, 0, 0, 0]);
    code.extend_from_slice(&[0x48, 0x81, 0xc4]);
    code.extend_from_slice(&FRAME_SIZE.to_le_bytes());
    Some(())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_near_jump(code: &mut Vec<u8>, condition: u8) -> usize {
    code.extend_from_slice(&[0x0f, condition]);
    let displacement = code.len();
    code.extend_from_slice(&0i32.to_le_bytes());
    displacement
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_unconditional_jump(code: &mut Vec<u8>) -> usize {
    code.push(0xe9);
    let displacement = code.len();
    code.extend_from_slice(&0i32.to_le_bytes());
    displacement
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_backward_jump(code: &mut Vec<u8>, target: usize) -> Option<()> {
    code.push(0xe9);
    let next = code.len().checked_add(4)?;
    let distance = i32::try_from(target as isize - next as isize).ok()?;
    code.extend_from_slice(&distance.to_le_bytes());
    Some(())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn patch_near_jump(code: &mut [u8], displacement: usize) -> Option<()> {
    patch_jump_to(code, displacement, code.len())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn patch_jump_to(code: &mut [u8], displacement: usize, target: usize) -> Option<()> {
    let next = displacement.checked_add(4)?;
    let distance = i32::try_from(target as isize - next as isize).ok()?;
    code[displacement..displacement + 4].copy_from_slice(&distance.to_le_bytes());
    Some(())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_bit_operation(code: &mut Vec<u8>, value: u8, operation: u8) {
    code.extend_from_slice(&[
        0x66,
        0x48,
        0x0f,
        0x7e,
        0xc0 | (value << 3),
        0x48,
        0x0f,
        0xba,
        operation,
        0x3f,
        0x66,
        0x48,
        0x0f,
        0x6e,
        0xc0 | (value << 3),
    ]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_spill(code: &mut Vec<u8>, registers: u8) {
    code.extend_from_slice(&[0x48, 0x83, 0xec, 0x48]);
    // The generated function keeps its argument-array base in caller-saved
    // RDI, so every native helper call must preserve it as well as live XMMs.
    code.extend_from_slice(&[0x48, 0x89, 0x7c, 0x24, 0x40]);
    for register in 0..registers {
        code.extend_from_slice(&[0xf2, 0x0f, 0x11, 0x44 | (register << 3), 0x24, register * 8]);
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_restore(code: &mut Vec<u8>, registers: u8) {
    for register in 0..registers {
        code.extend_from_slice(&[0xf2, 0x0f, 0x10, 0x44 | (register << 3), 0x24, register * 8]);
    }
    code.extend_from_slice(&[0x48, 0x8b, 0x7c, 0x24, 0x40]);
    code.extend_from_slice(&[0x48, 0x83, 0xc4, 0x48]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_unary_call(code: &mut Vec<u8>, function: u64, value: u8) {
    emit_spill(code, value);
    emit_move(code, 0, value);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&function.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, value, 0);
    emit_restore(code, value);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_unary_presence_call(code: &mut Vec<u8>, function: u64, value: u8) {
    emit_spill(code, value);
    emit_move(code, 0, value);
    code.extend_from_slice(&[0x0f, 0xb6, 0x44, 0x24, 72 + value]);
    code.extend_from_slice(&[0xf2, 0x48, 0x0f, 0x2a, 0xc8]); // cvtsi2sd xmm1, rax
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&function.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, value, 0);
    emit_restore(code, value);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_binary_call(code: &mut Vec<u8>, function: u64, left: u8) {
    emit_spill(code, left);
    emit_move(code, 0, left);
    emit_move(code, 1, left + 1);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&function.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, left, 0);
    emit_restore(code, left);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_binary_call_reversed(code: &mut Vec<u8>, function: u64, left: u8) {
    emit_spill(code, left);
    // Slots below `left` hold live registers; the next slot is free.
    code.extend_from_slice(&[0xf2, 0x0f, 0x11, 0x44 | (left << 3), 0x24, left * 8]);
    emit_move(code, 0, left + 1);
    code.extend_from_slice(&[0xf2, 0x0f, 0x10, 0x4c, 0x24, left * 8]);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&function.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, left, 0);
    emit_restore(code, left);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_ternary_call(code: &mut Vec<u8>, function: u64, left: u8) {
    emit_spill(code, left);
    emit_move(code, 0, left);
    emit_move(code, 1, left + 1);
    emit_move(code, 2, left + 2);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&function.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, left, 0);
    emit_restore(code, left);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_quaternary_call(code: &mut Vec<u8>, function: u64, left: u8) {
    emit_spill(code, left);
    emit_move(code, 0, left);
    emit_move(code, 1, left + 1);
    emit_move(code, 2, left + 2);
    emit_move(code, 3, left + 3);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&function.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, left, 0);
    emit_restore(code, left);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_compare(code: &mut Vec<u8>, left: u8, right: u8, operation: CompareOp) {
    code.extend_from_slice(&[0x66, 0x0f, 0x2e, 0xc0 | (left << 3) | right]);
    let condition = match operation {
        CompareOp::Less => 0x92,
        CompareOp::LessEqual => 0x96,
        CompareOp::Greater => 0x97,
        CompareOp::GreaterEqual => 0x93,
        CompareOp::Equal => 0x94,
        CompareOp::NotEqual => 0x95,
    };
    code.extend_from_slice(&[0x0f, condition, 0xc0]);
    match operation {
        CompareOp::Less | CompareOp::LessEqual | CompareOp::Equal => {
            code.extend_from_slice(&[0x0f, 0x9b, 0xc2, 0x20, 0xd0]);
        }
        CompareOp::NotEqual => {
            code.extend_from_slice(&[0x0f, 0x9a, 0xc2, 0x08, 0xd0]);
        }
        CompareOp::Greater | CompareOp::GreaterEqual => {}
    }
    code.extend_from_slice(&[0x0f, 0xb6, 0xc0, 0xf2, 0x0f, 0x2a, 0xc0 | (left << 3)]);
}

struct Code {
    memory: *mut libc::c_void,
    mapped_size: usize,
    globals: *const JitGlobals,
}

unsafe impl Send for Code {}
unsafe impl Sync for Code {}

impl Drop for Code {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.memory, self.mapped_size) };
    }
}

fn page_size() -> usize {
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if size > 0 {
        size as usize
    } else {
        4096
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn mapped_code_size(bytes_len: usize) -> Option<usize> {
    let page = page_size();
    bytes_len.max(1).checked_add(page - 1)?.checked_div(page)?.checked_mul(page)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn map_executable_code(bytes: &[u8]) -> Result<(*mut libc::c_void, usize), *const c_char> {
    let size = mapped_code_size(bytes.len()).ok_or_else(|| ALLOCATION_FAILED.as_ptr().cast())?;
    let memory = unsafe {
        libc::mmap(
            ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if memory == libc::MAP_FAILED {
        return Err(ALLOCATION_FAILED.as_ptr().cast());
    }
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), memory.cast(), bytes.len());
        if libc::mprotect(memory, size, libc::PROT_READ | libc::PROT_EXEC) != 0 {
            libc::munmap(memory, size);
            return Err(ALLOCATION_FAILED.as_ptr().cast());
        }
    }
    Ok((memory, size))
}

#[cfg(all(test, target_arch = "x86_64", target_family = "unix"))]
mod executable_mapping_tests {
    use super::*;

    // Unrun regression: the byte just past one page must be inside the
    // mapped, executable span, and Code's destructor receives that span.
    #[test]
    fn generated_code_can_cross_a_page_boundary() {
        let page = page_size();
        let bytes = vec![0xc3; page + 1];
        let (memory, mapped_size) = map_executable_code(&bytes).unwrap();
        assert_eq!(mapped_size, page * 2);
        assert_eq!(unsafe { *memory.cast::<u8>().add(page) }, 0xc3);
        drop(Code {
            memory,
            mapped_size,
            globals: ptr::null(),
        });
    }

    #[test]
    fn mapping_size_overflow_fails_before_allocation() {
        assert!(mapped_code_size(usize::MAX).is_none());
    }
}

fn cache() -> &'static Mutex<HashMap<String, Code>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Code>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn globals_cache() -> &'static Mutex<HashMap<String, Box<JitGlobals>>> {
    static GLOBALS: OnceLock<Mutex<HashMap<String, Box<JitGlobals>>>> = OnceLock::new();
    GLOBALS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn global_namespace(symbol: &str) -> &str {
    if let Some((qualified, _)) = symbol.rsplit_once("::") {
        return qualified
            .rsplit_once(':')
            .map_or(qualified, |(_, module)| module);
    }
    let label = symbol.rsplit_once(':').map_or(symbol, |(_, label)| label);
    label
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn compile(
    symbol: &str,
    program: &NumericProgram,
) -> Result<(*mut libc::c_void, *const JitGlobals), *const c_char> {
    let mut cache = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(code) = cache.get(symbol) {
        return Ok((code.memory, code.globals));
    }
    let bytes = program
        .machine_code()
        .ok_or_else(|| INVALID_SYMBOL.as_ptr().cast())?;
    let (memory, mapped_size) = map_executable_code(&bytes)?;
    let globals_ptr = {
        let mut globals = globals_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        globals
            .entry(global_namespace(symbol).to_owned())
            .or_insert_with(|| {
                Box::new(JitGlobals {
                    slots: std::array::from_fn(|_| OnceLock::new()),
                    callable_entries: Mutex::new(HashMap::new()),
                })
            })
            .as_ref() as *const JitGlobals
    };
    cache.insert(
        symbol.to_owned(),
        Code {
            memory,
            mapped_size,
            globals: globals_ptr,
        },
    );
    Ok((memory, globals_ptr))
}

#[cfg(not(all(target_arch = "x86_64", target_family = "unix")))]
fn compile(
    _symbol: &str,
    _program: &NumericProgram,
) -> Result<(*mut libc::c_void, *const JitGlobals), *const c_char> {
    Err(UNSUPPORTED_TARGET.as_ptr().cast())
}

/// Compiles a validated numeric expression on first use and executes it.
///
/// # Safety
///
/// `symbol` must point to a live NUL-terminated string for this call. When
/// `arg_count` is nonzero, `args` must reference at least that many `f64`s.
/// `arena_alloc`, when supplied for a string-returning program, must return a
/// writable allocation of the requested size and alignment. `number_to_string`
/// must return an arena-backed NUL-terminated string when numeric coercion is
/// used. String parser callbacks must accept live NUL-terminated strings, and
/// zero-argument number callbacks, when supplied, must be safe to call for the
/// duration of this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_jit_call_f64(
    symbol: *const c_char,
    args: *const f64,
    arg_count: usize,
    arena_alloc: Option<ArenaAlloc>,
    number_to_string: Option<NumberToString>,
    string_to_number: Option<StringToNumber>,
    parse_float: Option<ParseFloat>,
    parse_int: Option<ParseInt>,
    number_format: Option<NumberFormat>,
    array_search: Option<ArraySearch>,
    array_format: Option<ArrayFormat>,
    string_normalize: Option<StringNormalize>,
    string_split: Option<StringSplit>,
    array_slice: Option<ArraySlice>,
    array_concat: Option<ArrayConcat>,
    array_append: Option<ArrayAppend>,
    array_to_reversed: Option<ArrayToReversed>,
    array_to_sorted: Option<ArrayToSorted>,
    array_reverse: Option<ArrayReverse>,
    array_sort: Option<ArraySort>,
    array_fill: Option<ArrayFill>,
    array_copy_within: Option<ArrayCopyWithin>,
    array_push: Option<ArrayPush>,
    array_unshift: Option<ArrayPush>,
    array_remove: Option<ArrayRemove>,
    array_splice: Option<ArraySplice>,
    array_set: Option<ArraySet>,
    array_with: Option<ArrayWith>,
    math_random: Option<NumberSource>,
    date_now: Option<NumberSource>,
    performance_now: Option<NumberSource>,
    process_pid: Option<NumberSource>,
    process_ppid: Option<NumberSource>,
    string_to_array: Option<StringToArray>,
    string_from_char_code: Option<NumberToString>,
    string_from_code_point: Option<NumberToString>,
    dictionary_get: Option<DictionaryGet>,
    dictionary_mutate: Option<DictionaryMutate>,
    dictionary_query: Option<DictionaryQuery>,
    dynamic_object_query: Option<DynamicObjectQuery>,
    array_index_present: Option<ArrayIndexPresent>,
    array_presence_slice: Option<ArrayPresenceSlice>,
) -> ThawJitResult {
    let Some(symbol) = (!symbol.is_null())
        .then(|| CStr::from_ptr(symbol).to_str().ok())
        .flatten()
    else {
        return ThawJitResult {
            value: 0.0,
            error: INVALID_SYMBOL.as_ptr().cast(),
        };
    };
    let Some(program) = NumericProgram::parse(symbol) else {
        return ThawJitResult {
            value: 0.0,
            error: INVALID_SYMBOL.as_ptr().cast(),
        };
    };
    if program.required_args() > arg_count || (arg_count != 0 && args.is_null()) {
        return ThawJitResult {
            value: 0.0,
            error: INVALID_SYMBOL.as_ptr().cast(),
        };
    }
    let (code, globals) = match compile(symbol, &program) {
        Ok(compiled) => compiled,
        Err(error) => return ThawJitResult { value: 0.0, error },
    };
    let function = std::mem::transmute::<*mut libc::c_void, extern "C" fn(*const f64) -> f64>(code);
    let previous_allocator = ARENA_ALLOC.with(|allocator| allocator.replace(arena_alloc));
    let previous_formatter = NUMBER_TO_STRING.with(|formatter| formatter.replace(number_to_string));
    let previous_parser = STRING_TO_NUMBER.with(|parser| parser.replace(string_to_number));
    let previous_parse_float = PARSE_FLOAT.with(|parser| parser.replace(parse_float));
    let previous_parse_int = PARSE_INT.with(|parser| parser.replace(parse_int));
    let previous_number_format = NUMBER_FORMAT.with(|format| format.replace(number_format));
    let previous_array_search = ARRAY_SEARCH.with(|search| search.replace(array_search));
    let previous_array_index_present = ARRAY_INDEX_PRESENT.with(|query| query.replace(array_index_present));
    let previous_array_format = ARRAY_FORMAT.with(|format| format.replace(array_format));
    let previous_string_normalize =
        STRING_NORMALIZE.with(|normalize| normalize.replace(string_normalize));
    let previous_string_split = STRING_SPLIT.with(|split| split.replace(string_split));
    let previous_string_to_array = STRING_TO_ARRAY.with(|convert| convert.replace(string_to_array));
    let previous_string_from_char_code =
        STRING_FROM_CHAR_CODE.with(|convert| convert.replace(string_from_char_code));
    let previous_string_from_code_point =
        STRING_FROM_CODE_POINT.with(|convert| convert.replace(string_from_code_point));
    let previous_array_slice = ARRAY_SLICE.with(|slice| slice.replace(array_slice));
    let previous_array_presence_slice = ARRAY_PRESENCE_SLICE.with(|slice| slice.replace(array_presence_slice));
    let previous_array_concat = ARRAY_CONCAT.with(|concat| concat.replace(array_concat));
    let previous_array_append = ARRAY_APPEND.with(|append| append.replace(array_append));
    let previous_array_to_reversed =
        ARRAY_TO_REVERSED.with(|reverse| reverse.replace(array_to_reversed));
    let previous_array_to_sorted = ARRAY_TO_SORTED.with(|sort| sort.replace(array_to_sorted));
    let previous_array_reverse = ARRAY_REVERSE.with(|reverse| reverse.replace(array_reverse));
    let previous_array_sort = ARRAY_SORT.with(|sort| sort.replace(array_sort));
    let previous_array_fill = ARRAY_FILL.with(|fill| fill.replace(array_fill));
    let previous_array_copy_within = ARRAY_COPY_WITHIN.with(|copy| copy.replace(array_copy_within));
    let previous_array_push = ARRAY_PUSH.with(|push| push.replace(array_push));
    let previous_array_unshift = ARRAY_UNSHIFT.with(|unshift| unshift.replace(array_unshift));
    let previous_array_remove = ARRAY_REMOVE.with(|remove| remove.replace(array_remove));
    let previous_array_splice = ARRAY_SPLICE.with(|splice| splice.replace(array_splice));
    let previous_array_set = ARRAY_SET.with(|set| set.replace(array_set));
    let previous_array_with = ARRAY_WITH.with(|replace| replace.replace(array_with));
    let previous_math_random = MATH_RANDOM.with(|random| random.replace(math_random));
    let previous_date_now = DATE_NOW.with(|now| now.replace(date_now));
    let previous_performance_now = PERFORMANCE_NOW.with(|now| now.replace(performance_now));
    let previous_process_pid = PROCESS_PID.with(|pid| pid.replace(process_pid));
    let previous_process_ppid = PROCESS_PPID.with(|ppid| ppid.replace(process_ppid));
    let previous_dictionary_get = DICTIONARY_GET.with(|get| get.replace(dictionary_get));
    let previous_dictionary_mutate =
        DICTIONARY_MUTATE.with(|mutate| mutate.replace(dictionary_mutate));
    let previous_dictionary_query = DICTIONARY_QUERY.with(|query| query.replace(dictionary_query));
    let previous_dynamic_object_query =
        DYNAMIC_OBJECT_QUERY.with(|query| query.replace(dynamic_object_query));
    let previous_error = CALL_ERROR.with(|error| error.replace(ptr::null()));
    let previous_present = CALL_PRESENT.with(|present| present.replace(true));
    let previous_absence = CALL_ABSENCE.with(|absence| absence.replace(1));
    let previous_globals = JIT_GLOBALS.with(|slot| slot.replace(globals));
    let previous_dynamic_values =
        DYNAMIC_VALUES.with(|values| std::mem::take(&mut *values.borrow_mut()));
    let value = function(args);
    let error = CALL_ERROR.with(|error| error.replace(previous_error));
    let present = CALL_PRESENT.with(|state| state.replace(previous_present));
    let absence = CALL_ABSENCE.with(|state| state.replace(previous_absence));
    JIT_GLOBALS.with(|slot| slot.set(previous_globals));
    DYNAMIC_VALUES.with(|values| *values.borrow_mut() = previous_dynamic_values);
    ARENA_ALLOC.with(|allocator| allocator.set(previous_allocator));
    NUMBER_TO_STRING.with(|formatter| formatter.set(previous_formatter));
    STRING_TO_NUMBER.with(|parser| parser.set(previous_parser));
    PARSE_FLOAT.with(|parser| parser.set(previous_parse_float));
    PARSE_INT.with(|parser| parser.set(previous_parse_int));
    NUMBER_FORMAT.with(|format| format.set(previous_number_format));
    ARRAY_SEARCH.with(|search| search.set(previous_array_search));
    ARRAY_INDEX_PRESENT.with(|query| query.set(previous_array_index_present));
    ARRAY_FORMAT.with(|format| format.set(previous_array_format));
    STRING_NORMALIZE.with(|normalize| normalize.set(previous_string_normalize));
    STRING_SPLIT.with(|split| split.set(previous_string_split));
    STRING_TO_ARRAY.with(|convert| convert.set(previous_string_to_array));
    STRING_FROM_CHAR_CODE.with(|convert| convert.set(previous_string_from_char_code));
    STRING_FROM_CODE_POINT.with(|convert| convert.set(previous_string_from_code_point));
    ARRAY_SLICE.with(|slice| slice.set(previous_array_slice));
    ARRAY_PRESENCE_SLICE.with(|slice| slice.set(previous_array_presence_slice));
    ARRAY_CONCAT.with(|concat| concat.set(previous_array_concat));
    ARRAY_APPEND.with(|append| append.set(previous_array_append));
    ARRAY_TO_REVERSED.with(|reverse| reverse.set(previous_array_to_reversed));
    ARRAY_TO_SORTED.with(|sort| sort.set(previous_array_to_sorted));
    ARRAY_REVERSE.with(|reverse| reverse.set(previous_array_reverse));
    ARRAY_SORT.with(|sort| sort.set(previous_array_sort));
    ARRAY_FILL.with(|fill| fill.set(previous_array_fill));
    ARRAY_COPY_WITHIN.with(|copy| copy.set(previous_array_copy_within));
    ARRAY_PUSH.with(|push| push.set(previous_array_push));
    ARRAY_UNSHIFT.with(|unshift| unshift.set(previous_array_unshift));
    ARRAY_REMOVE.with(|remove| remove.set(previous_array_remove));
    ARRAY_SPLICE.with(|splice| splice.set(previous_array_splice));
    ARRAY_SET.with(|set| set.set(previous_array_set));
    ARRAY_WITH.with(|replace| replace.set(previous_array_with));
    MATH_RANDOM.with(|random| random.set(previous_math_random));
    DATE_NOW.with(|now| now.set(previous_date_now));
    PERFORMANCE_NOW.with(|now| now.set(previous_performance_now));
    PROCESS_PID.with(|pid| pid.set(previous_process_pid));
    PROCESS_PPID.with(|ppid| ppid.set(previous_process_ppid));
    DICTIONARY_GET.with(|get| get.set(previous_dictionary_get));
    DICTIONARY_MUTATE.with(|mutate| mutate.set(previous_dictionary_mutate));
    DICTIONARY_QUERY.with(|query| query.set(previous_dictionary_query));
    DYNAMIC_OBJECT_QUERY.with(|query| query.set(previous_dynamic_object_query));
    if !error.is_null() {
        return ThawJitResult { value: 0.0, error };
    }
    if !present {
        return ThawJitResult {
            value: 0.0,
            error: if absence == 2 {
                NULL_STATUS
            } else {
                ABSENT_STATUS
            },
        };
    }
    ThawJitResult {
        value,
        error: ptr::null(),
    }
}
