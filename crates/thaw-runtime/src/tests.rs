use super::*;
use std::net::TcpListener;
use std::process::Command;

use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

// Lambda invocations update a process-wide variable. Keep every direct
// invocation fixture serial and restore the parent test process's value.
static LAMBDA_TRACE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct LambdaTraceEnvGuard {
    previous: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for LambdaTraceEnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var("_X_AMZN_TRACE_ID", value),
            None => std::env::remove_var("_X_AMZN_TRACE_ID"),
        }
    }
}

fn guard_lambda_trace_env() -> LambdaTraceEnvGuard {
    let lock = LAMBDA_TRACE_ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    LambdaTraceEnvGuard { previous: std::env::var_os("_X_AMZN_TRACE_ID"), _lock: lock }
}

#[test]
fn bytes_encoding_preserves_native_units_and_stops_at_invalid_hex() {
    let face = thaw_arena::arena_string("😀".as_bytes());
    let latin1 = c"latin1".as_ptr();
    assert_eq!(unsafe { read_byte_array(thaw_bytes_from_string(face, latin1)) }.unwrap(), [0x3d, 0x00]);
    assert_eq!(unsafe { thaw_bytes_byte_length(face, latin1) }, 2.0);
    let lone = thaw_string_from_char_code(0xD83D as f64);
    assert_eq!(unsafe { read_byte_array(thaw_bytes_from_string(lone, latin1)) }.unwrap(), [0x3d]);
    let nul = thaw_arena::arena_string(b"a\0b");
    assert_eq!(unsafe { read_byte_array(thaw_bytes_from_string(nul, c"utf8".as_ptr())) }.unwrap(), b"a\0b");
    assert_eq!(unsafe { thaw_bytes_byte_length(nul, c"utf8".as_ptr()) }, 3.0);
    assert_eq!(unsafe { thaw_bytes_byte_length(c"1ag123".as_ptr(), c"hex".as_ptr()) }, 3.0);
    assert_eq!(unsafe { thaw_bytes_byte_length(c"Zg==".as_ptr(), c"base64".as_ptr()) }, 1.0);
    assert_eq!(unsafe { read_byte_array(thaw_bytes_from_string(c"1a7".as_ptr(), c"hex".as_ptr())) }.unwrap(), [0x1a]);
    assert_eq!(unsafe { read_byte_array(thaw_bytes_from_string(c"1ag123".as_ptr(), c"hex".as_ptr())) }.unwrap(), [0x1a]);
    let output = unsafe { write_byte_array(&[0, 0, 0]) };
    assert_eq!(unsafe { thaw_bytes_set_from_string(output, c"1ag123".as_ptr(), c"hex".as_ptr()) }, 1.0);
    assert_eq!(unsafe { read_byte_array(output) }.unwrap(), [0x1a, 0, 0]);
    let ascii = unsafe { thaw_bytes_to_string(unsafe { write_byte_array(&[0xc1]) }, c"ascii".as_ptr()) };
    assert_eq!(unsafe { CStr::from_ptr(ascii) }.to_bytes(), b"A");
}

#[test]
fn bytes_accessors_reject_overflowed_offsets() {
    let buffer = unsafe { write_byte_array(&[1, 2, 3, 4, 5, 6, 7, 8]) };
    assert_eq!(unsafe { thaw_bytes_read(buffer, f64::MAX, 4.0, 0.0, 1.0) }, 0.0);
    assert_eq!(unsafe { thaw_bytes_read_i64(buffer, f64::MAX, 1.0) }, 0);
    unsafe { thaw_bytes_write(buffer, f64::MAX, 42.0, 4.0, 0.0, 1.0) };
    unsafe { thaw_bytes_write_i64(buffer, f64::MAX, 42, 1.0) };
    assert_eq!(unsafe { read_byte_array(buffer) }.unwrap(), [1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn bytes_forward_search_distinguishes_infinity_and_nan() {
    let haystack = unsafe { write_byte_array(&[1, 2, 1]) };
    let needle = unsafe { write_byte_array(&[1]) };
    assert_eq!(unsafe { thaw_bytes_index_of(haystack, needle, f64::INFINITY, 0.0) }, -1.0);
    assert_eq!(unsafe { thaw_bytes_index_of(haystack, needle, f64::NEG_INFINITY, 0.0) }, 0.0);
    assert_eq!(unsafe { thaw_bytes_index_of(haystack, needle, f64::NAN, 0.0) }, 0.0);
    assert_eq!(unsafe { thaw_bytes_index_of(haystack, needle, f64::INFINITY, 1.0) }, 2.0);
}

#[test]
fn native_string_operations_preserve_utf16_units() {
    let lone = thaw_string_from_char_code(0xD800 as f64);
    let low = thaw_string_from_char_code(0xDC00 as f64);
    let mut pair_bytes = unsafe { wtf8_bytes(lone) }.to_vec();
    pair_bytes.extend_from_slice(unsafe { wtf8_bytes(low) });
    let pair = arena_wtf8(&pair_bytes).unwrap().cast();
    assert!(unsafe { thaw_string_is_well_formed(pair) });
    assert_eq!(wtf8_decode_utf16(unsafe { wtf8_bytes(lone) }), [0xD800]);
    assert_eq!(wtf8_decode_utf16(unsafe { wtf8_bytes(unsafe { thaw_string_trim(lone) }) }), [0xD800]);
    assert_eq!(wtf8_decode_utf16(unsafe { wtf8_bytes(unsafe { thaw_string_normalize(lone, c"NFC".as_ptr()) }) }), [0xD800]);
    assert_eq!(wtf8_decode_utf16(unsafe { wtf8_bytes(unsafe { thaw_unescape(c"%uD800".as_ptr()) }) }), [0xD800]);
    assert_eq!(unsafe { CStr::from_ptr(thaw_escape(lone)) }.to_str().unwrap(), "%uD800");
    assert_eq!(unsafe { thaw_string_char_code_at(thaw_string_from_char_code(2f64.powi(63)), 0.0) }, 0.0);
    assert_eq!(wtf8_decode_utf16(unsafe { wtf8_bytes(thaw_string_from_code_point(0xD800 as f64)) }), [0xD800]);
    assert!(unsafe { thaw_encode_uri(lone) }.is_null());
    assert!(unsafe { thaw_decode_uri_component(c"%+1".as_ptr()) }.is_null());
    assert_eq!(wtf8_decode_utf16(unsafe { wtf8_bytes(unsafe { thaw_decode_uri_component(lone) }) }), [0xD800]);
    assert_eq!(unsafe { thaw_string_last_index_of(c"aba".as_ptr(), c"a".as_ptr(), f64::NAN) }, 2.0);
    let face = thaw_string_from_code_point(0x1F600 as f64);
    let replaced = unsafe { thaw_string_replace_all(face, c"".as_ptr(), c"x".as_ptr()) };
    assert_eq!(wtf8_decode_utf16(unsafe { wtf8_bytes(replaced) }), [b'x' as u16, 0xD83D, b'x' as u16, 0xDE00, b'x' as u16]);
    let symbol = unsafe { thaw_symbol_new(c"x".as_ptr()) };
    assert!(unsafe { thaw_symbol_key_for(symbol) }.is_null());
    let first = unsafe { thaw_symbol_for(lone) };
    let second = unsafe { thaw_symbol_for(thaw_string_from_char_code(0xD801 as f64)) };
    assert_ne!(unsafe { wtf8_bytes(first) }, unsafe { wtf8_bytes(second) });
    let sigma = unsafe { thaw_string_to_lower_case(c"ΟΣ".as_ptr()) };
    assert_eq!(unsafe { CStr::from_ptr(sigma) }.to_str().unwrap(), "ος");
    let turkic = unsafe { thaw_string_to_locale_lower_case(c"I\u{0307}".as_ptr(), c"tr".as_ptr()) };
    assert_eq!(unsafe { CStr::from_ptr(turkic) }.to_str().unwrap(), "i");
    let embedded_nul = thaw_arena::arena_string(b" a\0b ");
    let trimmed = unsafe { thaw_string_trim(embedded_nul) };
    assert_eq!(unsafe { wtf8_bytes(trimmed) }, b"a\0b");
}

static TLS_TEST_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static UNHANDLED_REJECTIONS: AtomicU64 = AtomicU64::new(0);

extern "C" fn record_unhandled_rejection(_: *const u8) -> u8 {
    UNHANDLED_REJECTIONS.fetch_add(1, Ordering::Relaxed);
    1
}

extern "C" fn ignore_promise_result(_: *mut u8, _: *const u8) {}

/// A settled `Promise<Array<_>>`'s (or `Promise<Tuple<_>>`'s) own resolved
/// value is an array/tuple handle - a one-word cell holding the raw
/// `[i64 len][elem...]` buffer address - matching the indirection
/// `wrap_array_handle` applies in `promises.rs` and `compile_array_wrap`
/// applies at the LLVM codegen boundary. `thaw_runtime_run_until_resolved`
/// only unwraps the generic "resolve slot" layer, so tests that assert on
/// the buffer's own bytes need this one extra dereference.
fn resolved_array_buffer(promise: *const ThawPromise) -> *const u64 {
    let handle = unsafe { *thaw_runtime_run_until_resolved(promise).cast::<*const u64>() };
    unsafe { *handle.cast::<*const u64>() }
}

#[test]
fn formats_numbers_with_javascript_string_boundaries() {
    let cases = [
        (0.0, "0"),
        (-0.0, "0"),
        (f64::NAN, "NaN"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
        (1.5, "1.5"),
        (1e20, "100000000000000000000"),
        (1e21, "1e+21"),
        (1e-6, "0.000001"),
        (1e-7, "1e-7"),
        (1.2345678901234567, "1.2345678901234567"),
    ];
    for (value, expected) in cases {
        assert_eq!(javascript_number_string(value), expected, "value={value:?}");
    }
}

#[test]
fn creates_numeric_property_keys_for_native_arrays() {
    let array = [3_i64, 0, 0, 0];
    let keys = unsafe { thaw_array_keys(array.as_ptr().cast(), std::ptr::null(), 0) };
    assert_eq!(unsafe { keys.cast::<i64>().read() }, 3);
    for index in 0..3 {
        let key = unsafe {
            keys.add(8 + index * 8)
                .cast::<*const c_char>()
                .read_unaligned()
        };
        assert_eq!(
            unsafe { CStr::from_ptr(key) }.to_string_lossy(),
            index.to_string()
        );
    }
}

#[test]
fn parses_strings_with_javascript_number_grammar() {
    let cases = [
        ("", 0.0),
        ("  \n\t", 0.0),
        ("42", 42.0),
        ("+1.5", 1.5),
        (".25", 0.25),
        ("2.", 2.0),
        ("1e3", 1000.0),
        ("0xff", 255.0),
        ("\u{feff}1\u{feff}", 1.0),
        ("0x10000000000000000", 18_446_744_073_709_551_616.0),
        ("0x20000000000001", 9_007_199_254_740_992.0),
        ("0x20000000000003", 9_007_199_254_740_996.0),
        ("0o10", 8.0),
        ("0b101", 5.0),
        ("Infinity", f64::INFINITY),
        ("-Infinity", f64::NEG_INFINITY),
    ];
    for (text, expected) in cases {
        assert_eq!(javascript_string_number(text), expected, "text={text:?}");
    }
    assert!(javascript_string_number("nope").is_nan());
    assert!(javascript_string_number("1e").is_nan());
    assert!(javascript_string_number("+0x1").is_nan());
    assert!(javascript_string_number("inf").is_nan());
    assert!(javascript_string_number("-0").is_sign_negative());
}

#[test]
fn parses_float_and_integer_prefixes_like_javascript() {
    assert_eq!(javascript_parse_float("  -12.5px"), -12.5);
    assert_eq!(javascript_parse_float("1e2rest"), 100.0);
    assert_eq!(javascript_parse_float("1e+"), 1.0);
    assert_eq!(javascript_parse_float("+Infinity!"), f64::INFINITY);
    assert!(javascript_parse_float("0x10").is_sign_positive());
    assert_eq!(javascript_parse_float("0x10"), 0.0);
    assert!(javascript_parse_float("words").is_nan());

    assert_eq!(javascript_parse_int("  -0x10more", 0.0), -16.0);
    assert_eq!(javascript_parse_int("11", 2.0), 3.0);
    assert_eq!(javascript_parse_int("0x20", 16.0), 32.0);
    assert_eq!(javascript_parse_int("010", 0.0), 10.0);
    assert_eq!(javascript_parse_int("15px", 10.0), 15.0);
    assert!(javascript_parse_int("10", 1.0).is_nan());
    assert!(javascript_parse_int("xyz", 36.0).is_finite());
    assert!(javascript_parse_int("-0", 10.0).is_sign_negative());
}

#[test]
fn compares_strings_in_javascript_utf16_order() {
    let supplementary = CString::new("\u{10000}").unwrap();
    let bmp = CString::new("\u{e000}").unwrap();
    assert_eq!(
        unsafe { thaw_string_compare(supplementary.as_ptr(), bmp.as_ptr()) },
        -1
    );
    assert_eq!(
        unsafe { thaw_string_compare(bmp.as_ptr(), supplementary.as_ptr()) },
        1
    );
    assert_eq!(
        unsafe { thaw_string_compare(bmp.as_ptr(), bmp.as_ptr()) },
        0
    );
}

fn thaw_runtime_run_until_resolved(promise: *const ThawPromise) -> *const u8 {
    unsafe { super::thaw_runtime_run_until_resolved(promise) }
}

fn thaw_promise_state(promise: *const ThawPromise) -> u8 {
    unsafe { super::thaw_promise_state(promise) }
}

fn thaw_promise_subscribe(
    promise: *mut ThawPromise,
    resume: PromiseResumeFn,
    frame: *mut u8,
) -> u8 {
    unsafe { super::thaw_promise_subscribe(promise, resume, frame) }
}

extern "C" fn echo_handler(event: *const c_char) -> *const c_char {
    let event = unsafe { CStr::from_ptr(event) }
        .to_string_lossy()
        .into_owned();
    // Leaked on purpose: matches the arena/global-lifetime string model
    // compiled Thaw code uses (nothing frees heap strings yet).
    CString::new(format!("echo:{event}")).unwrap().into_raw() as *const c_char
}

static TEST_HANDLER_ERROR: &[u8] = b"handler exploded\0";
static mut TEST_PENDING_EXCEPTION: *const c_char = std::ptr::null();

extern "C" fn failing_handler(_: *const c_char) -> *const c_char {
    unsafe { TEST_PENDING_EXCEPTION = TEST_HANDLER_ERROR.as_ptr().cast() };
    std::ptr::null()
}

#[repr(C)]
struct ResumeRecord {
    calls: usize,
    result: *const u8,
}

extern "C" fn record_resume(frame: *mut u8, result: *const u8) {
    let record = unsafe { &mut *(frame as *mut ResumeRecord) };
    record.calls += 1;
    record.result = result;
}

struct ChainNumberContext {
    calls: usize,
    add: f64,
}

extern "C" fn transform_chain_number(
    context: *mut u8,
    output: *mut ThawPromise,
    _input: *mut ThawPromise,
    result: *const u8,
) {
    let context = unsafe { &mut *context.cast::<ChainNumberContext>() };
    context.calls += 1;
    let value = unsafe { *result.cast::<f64>() } + context.add;
    let slot = thaw_arena::thaw_arena_alloc(size_of::<f64>(), align_of::<f64>()).cast::<f64>();
    unsafe { slot.write(value) };
    thaw_promise_resolve(output, slot.cast());
}

struct TimedValueContext {
    timer: *mut ThawPromise,
    output: *mut ThawPromise,
    value: *const u8,
}

extern "C" fn resolve_timed_value(frame: *mut u8, _: *const u8) {
    let context = unsafe { Box::from_raw(frame.cast::<TimedValueContext>()) };
    unsafe { thaw_promise_destroy(context.timer) };
    thaw_promise_resolve(context.output, context.value);
}

fn timed_value(milliseconds: u64, value: f64) -> *mut ThawPromise {
    let timer = thaw_sleep_ms(milliseconds);
    let output = thaw_promise_new();
    let slot = thaw_arena::thaw_arena_alloc(size_of::<f64>(), align_of::<f64>()).cast::<f64>();
    unsafe { slot.write(value) };
    let context = Box::into_raw(Box::new(TimedValueContext {
        timer,
        output,
        value: slot.cast(),
    }));
    assert_eq!(
        thaw_promise_subscribe(timer, resolve_timed_value, context.cast()),
        1
    );
    output
}

#[test]
fn reports_only_unsubscribed_rejected_promises() {
    UNHANDLED_REJECTIONS.store(0, Ordering::Relaxed);
    thaw_promise_set_unhandled_reporter(Some(record_unhandled_rejection));
    let unhandled = thaw_promise_new();
    thaw_promise_reject(unhandled, c"boom".as_ptr().cast());
    assert_eq!(thaw_runtime_poll_one(), 0);
    assert_eq!(UNHANDLED_REJECTIONS.load(Ordering::Relaxed), 1);
    assert_eq!(
        thaw_promise_drain_unhandled(Some(record_unhandled_rejection)),
        0
    );
    assert_eq!(UNHANDLED_REJECTIONS.load(Ordering::Relaxed), 1);
    assert_eq!(
        thaw_promise_drain_unhandled(Some(record_unhandled_rejection)),
        0
    );
    assert_eq!(UNHANDLED_REJECTIONS.load(Ordering::Relaxed), 1);
    let handled = thaw_promise_new();
    assert_eq!(
        thaw_promise_subscribe(handled, ignore_promise_result, std::ptr::null_mut()),
        1
    );
    thaw_promise_reject(handled, c"handled".as_ptr().cast());
    thaw_runtime_run_until_idle();
    let polled = thaw_promise_new();
    assert_eq!(unsafe { thaw_promise_mark_handled(polled) }, 1);
    thaw_promise_reject(polled, c"polled".as_ptr().cast());
    thaw_runtime_run_until_idle();
    assert_eq!(
        thaw_promise_drain_unhandled(Some(record_unhandled_rejection)),
        0
    );
    assert_eq!(UNHANDLED_REJECTIONS.load(Ordering::Relaxed), 1);
    unsafe { thaw_promise_destroy(unhandled) };
    unsafe { thaw_promise_destroy(handled) };
    unsafe { thaw_promise_destroy(polled) };
    thaw_promise_set_unhandled_reporter(None);
}

#[test]
fn reports_a_rejection_handled_after_the_unhandled_checkpoint() {
    static HANDLED: AtomicU64 = AtomicU64::new(0);
    extern "C" fn record_handled() {
        HANDLED.fetch_add(1, Ordering::Relaxed);
    }

    HANDLED.store(0, Ordering::Relaxed);
    thaw_promise_set_unhandled_reporter(Some(record_unhandled_rejection));
    thaw_promise_set_rejection_handled_reporter(Some(record_handled));
    let promise = thaw_promise_new();
    thaw_promise_reject(promise, c"late".as_ptr().cast());
    assert_eq!(thaw_runtime_poll_one(), 0);
    assert_eq!(
        thaw_promise_subscribe(promise, ignore_promise_result, std::ptr::null_mut()),
        1
    );
    assert_eq!(thaw_runtime_poll_one(), 1);
    assert_eq!(thaw_runtime_poll_one(), 0);
    assert_eq!(HANDLED.load(Ordering::Relaxed), 1);
    unsafe { thaw_promise_destroy(promise) };
    thaw_promise_set_unhandled_reporter(None);
    thaw_promise_set_rejection_handled_reporter(None);
}

fn local_tls_configs() -> (Arc<ClientConfig>, Arc<ServerConfig>) {
    let sequence = TLS_TEST_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("thaw-tls-test-{}-{sequence}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let key_pem = dir.join("key.pem");
    let cert_pem = dir.join("cert.pem");
    let key_der = dir.join("key.der");
    let cert_der = dir.join("cert.der");
    assert!(Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-addext",
            "keyUsage=critical,digitalSignature,keyEncipherment",
            "-addext",
            "extendedKeyUsage=serverAuth",
            "-keyout",
        ])
        .arg(&key_pem)
        .arg("-out")
        .arg(&cert_pem)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("openssl")
        .args(["x509", "-in"])
        .arg(&cert_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&cert_der)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("openssl")
        .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
        .arg(&key_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&key_der)
        .status()
        .unwrap()
        .success());

    let certificate = CertificateDer::from(std::fs::read(&cert_der).unwrap());
    let private_key = PrivatePkcs8KeyDer::from(std::fs::read(&key_der).unwrap()).into();
    let mut roots = RootCertStore::empty();
    roots.add(certificate.clone()).unwrap();
    let client = Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let server = Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], private_key)
            .unwrap(),
    );
    let _ = std::fs::remove_dir_all(dir);
    (client, server)
}

