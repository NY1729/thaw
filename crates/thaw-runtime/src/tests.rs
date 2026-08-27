use super::*;
use std::net::TcpListener;
use std::process::Command;

use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

static TLS_TEST_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    let keys = unsafe { thaw_array_keys(array.as_ptr().cast()) };
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
    let result = unsafe { *thaw_runtime_run_until_resolved(joined).cast::<*const u64>() };
    assert_eq!(unsafe { result.read() }, 2);
    assert_eq!(f64::from_bits(unsafe { result.add(1).read() }), 3.0);
    assert_eq!(f64::from_bits(unsafe { result.add(2).read() }), 7.0);
    unsafe { thaw_promise_destroy(joined) };

    let empty = unsafe { thaw_promise_all_f64(std::ptr::null(), 0) };
    assert_eq!(thaw_promise_state(empty), 1);
    let result = unsafe { *thaw_runtime_run_until_resolved(empty).cast::<*const u64>() };
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
    let result = unsafe { *thaw_runtime_run_until_resolved(joined).cast::<*const u64>() };
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
    let result = unsafe { *thaw_runtime_run_until_resolved(joined).cast::<*const u8>() };
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
fn promise_race_rejects_an_empty_input() {
    let raced = unsafe { thaw_promise_race(std::ptr::null(), 0) };
    assert_eq!(thaw_promise_state(raced), 2);
    assert_eq!(
        thaw_runtime_run_until_resolved(raced),
        PROMISE_RACE_EMPTY_ERROR.as_ptr()
    );
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
    let result = unsafe { *thaw_runtime_run_until_resolved(settled).cast::<*const u64>() };
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
    let result = unsafe { *thaw_runtime_run_until_resolved(settled).cast::<*const u64>() };
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
    let result = unsafe { *thaw_runtime_run_until_resolved(settled).cast::<*const u64>() };
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
    let result = unsafe { *thaw_runtime_run_until_resolved(joined).cast::<*const u64>() };
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
fn incrementally_decodes_chunked_response() {
    let partial = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWi";
    assert!(parse_http_response(partial, false).unwrap().is_none());
    let complete = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4;name=value\r\nWiki\r\n5\r\npedia\r\n0\r\nX-End: yes\r\n\r\n";
    let parsed = parse_http_response(complete, false).unwrap().unwrap();
    assert_eq!(parsed.body, b"Wikipedia");
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
fn posts_uncaught_handler_exception_to_the_lambda_error_endpoint() {
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
    assert!(request.contains(r#"{"errorMessage":"handler exploded","errorType":"ThawError"}"#));
}

#[test]
fn surfaces_http_error_status_as_an_error() {
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
