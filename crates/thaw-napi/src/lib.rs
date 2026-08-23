//! Minimal synchronous Node-API host for loading `.node` addons.

use libc::{c_char, c_void};
use serde_json::Value as JsonValue;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::io::Write;
#[cfg(target_os = "linux")]
use std::os::fd::FromRawFd;
use std::ptr;

type NapiEnv = *mut Env;
type NapiValue = *mut Value;
type NapiCallbackInfo = *mut CallbackInfo;
type NapiStatus = i32;
type NapiCallback = unsafe extern "C" fn(NapiEnv, NapiCallbackInfo) -> NapiValue;
const NAPI_OK: NapiStatus = 0;
const NAPI_INVALID_ARG: NapiStatus = 1;
const NAPI_NUMBER_EXPECTED: NapiStatus = 6;
const NAPI_STRING_EXPECTED: NapiStatus = 3;
const NAPI_BOOLEAN_EXPECTED: NapiStatus = 7;
const NAPI_AUTO_LENGTH: usize = usize::MAX;

#[derive(Clone)]
pub struct Function {
    callback: NapiCallback,
    data: *mut c_void,
}

pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Object(HashMap<String, NapiValue>),
    Array(Vec<NapiValue>),
    Buffer(Vec<u8>),
    Function(Function),
    Error(String),
}

pub struct Env {
    values: Vec<NapiValue>,
    exception: Option<NapiValue>,
}

impl Env {
    fn new() -> Self {
        Self {
            values: Vec::new(),
            exception: None,
        }
    }

    fn alloc(&mut self, value: Value) -> NapiValue {
        let value = Box::into_raw(Box::new(value));
        self.values.push(value);
        value
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for value in self.values.drain(..) {
            unsafe {
                drop(Box::from_raw(value));
            }
        }
    }
}

pub struct CallbackInfo {
    args: Vec<NapiValue>,
    this_arg: NapiValue,
    data: *mut c_void,
}

struct Host {
    functions: HashMap<String, Function>,
    libraries: Vec<*mut c_void>,
    module_envs: Vec<Box<Env>>,
    last_error: String,
}

impl Host {
    fn new() -> Self {
        Self {
            functions: HashMap::new(),
            libraries: Vec::new(),
            module_envs: Vec::new(),
            last_error: String::new(),
        }
    }
}

thread_local! {
    static HOST: RefCell<Host> = RefCell::new(Host::new());
    static PENDING_MODULE: RefCell<Option<NapiModule>> = const { RefCell::new(None) };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NapiModule {
    nm_version: i32,
    nm_flags: u32,
    nm_filename: *const c_char,
    nm_register_func: Option<unsafe extern "C" fn(NapiEnv, NapiValue) -> NapiValue>,
    nm_modname: *const c_char,
    nm_priv: *mut c_void,
    reserved: [*mut c_void; 4],
}

#[repr(C)]
pub struct ThawResult {
    pub value: *mut c_char,
    pub error: *mut c_char,
}

unsafe fn text(ptr: *const c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Err("null string pointer".into());
    }
    Ok(CStr::from_ptr(ptr).to_string_lossy().into_owned())
}

unsafe fn env_mut<'a>(env: NapiEnv) -> Result<&'a mut Env, NapiStatus> {
    env.as_mut().ok_or(NAPI_INVALID_ARG)
}

unsafe fn value_ref<'a>(value: NapiValue) -> Result<&'a Value, NapiStatus> {
    value.as_ref().ok_or(NAPI_INVALID_ARG)
}

unsafe fn write_value(out: *mut NapiValue, value: NapiValue) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    *out = value;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_module_register(module: *mut NapiModule) {
    if let Some(module) = module.as_ref() {
        PENDING_MODULE.with(|slot| *slot.borrow_mut() = Some(*module));
    }
}

type RegisterV1 = unsafe extern "C" fn(NapiEnv, NapiValue) -> NapiValue;