#[test]
fn promise_chain_selects_callbacks_for_then_and_catch() {
    let input = thaw_promise_new();
    let mut then_context = ChainNumberContext { calls: 0, add: 2.0 };
    let chained = unsafe {
        thaw_promise_chain(
            input,
            transform_chain_number,
            (&mut then_context as *mut ChainNumberContext).cast(),
            0,
        )
    };
    let value = 40.0f64;
    thaw_promise_resolve(input, (&value as *const f64).cast());
    let result = thaw_runtime_run_until_resolved(chained);
    assert_eq!(unsafe { *result.cast::<f64>() }, 42.0);
    assert_eq!(then_context.calls, 1);
    unsafe { thaw_promise_destroy(chained) };

    let input = thaw_promise_new();
    let mut catch_context = ChainNumberContext { calls: 0, add: 1.0 };
    let chained = unsafe {
        thaw_promise_chain(
            input,
            transform_chain_number,
            (&mut catch_context as *mut ChainNumberContext).cast(),
            1,
        )
    };
    let value = 7.0f64;
    thaw_promise_resolve(input, (&value as *const f64).cast());
    let result = thaw_runtime_run_until_resolved(chained);
    assert_eq!(unsafe { *result.cast::<f64>() }, 7.0);
    assert_eq!(catch_context.calls, 0);
    unsafe { thaw_promise_destroy(chained) };

    let cycle = thaw_promise_new();
    assert_eq!(unsafe { thaw_promise_adopt(cycle, cycle) }, 0);
    let error = thaw_runtime_run_until_resolved(cycle);
    assert_eq!(thaw_promise_state(cycle), 2);
    assert_eq!(
        unsafe { CStr::from_ptr(error.cast()) }.to_string_lossy(),
        "Chaining cycle detected for promise"
    );
    unsafe { thaw_promise_destroy(cycle) };
}

#[test]
fn promise_resumes_subscribers_in_registration_order() {
    let promise = thaw_promise_new();
    assert_eq!(thaw_promise_state(promise), 0);

    let mut first = ResumeRecord {
        calls: 0,
        result: std::ptr::null(),
    };
    let mut second = ResumeRecord {
        calls: 0,
        result: std::ptr::null(),
    };
    assert_eq!(
        thaw_promise_subscribe(
            promise,
            record_resume,
            (&mut first as *mut ResumeRecord).cast()
        ),
        1
    );
    assert_eq!(
        thaw_promise_subscribe(
            promise,
            record_resume,
            (&mut second as *mut ResumeRecord).cast()
        ),
        1
    );

    let result = 42u8;
    assert_eq!(thaw_promise_resolve(promise, &result), 1);
    assert_eq!(thaw_promise_state(promise), 1);
    assert_eq!(first.calls, 0);
    assert_eq!(second.calls, 0);
    assert_eq!(thaw_runtime_run_until_idle(), 2);
    assert_eq!(first.calls, 1);
    assert_eq!(second.calls, 1);
    assert_eq!(first.result, &result);
    assert_eq!(second.result, &result);
    assert_eq!(thaw_promise_resolve(promise, &result), 0);

    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn subscribing_after_resolution_queues_resume_immediately() {
    let promise = thaw_promise_new();
    let result = 7u8;
    assert_eq!(thaw_promise_resolve(promise, &result), 1);

    let mut record = ResumeRecord {
        calls: 0,
        result: std::ptr::null(),
    };
    assert_eq!(
        thaw_promise_subscribe(
            promise,
            record_resume,
            (&mut record as *mut ResumeRecord).cast()
        ),
        1
    );
    assert_eq!(record.calls, 0);
    assert_eq!(thaw_runtime_poll_one(), 1);
    assert_eq!(record.calls, 1);
    assert_eq!(record.result, &result);
    assert_eq!(thaw_runtime_poll_one(), 0);

    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn promise_abi_rejects_invalid_handles() {
    assert_eq!(thaw_promise_state(std::ptr::null()), u8::MAX);
    assert_eq!(
        thaw_promise_subscribe(std::ptr::null_mut(), record_resume, std::ptr::null_mut()),
        0
    );
    assert_eq!(
        thaw_promise_resolve(std::ptr::null_mut(), std::ptr::null()),
        0
    );
    assert_eq!(
        thaw_promise_reject(std::ptr::null_mut(), std::ptr::null()),
        0
    );
    unsafe { thaw_promise_destroy(std::ptr::null_mut()) };
}

#[test]
fn rejected_promise_queues_subscribers_and_settles_once() {
    let promise = thaw_promise_new();
    let mut record = ResumeRecord {
        calls: 0,
        result: std::ptr::null(),
    };
    assert_eq!(
        thaw_promise_subscribe(
            promise,
            record_resume,
            (&mut record as *mut ResumeRecord).cast()
        ),
        1
    );
    let error = 99u8;
    assert_eq!(thaw_promise_reject(promise, &error), 1);
    assert_eq!(thaw_promise_state(promise), 2);
    assert_eq!(thaw_promise_resolve(promise, &error), 0);
    assert_eq!(record.calls, 0);
    assert_eq!(thaw_runtime_run_until_idle(), 1);
    assert_eq!(record.calls, 1);
    assert_eq!(record.result, &error);
    assert_eq!(thaw_runtime_run_until_resolved(promise), &error);
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn rejected_promise_preserves_typed_exception_metadata() {
    let promise = thaw_promise_new();
    let error = 1u8;
    let object = 2u8;
    assert_eq!(
        unsafe { thaw_promise_reject_typed(promise, &error, 1, 42.5, 43, true, &object) },
        1
    );
    unsafe {
        assert_eq!(thaw_promise_exception_tag(promise), 1);
        assert_eq!(thaw_promise_exception_f64(promise), 42.5);
        assert_eq!(thaw_promise_exception_i64(promise), 43);
        assert!(thaw_promise_exception_bool(promise));
        assert_eq!(thaw_promise_exception_object(promise), &object);
        thaw_promise_destroy(promise);
    }
}

#[test]
fn detached_rejection_report_owns_text_and_never_reads_opaque_results() {
    let mut pending = std::ptr::null();
    let opaque = thaw_promise_new();
    assert_eq!(unsafe { thaw_promise_detach_for_report(opaque, &mut pending) }, 1);
    assert_eq!(thaw_promise_reject(opaque, 1usize as *const u8), 1);
    thaw_runtime_run_until_idle();
    assert_eq!(unsafe { CStr::from_ptr(pending.cast()) }.to_bytes(), b"Unhandled opaque Promise rejection");

    let later = thaw_promise_new();
    assert_eq!(unsafe { thaw_promise_detach_for_report(later, &mut pending) }, 1);
    assert_eq!(unsafe { thaw_promise_reject_typed(later, 1usize as *const u8, 2, 0.0, 42, false, std::ptr::null()) }, 1);
    thaw_runtime_run_until_idle();
    assert_eq!(unsafe { CStr::from_ptr(pending.cast()) }.to_bytes(), b"Unhandled opaque Promise rejection");
    unsafe { thaw_arena::destroy_string(pending.cast_mut().cast()) };

    let mut pending = std::ptr::null();
    let typed = thaw_promise_new();
    assert_eq!(unsafe { thaw_promise_detach_for_report(typed, &mut pending) }, 1);
    assert_eq!(unsafe { thaw_promise_reject_typed(typed, 1usize as *const u8, 2, 0.0, 42, false, std::ptr::null()) }, 1);
    thaw_runtime_run_until_idle();
    assert_eq!(unsafe { CStr::from_ptr(pending.cast()) }.to_bytes(), b"42");
    unsafe { thaw_arena::destroy_string(pending.cast_mut().cast()) };

    let mut pending = std::ptr::null();
    let native = thaw_promise_new();
    assert_eq!(unsafe { thaw_promise_detach_for_report(native, &mut pending) }, 1);
    let text = b"native source\0".to_vec();
    assert_eq!(unsafe { thaw_promise_reject_native_text(native, text.as_ptr()) }, 1);
    thaw_runtime_run_until_idle();
    drop(text);
    assert_eq!(unsafe { CStr::from_ptr(pending.cast()) }.to_bytes(), b"native source");
    unsafe { thaw_arena::destroy_string(pending.cast_mut().cast()) };
}

#[test]
fn compiler_text_reporter_snapshots_opaque_and_typed_rejections() {
    thread_local! { static SEEN: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) }; }
    extern "C" fn reporter(text: *const u8) -> PromiseReportResult {
        SEEN.with(|seen| seen.borrow_mut().push(unsafe { CStr::from_ptr(text.cast()) }.to_bytes().to_vec()));
        PromiseReportResult { value: 1, error: std::ptr::null() }
    }
    SEEN.with(|seen| seen.borrow_mut().clear());
    thaw_promise_set_unhandled_reporter_text_result(Some(reporter));
    let opaque = thaw_promise_new();
    assert_eq!(thaw_promise_reject(opaque, 1usize as *const u8), 1);
    let typed = thaw_promise_new();
    assert_eq!(unsafe { thaw_promise_reject_typed(typed, 1usize as *const u8, 2, 0.0, 42, false, std::ptr::null()) }, 1);
    assert_eq!(thaw_runtime_poll_one(), 0);
    SEEN.with(|seen| assert_eq!(&*seen.borrow(), &[b"Unhandled opaque Promise rejection".to_vec(), b"42".to_vec()]));
    thaw_promise_set_unhandled_reporter_text_result(None);
    unsafe { thaw_promise_destroy(opaque); thaw_promise_destroy(typed) };
}

#[test]
fn promise_forwarders_preserve_typed_exception_metadata() {
    fn rejected(error: &u8, object: &u8) -> *mut ThawPromise {
        let promise = thaw_promise_new();
        assert_eq!(
            unsafe { thaw_promise_reject_typed(promise, error, 1, 42.5, 43, true, object) },
            1
        );
        promise
    }

    unsafe fn assert_metadata(promise: *mut ThawPromise, object: &u8) {
        assert_eq!(unsafe { thaw_promise_exception_tag(promise) }, 1);
        assert_eq!(unsafe { thaw_promise_exception_f64(promise) }, 42.5);
        assert_eq!(unsafe { thaw_promise_exception_i64(promise) }, 43);
        assert!(unsafe { thaw_promise_exception_bool(promise) });
        assert_eq!(unsafe { thaw_promise_exception_object(promise) }, object);
    }

    let error = 1u8;
    let object = 2u8;

    let adopted = thaw_promise_new();
    unsafe { thaw_promise_adopt(adopted, rejected(&error, &object)) };
    thaw_runtime_run_until_idle();
    unsafe { assert_metadata(adopted, &object) };

    let chained = unsafe {
        thaw_promise_chain(
            rejected(&error, &object),
            transform_chain_number,
            std::ptr::null_mut(),
            0,
        )
    };
    thaw_runtime_run_until_idle();
    unsafe { assert_metadata(chained, &object) };

    let all_input = rejected(&error, &object);
    let all = unsafe { thaw_promise_all_slots(&all_input, 1, size_of::<u64>()) };
    thaw_runtime_run_until_idle();
    unsafe { assert_metadata(all, &object) };

    let race_input = rejected(&error, &object);
    let race = unsafe { thaw_promise_race(&race_input, 1) };
    thaw_runtime_run_until_idle();
    unsafe { assert_metadata(race, &object) };

    unsafe {
        thaw_promise_destroy(adopted);
        thaw_promise_destroy(chained);
        thaw_promise_destroy(all);
        thaw_promise_destroy(race);
    }
}

#[test]
fn promise_all_preserves_input_order_and_resolves_empty_inputs() {
    let first = thaw_promise_new();
    let second = thaw_promise_new();
    let children = [first, second];
    let joined = unsafe { thaw_promise_all_f64(children.as_ptr(), children.len()) };
    let first_value = 3.0f64;
    let second_value = 7.0f64;
    assert_eq!(
        thaw_promise_resolve(second, (&second_value as *const f64).cast()),
        1
    );
    assert_eq!(thaw_promise_state(joined), 0);
    assert_eq!(
        thaw_promise_resolve(first, (&first_value as *const f64).cast()),
        1
    );
    thaw_runtime_run_until_idle();
    assert_eq!(thaw_promise_state(joined), 1);
    let result = resolved_array_buffer(joined);
    assert_eq!(unsafe { result.read() }, 2);
    assert_eq!(f64::from_bits(unsafe { result.add(1).read() }), 3.0);
    assert_eq!(f64::from_bits(unsafe { result.add(2).read() }), 7.0);
    unsafe { thaw_promise_destroy(joined) };

    let empty = unsafe { thaw_promise_all_f64(std::ptr::null(), 0) };
    assert_eq!(thaw_promise_state(empty), 1);
    let result = resolved_array_buffer(empty);
    assert_eq!(unsafe { result.read() }, 0);
    unsafe { thaw_promise_destroy(empty) };
}

#[test]
fn promise_all_typed_copies_position_sizes_and_deduplicates_handles() {
    let flag = thaw_promise_new();
    let pointer = thaw_promise_new();
    let children = [flag, pointer, flag];
    let sizes = [1usize, 8, 1];
    let joined =
        unsafe { thaw_promise_all_typed(children.as_ptr(), sizes.as_ptr(), children.len()) };
    let pointer_value = 0x1234_5678_9abc_def0u64;
    let flag_value = 1u8;
    assert_eq!(
        thaw_promise_resolve(pointer, (&pointer_value as *const u64).cast()),
        1
    );
    assert_eq!(thaw_promise_resolve(flag, &flag_value), 1);
    thaw_runtime_run_until_idle();
    let result = resolved_array_buffer(joined);
    assert_eq!(unsafe { result.read() }, 3);
    assert_eq!(unsafe { result.add(1).read() }, 1);
    assert_eq!(unsafe { result.add(2).read() }, pointer_value);
    assert_eq!(unsafe { result.add(3).read() }, 1);
    unsafe { thaw_promise_destroy(joined) };
}

#[test]
fn promise_all_typed_preserves_wide_values() {
    let first = thaw_promise_new();
    let second = thaw_promise_new();
    let children = [first, second];
    let sizes = [16usize, 16];
    let joined =
        unsafe { thaw_promise_all_typed(children.as_ptr(), sizes.as_ptr(), children.len()) };
    let first_value = [1u64, 11];
    let second_value = [2u64, 22];
    assert_eq!(thaw_promise_resolve(first, first_value.as_ptr().cast()), 1);
    assert_eq!(
        thaw_promise_resolve(second, second_value.as_ptr().cast()),
        1
    );
    thaw_runtime_run_until_idle();
    let result = resolved_array_buffer(joined).cast::<u8>();
    assert_eq!(unsafe { result.cast::<u64>().read() }, 2);
    assert_eq!(
        unsafe { result.add(8).cast::<[u64; 2]>().read() },
        first_value
    );
    assert_eq!(
        unsafe { result.add(24).cast::<[u64; 2]>().read() },
        second_value
    );
    unsafe { thaw_promise_destroy(joined) };
}

#[test]
fn promise_all_rejects_with_the_first_observed_error() {
    let first = thaw_promise_new();
    let second = thaw_promise_new();
    let children = [first, second];
    let joined = unsafe { thaw_promise_all_f64(children.as_ptr(), children.len()) };
    let error = b"joined failure\0";
    assert_eq!(thaw_promise_reject(second, error.as_ptr()), 1);
    thaw_runtime_run_until_idle();
    assert_eq!(thaw_promise_state(joined), 2);
    let value = 1.0f64;
    assert_eq!(
        thaw_promise_resolve(first, (&value as *const f64).cast()),
        1
    );
    thaw_runtime_run_until_idle();
    assert_eq!(thaw_promise_state(joined), 2);
    assert_eq!(thaw_runtime_run_until_resolved(joined), error.as_ptr());
    unsafe { thaw_promise_destroy(joined) };
}

#[test]
fn rejected_promise_all_drains_children_after_parent_destruction() {
    let failed = thaw_promise_new();
    let slow = timed_value(25, 4.0);
    let children = [failed, slow];
    let joined = unsafe { thaw_promise_all_f64(children.as_ptr(), children.len()) };
    let error = b"early failure\0";
    assert_eq!(thaw_promise_reject(failed, error.as_ptr()), 1);
    thaw_runtime_run_until_idle();
    assert_eq!(thaw_promise_state(joined), 2);
    unsafe { thaw_promise_destroy(joined) };
    assert_eq!(ACTIVE_PROMISE_JOINS.with(Cell::get), 1);
    thaw_runtime_drain_detached();
    assert_eq!(ACTIVE_PROMISE_JOINS.with(Cell::get), 0);
}

#[test]
fn promise_all_drives_children_concurrently() {
    let started = Instant::now();
    let children = [
        timed_value(80, 1.0),
        timed_value(80, 2.0),
        timed_value(80, 3.0),
    ];
    let joined = unsafe { thaw_promise_all_f64(children.as_ptr(), children.len()) };
    assert!(!thaw_runtime_run_until_resolved(joined).is_null());
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(180),
        "three 80ms children ran serially: {elapsed:?}"
    );
    unsafe { thaw_promise_destroy(joined) };
}

#[test]
fn invocation_deadline_rejects_a_pending_promise_without_waiting_for_it() {
    // 10s away: long enough that the test would time out itself if the
    // deadline mechanism failed to preempt it.
    let pending = timed_value(10_000, 0.0);
    set_invocation_deadline(Some(30));

    let started = Instant::now();
    let result = thaw_runtime_run_until_resolved(pending);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the deadline should reject long before the far-future timer fires"
    );

    assert_eq!(thaw_promise_state(pending), 2);
    let message = unsafe { CStr::from_ptr(result.cast()) }.to_str().unwrap();
    assert_eq!(message, "Task timed out after 0.03 seconds");

    // The abandoned timer is still registered until something purges it --
    // exactly the state `InvocationArenaReset` must clear before the next
    // invocation's event loop could otherwise resume it into reused memory.
    assert!(TIMERS.with(|timers| !timers.borrow().is_empty()));
    purge_pending_async_state();
    assert!(TIMERS.with(|timers| timers.borrow().is_empty()));
    assert_eq!(invocation_deadline_remaining(), None);

    unsafe { thaw_promise_destroy(pending) };
}

#[test]
fn invocation_deadline_does_not_affect_a_promise_that_settles_in_time() {
    set_invocation_deadline(Some(500));
    let fast = timed_value(10, 42.0);
    assert_eq!(
        unsafe { *thaw_runtime_run_until_resolved(fast).cast::<f64>() },
        42.0
    );
    assert_eq!(thaw_promise_state(fast), 1);
    purge_pending_async_state();
    unsafe { thaw_promise_destroy(fast) };
}

#[test]
fn promise_race_uses_completion_order_and_drains_the_loser() {
    let slow = timed_value(30, 1.0);
    let fast = timed_value(2, 2.0);
    let children = [slow, fast];
    let raced = unsafe { thaw_promise_race(children.as_ptr(), children.len()) };
    let result = thaw_runtime_run_until_resolved(raced);
    assert_eq!(unsafe { result.cast::<f64>().read_unaligned() }, 2.0);
    assert_eq!(ACTIVE_PROMISE_JOINS.with(Cell::get), 1);
    unsafe { thaw_promise_destroy(raced) };
    thaw_runtime_drain_detached();
    assert_eq!(ACTIVE_PROMISE_JOINS.with(Cell::get), 0);
}

#[test]
fn promise_race_forwards_first_rejection_and_deduplicates_handles() {
    let failed = thaw_promise_new();
    let slow = timed_value(20, 4.0);
    let children = [failed, slow, failed];
    let raced = unsafe { thaw_promise_race(children.as_ptr(), children.len()) };
    let error = b"race failure\0";
    assert_eq!(thaw_promise_reject(failed, error.as_ptr()), 1);
    thaw_runtime_run_until_idle();
    assert_eq!(thaw_promise_state(raced), 2);
    assert_eq!(thaw_runtime_run_until_resolved(raced), error.as_ptr());
    unsafe { thaw_promise_destroy(raced) };
    thaw_runtime_drain_detached();
    assert_eq!(ACTIVE_PROMISE_JOINS.with(Cell::get), 0);
}

#[test]
fn promise_race_keeps_an_empty_input_pending() {
    let raced = unsafe { thaw_promise_race(std::ptr::null(), 0) };
    assert_eq!(thaw_promise_state(raced), 0);
    assert!(thaw_runtime_run_until_resolved(raced).is_null());
    assert_eq!(thaw_promise_state(raced), 0);
    unsafe { thaw_promise_destroy(raced) };
}

#[test]
fn promise_any_ignores_rejections_and_uses_the_first_fulfillment() {
    let failed = thaw_promise_new();
    let slow = timed_value(25, 1.0);
    let fast = timed_value(2, 2.0);
    let children = [failed, slow, fast, failed];
    let any = unsafe { thaw_promise_any(children.as_ptr(), children.len()) };
    let error = b"ignored failure\0";
    assert_eq!(thaw_promise_reject(failed, error.as_ptr()), 1);
    let result = thaw_runtime_run_until_resolved(any);
    assert_eq!(unsafe { result.cast::<f64>().read_unaligned() }, 2.0);
    unsafe { thaw_promise_destroy(any) };
    thaw_runtime_drain_detached();
    assert_eq!(ACTIVE_PROMISE_JOINS.with(Cell::get), 0);
}