unsafe fn load_impl(path: &str, root_name: Option<&str>) -> Result<(), String> {
    let path = CString::new(path).map_err(|_| "addon path contains NUL".to_string())?;
    PENDING_MODULE.with(|slot| slot.borrow_mut().take());
    libc::dlerror();
    let handle = libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
    if handle.is_null() {
        let error = libc::dlerror();
        return Err(if error.is_null() {
            "dlopen failed".into()
        } else {
            CStr::from_ptr(error).to_string_lossy().into_owned()
        });
    }
    let direct = libc::dlsym(handle, b"napi_register_module_v1\0".as_ptr().cast());
    let registered = PENDING_MODULE.with(|slot| slot.borrow_mut().take());
    let init: RegisterV1 = if !direct.is_null() {
        std::mem::transmute(direct)
    } else if let Some(module) = registered {
        module
            .nm_register_func
            .ok_or("registered N-API module has no init function")?
    } else {
        libc::dlclose(handle);
        return Err(
            "addon exports neither napi_register_module_v1 nor a registered N-API module".into(),
        );
    };

    let mut env = Box::new(Env::new());
    let env_ptr = &mut *env as NapiEnv;
    let exports = env.alloc(Value::Object(HashMap::new()));
    let returned = init(env_ptr, exports);
    let exports = if returned.is_null() {
        exports
    } else {
        returned
    };
    if let Some(exception) = env.exception {
        let message = json_from_value(exception)?.to_string();
        libc::dlclose(handle);
        return Err(format!("addon initialization threw: {message}"));
    }
    let mut functions = Vec::new();
    match value_ref(exports).map_err(|_| "invalid exports value")? {
        Value::Object(object) => {
            for (name, value) in object {
                if let Value::Function(function) =
                    value_ref(*value).map_err(|_| "invalid export value")?
                {
                    functions.push((name.clone(), function.clone()));
                }
            }
        }
        Value::Function(function) => {
            functions.push((root_name.unwrap_or("default").to_string(), function.clone()))
        }
        _ => return Err("addon initialization returned neither an object nor a function".into()),
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.functions.extend(functions);
        host.libraries.push(handle);
        host.module_envs.push(env);
    });
    Ok(())
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_load(path: *const c_char) -> u8 {
    match text(path).and_then(|path| load_impl(&path, None)) {
        Ok(()) => 1,
        Err(error) => {
            HOST.with(|host| host.borrow_mut().last_error = error.clone());
            eprintln!("thaw-napi: {error}");
            0
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_load_named(path: *const c_char, root_name: *const c_char) -> u8 {
    let result = text(path)
        .and_then(|path| text(root_name).and_then(|root_name| load_impl(&path, Some(&root_name))));
    match result {
        Ok(()) => 1,
        Err(error) => {
            HOST.with(|host| host.borrow_mut().last_error = error.clone());
            eprintln!("thaw-napi: {error}");
            0
        }
    }
}

fn decode_hex(input: &str) -> Result<Vec<u8>, String> {
    if input.len() % 2 != 0 {
        return Err("embedded addon hex has an odd length".into());
    }
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|error| error.to_string())?;
            u8::from_str_radix(text, 16).map_err(|error| error.to_string())
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn load_embedded_impl(bytes: &[u8], root_name: Option<&str>) -> Result<(), String> {
    let name = CString::new("thaw-native-addon").unwrap();
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(format!(
            "memfd_create failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(bytes)
        .map_err(|error| format!("failed to write embedded addon: {error}"))?;
    unsafe { load_impl(&format!("/proc/self/fd/{fd}"), root_name) }
}

#[cfg(not(target_os = "linux"))]
fn load_embedded_impl(bytes: &[u8], root_name: Option<&str>) -> Result<(), String> {
    let path = std::env::temp_dir().join(format!("thaw-native-addon-{}.node", std::process::id()));
    std::fs::write(&path, bytes)
        .map_err(|error| format!("failed to write embedded addon: {error}"))?;
    unsafe { load_impl(&path.to_string_lossy(), root_name) }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_load_embedded_hex(
    hex: *const c_char,
    root_name: *const c_char,
) -> u8 {
    let result = text(hex).and_then(|hex| {
        let bytes = decode_hex(&hex)?;
        let root_name = text(root_name)?;
        let root_name = (!root_name.is_empty()).then_some(root_name.as_str());
        load_embedded_impl(&bytes, root_name)
    });
    match result {
        Ok(()) => 1,
        Err(error) => {
            HOST.with(|host| host.borrow_mut().last_error = error.clone());
            eprintln!("thaw-napi: {error}");
            0
        }
    }
}

fn value_from_json(env: &mut Env, json: &JsonValue) -> NapiValue {
    match json {
        JsonValue::Null => env.alloc(Value::Null),
        JsonValue::Bool(value) => env.alloc(Value::Bool(*value)),
        JsonValue::Number(value) => env.alloc(Value::Number(value.as_f64().unwrap_or(0.0))),
        JsonValue::String(value) => env.alloc(Value::String(value.clone())),
        JsonValue::Array(values) => {
            let values = values
                .iter()
                .map(|value| value_from_json(env, value))
                .collect();
            env.alloc(Value::Array(values))
        }
        JsonValue::Object(values) => {
            if values.get("type").and_then(JsonValue::as_str) == Some("Buffer") {
                if let Some(bytes) = values.get("data").and_then(JsonValue::as_array) {
                    return env.alloc(Value::Buffer(
                        bytes
                            .iter()
                            .map(|value| value.as_u64().unwrap_or(0) as u8)
                            .collect(),
                    ));
                }
            }
            let values = values
                .iter()
                .map(|(key, value)| (key.clone(), value_from_json(env, value)))
                .collect();
            env.alloc(Value::Object(values))
        }
    }
}

unsafe fn json_from_value(value: NapiValue) -> Result<JsonValue, String> {
    Ok(match value_ref(value).map_err(|_| "invalid napi_value")? {
        Value::Undefined => JsonValue::Null,
        Value::Null => JsonValue::Null,
        Value::Bool(value) => JsonValue::Bool(*value),
        Value::Number(value) => serde_json::Number::from_f64(*value)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Value::String(value) | Value::Error(value) => JsonValue::String(value.clone()),
        Value::Array(values) => JsonValue::Array(
            values
                .iter()
                .map(|value| json_from_value(*value))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(values) => JsonValue::Object(
            values
                .iter()
                .map(|(key, value)| Ok((key.clone(), json_from_value(*value)?)))
                .collect::<Result<_, String>>()?,
        ),
        Value::Buffer(values) => {
            JsonValue::Array(values.iter().map(|value| JsonValue::from(*value)).collect())
        }
        Value::Function(_) => return Err("cannot JSON-encode a function".into()),
    })
}

unsafe fn call_impl(name: &str, args_json: &str) -> Result<String, String> {
    let args: Vec<JsonValue> = serde_json::from_str(args_json)
        .map_err(|error| format!("invalid argument JSON: {error}"))?;
    let function = HOST
        .with(|host| host.borrow().functions.get(name).cloned())
        .ok_or_else(|| format!("no such native addon function `{name}`"))?;
    let mut env = Box::new(Env::new());
    let args = args
        .iter()
        .map(|value| value_from_json(&mut env, value))
        .collect();
    let this_arg = env.alloc(Value::Undefined);
    let mut info = CallbackInfo {
        args,
        this_arg,
        data: function.data,
    };
    let result = (function.callback)(&mut *env, &mut info);
    if let Some(exception) = env.exception {
        let message = match value_ref(exception).map_err(|_| "invalid exception")? {
            Value::Error(message) | Value::String(message) => message.clone(),
            _ => json_from_value(exception)?.to_string(),
        };
        return Err(message);
    }
    serde_json::to_string(&json_from_value(result)?).map_err(|error| error.to_string())
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_result(
    name: *const c_char,
    args: *const c_char,
) -> ThawResult {
    let result = text(name).and_then(|name| text(args).and_then(|args| call_impl(&name, &args)));
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap().into_raw(),
            error: ptr::null_mut(),
        },
        Err(error) => ThawResult {
            value: ptr::null_mut(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call(name: *const c_char, args: *const c_char) -> *const c_char {
    let result = thaw_napi_call_result(name, args);
    if result.error.is_null() {
        result.value
    } else {
        let error = CStr::from_ptr(result.error).to_string_lossy();
        CString::new(format!(
            "{{\"__thaw_error__\":{}}}",
            serde_json::to_string(error.as_ref()).unwrap()
        ))
        .unwrap()
        .into_raw()
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_undefined(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Undefined);
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_null(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Null);
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_boolean(
    env: NapiEnv,
    value: bool,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Bool(value));
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_double(
    env: NapiEnv,
    value: f64,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Number(value));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_string_utf8(
    env: NapiEnv,
    value: *const c_char,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() {
        return NAPI_INVALID_ARG;
    }
    let bytes = if length == NAPI_AUTO_LENGTH {
        CStr::from_ptr(value).to_bytes()
    } else {
        std::slice::from_raw_parts(value.cast(), length)
    };
    let string = String::from_utf8_lossy(bytes).into_owned();
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::String(string));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_object(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Object(HashMap::new()));
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_array(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    napi_create_array_with_length(env, 0, out)
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_array_with_length(
    env: NapiEnv,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let undefined = env.alloc(Value::Undefined);
    let value = env.alloc(Value::Array(vec![undefined; length]));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_function(
    env: NapiEnv,
    _name: *const c_char,
    _length: usize,
    callback: Option<NapiCallback>,
    data: *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    let Some(callback) = callback else {
        return NAPI_INVALID_ARG;
    };
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Function(Function { callback, data }));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_cb_info(
    _env: NapiEnv,
    info: NapiCallbackInfo,
    argc: *mut usize,
    argv: *mut NapiValue,
    this_arg: *mut NapiValue,
    data: *mut *mut c_void,
) -> NapiStatus {
    let Some(info) = info.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    if !argc.is_null() {
        let capacity = *argc;
        *argc = info.args.len();
        if !argv.is_null() {
            ptr::copy_nonoverlapping(info.args.as_ptr(), argv, capacity.min(info.args.len()));
        }
    }
    if !this_arg.is_null() {
        *this_arg = info.this_arg;
    }
    if !data.is_null() {
        *data = info.data;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_named_property(
    _env: NapiEnv,
    object: NapiValue,
    name: *const c_char,
    value: NapiValue,
) -> NapiStatus {
    let Ok(name) = text(name) else {
        return NAPI_INVALID_ARG;
    };
    match object.as_mut() {
        Some(Value::Object(values)) => {
            values.insert(name, value);
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_named_property(
    env: NapiEnv,
    object: NapiValue,
    name: *const c_char,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(name) = text(name) else {
        return NAPI_INVALID_ARG;
    };
    let value = match value_ref(object) {
        Ok(Value::Object(values)) => values.get(&name).copied(),
        _ => None,
    };
    match value {
        Some(value) => write_value(out, value),
        None => napi_get_undefined(env, out),
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_double(
    _env: NapiEnv,
    value: NapiValue,
    out: *mut f64,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    match value_ref(value) {
        Ok(Value::Number(number)) => {
            *out = *number;
            NAPI_OK
        }
        _ => NAPI_NUMBER_EXPECTED,
    }
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bool(
    _env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    match value_ref(value) {
        Ok(Value::Bool(boolean)) => {
            *out = *boolean;
            NAPI_OK
        }
        _ => NAPI_BOOLEAN_EXPECTED,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_string_utf8(
    _env: NapiEnv,
    value: NapiValue,
    buffer: *mut c_char,
    size: usize,
    written: *mut usize,
) -> NapiStatus {
    let string = match value_ref(value) {
        Ok(Value::String(string)) => string,
        _ => return NAPI_STRING_EXPECTED,
    };
    if !written.is_null() {
        *written = string.len();
    }
    if !buffer.is_null() && size > 0 {
        let count = string.len().min(size - 1);
        ptr::copy_nonoverlapping(string.as_ptr(), buffer.cast(), count);
        *buffer.add(count) = 0;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_throw_error(
    env: NapiEnv,
    _code: *const c_char,
    message: *const c_char,
) -> NapiStatus {
    let message = text(message).unwrap_or_else(|_| "native addon error".into());
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let error = env.alloc(Value::Error(message));
    env.exception = Some(error);
    NAPI_OK
}
#[no_mangle]
pub unsafe extern "C" fn napi_throw_type_error(
    env: NapiEnv,
    code: *const c_char,
    message: *const c_char,
) -> NapiStatus {
    napi_throw_error(env, code, message)
}
#[no_mangle]
pub unsafe extern "C" fn napi_is_exception_pending(env: NapiEnv, out: *mut bool) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    *out = env.exception.is_some();
    NAPI_OK
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_and_clear_last_exception(
    env: NapiEnv,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env
        .exception
        .take()
        .unwrap_or_else(|| env.alloc(Value::Undefined));
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_version(_env: NapiEnv, out: *mut u32) -> NapiStatus {
    if out.is_null() {
        NAPI_INVALID_ARG
    } else {
        *out = 8;
        NAPI_OK
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_typeof(_env: NapiEnv, value: NapiValue, out: *mut i32) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    *out = match value_ref(value) {
        Ok(Value::Undefined) => 0,
        Ok(Value::Null) => 1,
        Ok(Value::Bool(_)) => 2,
        Ok(Value::Number(_)) => 3,
        Ok(Value::String(_)) => 4,
        Ok(Value::Function(_)) => 7,
        Ok(_) => 6,
        Err(_) => return NAPI_INVALID_ARG,
    };
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_array(
    _env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    *out = matches!(value_ref(value), Ok(Value::Array(_)));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_array_length(
    _env: NapiEnv,
    value: NapiValue,
    out: *mut u32,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    match value_ref(value) {
        Ok(Value::Array(values)) => {
            *out = values.len() as u32;
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_element(
    env: NapiEnv,
    array: NapiValue,
    index: u32,
    value: NapiValue,
) -> NapiStatus {
    let Ok(host_env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    match array.as_mut() {
        Some(Value::Array(values)) => {
            while values.len() <= index as usize {
                values.push(host_env.alloc(Value::Undefined));
            }
            values[index as usize] = value;
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_element(
    env: NapiEnv,
    array: NapiValue,
    index: u32,
    out: *mut NapiValue,
) -> NapiStatus {
    let value = match value_ref(array) {
        Ok(Value::Array(values)) => values.get(index as usize).copied(),
        _ => None,
    };
    match value {
        Some(value) => write_value(out, value),
        None => napi_get_undefined(env, out),
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_buffer(
    env: NapiEnv,
    length: usize,
    data: *mut *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Buffer(vec![0; length]));
    if !data.is_null() {
        if let Value::Buffer(bytes) = &mut *value {
            *data = bytes.as_mut_ptr().cast();
        }
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_buffer_copy(
    env: NapiEnv,
    length: usize,
    source: *const c_void,
    data: *mut *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    if source.is_null() && length != 0 {
        return NAPI_INVALID_ARG;
    }
    let status = napi_create_buffer(env, length, data, out);
    if status == NAPI_OK && length != 0 {
        let target = if data.is_null() {
            match (*out).as_mut() {
                Some(Value::Buffer(bytes)) => bytes.as_mut_ptr().cast(),
                _ => return NAPI_INVALID_ARG,
            }
        } else {
            *data
        };
        ptr::copy_nonoverlapping(source.cast::<u8>(), target.cast::<u8>(), length);
    }
    status
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_buffer_info(
    _env: NapiEnv,
    value: NapiValue,
    data: *mut *mut c_void,
    length: *mut usize,
) -> NapiStatus {
    match value.as_mut() {
        Some(Value::Buffer(bytes)) => {
            if !data.is_null() {
                *data = bytes.as_mut_ptr().cast();
            }
            if !length.is_null() {
                *length = bytes.len();
            }
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_call_function(
    env: NapiEnv,
    this_arg: NapiValue,
    function: NapiValue,
    argc: usize,
    argv: *const NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    let function = match value_ref(function) {
        Ok(Value::Function(function)) => function.clone(),
        _ => return NAPI_INVALID_ARG,
    };
    let args = if argc == 0 {
        Vec::new()
    } else if argv.is_null() {
        return NAPI_INVALID_ARG;
    } else {
        std::slice::from_raw_parts(argv, argc).to_vec()
    };
    let mut info = CallbackInfo {
        args,
        this_arg,
        data: function.data,
    };
    let result = (function.callback)(env, &mut info);
    if env_mut(env)
        .map(|env| env.exception.is_some())
        .unwrap_or(false)
    {
        return NAPI_PENDING_EXCEPTION;
    }
    if out.is_null() {
        NAPI_OK
    } else {
        write_value(out, result)
    }
}

const NAPI_PENDING_EXCEPTION: NapiStatus = 10;

#[no_mangle]
pub unsafe extern "C" fn napi_create_error(
    env: NapiEnv,
    _code: NapiValue,
    message: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    let message = match value_ref(message) {
        Ok(Value::String(message)) => message.clone(),
        _ => return NAPI_STRING_EXPECTED,
    };
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Error(message));
    write_value(out, value)
}

#[repr(C)]
pub struct NapiNodeVersion {
    major: u32,
    minor: u32,
    patch: u32,
    release: *const c_char,
}
static NODE_RELEASE: &[u8] = b"thaw\0";
static NODE_VERSION: NapiNodeVersion = NapiNodeVersion {
    major: 20,
    minor: 0,
    patch: 0,
    release: NODE_RELEASE.as_ptr().cast(),
};
unsafe impl Sync for NapiNodeVersion {}
#[no_mangle]
pub unsafe extern "C" fn napi_get_node_version(
    _env: NapiEnv,
    out: *mut *const NapiNodeVersion,
) -> NapiStatus {
    if out.is_null() {
        NAPI_INVALID_ARG
    } else {
        *out = &NODE_VERSION;
        NAPI_OK
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_open_handle_scope(env: NapiEnv, out: *mut *mut c_void) -> NapiStatus {
    if env.is_null() || out.is_null() {
        NAPI_INVALID_ARG
    } else {
        *out = env.cast();
        NAPI_OK
    }
}
#[no_mangle]
pub unsafe extern "C" fn napi_close_handle_scope(_env: NapiEnv, _scope: *mut c_void) -> NapiStatus {
    NAPI_OK
}
#[no_mangle]
pub unsafe extern "C" fn napi_open_escapable_handle_scope(
    env: NapiEnv,
    out: *mut *mut c_void,
) -> NapiStatus {
    napi_open_handle_scope(env, out)
}
#[no_mangle]
pub unsafe extern "C" fn napi_close_escapable_handle_scope(
    env: NapiEnv,
    scope: *mut c_void,
) -> NapiStatus {
    napi_close_handle_scope(env, scope)
}
#[no_mangle]
pub unsafe extern "C" fn napi_escape_handle(
    _env: NapiEnv,
    _scope: *mut c_void,
    value: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    write_value(out, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn loads_and_calls_a_real_napi_addon() {
        let dir = std::env::temp_dir().join(format!("thaw-napi-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("addon.c");
        let addon = dir.join("addon.node");
        std::fs::write(&source, r#"
            #include <stddef.h>
            typedef void* napi_env; typedef void* napi_value; typedef void* napi_callback_info;
            typedef int napi_status;
            extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t*, napi_value*, napi_value*, void**);
            extern napi_status napi_get_value_double(napi_env, napi_value, double*);
            extern napi_status napi_get_value_bool(napi_env, napi_value, _Bool*);
            extern napi_status napi_get_value_string_utf8(napi_env, napi_value, char*, size_t, size_t*);
            extern napi_status napi_create_double(napi_env, double, napi_value*);
            extern napi_status napi_get_boolean(napi_env, _Bool, napi_value*);
            extern napi_status napi_create_string_utf8(napi_env, const char*, size_t, napi_value*);
            extern napi_status napi_create_function(napi_env, const char*, size_t, napi_value (*)(napi_env,napi_callback_info), void*, napi_value*);
            extern napi_status napi_set_named_property(napi_env, napi_value, const char*, napi_value);
            static napi_value add(napi_env env, napi_callback_info info) {
                size_t argc = 2; napi_value argv[2]; double a, b; napi_value result;
                napi_get_cb_info(env, info, &argc, argv, 0, 0);
                napi_get_value_double(env, argv[0], &a); napi_get_value_double(env, argv[1], &b);
                napi_create_double(env, a + b, &result); return result;
            }
            static napi_value negate(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, result; _Bool value;
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_get_value_bool(env, arg, &value); napi_get_boolean(env, !value, &result); return result;
            }
            static napi_value echo(napi_env env, napi_callback_info info) {
                size_t argc = 1, length; napi_value arg, result; char text[64];
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_get_value_string_utf8(env, arg, text, sizeof(text), &length);
                napi_create_string_utf8(env, text, length, &result); return result;
            }
            __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
                napi_value fn;
                napi_create_function(env, "add", 3, add, 0, &fn); napi_set_named_property(env, exports, "add", fn);
                napi_create_function(env, "negate", 6, negate, 0, &fn); napi_set_named_property(env, exports, "negate", fn);
                napi_create_function(env, "echo", 4, echo, 0, &fn); napi_set_named_property(env, exports, "echo", fn);
                return exports;
            }
        "#).unwrap();
        assert!(Command::new("cc")
            .args(["-shared", "-fPIC"])
            .arg(&source)
            .arg("-o")
            .arg(&addon)
            .status()
            .unwrap()
            .success());
        let path = CString::new(addon.to_string_lossy().as_bytes()).unwrap();
        let name = CString::new("add").unwrap();
        let args = CString::new("[20,22]").unwrap();
        unsafe {
            assert_eq!(thaw_napi_load(path.as_ptr()), 1);
            let result = thaw_napi_call_result(name.as_ptr(), args.as_ptr());
            assert!(result.error.is_null());
            assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "42.0");
            let negate = CString::new("negate").unwrap();
            let boolean = CString::new("[true]").unwrap();
            let result = thaw_napi_call_result(negate.as_ptr(), boolean.as_ptr());
            assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "false");
            let echo = CString::new("echo").unwrap();
            let string = CString::new("[\"hello\"]").unwrap();
            let result = thaw_napi_call_result(echo.as_ptr(), string.as_ptr());
            assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "\"hello\"");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn runs_utf8_validate_prebuild_when_supplied() {
        let Ok(path) = std::env::var("THAW_UTF8_VALIDATE_NODE") else {
            return;
        };
        let path = CString::new(path).unwrap();
        let name = CString::new("isValidUTF8").unwrap();
        let valid = CString::new(r#"[{"type":"Buffer","data":[240,144,128,128]}]"#).unwrap();
        let invalid = CString::new(r#"[{"type":"Buffer","data":[255]}]"#).unwrap();
        unsafe {
            assert_eq!(thaw_napi_load_named(path.as_ptr(), name.as_ptr()), 1);
            let result = thaw_napi_call_result(name.as_ptr(), valid.as_ptr());
            assert!(result.error.is_null());
            assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "true");
            let result = thaw_napi_call_result(name.as_ptr(), invalid.as_ptr());
            assert!(result.error.is_null());
            assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "false");
        }
    }
}