#[test]
fn promise_any_rejects_only_after_every_input_rejects() {
    let first = thaw_promise_new();
    let second = thaw_promise_new();
    let children = [first, second];
    let any = unsafe { thaw_promise_any(children.as_ptr(), children.len()) };
    let first_error = [b'f', b'i', b'r', b's', b't', 0];
    let second_error = [b's', b'e', b'c', b'o', b'n', b'd', 0];
    assert_eq!(thaw_promise_reject(first, first_error.as_ptr()), 1);
    thaw_runtime_run_until_idle();
    assert_eq!(thaw_promise_state(any), 0);
    assert_eq!(thaw_promise_reject(second, second_error.as_ptr()), 1);
    thaw_runtime_run_until_idle();
    assert_eq!(thaw_promise_state(any), 2);
    assert_eq!(
        thaw_runtime_run_until_resolved(any),
        PROMISE_ANY_REJECTED_ERROR.as_ptr()
    );
    unsafe { thaw_promise_destroy(any) };
}

#[test]
fn promise_any_rejects_an_empty_input() {
    let any = unsafe { thaw_promise_any(std::ptr::null(), 0) };
    assert_eq!(thaw_promise_state(any), 2);
    assert_eq!(
        thaw_runtime_run_until_resolved(any),
        PROMISE_ANY_REJECTED_ERROR.as_ptr()
    );
    unsafe { thaw_promise_destroy(any) };
}

#[test]
fn promise_all_settled_preserves_order_and_turns_rejections_into_values() {
    let fulfilled = thaw_promise_new();
    let rejected = thaw_promise_new();
    let children = [fulfilled, rejected, fulfilled];
    let settled =
        unsafe { thaw_promise_all_settled(children.as_ptr(), children.len(), size_of::<f64>()) };
    let error = b"settled failure\0";
    let number = 7.0f64;
    assert_eq!(thaw_promise_reject(rejected, error.as_ptr()), 1);
    thaw_runtime_run_until_idle();
    assert_eq!(thaw_promise_state(settled), 0);
    assert_eq!(
        thaw_promise_resolve(fulfilled, (&number as *const f64).cast()),
        1
    );
    thaw_runtime_run_until_idle();
    let result = resolved_array_buffer(settled);
    assert_eq!(unsafe { result.read() }, 3);
    let first = unsafe { result.add(1).read() as *const u64 };
    let second = unsafe { result.add(2).read() as *const u64 };
    let third = unsafe { result.add(3).read() as *const u64 };
    assert_eq!(
        unsafe { first.read() as *const u8 },
        PROMISE_SETTLED_FULFILLED.as_ptr()
    );
    assert_eq!(f64::from_bits(unsafe { first.add(1).read() }), 7.0);
    assert_eq!(
        unsafe { first.add(2).read() as *const u8 },
        PROMISE_SETTLED_EMPTY_REASON.as_ptr()
    );
    assert_eq!(
        unsafe { second.read() as *const u8 },
        PROMISE_SETTLED_REJECTED.as_ptr()
    );
    assert_eq!(unsafe { second.add(1).read() }, 0);
    assert_eq!(unsafe { second.add(2).read() as *const u8 }, error.as_ptr());
    assert_eq!(
        unsafe { third.read() as *const u8 },
        PROMISE_SETTLED_FULFILLED.as_ptr()
    );
    assert_eq!(f64::from_bits(unsafe { third.add(1).read() }), 7.0);
    unsafe { thaw_promise_destroy(settled) };
}

#[test]
fn promise_all_settled_preserves_wide_values() {
    let fulfilled = thaw_promise_new();
    let children = [fulfilled];
    let settled = unsafe { thaw_promise_all_settled(children.as_ptr(), 1, 16) };
    let value = [3u64, 33];
    assert_eq!(thaw_promise_resolve(fulfilled, value.as_ptr().cast()), 1);
    thaw_runtime_run_until_idle();
    let result = resolved_array_buffer(settled);
    let object = unsafe { result.add(1).read() as *const u8 };
    assert_eq!(
        unsafe { object.cast::<u64>().read() as *const u8 },
        PROMISE_SETTLED_FULFILLED.as_ptr()
    );
    assert_eq!(unsafe { object.add(8).cast::<[u64; 2]>().read() }, value);
    assert_eq!(
        unsafe { object.add(24).cast::<u64>().read() as *const u8 },
        PROMISE_SETTLED_EMPTY_REASON.as_ptr()
    );
    unsafe { thaw_promise_destroy(settled) };
}

#[test]
fn promise_all_settled_resolves_an_empty_input() {
    let settled = unsafe { thaw_promise_all_settled(std::ptr::null(), 0, size_of::<f64>()) };
    assert_eq!(thaw_promise_state(settled), 1);
    let result = resolved_array_buffer(settled);
    assert_eq!(unsafe { result.read() }, 0);
    unsafe { thaw_promise_destroy(settled) };
}

#[test]
fn promise_all_settled_fulfills_when_every_input_rejects() {
    let first = thaw_promise_new();
    let second = thaw_promise_new();
    let children = [first, second];
    let settled =
        unsafe { thaw_promise_all_settled(children.as_ptr(), children.len(), size_of::<f64>()) };
    let first_error = [b'f', b'i', b'r', b's', b't', 0];
    let second_error = [b's', b'e', b'c', b'o', b'n', b'd', 0];
    assert_eq!(thaw_promise_reject(first, first_error.as_ptr()), 1);
    assert_eq!(thaw_promise_reject(second, second_error.as_ptr()), 1);
    thaw_runtime_run_until_idle();
    assert_eq!(thaw_promise_state(settled), 1);
    unsafe { thaw_promise_destroy(settled) };
}

#[test]
fn promise_all_deduplicates_repeated_handles() {
    let child = thaw_promise_new();
    let children = [child, child];
    let joined = unsafe { thaw_promise_all_f64(children.as_ptr(), children.len()) };
    let value = 9.0f64;
    assert_eq!(
        thaw_promise_resolve(child, (&value as *const f64).cast()),
        1
    );
    thaw_runtime_run_until_idle();
    assert_eq!(thaw_promise_state(joined), 1);
    let result = resolved_array_buffer(joined);
    assert_eq!(f64::from_bits(unsafe { result.add(1).read() }), 9.0);
    assert_eq!(f64::from_bits(unsafe { result.add(2).read() }), 9.0);
    unsafe { thaw_promise_destroy(joined) };
}

#[test]
fn timer_promise_is_driven_without_a_worker_thread() {
    let promise = thaw_sleep_ms(1);
    let mut record = ResumeRecord {
        calls: 0,
        result: std::ptr::null(),
    };
    assert_eq!(
        thaw_promise_subscribe(
            promise,
            record_resume,
            (&mut record as *mut ResumeRecord).cast()
        ),
        1
    );

    let result = thaw_runtime_run_until_resolved(promise);
    assert!(!result.is_null());
    assert_eq!(record.calls, 1);
    assert_eq!(record.result, result);
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn fd_readiness_resolves_and_resumes_a_subscriber() {
    let mut fds = [0; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let promise = thaw_runtime_wait_fd(fds[0], THAW_FD_READABLE);
    let mut record = ResumeRecord {
        calls: 0,
        result: std::ptr::null(),
    };
    assert_eq!(
        thaw_promise_subscribe(
            promise,
            record_resume,
            (&mut record as *mut ResumeRecord).cast(),
        ),
        1
    );
    assert_eq!(
        unsafe { libc::write(fds[1], b"ready".as_ptr().cast(), 5) },
        5
    );
    assert!(!thaw_runtime_run_until_resolved(promise).is_null());
    assert_eq!(thaw_promise_state(promise), 1);
    assert_eq!(record.calls, 1);
    assert_eq!(thaw_runtime_run_until_idle(), 0);
    unsafe { thaw_promise_destroy(promise) };
    unsafe {
        libc::close(fds[0]);
        libc::close(fds[1]);
    }
}

#[test]
fn persistent_fd_watcher_dispatches_through_the_shared_event_loop() {
    extern "C" fn record_watcher(context: *mut u8, events: i16) {
        let record = unsafe { &mut *(context as *mut (usize, i16)) };
        record.0 += 1;
        record.1 = events;
    }

    let mut fds = [0; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let mut record = (0usize, 0i16);
    let watcher = thaw_runtime_watch_fd(
        fds[0],
        THAW_FD_READABLE,
        record_watcher,
        (&mut record as *mut (usize, i16)).cast(),
    );
    assert_ne!(watcher, 0);
    assert_eq!(
        unsafe { libc::write(fds[1], b"ready".as_ptr().cast(), 5) },
        5
    );
    assert!(thaw_runtime_run_one_event());
    assert_eq!(record.0, 1);
    assert_ne!(record.1 & libc::POLLIN, 0);
    assert!(thaw_runtime_unwatch_fd(watcher));
    assert!(!thaw_runtime_unwatch_fd(watcher));
    unsafe {
        libc::close(fds[0]);
        libc::close(fds[1]);
    }
}

#[test]
fn timer_completes_while_a_persistent_watcher_is_idle() {
    extern "C" fn ignore_watcher(_context: *mut u8, _events: i16) {}

    let mut fds = [0; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let watcher = thaw_runtime_watch_fd(
        fds[0],
        THAW_FD_READABLE,
        ignore_watcher,
        std::ptr::null_mut(),
    );
    let timer = thaw_sleep_ms(1);
    assert!(!thaw_runtime_run_until_resolved(timer).is_null());
    assert_eq!(thaw_promise_state(timer), 1);
    assert!(thaw_runtime_unwatch_fd(watcher));
    unsafe {
        thaw_promise_destroy(timer);
        libc::close(fds[0]);
        libc::close(fds[1]);
    }
}

#[test]
fn timer_can_finish_while_an_fd_wait_remains_pending() {
    let mut fds = [0; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let fd_promise = thaw_runtime_wait_fd(fds[0], THAW_FD_READABLE);
    let timer = thaw_sleep_ms(1);
    assert!(!thaw_runtime_run_until_resolved(timer).is_null());
    assert_eq!(thaw_promise_state(timer), 1);
    assert_eq!(thaw_promise_state(fd_promise), 0);
    unsafe {
        thaw_promise_destroy(timer);
        thaw_promise_destroy(fd_promise);
        libc::close(fds[0]);
        libc::close(fds[1]);
    }
}

#[test]
fn dns_resolution_completes_through_the_fd_event_loop() {
    let (fd, result) = start_dns_resolution("localhost".to_string(), 80).unwrap();
    let readiness = thaw_runtime_wait_fd_timeout(fd, THAW_FD_READABLE, 5_000);
    assert!(!thaw_runtime_run_until_resolved(readiness).is_null());
    assert_eq!(thaw_promise_state(readiness), 1);
    let mut byte = [0u8; 1];
    assert_eq!(unsafe { libc::read(fd, byte.as_mut_ptr().cast(), byte.len()) }, 1);
    assert_eq!(byte, [1]);
    let resolved = result
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .expect("DNS worker must publish before notifying the pipe")
        .unwrap();
    assert!(!resolved.is_empty());
    unsafe {
        thaw_promise_destroy(readiness);
        libc::close(fd);
    }
}

#[test]
fn dns_completion_after_cancellation_does_not_raise_sigpipe() {
    let mut ends = [-1; 2];
    assert_eq!(
        unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                0,
                ends.as_mut_ptr(),
            )
        },
        0
    );
    unsafe { libc::close(ends[0]) };
    let error = notify_dns_completion(ends[1]).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::EPIPE));
}

#[test]
fn invalid_fd_wait_is_rejected() {
    let promise = thaw_runtime_wait_fd(-1, THAW_FD_READABLE);
    assert_eq!(thaw_promise_state(promise), 2);
    assert_eq!(
        thaw_runtime_run_until_resolved(promise),
        INVALID_FD_ERROR.as_ptr()
    );
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn fd_wait_timeout_rejects_without_readiness() {
    let mut fds = [0; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let promise = thaw_runtime_wait_fd_timeout(fds[0], THAW_FD_READABLE, 1);
    assert!(!thaw_runtime_run_until_resolved(promise).is_null());
    assert_eq!(thaw_promise_state(promise), 2);
    unsafe {
        thaw_promise_destroy(promise);
        libc::close(fds[0]);
        libc::close(fds[1]);
    }
}

#[test]
fn parses_async_http_urls() {
    assert_eq!(
        parse_http_url("http://example.com:8080/api?q=1").unwrap(),
        (
            false,
            "example.com".to_string(),
            8080,
            "/api?q=1".to_string()
        )
    );
    assert_eq!(
        parse_http_url("https://[::1]/").unwrap(),
        (true, "::1".to_string(), 443, "/".to_string())
    );
    assert_eq!(
        parse_http_url("http://example.com?x=1#fragment").unwrap(),
        (false, "example.com".to_string(), 80, "/?x=1".to_string())
    );
    assert_eq!(
        parse_http_url("https://[::1]:8443?x=1#fragment").unwrap(),
        (true, "::1".to_string(), 8443, "/?x=1".to_string())
    );
    assert_eq!(
        parse_http_url("http://example.com#fragment").unwrap().3,
        "/"
    );
    assert_eq!(
        parse_http_url("http://example.com/a/./b/../?x=1").unwrap().3,
        "/a/?x=1"
    );
}

#[test]
fn resolves_async_http_redirect_paths_without_losing_query_or_slashes() {
    let redirect = |current_path: &str, location: &str| {
        redirect_url(false, "example.com", 80, current_path, location).unwrap()
    };
    assert_eq!(
        redirect("/a/page?next=/x/y", "child"),
        "http://example.com/a/child"
    );
    assert_eq!(
        redirect("/a/page?next=/x/y", "#fragment"),
        "http://example.com/a/page?next=/x/y#fragment"
    );
    assert_eq!(
        parse_http_url(&redirect("/a/page?next=/x/y", "#fragment"))
            .unwrap()
            .3,
        "/a/page?next=/x/y"
    );
    assert_eq!(
        redirect("/a/page?old=1", "?new=2"),
        "http://example.com/a/page?new=2"
    );
    assert_eq!(redirect("/a/page?old=1", ""), "http://example.com/a/page?old=1");
    assert_eq!(redirect("/a/page", "/dir/"), "http://example.com/dir/");
    assert_eq!(redirect("/a/page", "child/../"), "http://example.com/a/");
    assert_eq!(redirect("/a/page", "../"), "http://example.com/");
    assert_eq!(
        redirect("/a/page", "/a//b/./../c/"),
        "http://example.com/a//c/"
    );
    assert_eq!(
        redirect("/a/page", "//other.example/dir/"),
        "http://other.example/dir/"
    );
}

#[test]
fn incrementally_parses_content_length_response() {
    let partial = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhe";
    assert!(parse_http_response(partial, false).unwrap().is_none());
    let complete = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhelloignored";
    let parsed = parse_http_response(complete, false).unwrap().unwrap();
    assert_eq!(parsed.status, 200);
    assert_eq!(parsed.body, b"hello");
    assert!(parse_http_response(partial, true).is_err());
    let redirect = b"HTTP/1.1 302 Found\r\nLocation: ../next\r\nContent-Length: 0\r\n\r\n";
    let parsed = parse_http_response(redirect, false).unwrap().unwrap();
    assert_eq!(parsed.location.as_deref(), Some("../next"));
}

#[test]
fn skips_complete_interim_responses_until_final_http_response() {
    let interim = b"HTTP/1.1 100 Continue\r\nX-Interim: yes\r\n\r\nHTTP/1.1 102 Processing\r\n\r\nHTTP/1.1 103 Early Hints\r\nLocation: /interim\r\n\r\n";
    let partial_interim = b"HTTP/1.1 100 Continue\r\nX-Interim:";
    assert!(parse_http_response(partial_interim, false).unwrap().is_none());
    assert!(parse_http_response(partial_interim, true).is_err());
    let partial_head = [&interim[..], &b"HTTP/1.1 302 Found\r\nLoc"[..]].concat();
    assert!(parse_http_response(&partial_head, false).unwrap().is_none());
    assert!(parse_http_response(&partial_head, true).is_err());

    let final_head = b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 5\r\n\r\n";
    let partial_body = [&interim[..], &final_head[..], &b"he"[..]].concat();
    assert!(parse_http_response(&partial_body, false).unwrap().is_none());
    let complete = [&interim[..], &final_head[..], &b"hello"[..]].concat();
    let parsed = parse_http_response(&complete, false).unwrap().unwrap();
    assert_eq!(parsed.status, 302);
    assert_eq!(parsed.body, b"hello");
    assert_eq!(parsed.location.as_deref(), Some("/final"));
}

#[test]
fn interim_response_waits_for_complete_chunked_final_response() {
    let interim = b"HTTP/1.1 103 Early Hints\r\n\r\n";
    let final_head = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
    let partial = [&interim[..], &final_head[..], &b"4\r\nWi"[..]].concat();
    assert!(parse_http_response(&partial, false).unwrap().is_none());
    let complete = [&interim[..], &final_head[..], &b"4\r\nWiki\r\n0\r\n\r\n"[..]].concat();
    let parsed = parse_http_response(&complete, false).unwrap().unwrap();
    assert_eq!(parsed.status, 200);
    assert_eq!(parsed.body, b"Wiki");
    assert_eq!(parsed.location, None);
}

#[test]
fn interim_only_eof_and_protocol_upgrade_are_errors() {
    let interim = b"HTTP/1.1 100 Continue\r\n\r\n";
    assert!(parse_http_response(interim, false).unwrap().is_none());
    assert!(parse_http_response(interim, true).is_err());
    let upgrade = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n";
    assert!(parse_http_response(upgrade, false)
        .err()
        .unwrap()
        .contains("upgrade"));
    let invalid_interim = b"HTTP/1.1 103 Early Hints\r\nContent-Length: invalid\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
    assert!(parse_http_response(invalid_interim, false).is_err());
}

#[test]
fn incrementally_decodes_chunked_response() {
    let partial = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWi";
    assert!(parse_http_response(partial, false).unwrap().is_none());
    let complete = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4;name=value\r\nWiki\r\n5\r\npedia\r\n0\r\nX-End: yes\r\n\r\n";
    let parsed = parse_http_response(complete, false).unwrap().unwrap();
    assert_eq!(parsed.body, b"Wikipedia");
}

#[test]
fn chunk_size_near_address_limit_does_not_overflow_terminator_offset() {
    // Unrun regression for the shared decoder's chunk_end + CRLF boundary.
    let header_len = format!("{:x}", usize::MAX).len() + 2;
    let size = usize::MAX - header_len;
    let chunk = format!("{size:x}\r\n");
    assert!(decode_chunked(chunk.as_bytes())
        .unwrap_err()
        .contains("overflows address space"));
}

#[test]
fn async_http_rejects_unsupported_scheme_without_blocking() {
    let url = CString::new("ftp://example.com/").unwrap();
    let promise = thaw_http_get_async(url.as_ptr());
    assert_eq!(thaw_promise_state(promise), 2);
    assert!(!thaw_runtime_run_until_resolved(promise).is_null());
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn async_http_and_timer_share_the_event_loop() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let _ = connection.read(&mut request).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        connection
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .unwrap();
    });
    let url = CString::new(format!("http://{addr}/data")).unwrap();
    let http = thaw_http_get_async_timeout(url.as_ptr(), 1_000);
    let timer = thaw_sleep_ms(1);
    assert!(!thaw_runtime_run_until_resolved(timer).is_null());
    assert_eq!(thaw_promise_state(timer), 1);
    assert_eq!(thaw_promise_state(http), 0);
    let result_slot = thaw_runtime_run_until_resolved(http) as *const *const c_char;
    assert_eq!(thaw_promise_state(http), 1);
    let body = unsafe { CStr::from_ptr(*result_slot) }.to_string_lossy();
    assert_eq!(body, "ok");
    server.join().unwrap();
    unsafe {
        thaw_promise_destroy(timer);
        thaw_promise_destroy(http);
    }
}

#[test]
fn async_http_ready_response_cannot_complete_after_absolute_deadline() {
    let mut fds = [-1; 2];
    assert_eq!(
        unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) },
        0
    );
    let body = vec![b'x'; 16_384];
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(&body);
    assert_eq!(
        unsafe { libc::send(fds[1], response.as_ptr().cast(), response.len(), libc::MSG_NOSIGNAL) },
        response.len() as isize
    );
    let completion = thaw_promise_new();
    let readiness = thaw_promise_new();
    assert_eq!(thaw_promise_resolve(readiness, std::ptr::dangling::<u8>()), 1);
    let task = Box::into_raw(Box::new(AsyncHttpGet {
        fd: fds[0],
        request: Vec::new(),
        written: 0,
        response: Vec::new(),
        state: AsyncHttpState::Reading,
        completion,
        readiness,
        deadline: Instant::now() - Duration::from_millis(1),
        tls: None,
        tls_config: tls_client_config(),
        use_tls: false,
        host: "localhost".to_string(),
        port: 80,
        path: "/".to_string(),
        redirects: 0,
        resolution: None,
        remaining_addresses: Vec::new(),
    }));
    resume_async_http(task.cast(), std::ptr::null());
    assert_eq!(thaw_promise_state(completion), 2);
    unsafe {
        libc::close(fds[1]);
        thaw_promise_destroy(completion);
    }
}

#[test]
fn async_http_retries_remaining_address_after_connect_so_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let good_address = listener.local_addr().unwrap();
    let refused_address = std::net::SocketAddr::from(([127, 0, 0, 2], good_address.port()));
    let first_fd = open_nonblocking_socket(refused_address)
        .expect("loopback connection refusal should complete asynchronously");
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let read = connection.read(&mut request).unwrap();
        assert!(request[..read].starts_with(b"GET /retry HTTP/1.1\r\n"));
        connection
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .unwrap();
    });
    let completion = thaw_promise_new();
    let task = Box::into_raw(Box::new(AsyncHttpGet {
        fd: first_fd,
        request: format!(
            "GET /retry HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: */*\r\n\r\n",
            good_address
        )
        .into_bytes(),
        written: 0,
        response: Vec::new(),
        state: AsyncHttpState::Connecting,
        completion,
        readiness: std::ptr::null_mut(),
        deadline: Instant::now() + Duration::from_secs(5),
        tls: None,
        tls_config: tls_client_config(),
        use_tls: false,
        host: "127.0.0.1".to_string(),
        port: good_address.port(),
        path: "/retry".to_string(),
        redirects: 0,
        resolution: None,
        remaining_addresses: vec![good_address],
    }));
    schedule_async_http(task, THAW_FD_WRITABLE);
    let result_slot = thaw_runtime_run_until_resolved(completion) as *const *const c_char;
    assert_eq!(thaw_promise_state(completion), 1);
    assert_eq!(unsafe { CStr::from_ptr(*result_slot) }.to_bytes(), b"ok");
    server.join().unwrap();
    unsafe { thaw_promise_destroy(completion) };
}

#[test]
fn async_http_finishes_content_length_before_keep_alive_closes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let _ = connection.read(&mut request).unwrap();
        connection
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: keep-alive\r\n\r\nhello",
            )
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));
    });
    let url = CString::new(format!("http://{addr}/keep-alive")).unwrap();
    let started = Instant::now();
    let promise = thaw_http_get_async_timeout(url.as_ptr(), 500);
    let result = thaw_runtime_run_until_resolved(promise);
    assert_eq!(
        thaw_promise_state(promise),
        1,
        "HTTP fetch rejected: {}",
        unsafe { CStr::from_ptr(result.cast()) }.to_string_lossy()
    );
    let result_slot = result as *const *const c_char;
    assert!(started.elapsed() < Duration::from_millis(150));
    assert_eq!(
        unsafe { CStr::from_ptr(*result_slot) }.to_string_lossy(),
        "hello"
    );
    server.join().unwrap();
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn async_http_decodes_chunked_keep_alive_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let _ = connection.read(&mut request).unwrap();
        connection
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n5\r\nhello\r\n1\r\n \r\n5\r\nworld\r\n0\r\n\r\n")
                .unwrap();
        std::thread::sleep(Duration::from_millis(20));
    });
    let url = CString::new(format!("http://{addr}/chunked")).unwrap();
    let promise = thaw_http_get_async_timeout(url.as_ptr(), 500);
    let result_slot = thaw_runtime_run_until_resolved(promise) as *const *const c_char;
    assert_eq!(thaw_promise_state(promise), 1);
    assert_eq!(
        unsafe { CStr::from_ptr(*result_slot) }.to_string_lossy(),
        "hello world"
    );
    server.join().unwrap();
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn async_https_handshakes_and_reads_a_verified_response() {
    let (client_config, server_config) = local_tls_configs();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (connection, _) = listener.accept().unwrap();
        let tls = ServerConnection::new(server_config).unwrap();
        let mut stream = StreamOwned::new(tls, connection);
        let mut request = [0u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: keep-alive\r\n\r\nsecure",
            )
            .unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(20));
    });
    let url = CString::new(format!("https://{addr}/secure")).unwrap();
    let promise = thaw_http_get_async_with_config(url.as_ptr(), 1_000, Some(client_config));
    let result = thaw_runtime_run_until_resolved(promise);
    assert_eq!(
        thaw_promise_state(promise),
        1,
        "TLS fetch rejected: {}",
        unsafe { CStr::from_ptr(result.cast()) }.to_string_lossy()
    );
    let result_slot = result as *const *const c_char;
    assert_eq!(
        unsafe { CStr::from_ptr(*result_slot) }.to_string_lossy(),
        "secure"
    );
    server.join().unwrap();
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn async_https_drains_plaintext_while_receiving_large_response() {
    let (client_config, server_config) = local_tls_configs();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let body = vec![b'x'; 256 * 1024];
    let expected = body.clone();
    let server = std::thread::spawn(move || {
        let (connection, _) = listener.accept().unwrap();
        let tls = ServerConnection::new(server_config).unwrap();
        let mut stream = StreamOwned::new(tls, connection);
        let mut request = [0u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(header.as_bytes()).unwrap();
        let _ = stream.write_all(&body);
        let _ = stream.flush();
    });

    let url = CString::new(format!("https://{addr}/large")).unwrap();
    let promise = thaw_http_get_async_with_config(url.as_ptr(), 5_000, Some(client_config));
    let result = thaw_runtime_run_until_resolved(promise);
    assert_eq!(
        thaw_promise_state(promise),
        1,
        "large TLS fetch rejected: {}",
        unsafe { CStr::from_ptr(result.cast()) }.to_string_lossy()
    );
    let result_slot = result as *const *const c_char;
    assert_eq!(unsafe { CStr::from_ptr(*result_slot) }.to_bytes(), expected.as_slice());
    server.join().unwrap();
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn async_https_rejects_an_untrusted_certificate() {
    let (_client_config, server_config) = local_tls_configs();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (connection, _) = listener.accept().unwrap();
        let tls = ServerConnection::new(server_config).unwrap();
        let mut stream = StreamOwned::new(tls, connection);
        let mut request = [0u8; 64];
        let _ = stream.read(&mut request);
    });
    let url = CString::new(format!("https://{addr}/untrusted")).unwrap();
    let promise = thaw_http_get_async_timeout(url.as_ptr(), 1_000);
    assert!(!thaw_runtime_run_until_resolved(promise).is_null());
    assert_eq!(thaw_promise_state(promise), 2);
    server.join().unwrap();
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn async_https_handshake_obeys_the_total_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (_connection, _) = listener.accept().unwrap();
        std::thread::sleep(Duration::from_millis(50));
    });
    let url = CString::new(format!("https://{addr}/stall")).unwrap();
    let promise = thaw_http_get_async_timeout(url.as_ptr(), 5);
    assert!(!thaw_runtime_run_until_resolved(promise).is_null());
    assert_eq!(thaw_promise_state(promise), 2);
    server.join().unwrap();
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn async_http_redirect_can_upgrade_to_https() {
    let (client_config, server_config) = local_tls_configs();
    let http_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let http_addr = http_listener.local_addr().unwrap();
    let tls_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let tls_addr = tls_listener.local_addr().unwrap();
    let redirect_server = std::thread::spawn(move || {
        let (mut connection, _) = http_listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let _ = connection.read(&mut request).unwrap();
        connection
                .write_all(
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: https://{tls_addr}/secure\r\nContent-Length: 0\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .unwrap();
    });
    let tls_server = std::thread::spawn(move || {
        let (connection, _) = tls_listener.accept().unwrap();
        let tls = ServerConnection::new(server_config).unwrap();
        let mut stream = StreamOwned::new(tls, connection);
        let mut request = [0u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nupgraded")
            .unwrap();
        stream.flush().unwrap();
    });
    let url = CString::new(format!("http://{http_addr}/upgrade")).unwrap();
    let promise = thaw_http_get_async_with_config(url.as_ptr(), 2_000, Some(client_config));
    let result_slot = thaw_runtime_run_until_resolved(promise) as *const *const c_char;
    assert_eq!(thaw_promise_state(promise), 1);
    assert_eq!(
        unsafe { CStr::from_ptr(*result_slot) }.to_string_lossy(),
        "upgraded"
    );
    redirect_server.join().unwrap();
    tls_server.join().unwrap();
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn async_http_timeout_rejects_a_stalled_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let _ = connection.read(&mut request).unwrap();
        std::thread::sleep(Duration::from_millis(50));
    });
    let url = CString::new(format!("http://{addr}/slow")).unwrap();
    let promise = thaw_http_get_async_timeout(url.as_ptr(), 5);
    assert!(!thaw_runtime_run_until_resolved(promise).is_null());
    assert_eq!(thaw_promise_state(promise), 2);
    server.join().unwrap();
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn async_http_rejects_when_peer_disconnects_without_a_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let _ = connection.read(&mut request).unwrap();
    });
    let url = CString::new(format!("http://{addr}/disconnect")).unwrap();
    let promise = thaw_http_get_async_timeout(url.as_ptr(), 1_000);
    assert!(!thaw_runtime_run_until_resolved(promise).is_null());
    assert_eq!(thaw_promise_state(promise), 2);
    server.join().unwrap();
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn async_http_follows_a_relative_redirect() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let read = first.read(&mut request).unwrap();
        assert!(String::from_utf8_lossy(&request[..read]).starts_with("GET /one/start "));
        first
            .write_all(b"HTTP/1.1 302 Found\r\nLocation: ../final\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        drop(first);

        let (mut second, _) = listener.accept().unwrap();
        let read = second.read(&mut request).unwrap();
        assert!(String::from_utf8_lossy(&request[..read]).starts_with("GET /final "));
        second
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nredirected")
            .unwrap();
    });
    let url = CString::new(format!("http://{addr}/one/start")).unwrap();
    let promise = thaw_http_get_async_timeout(url.as_ptr(), 1_000);
    let result_slot = thaw_runtime_run_until_resolved(promise) as *const *const c_char;
    assert_eq!(thaw_promise_state(promise), 1);
    assert_eq!(
        unsafe { CStr::from_ptr(*result_slot) }.to_string_lossy(),
        "redirected"
    );
    server.join().unwrap();
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn async_http_redirect_limit_rejects_a_loop() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for _ in 0..11 {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = connection.read(&mut request).unwrap();
            connection
                .write_all(b"HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        }
    });
    let url = CString::new(format!("http://{addr}/loop")).unwrap();
    let promise = thaw_http_get_async_timeout(url.as_ptr(), 2_000);
    assert!(!thaw_runtime_run_until_resolved(promise).is_null());
    assert_eq!(thaw_promise_state(promise), 2);
    server.join().unwrap();
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn unresolved_promise_without_an_event_source_reports_no_progress() {
    let promise = thaw_promise_new();
    assert!(thaw_runtime_run_until_resolved(promise).is_null());
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn invocation_guard_resets_request_arena() {
    thaw_arena::thaw_arena_reset();
    let first = thaw_arena::thaw_arena_alloc(32, 8);
    assert!(!first.is_null());

    {
        let _guard = InvocationArenaReset;
        let second = thaw_arena::thaw_arena_alloc(32, 8);
        assert_ne!(first, second);
    }

    let after_reset = thaw_arena::thaw_arena_alloc(32, 8);
    assert_eq!(first, after_reset);
    thaw_arena::thaw_arena_reset();
}

#[test]
fn polls_an_invocation_and_posts_the_handler_result() {
    let _trace_guard = guard_lambda_trace_env();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let n = conn.read(&mut buf).unwrap();
        let req = String::from_utf8_lossy(&buf[..n]);
        assert!(req.starts_with("GET /2018-06-01/runtime/invocation/next"));

        let body = "\"hello\"";
        let response = format!(
                "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: req-123\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
        conn.write_all(response.as_bytes()).unwrap();
        drop(conn);

        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        conn.read_to_end(&mut buf).unwrap();
        tx.send(String::from_utf8_lossy(&buf).into_owned()).unwrap();

        let response = "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        conn.write_all(response.as_bytes()).unwrap();
    });

    handle_one_invocation(&addr, echo_handler, std::ptr::null_mut()).unwrap();
    server.join().unwrap();

    let post_request = rx.recv().unwrap();
    assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/req-123/response"));
    assert!(post_request.ends_with("echo:\"hello\""));
}

#[test]
fn lambda_decodes_chunked_utf8_event_before_handler_and_post_response() {
    let _trace_guard = guard_lambda_trace_env();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        conn.read_to_end(&mut request).unwrap();
        let body = "{\"name\":\"é\"}".as_bytes();
        let split = body.iter().position(|byte| *byte == 0xc3).unwrap() + 1;
        let mut response = b"HTTP/1.1 200 OK\r\nlambda-runtime-aws-request-id: chunk-utf8\r\nTransfer-Encoding: ChUnKeD\r\nConnection: close\r\n\r\n".to_vec();
        for chunk in [&body[..split], &body[split..]] {
            response.extend_from_slice(format!("{:x};part=test\r\n", chunk.len()).as_bytes());
            response.extend_from_slice(chunk);
            response.extend_from_slice(b"\r\n");
        }
        response.extend_from_slice(b"0\r\nX-End: yes\r\n\r\n");
        conn.write_all(&response).unwrap();
        drop(conn);

        let (mut conn, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        conn.read_to_end(&mut request).unwrap();
        tx.send(String::from_utf8_lossy(&request).into_owned()).unwrap();
        conn.write_all(b"HTTP/1.1 202 Accepted\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n0\r\n\r\n").unwrap();
    });
    handle_one_invocation(&addr, echo_handler, std::ptr::null_mut()).unwrap();
    server.join().unwrap();
    let posted = rx.recv().unwrap();
    assert!(posted.starts_with("POST /2018-06-01/runtime/invocation/chunk-utf8/response"));
    assert!(posted.ends_with("echo:{\"name\":\"é\"}"));
}

#[test]
fn lambda_rejects_incomplete_and_malformed_chunked_responses() {
    for (wire, expected) in [
        (b"4\r\nWi".as_slice(), "incomplete chunked HTTP response"),
        (b"4\r\nWikiX\n0\r\n\r\n".as_slice(), "missing its CRLF terminator"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            conn.read_to_end(&mut request).unwrap();
            conn.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").unwrap();
            conn.write_all(wire).unwrap();
        });
        let error = http_request(&addr, "GET", "/2018-06-01/runtime/invocation/next", None)
            .err()
            .expect("malformed chunked response must fail");
        assert!(error.contains(expected), "{error}");
        server.join().unwrap();
    }
}

thread_local! {
    static LAMBDA_TAGGED_ERROR: Cell<*const c_char> = const { Cell::new(std::ptr::null()) };
    static LAMBDA_TAGGED_SLOT: Cell<*mut *const c_char> = const { Cell::new(std::ptr::null_mut()) };
}

extern "C" fn tagged_lambda_error_handler(_: *const c_char) -> *const c_char {
    let error = LAMBDA_TAGGED_ERROR.with(Cell::get);
    LAMBDA_TAGGED_SLOT.with(|slot| unsafe { *slot.get() = error });
    std::ptr::null()
}

#[test]
fn lambda_error_type_uses_public_name_instead_of_internal_ancestry() {
    let _trace_guard = guard_lambda_trace_env();
    for (raw, expected) in [
        (b"\x01\x1eSub\x1fMyError\x1fError\x01boom\0".as_slice(), "Sub"),
        (b"\x01\x1eSub\x1fMyError\x1fError\x01boom\x04Visible\0".as_slice(), "Visible"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (tx, rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            conn.read_to_end(&mut request).unwrap();
            conn.write_all(b"HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: tagged-error\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
            drop(conn);

            let (mut conn, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            conn.read_to_end(&mut request).unwrap();
            tx.send(String::from_utf8_lossy(&request).into_owned()).unwrap();
            conn.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let mut error_slot = std::ptr::null();
        LAMBDA_TAGGED_ERROR.with(|value| value.set(raw.as_ptr().cast()));
        LAMBDA_TAGGED_SLOT.with(|value| value.set(&raw mut error_slot));
        handle_one_invocation(&addr, tagged_lambda_error_handler, &raw mut error_slot).unwrap();
        LAMBDA_TAGGED_SLOT.with(|value| value.set(std::ptr::null_mut()));
        server.join().unwrap();
        let posted = rx.recv().unwrap();
        assert!(posted.starts_with("POST /2018-06-01/runtime/invocation/tagged-error/error"));
        assert!(posted.contains(&format!("{{\"errorMessage\":\"boom\",\"errorType\":\"{expected}\"}}")));
        assert!(!posted.contains("MyError"));
    }
}

#[test]
fn posts_uncaught_handler_exception_to_the_lambda_error_endpoint() {
    let _trace_guard = guard_lambda_trace_env();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = conn.read(&mut request).unwrap();
        conn.write_all(
                b"HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: req-error\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            )
            .unwrap();
        drop(conn);

        let (mut conn, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        conn.read_to_end(&mut request).unwrap();
        tx.send(String::from_utf8_lossy(&request).into_owned())
            .unwrap();
        conn.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });

    handle_one_invocation(&addr, failing_handler, &raw mut TEST_PENDING_EXCEPTION).unwrap();
    server.join().unwrap();
    let request = rx.recv().unwrap();
    assert!(request.starts_with("POST /2018-06-01/runtime/invocation/req-error/error"));
    // An untagged exception string (a plain `throw "..."`, or here the test's
    // own raw error slot) has no class name of its own and defaults to
    // `Error`, matching real JavaScript's own default `Error.prototype.name`.
    assert!(request.contains(r#"{"errorMessage":"handler exploded","errorType":"Error"}"#));
}

static mut TEST_TIMEOUT_EXCEPTION: *const c_char = std::ptr::null();

/// Mimics the generated JSON handler adapter's own rejection path: drive a
/// promise that would only settle far in the future, and if the invocation
/// deadline rejects it first, report that through `error_slot` like a real
/// async handler would.
extern "C" fn slow_async_handler(_: *const c_char) -> *const c_char {
    let promise = timed_value(10_000, 0.0);
    let result = thaw_runtime_run_until_resolved(promise);
    assert_eq!(
        thaw_promise_state(promise),
        2,
        "expected the deadline to reject this handler"
    );
    unsafe { TEST_TIMEOUT_EXCEPTION = result.cast() };
    std::ptr::null()
}

#[test]
fn posts_a_timeout_error_when_the_deadline_header_elapses_before_the_handler_settles() {
    let _trace_guard = guard_lambda_trace_env();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = conn.read(&mut request).unwrap();
        let now_epoch_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let deadline_epoch_ms = now_epoch_ms + 30;
        conn.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: req-timeout\r\nLambda-Runtime-Deadline-Ms: {deadline_epoch_ms}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
            )
            .as_bytes(),
        )
        .unwrap();
        drop(conn);

        let (mut conn, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        conn.read_to_end(&mut request).unwrap();
        tx.send(String::from_utf8_lossy(&request).into_owned())
            .unwrap();
        conn.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });

    handle_one_invocation(&addr, slow_async_handler, &raw mut TEST_TIMEOUT_EXCEPTION).unwrap();
    server.join().unwrap();
    let request = rx.recv().unwrap();
    assert!(request.starts_with("POST /2018-06-01/runtime/invocation/req-timeout/error"));
    assert!(request.contains("Task timed out after 0.03 seconds"));
}

#[test]
fn surfaces_http_error_status_as_an_error() {
    let _trace_guard = guard_lambda_trace_env();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = conn.read(&mut buf).unwrap();
        let body = "boom";
        let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
        conn.write_all(response.as_bytes()).unwrap();
    });

    let err = handle_one_invocation(&addr, echo_handler, std::ptr::null_mut()).unwrap_err();
    assert!(err.contains("500"));
    server.join().unwrap();
}

#[test]
fn idle_drain_includes_work_created_by_rejection_reporters() {
    thread_local! {
        static MODE: Cell<u8> = const { Cell::new(0) };
        static CALLS: Cell<u32> = const { Cell::new(0) };
        static RAN: Cell<bool> = const { Cell::new(false) };
        static CHILDREN: RefCell<Vec<*mut ThawPromise>> = const { RefCell::new(Vec::new()) };
    }
    extern "C" fn resume(_: *mut u8, _: *const u8) {
        RAN.with(|ran| ran.set(true));
    }
    extern "C" fn report(_: *const u8) -> u8 {
        let first = CALLS.with(|calls| { let first = calls.get() == 0; calls.set(calls.get() + 1); first });
        if first {
            let child = thaw_promise_new();
            if MODE.with(Cell::get) == 0 {
                assert_eq!(unsafe { thaw_promise_subscribe(child, resume, std::ptr::null_mut()) }, 1);
                thaw_promise_resolve(child, std::ptr::null());
            } else {
                thaw_promise_reject(child, c"listener-created".as_ptr().cast());
            }
            CHILDREN.with(|children| children.borrow_mut().push(child));
        }
        1
    }
    for mode in [0, 1] {
        MODE.with(|value| value.set(mode));
        CALLS.with(|calls| calls.set(0));
        RAN.with(|ran| ran.set(false));
        thaw_promise_set_unhandled_reporter(Some(report));
        let original = thaw_promise_new();
        thaw_promise_reject(original, c"original".as_ptr().cast());
        assert!(thaw_runtime_run_until_idle() > 0);
        assert_eq!(RAN.with(Cell::get), mode == 0);
        assert_eq!(CALLS.with(Cell::get), if mode == 0 { 1 } else { 2 });
        assert_eq!(thaw_runtime_poll_one(), 0);
        thaw_promise_set_unhandled_reporter(None);
        unsafe { thaw_promise_destroy(original) };
        for child in CHILDREN.with(|children| children.take()) {
            unsafe { thaw_promise_destroy(child) };
        }
    }
}

#[test]
fn result_reporters_keep_multiple_listener_failures_and_latch_status() {
    thread_local! { static REPORTS: Cell<u32> = const { Cell::new(0) }; }
    extern "C" fn failed_listener(_: *const u8) -> PromiseReportResult {
        REPORTS.with(|reports| reports.set(reports.get() + 1));
        PromiseReportResult { value: 1, error: CString::new("listener failed").unwrap().into_raw() }
    }
    extern "C" fn failed_handled_listener() -> PromiseReportResult {
        PromiseReportResult { value: 0, error: CString::new("handled listener failed").unwrap().into_raw() }
    }
    REPORTS.with(|reports| reports.set(0));
    thaw_promise_set_unhandled_reporter_result(Some(failed_listener));
    thaw_promise_set_rejection_handled_reporter_result(Some(failed_handled_listener));
    let first = thaw_promise_new();
    let second = thaw_promise_new();
    thaw_promise_reject(first, c"first".as_ptr().cast());
    thaw_promise_reject(second, c"second".as_ptr().cast());
    assert_eq!(thaw_runtime_poll_one(), 0);
    REPORTS.with(|reports| assert_eq!(reports.get(), 2));
    assert_eq!(thaw_promise_take_unhandled_failure(), 1);
    assert_eq!(unsafe { thaw_promise_mark_handled(first) }, 1);
    assert_eq!(thaw_runtime_poll_one(), 0);
    assert_eq!(thaw_promise_take_unhandled_failure(), 1);
    thaw_promise_set_unhandled_reporter_result(None);
    thaw_promise_set_rejection_handled_reporter_result(None);
    unsafe { thaw_promise_destroy(first); thaw_promise_destroy(second); }
}

#[test]
fn reporter_setters_replace_the_other_abi_including_none() {
    thread_local! {
        static CALLS: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
        static HANDLED_CALLS: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
    }
    extern "C" fn legacy(_: *const u8) -> u8 {
        CALLS.with(|calls| { let (old, new) = calls.get(); calls.set((old + 1, new)); });
        1
    }
    extern "C" fn result(_: *const u8) -> PromiseReportResult {
        CALLS.with(|calls| { let (old, new) = calls.get(); calls.set((old, new + 1)); });
        PromiseReportResult { value: 1, error: std::ptr::null() }
    }
    extern "C" fn handled_legacy() {
        HANDLED_CALLS.with(|calls| { let (old, new) = calls.get(); calls.set((old + 1, new)); });
    }
    extern "C" fn handled_result() -> PromiseReportResult {
        HANDLED_CALLS.with(|calls| { let (old, new) = calls.get(); calls.set((old, new + 1)); });
        PromiseReportResult { value: 0, error: std::ptr::null() }
    }
    CALLS.with(|calls| calls.set((0, 0)));
    HANDLED_CALLS.with(|calls| calls.set((0, 0)));
    thaw_promise_set_unhandled_reporter_result(Some(result));
    thaw_promise_set_unhandled_reporter(Some(legacy));
    assert!(UNHANDLED_REPORTER_RESULT.with(|registered| registered.get().is_none()));
    let first = thaw_promise_new();
    thaw_promise_reject(first, c"first".as_ptr().cast());
    assert_eq!(thaw_runtime_poll_one(), 0);
    thaw_promise_set_unhandled_reporter_result(Some(result));
    assert!(UNHANDLED_REPORTER.with(|registered| registered.get().is_none()));
    let second = thaw_promise_new();
    thaw_promise_reject(second, c"second".as_ptr().cast());
    assert_eq!(thaw_runtime_poll_one(), 0);
    CALLS.with(|calls| assert_eq!(calls.get(), (1, 1)));
    thaw_promise_set_unhandled_reporter(None);
    assert!(UNHANDLED_REPORTER_RESULT.with(|registered| registered.get().is_none()));
    thaw_promise_set_unhandled_reporter(Some(legacy));
    thaw_promise_set_unhandled_reporter_result(None);
    assert!(UNHANDLED_REPORTER.with(|registered| registered.get().is_none()));
    thaw_promise_set_rejection_handled_reporter_result(Some(handled_result));
    thaw_promise_set_rejection_handled_reporter(Some(handled_legacy));
    assert!(REJECTION_HANDLED_REPORTER_RESULT.with(|registered| registered.get().is_none()));
    assert_eq!(unsafe { thaw_promise_mark_handled(first) }, 1);
    assert_eq!(thaw_runtime_poll_one(), 0);
    HANDLED_CALLS.with(|calls| assert_eq!(calls.get(), (1, 0)));
    thaw_promise_set_rejection_handled_reporter_result(Some(handled_result));
    assert!(REJECTION_HANDLED_REPORTER.with(|registered| registered.get().is_none()));
    assert_eq!(unsafe { thaw_promise_mark_handled(second) }, 1);
    assert_eq!(thaw_runtime_poll_one(), 0);
    HANDLED_CALLS.with(|calls| assert_eq!(calls.get(), (1, 1)));
    thaw_promise_set_rejection_handled_reporter(None);
    assert!(REJECTION_HANDLED_REPORTER_RESULT.with(|registered| registered.get().is_none()));
    thaw_promise_set_rejection_handled_reporter(Some(handled_legacy));
    thaw_promise_set_rejection_handled_reporter_result(None);
    assert!(REJECTION_HANDLED_REPORTER.with(|registered| registered.get().is_none()));
    unsafe { thaw_promise_destroy(first); thaw_promise_destroy(second); }
    assert_eq!(thaw_promise_take_unhandled_failure(), 0);
}

#[test]
fn reporter_drain_snapshots_later_rejection_before_callback_destroys_its_owner() {
    thread_local! {
        static LATER: Cell<(*mut ThawPromise, *mut c_char)> = const { Cell::new((std::ptr::null_mut(), std::ptr::null_mut())) };
        static SEEN: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
    }
    fn record(error: *const u8) {
        SEEN.with(|seen| seen.borrow_mut().push(unsafe { CStr::from_ptr(error.cast()).to_bytes().to_vec() }));
        if unsafe { CStr::from_ptr(error.cast()).to_bytes() } == b"first" {
            LATER.with(|later| {
                let (promise, message) = later.replace((std::ptr::null_mut(), std::ptr::null_mut()));
                unsafe { thaw_promise_destroy(promise); thaw_arena::destroy_string(message); }
            });
        }
    }
    extern "C" fn legacy(error: *const u8) -> u8 { record(error); 1 }
    extern "C" fn result(error: *const u8) -> PromiseReportResult {
        record(error);
        PromiseReportResult { value: 1, error: std::ptr::null() }
    }
    for result_abi in [false, true] {
        SEEN.with(|seen| seen.borrow_mut().clear());
        let first = thaw_promise_new();
        let second = thaw_promise_new();
        let later_error = thaw_arena::owned_string(b"second");
        thaw_promise_reject(first, c"first".as_ptr().cast());
        reject_native_text(second, later_error.cast());
        LATER.with(|later| later.set((second, later_error)));
        assert_eq!(if result_abi {
            thaw_promise_drain_unhandled_result(Some(result))
        } else {
            thaw_promise_drain_unhandled(Some(legacy))
        }, 0);
        SEEN.with(|seen| assert_eq!(&*seen.borrow(), &[b"first".to_vec(), b"second".to_vec()]));
        unsafe { thaw_promise_destroy(first) };
    }
}

#[test]
fn opaque_promise_rejection_is_forwarded_without_string_dereference() {
    thread_local! { static EXPECTED: Cell<*const u8> = const { Cell::new(std::ptr::null()) }; }
    extern "C" fn legacy(error: *const u8) -> u8 {
        EXPECTED.with(|expected| assert_eq!(error, expected.get()));
        1
    }
    extern "C" fn result(error: *const u8) -> PromiseReportResult {
        EXPECTED.with(|expected| assert_eq!(error, expected.get()));
        PromiseReportResult { value: 1, error: std::ptr::null() }
    }
    let value = 99u8;
    EXPECTED.with(|expected| expected.set(&value));
    for result_abi in [false, true] {
        let promise = thaw_promise_new();
        assert_eq!(thaw_promise_reject(promise, &value), 1);
        assert_eq!(if result_abi {
            thaw_promise_drain_unhandled_result(Some(result))
        } else {
            thaw_promise_drain_unhandled(Some(legacy))
        }, 0);
        unsafe { thaw_promise_destroy(promise) };
    }
}

#[test]
fn unhandled_opaque_promise_rejections_use_safe_fallback_for_every_reporter_abi() {
    thread_local! { static EXPECTED: Cell<*const u8> = const { Cell::new(std::ptr::null()) }; }
    extern "C" fn unhandled(error: *const u8) -> u8 {
        EXPECTED.with(|expected| assert_eq!(error, expected.get()));
        0
    }
    extern "C" fn unhandled_result(error: *const u8) -> PromiseReportResult {
        EXPECTED.with(|expected| assert_eq!(error, expected.get()));
        PromiseReportResult { value: 0, error: std::ptr::null() }
    }
    extern "C" fn throwing(error: *const u8) -> PromiseReportResult {
        EXPECTED.with(|expected| assert_eq!(error, expected.get()));
        PromiseReportResult { value: 1, error: thaw_arena::owned_string("listener failed") }
    }
    let value = 99u8;
    EXPECTED.with(|expected| expected.set(&value));
    let diagnostic = unhandled_rejection_report_text(&value, false);
    assert_eq!(unsafe { CStr::from_ptr(diagnostic) }.to_bytes(), b"Unhandled opaque Promise rejection");
    assert_ne!(diagnostic.cast::<u8>(), &value as *const u8);
    for mode in 0..4 {
        let promise = thaw_promise_new();
        assert_eq!(thaw_promise_reject(promise, &value), 1);
        let failed = match mode {
            0 => thaw_promise_drain_unhandled(None),
            1 => thaw_promise_drain_unhandled(Some(unhandled)),
            2 => thaw_promise_drain_unhandled_result(Some(unhandled_result)),
            _ => thaw_promise_drain_unhandled_result(Some(throwing)),
        };
        assert_eq!(failed, 1);
        unsafe { thaw_promise_destroy(promise) };
    }
    assert_eq!(thaw_promise_take_unhandled_failure(), 1);
}

#[test]
fn generated_native_text_and_promise_forwarding_preserve_diagnostic_provenance() {
    let source = thaw_promise_new();
    let output = thaw_promise_new();
    let error = c"\u{1}TypeError\u{1}Invalid native callback graph";
    assert_eq!(unsafe { thaw_promise_reject_native_text(source, error.as_ptr().cast()) }, 1);
    assert_eq!(unsafe { thaw_promise_forward_rejection(output, source, error.as_ptr().cast()) }, 1);
    assert_eq!(unsafe { (*output).rejection_text.as_deref() }, Some(error.to_bytes()));
    unsafe { thaw_promise_destroy(source) };
    assert_eq!(unsafe { (*output).rejection_text.as_deref() }, Some(error.to_bytes()));
    let copied = unsafe { thaw_promise_exception_native_text_copy(output) };
    unsafe { thaw_promise_destroy(output) };
    assert_eq!(unsafe { CStr::from_ptr(copied.cast()) }.to_bytes(), error.to_bytes());

    let typed_native = thaw_promise_new();
    assert_eq!(unsafe { thaw_promise_reject_typed_with_native_text(
        typed_native, error.as_ptr().cast(), 1, 0.0, 0, false,
        std::ptr::null(), error.as_ptr().cast(),
    ) }, 1);
    assert_eq!(unsafe { (*typed_native).rejection_text.as_deref() }, Some(error.to_bytes()));
    unsafe { thaw_promise_destroy(typed_native) };

    let scalar = 99u8;
    let opaque = thaw_promise_new();
    assert_eq!(unsafe { thaw_promise_reject_typed_with_native_text(
        opaque, &scalar, 1, 0.0, 0, false, std::ptr::null(), error.as_ptr().cast(),
    ) }, 1);
    assert!(unsafe { (*opaque).rejection_text.is_none() });
    unsafe { thaw_promise_destroy(opaque) };
}

#[test]
fn finally_adopt_snapshots_original_native_text_before_source_destruction() {
    let source = thaw_promise_new();
    let returned = thaw_promise_new();
    let output = thaw_promise_new();
    let error = c"\u{1}TypeError\u{1}original native failure";
    assert_eq!(unsafe { thaw_promise_reject_native_text(source, error.as_ptr().cast()) }, 1);
    assert_eq!(unsafe { thaw_promise_finally_adopt_with_source(
        output, returned, error.as_ptr().cast(), 1, 1, 0.0, 0, false,
        std::ptr::null(), source,
    ) }, 1);
    unsafe { thaw_promise_destroy(source) };
    assert_eq!(thaw_promise_resolve(returned, std::ptr::null()), 1);
    thaw_runtime_run_until_idle();
    assert_eq!(unsafe { thaw_promise_state(output) }, 2);
    assert_eq!(unsafe { (*output).rejection_text.as_deref() }, Some(error.to_bytes()));
    unsafe { thaw_promise_destroy(output) };
}

#[test]
fn terminal_reporting_activity_is_distinct_from_work_and_failure() {
    thaw_promise_take_report_activity();
    thaw_promise_take_unhandled_failure();
    thaw_promise_set_unhandled_reporter(Some(record_unhandled_rejection));
    let promise = thaw_promise_new();
    thaw_promise_reject(promise, c"reported".as_ptr().cast());
    // Reporting a settled rejection need not run a continuation.
    assert_eq!(thaw_runtime_run_until_idle(), 0);
    assert_eq!(thaw_promise_take_report_activity(), 1);
    assert_eq!(thaw_promise_take_report_activity(), 0);
    assert_eq!(thaw_promise_take_unhandled_failure(), 0);
    unsafe { thaw_promise_destroy(promise) };
    thaw_promise_set_unhandled_reporter(None);
}

#[test]
fn future_native_timer_remains_pending_after_an_idle_drain() {
    let timer = thaw_sleep_ms(5);
    assert_eq!(thaw_runtime_run_until_idle(), 0);
    assert_eq!(thaw_runtime_async_work_pending(), 1);
    thaw_runtime_run_until_resolved(timer);
    assert_eq!(thaw_runtime_async_work_pending(), 0);
    unsafe { thaw_promise_destroy(timer) };
}

#[test]
fn terminal_exception_report_uses_provenance_or_typed_scalar_without_reading_opaque_pointer() {
    let opaque = 1usize as *const std::os::raw::c_char;
    let scalar = unsafe { thaw_runtime_exception_report_text(
        opaque, std::ptr::null(), 2, 0.0, 42, false,
    ) };
    assert_eq!(unsafe { thaw_arena::NativeStr::from_ptr(scalar) }.to_bytes(), b"42");
    unsafe { thaw_arena::destroy_string(scalar) };

    let native = thaw_arena::owned_string(b"a\0b");
    let copy = unsafe { thaw_runtime_exception_report_text(
        native, native, 4, 0.0, 0, false,
    ) };
    assert_eq!(unsafe { thaw_arena::NativeStr::from_ptr(copy) }.to_bytes(), b"a\0b");
    unsafe {
        thaw_arena::destroy_string(copy);
        thaw_arena::destroy_string(native);
    }
}

#[test]
fn terminal_and_detached_f64_reports_use_javascript_number_spelling() {
    let cases = [
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
        (-0.0, "0"),
        (1e20, "100000000000000000000"),
        (1e21, "1e+21"),
        (1e-6, "0.000001"),
        (1e-7, "1e-7"),
    ];
    for (value, expected) in cases {
        let opaque = 1usize as *const u8;
        let terminal = unsafe { thaw_runtime_exception_report_text(
            opaque.cast(), std::ptr::null(), 1, value, 0, false,
        ) };
        assert_eq!(unsafe { thaw_arena::NativeStr::from_ptr(terminal) }.to_bytes(), expected.as_bytes());
        unsafe { thaw_arena::destroy_string(terminal) };

        let promise = thaw_promise_new();
        assert_eq!(unsafe { thaw_promise_reject_typed(
            promise, opaque, 1, value, 0, false, std::ptr::null(),
        ) }, 1);
        assert_eq!(promise_report_bytes(unsafe { &*promise }).as_slice(), expected.as_bytes());
        let mut pending: *const u8 = std::ptr::null();
        assert_eq!(unsafe { thaw_promise_detach_for_report(promise, &mut pending) }, 1);
        thaw_runtime_run_until_idle();
        assert_eq!(unsafe { thaw_arena::NativeStr::from_ptr(pending.cast()) }.to_bytes(), expected.as_bytes());
        unsafe { thaw_arena::destroy_string(pending.cast_mut().cast()) };
    }
}

extern "C" fn lambda_trace_env_handler(_: *const c_char) -> *const c_char {
    let trace = std::env::var("_X_AMZN_TRACE_ID").unwrap_or_else(|_| "<absent>".into());
    CString::new(trace).unwrap().into_raw()
}

#[test]
fn lambda_sets_and_clears_the_trace_environment_per_invocation() {
    let _trace_guard = guard_lambda_trace_env();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for (index, trace) in [Some("Root=first"), Some("Root=second"), None].into_iter().enumerate() {
            let (mut conn, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            conn.read_to_end(&mut request).unwrap();
            assert!(request.starts_with(b"GET /2018-06-01/runtime/invocation/next"));
            let trace_header = trace.map(|value| format!("Lambda-Runtime-Trace-Id: {value}\r\n")).unwrap_or_default();
            conn.write_all(format!(
                "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: trace-{index}\r\n{trace_header}Content-Length: 2\r\nConnection: close\r\n\r\n{}",
                "{}",
            ).as_bytes()).unwrap();
            drop(conn);

            let (mut conn, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            conn.read_to_end(&mut request).unwrap();
            tx.send(String::from_utf8_lossy(&request).into_owned()).unwrap();
            conn.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        }
    });
    for expected in ["Root=first", "Root=second", "<absent>"] {
        handle_one_invocation(&addr, lambda_trace_env_handler, std::ptr::null_mut()).unwrap();
        assert!(rx.recv().unwrap().ends_with(expected));
    }
    server.join().unwrap();
    assert!(std::env::var_os("_X_AMZN_TRACE_ID").is_none());
}

#[test]
fn lambda_rejects_nul_trace_header_before_handler() {
    let _trace_guard = guard_lambda_trace_env();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        conn.read_to_end(&mut request).unwrap();
        conn.write_all(b"HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: invalid-trace\r\nLambda-Runtime-Trace-Id: Root=bad\0value\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
    });
    let error = handle_one_invocation(&addr, lambda_trace_env_handler, std::ptr::null_mut()).unwrap_err();
    assert!(error.contains("invalid trace id header"), "{error}");
    server.join().unwrap();
}

// Source-only precursor coverage: the HIR accessor/cast stage is separate.
unsafe fn promise_any_reason_at(handle: *const u8, index: usize) -> (u8, u64) {
    let buffer = unsafe { handle.cast::<*const u8>().read_unaligned() };
    let slot = unsafe { buffer.add(8 + 16 * index) };
    unsafe { (slot.read(), slot.add(8).cast::<u64>().read_unaligned()) }
}

#[test]
fn promise_any_reason_slots_keep_input_order_duplicate_positions_and_empty_array() {
    let first = thaw_promise_new();
    let second = thaw_promise_new();
    let children = [first, second, first];
    let any = unsafe { thaw_promise_any(children.as_ptr(), children.len()) };
    assert_eq!(unsafe { thaw_promise_reject_typed(second, 1usize as *const u8, 3, 0.0, 0, true, std::ptr::null()) }, 1);
    thaw_runtime_run_until_idle();
    assert_eq!(unsafe { thaw_promise_state(any) }, 0);
    assert_eq!(unsafe { thaw_promise_reject_typed(first, 1usize as *const u8, 1, 42.5, 0, false, std::ptr::null()) }, 1);
    thaw_runtime_run_until_idle();
    assert_eq!(unsafe { thaw_promise_state(any) }, 2);
    let errors = unsafe { thaw_promise_exception_aggregate_errors(any) };
    assert!(!errors.is_null());
    let buffer = unsafe { errors.cast::<*const u8>().read_unaligned() };
    assert_eq!(unsafe { buffer.cast::<u64>().read_unaligned() }, 3);
    assert_eq!(unsafe { promise_any_reason_at(errors, 0) }, (0, 42.5f64.to_bits()));
    assert_eq!(unsafe { promise_any_reason_at(errors, 1) }, (2, 1));
    assert_eq!(unsafe { promise_any_reason_at(errors, 2) }, (0, 42.5f64.to_bits()));
    unsafe { thaw_promise_destroy(any) };

    let empty = unsafe { thaw_promise_any(std::ptr::null(), 0) };
    let errors = unsafe { thaw_promise_exception_aggregate_errors(empty) };
    assert!(!errors.is_null());
    let buffer = unsafe { errors.cast::<*const u8>().read_unaligned() };
    assert_eq!(unsafe { buffer.cast::<u64>().read_unaligned() }, 0);
    unsafe { thaw_promise_destroy(empty) };
}

#[test]
fn promise_any_nested_reason_and_native_text_outlive_child_handles() {
    let leaf = thaw_promise_new();
    let inner = unsafe { thaw_promise_any([leaf].as_ptr(), 1) };
    let native = thaw_promise_new();
    let outer = unsafe { thaw_promise_any([inner, native, inner].as_ptr(), 3) };
    let text = b"temporary native failure\0".to_vec();
    assert_eq!(unsafe { thaw_promise_reject_native_text(native, text.as_ptr()) }, 1);
    drop(text);
    assert_eq!(unsafe { thaw_promise_reject_typed(leaf, 1usize as *const u8, 2, 0.0, 77, false, std::ptr::null()) }, 1);
    thaw_runtime_run_until_idle();
    assert_eq!(unsafe { thaw_promise_state(outer) }, 2);
    let errors = unsafe { thaw_promise_exception_aggregate_errors(outer) };
    let (first_tag, first_payload) = unsafe { promise_any_reason_at(errors, 0) };
    let (native_tag, native_payload) = unsafe { promise_any_reason_at(errors, 1) };
    assert_eq!(first_tag, 7);
    assert_eq!(unsafe { promise_any_reason_at(errors, 2) }, (first_tag, first_payload));
    let nested = unsafe { &*(first_payload as *const PromiseAnyErrorRecord) };
    assert_eq!(unsafe { CStr::from_ptr(nested.text.cast()) }.to_bytes(), &PROMISE_ANY_REJECTED_ERROR[..PROMISE_ANY_REJECTED_ERROR.len() - 1]);
    assert_eq!(unsafe { promise_any_reason_at(nested.errors, 0) }, (1, 77));
    assert_eq!(native_tag, 9);
    let native = unsafe { &*(native_payload as *const PromiseAnyErrorRecord) };
    assert_eq!(unsafe { CStr::from_ptr(native.text.cast()) }.to_bytes(), b"temporary native failure");
    assert!(native.errors.is_null());
    unsafe { thaw_promise_destroy(outer) };
}

#[test]
fn promise_any_forwarding_and_finally_keep_only_successfully_settled_metadata() {
    let child = thaw_promise_new();
    let aggregate = unsafe { thaw_promise_any([child].as_ptr(), 1) };
    assert_eq!(unsafe { thaw_promise_reject_typed(child, 1usize as *const u8, 5, 0.0, 0, false, std::ptr::null()) }, 1);
    thaw_runtime_run_until_idle();
    let errors = unsafe { thaw_promise_exception_aggregate_errors(aggregate) };
    let forwarded = thaw_promise_new();
    assert_eq!(unsafe { thaw_promise_forward_rejection(forwarded, aggregate, PROMISE_ANY_REJECTED_ERROR.as_ptr()) }, 1);
    assert_eq!(unsafe { thaw_promise_exception_aggregate_errors(forwarded) }, errors);
    assert_eq!(unsafe { thaw_promise_forward_rejection(forwarded, aggregate, PROMISE_ANY_REJECTED_ERROR.as_ptr()) }, 0);
    assert_eq!(unsafe { thaw_promise_exception_aggregate_errors(forwarded) }, errors);

    let returned = thaw_promise_new();
    let after_finally = thaw_promise_new();
    assert_eq!(unsafe { thaw_promise_finally_adopt_with_source(
        after_finally, returned, PROMISE_ANY_REJECTED_ERROR.as_ptr(), 1,
        0, 0.0, 0, false, std::ptr::null(), aggregate,
    ) }, 1);
    unsafe { thaw_promise_destroy(aggregate) };
    assert_eq!(thaw_promise_resolve(returned, std::ptr::null()), 1);
    thaw_runtime_run_until_idle();
    assert_eq!(unsafe { thaw_promise_exception_aggregate_errors(after_finally) }, errors);
    unsafe { thaw_promise_destroy(forwarded); thaw_promise_destroy(after_finally) };
}

#[test]
fn compiler_aggregate_rejection_abi_attaches_only_on_first_settlement() {
    let promise = thaw_promise_new();
    let first = 1u8;
    let second = 2u8;
    assert_eq!(unsafe { thaw_promise_reject_typed_with_aggregate(
        promise, std::ptr::null(), 0, 0.0, 0, false, std::ptr::null(),
        std::ptr::null(), &first,
    ) }, 1);
    assert_eq!(unsafe { thaw_promise_reject_typed_with_aggregate(
        promise, std::ptr::null(), 0, 0.0, 0, false, std::ptr::null(),
        std::ptr::null(), &second,
    ) }, 0);
    assert_eq!(unsafe { thaw_promise_exception_aggregate_errors(promise) }, &first);
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn promise_any_reason_slots_keep_object_pointer_null_and_opaque_separate() {
    let object = 17u8;
    let object_child = thaw_promise_new();
    let null_child = thaw_promise_new();
    let opaque_child = thaw_promise_new();
    let children = [object_child, null_child, opaque_child];
    let any = unsafe { thaw_promise_any(children.as_ptr(), children.len()) };
    assert_eq!(unsafe { thaw_promise_reject_typed(
        object_child, 1usize as *const u8, 0, 0.0, 0, false, &object,
    ) }, 1);
    assert_eq!(unsafe { thaw_promise_reject_typed(
        null_child, 1usize as *const u8, 6, 0.0, 0, false, std::ptr::null(),
    ) }, 1);
    assert_eq!(thaw_promise_reject(opaque_child, 1usize as *const u8), 1);
    thaw_runtime_run_until_idle();
    let errors = unsafe { thaw_promise_exception_aggregate_errors(any) };
    assert_eq!(unsafe { promise_any_reason_at(errors, 0) }, (6, &object as *const u8 as u64));
    assert_eq!(unsafe { promise_any_reason_at(errors, 1) }, (5, 0));
    assert_eq!(unsafe { promise_any_reason_at(errors, 2) }, (8, 1));
    unsafe { thaw_promise_destroy(any) };
}

#[test]
fn explicit_null_exception_tag_reports_null_without_reading_an_opaque_pointer() {
    let opaque = 1usize as *const u8;
    let terminal = unsafe { thaw_runtime_exception_report_text(
        opaque.cast(), std::ptr::null(), 6, 0.0, 0, false,
    ) };
    assert_eq!(unsafe { thaw_arena::NativeStr::from_ptr(terminal) }.to_bytes(), b"null");
    unsafe { thaw_arena::destroy_string(terminal) };

    let promise = thaw_promise_new();
    assert_eq!(unsafe { thaw_promise_reject_typed(
        promise, opaque, 6, 0.0, 0, false, std::ptr::null(),
    ) }, 1);
    assert_eq!(promise_report_bytes(unsafe { &*promise }).as_slice(), b"null");
    unsafe { thaw_promise_destroy(promise) };
}

#[test]
fn promise_any_errors_handle_is_rooted_with_live_promise() {
    thaw_arena::thaw_arena_enable_tracing();
    let promise = thaw_promise_new();
    let root = thaw_arena::ArenaRoot::new(promise as usize);
    let errors = promise_any_errors(&[]);
    assert!(!errors.is_null());
    let text = c"AggregateError".as_ptr().cast::<u8>();
    assert_eq!(unsafe { thaw_promise_reject_typed_with_aggregate(
        promise, text, 0, 0.0, 0, false, std::ptr::null(), text, errors,
    ) }, 1);
    thaw_arena::thaw_arena_reset();
    assert!(!thaw_arena::was_reclaimed(errors as usize));
    assert_eq!(unsafe { thaw_promise_exception_aggregate_errors(promise) }, errors);
    unsafe { thaw_promise_destroy(promise) };
    drop(root);
    thaw_arena::thaw_arena_reset();
    assert!(thaw_arena::was_reclaimed(errors as usize));
}
