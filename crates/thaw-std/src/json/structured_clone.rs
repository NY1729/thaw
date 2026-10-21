use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CloneMode {
    Snapshot,
    Structured,
}

struct GraphCloner {
    mode: CloneMode,
    memo: HashMap<usize, Value>,
    jobs: Vec<CloneJob>,
    fills: Vec<PendingFill>,
}

/// Largest host array `structuredClone` materializes (dense storage).
const MAX_CLONED_HOST_ARRAY_LENGTH: usize = 1 << 24;

enum CloneJob {
    NativeArray {
        source: SharedArray,
        target: SharedArray,
    },
    NativeObject {
        source: SharedObject,
        target: SharedObject,
    },
    HostArray {
        source: Rc<HostLease>,
        target: SharedArray,
    },
    HostObject {
        source: Rc<HostLease>,
        target: SharedObject,
    },
    /// A host `Map` (`is_map`) or `Set`, rebuilt as the native branded collection wrapper.
    HostCollection {
        source: Rc<HostLease>,
        target: SharedObject,
        is_map: bool,
    },
}

enum PendingFill {
    Array {
        target: SharedArray,
        values: Vec<Value>,
        holes: HashSet<usize>,
    },
    Object {
        target: SharedObject,
        fields: indexmap::IndexMap<Vec<u8>, Value>,
    },
}

impl GraphCloner {
    fn new(mode: CloneMode) -> Self {
        Self {
            mode,
            memo: HashMap::new(),
            jobs: Vec::new(),
            fills: Vec::new(),
        }
    }

    fn intern(&mut self, source: &Value) -> Result<Value, ()> {
        let Some(identity) = object_identity_key(source) else {
            return Ok(source.clone());
        };
        if let Some(value) = self.memo.get(&identity) {
            return Ok(value.clone());
        }
        if let Value::Host(lease) = source {
            if self.mode == CloneMode::Snapshot {
                return Ok(Value::Host(Rc::clone(lease)));
            }
            let kind = classify_host_for_clone(lease, self.mode)?;
            let copy = match kind {
                HostCloneKind::Shared => Value::Host(Rc::clone(lease)),
                HostCloneKind::Array => {
                    let target = Value::shared_array(Vec::new());
                    let Value::Array(target_rc) = &target else {
                        unreachable!()
                    };
                    self.jobs.push(CloneJob::HostArray {
                        source: Rc::clone(lease),
                        target: target_rc.clone(),
                    });
                    target
                }
                HostCloneKind::Object => {
                    let target = Value::shared_object(indexmap::IndexMap::new());
                    let Value::Object(target_rc) = &target else {
                        unreachable!()
                    };
                    self.jobs.push(CloneJob::HostObject {
                        source: Rc::clone(lease),
                        target: target_rc.clone(),
                    });
                    target
                }
                HostCloneKind::Map | HostCloneKind::Set => {
                    let target =
                        brand_internal_value(Value::shared_object(indexmap::IndexMap::new()));
                    let Value::Object(target_rc) = &target else {
                        unreachable!()
                    };
                    self.jobs.push(CloneJob::HostCollection {
                        source: Rc::clone(lease),
                        target: target_rc.clone(),
                        is_map: kind == HostCloneKind::Map,
                    });
                    target
                }
            };
            self.memo.insert(identity, copy.clone());
            return Ok(copy);
        }
        let copy = match source {
            Value::Array(source) => {
                let target = Value::shared_array(Vec::new());
                let Value::Array(target_rc) = &target else {
                    unreachable!()
                };
                self.jobs.push(CloneJob::NativeArray {
                    source: source.clone(),
                    target: target_rc.clone(),
                });
                target
            }
            Value::Object(source) => {
                let target = Value::shared_object_with_lease(
                    indexmap::IndexMap::new(),
                    source.napi_lease.borrow().clone(),
                );
                let Value::Object(target_rc) = &target else {
                    unreachable!()
                };
                self.jobs.push(CloneJob::NativeObject {
                    source: source.clone(),
                    target: target_rc.clone(),
                });
                if is_branded_wrapper(&source_value(source)) {
                    brand_internal_value(target.clone());
                } else if is_graph_plain_object(&source_value(source)) {
                    mark_graph_plain_object(&target);
                }
                target
            }
            _ => unreachable!(),
        };
        self.memo.insert(identity, copy.clone());
        Ok(copy)
    }

    fn discover(mut self, source: &Value) -> Result<(Value, Vec<PendingFill>), ()> {
        let result = self.discover_graph(source);
        match result {
            Ok(root) => Ok((root, std::mem::take(&mut self.fills))),
            Err(()) => {
                let original_host_error = HOST_ERROR.with(|slot| slot.borrow_mut().take());
                self.fills.clear();
                drop(self);
                HOST_ERROR.with(|slot| *slot.borrow_mut() = original_host_error);
                Err(())
            }
        }
    }

    fn discover_graph(&mut self, source: &Value) -> Result<Value, ()> {
        let root = self.intern(source)?;
        while let Some(job) = self.jobs.pop() {
            self.process_job(job)?;
        }
        Ok(root)
    }

    fn process_job(&mut self, job: CloneJob) -> Result<(), ()> {
        match job {
            CloneJob::NativeArray { source, target } => {
                let values = shared_array_ref(&source).clone();
                let holes = ARRAY_HOLES.with(|holes| {
                    holes
                        .borrow()
                        .get(&(Rc::as_ptr(&source) as usize))
                        .cloned()
                        .unwrap_or_default()
                });
                let values = values
                    .iter()
                    .map(|value| self.intern(value))
                    .collect::<Result<Vec<_>, _>>()?;
                self.fills.push(PendingFill::Array {
                    target,
                    values,
                    holes,
                });
            }
            CloneJob::NativeObject { source, target } => {
                let fields = shared_object_ref(&source).clone();
                let fields = fields
                    .into_iter()
                    .map(|(key, value)| Ok((key, self.intern(&value)?)))
                    .collect::<Result<indexmap::IndexMap<_, _>, ()>>()?;
                self.fills.push(PendingFill::Object { target, fields });
            }
            CloneJob::HostArray { source, target } => {
                let length = match host_get_property(&source, b"length") {
                    Value::Number(length) => length
                        .as_u64()
                        .and_then(|length| usize::try_from(length).ok()),
                    _ => None,
                };
                if host_operation_failed() {
                    return Err(());
                }
                let Some(length) = length else {
                    set_host_error("Invalid host array length".into());
                    return Err(());
                };
                // ponytail: arrays are dense Vec<Value>, so a host array with a huge
                // sparse length cannot be represented; fail fast instead of allocating it.
                if length > MAX_CLONED_HOST_ARRAY_LENGTH {
                    let message = format!(
                        "structuredClone of a host array longer than {MAX_CLONED_HOST_ARRAY_LENGTH} elements is not supported"
                    );
                    set_host_error(
                        thaw_arena::error_wire::encode_tagged(
                            b"RangeError",
                            message.as_bytes(),
                            None,
                        )
                        .and_then(|bytes| String::from_utf8(bytes).ok())
                        .unwrap_or(message),
                    );
                    return Err(());
                }
                let mut values = Vec::new();
                if values.try_reserve_exact(length).is_err() {
                    set_host_error("Unable to allocate host array".into());
                    return Err(());
                }
                // One enumeration finds the present indices; probing every index costs two
                // host calls per slot, which is unusable for sparse arrays.
                let keys = host_enumerate_values(&source, 2);
                if host_operation_failed() {
                    return Err(());
                }
                let mut present: Vec<usize> = keys
                    .iter()
                    .filter_map(|key| match key {
                        Value::String(text) => text
                            .parse::<usize>()
                            .ok()
                            .filter(|index| *index < length && index.to_string() == *text),
                        _ => None,
                    })
                    .collect();
                present.sort_unstable();
                present.dedup();
                let mut present = present.into_iter().peekable();
                let mut holes = HashSet::new();
                for index in 0..length {
                    if present.peek() != Some(&index) {
                        values.push(napi_undefined_value());
                        if holes.try_reserve(1).is_err() {
                            set_host_error("Unable to allocate host array holes".into());
                            return Err(());
                        }
                        holes.insert(index);
                        continue;
                    }
                    present.next();
                    let value = host_get_property(&source, index.to_string().as_bytes());
                    if host_operation_failed() {
                        return Err(());
                    }
                    values.push(self.intern(&value)?);
                }
                self.fills.push(PendingFill::Array {
                    target,
                    values,
                    holes,
                });
            }
            CloneJob::HostCollection {
                source,
                target,
                is_map,
            } => {
                let (field, items) = if is_map {
                    let keys = host_enumerate_values(&source, 4);
                    let values = host_enumerate_values(&source, 6);
                    if host_operation_failed() {
                        return Err(());
                    }
                    let mut entries = Vec::with_capacity(keys.len());
                    for (key, value) in keys.iter().zip(values.iter()) {
                        entries.push(Value::shared_array(vec![
                            self.intern(key)?,
                            self.intern(value)?,
                        ]));
                    }
                    (b"__thaw_map_entries__".as_slice(), entries)
                } else {
                    let values = host_enumerate_values(&source, 5);
                    if host_operation_failed() {
                        return Err(());
                    }
                    let items = values
                        .iter()
                        .map(|value| self.intern(value))
                        .collect::<Result<Vec<_>, _>>()?;
                    (b"__thaw_set_values__".as_slice(), items)
                };
                let mut fields = indexmap::IndexMap::new();
                fields.insert(field.to_vec(), Value::shared_array(items));
                self.fills.push(PendingFill::Object { target, fields });
            }
            CloneJob::HostObject { source, target } => {
                let entries = host_enumerable_entries(&source);
                if host_operation_failed() {
                    return Err(());
                }
                let fields = entries
                    .into_iter()
                    .map(|(key, value)| Ok((key, self.intern(&value)?)))
                    .collect::<Result<indexmap::IndexMap<_, _>, ()>>()?;
                self.fills.push(PendingFill::Object { target, fields });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostCloneKind {
    Shared,
    Array,
    Object,
    Map,
    Set,
}

fn host_operation_failed() -> bool {
    HOST_ERROR.with(|error| error.borrow().is_some())
}

fn classify_host_for_clone(lease: &Rc<HostLease>, _mode: CloneMode) -> Result<HostCloneKind, ()> {
    let Some(kind) = host_query(lease, 0) else {
        return Err(());
    };
    if host_operation_failed() {
        return Err(());
    }
    if kind == "function" || kind == "symbol" {
        // HTML structured clone: functions and symbols are not cloneable.
        let text = host_query(lease, 2).unwrap_or_default();
        // A compiled callback reaches the host as a generated wrapper whose source is noise.
        let text = if text.contains("__thaw_") {
            kind.to_string()
        } else {
            text
        };
        let message = format!("{text} could not be cloned.");
        set_host_error(
            thaw_arena::error_wire::encode_tagged(b"DataCloneError", message.as_bytes(), None)
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or(message),
        );
        return Err(());
    }
    if kind != "object" {
        return Ok(HostCloneKind::Shared);
    }

    let Some(is_null) = host_query(lease, 7) else {
        return Err(());
    };
    if host_operation_failed() {
        return Err(());
    }
    if is_null != "0" {
        return Ok(HostCloneKind::Shared);
    }

    match host_query(lease, 24).as_deref() {
        Some("map") => return Ok(HostCloneKind::Map),
        Some("set") => return Ok(HostCloneKind::Set),
        _ => {}
    }
    if host_operation_failed() {
        return Err(());
    }

    let Some(is_non_plain) = host_query(lease, 12) else {
        return Err(());
    };
    if host_operation_failed() {
        return Err(());
    }
    if is_non_plain != "0" {
        return Ok(HostCloneKind::Shared);
    }

    let Some(is_array) = host_query(lease, 4) else {
        return Err(());
    };
    if host_operation_failed() {
        return Err(());
    }
    Ok(if is_array == "1" {
        HostCloneKind::Array
    } else {
        HostCloneKind::Object
    })
}

fn source_value(source: &SharedObject) -> Value {
    Value::Object(source.clone())
}

fn prepare_array_hole_capacity(fills: &[PendingFill]) {
    let hole_entries = fills
        .iter()
        .filter(|fill| matches!(fill, PendingFill::Array { holes, .. } if !holes.is_empty()))
        .count();
    ARRAY_HOLES.with(|entries| entries.borrow_mut().reserve(hole_entries));
}

fn commit_fills(fills: Vec<PendingFill>) {
    for fill in fills {
        match fill {
            PendingFill::Array {
                target,
                values,
                holes,
            } => {
                *shared_array_ref_mut(&target) = values;
                if !holes.is_empty() {
                    install_array_holes(&target, holes);
                }
            }
            PendingFill::Object { target, fields } => *shared_object_ref_mut(&target) = fields,
        }
    }
}

pub(super) fn clone_value(source: &Value, mode: CloneMode) -> Result<Value, ()> {
    let (root, fills) = GraphCloner::new(mode).discover(source)?;
    prepare_array_hole_capacity(&fills);
    commit_fills(fills);
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::CloneMode;
    use std::process::Command;

    fn strict_identity(left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Array(left), Value::Array(right)) => Rc::ptr_eq(left, right),
            (Value::Object(left), Value::Object(right)) => Rc::ptr_eq(left, right),
            (Value::Host(left), Value::Host(right)) => left.handle == right.handle,
            (Value::Null, Value::Null) => true,
            (Value::Bool(left), Value::Bool(right)) => left == right,
            (Value::Number(left), Value::Number(right)) => left == right,
            (Value::BigInt(left), Value::BigInt(right))
            | (Value::String(left), Value::String(right)) => left == right,
            (Value::Wtf8(left), Value::Wtf8(right)) => left == right,
            _ => false,
        }
    }

    thread_local! {
        static HOST_ENUM_COUNTS: RefCell<HashMap<u64, usize>> = RefCell::new(HashMap::new());
        static HOST_RELEASE_COUNTS: RefCell<HashMap<u64, usize>> = RefCell::new(HashMap::new());
        static HOST_RETAIN_COUNTS: RefCell<HashMap<u64, usize>> = RefCell::new(HashMap::new());
    }

    extern "C" fn clone_test_retain(handle: u64) -> u8 {
        HOST_RETAIN_COUNTS.with(|counts| *counts.borrow_mut().entry(handle).or_default() += 1);
        1
    }

    extern "C" fn clone_test_release(handle: u64) -> u8 {
        HOST_RELEASE_COUNTS.with(|counts| *counts.borrow_mut().entry(handle).or_default() += 1);
        1
    }

    extern "C" fn clone_test_query(handle: u64, operation: u8) -> HostTextResult {
        let value: &[u8] = match (handle, operation) {
            (706, 0) => b"number",
            (707..=709 | 713 | 714, 0) => b"string",
            (715..=717, 0) => b"number",
            (752, 0) => b"function",
            (752, 2) => b"function f() {}",
            (_, 0) => b"object",
            (_, 7 | 12) => b"0",
            (750, 24) => b"map",
            (751, 24) => b"set",
            (_, 24) => b"",
            (711 | 760 | 761, 4) => b"1",
            (_, 4) => b"0",
            (706, 10) => b"1",
            (707, 3) => br#""self""#,
            (708, 3) => br#""left""#,
            (709, 3) => br#""right""#,
            (713, 3) => br#""list""#,
            (714, 3) => br#""x""#,
            (715, 10) => b"1",
            (716, 10) => b"1000000",
            (717, 10) => b"16777217",
            _ => panic!("unexpected clone query {handle}/{operation}"),
        };
        HostTextResult {
            value: thaw_arena::owned_string(value),
            error: std::ptr::null(),
        }
    }

    extern "C" fn clone_test_enumerate(handle: u64, operation: u8) -> HostTextResult {
        HOST_ENUM_COUNTS.with(|counts| *counts.borrow_mut().entry(handle).or_default() += 1);
        let wire: &[u8] = match (handle, operation) {
            (700, 0) => br#"{"root":{"r":0},"nodes":[{"a":[{"r":1},{"r":2},{"r":3}]},{"hdl":702},{"hdl":703},{"hdl":704}],"leases":[],"napiLeases":[]}"#,
            (701, 0) => br#"{"root":{"r":0},"nodes":[{"a":[{"r":1}]},{"hdl":705}],"leases":[],"napiLeases":[]}"#,
            (710, 0) => br#"{"root":{"r":0},"nodes":[{"a":[{"r":1}]},{"hdl":712}],"leases":[],"napiLeases":[]}"#,
            (711, 3) => br#"{"root":{"r":0},"nodes":[{"a":[{"r":1}]},{"hdl":710}],"leases":[],"napiLeases":[]}"#,
            (711, 2) => br#"{"root":{"r":0},"nodes":[{"a":[{"v":"0"}]}],"leases":[],"napiLeases":[]}"#,
            (760, 2) => br#"{"root":{"r":0},"nodes":[{"a":[{"v":"7"},{"v":"999999"},{"v":"length"}]}],"leases":[],"napiLeases":[]}"#,
            (750, 4) => br#"{"root":{"r":0},"nodes":[{"a":[{"v":1},{"v":2}]}],"leases":[],"napiLeases":[]}"#,
            (750, 6) => br#"{"root":{"r":0},"nodes":[{"a":[{"v":"a"},{"v":"b"}]}],"leases":[],"napiLeases":[]}"#,
            (751, 5) => br#"{"root":{"r":0},"nodes":[{"a":[{"v":3},{"v":4}]}],"leases":[],"napiLeases":[]}"#,
            _ => panic!("unexpected clone enumeration {handle}/{operation}"),
        };
        HostTextResult {
            value: thaw_arena::owned_string(wire),
            error: std::ptr::null(),
        }
    }

    extern "C" fn clone_test_get(handle: u64, key: *const c_char) -> HostHandleResult {
        let key = to_str(key);
        let value = match (handle, key.as_str()) {
            (702, r#""0""#) => 707,
            (702, r#""1""#) => 700,
            (703, r#""0""#) => 708,
            (703, r#""1""#) => 701,
            (704, r#""0""#) => 709,
            (704, r#""1""#) => 701,
            (705, r#""0""#) => 714,
            (705, r#""1""#) => 706,
            (712, r#""0""#) => 713,
            (712, r#""1""#) => 711,
            (711, r#""length""#) => 715,
            (760, r#""length""#) => 716,
            (761, r#""length""#) => 717,
            (760, r#""7""#) => 714,
            (760, r#""999999""#) => 714,
            (711, r#""0""#) => 710,
            _ => panic!("unexpected clone property {handle}/{key}"),
        };
        HostHandleResult {
            value,
            error: std::ptr::null(),
        }
    }
    thread_local! {
        static FAILURE_RELEASE_ARMED: Cell<bool> = const { Cell::new(false) };
    }

    extern "C" fn failure_clone_release(handle: u64) -> u8 {
        HOST_RELEASE_COUNTS.with(|counts| *counts.borrow_mut().entry(handle).or_default() += 1);
        if FAILURE_RELEASE_ARMED.with(Cell::get) {
            HOST_ERROR.with(|error| *error.borrow_mut() = Some("cleanup reentry error".into()));
        }
        1
    }

    extern "C" fn failure_clone_query(handle: u64, operation: u8) -> HostTextResult {
        let value: &[u8] = match (handle, operation) {
            (_, 0) => b"object",
            (_, 7 | 12 | 4) => b"0",
            (_, 24) => b"",
            (728, 3) => br#""first""#,
            (729, 3) => br#""boom""#,
            _ => panic!("unexpected failure query {handle}/{operation}"),
        };
        HostTextResult {
            value: thaw_arena::owned_string(value),
            error: std::ptr::null(),
        }
    }

    extern "C" fn failure_clone_enumerate(handle: u64, operation: u8) -> HostTextResult {
        assert_eq!((handle, operation), (720, 0));
        let wire = br#"{"root":{"r":0},"nodes":[{"a":[{"r":1},{"r":2}]},{"hdl":722},{"hdl":723}],"leases":[],"napiLeases":[]}"#;
        HostTextResult {
            value: thaw_arena::owned_string(wire),
            error: std::ptr::null(),
        }
    }

    extern "C" fn failure_clone_get(handle: u64, key: *const c_char) -> HostHandleResult {
        let key = to_str(key);
        match (handle, key.as_str()) {
            (722, r#""0""#) => HostHandleResult {
                value: 728,
                error: std::ptr::null(),
            },
            (722, r#""1""#) => HostHandleResult {
                value: 721,
                error: std::ptr::null(),
            },
            (723, r#""0""#) => HostHandleResult {
                value: 729,
                error: std::ptr::null(),
            },
            (723, r#""1""#) => {
                FAILURE_RELEASE_ARMED.with(|armed| armed.set(true));
                HostHandleResult {
                    value: 0,
                    error: thaw_arena::owned_string(b"clone getter exploded"),
                }
            }
            _ => panic!("unexpected failing property {handle}/{key}"),
        }
    }

    fn install_failure_clone_host_operations() {
        HOST_ERROR.with(|error| error.borrow_mut().take());
        HOST_RELEASE_COUNTS.with(|counts| counts.borrow_mut().clear());
        HOST_RETAIN_COUNTS.with(|counts| counts.borrow_mut().clear());
        FAILURE_RELEASE_ARMED.with(|armed| armed.set(false));
        HOST_OPERATIONS.with(|slot| {
            slot.set(Some(HostOperations {
                retain: clone_test_retain,
                release: failure_clone_release,
                get: failure_clone_get,
                query: failure_clone_query,
                date_set: unused_clone_date,
                set: unused_clone_set,
                predicate: unused_clone_predicate,
                enumerate: failure_clone_enumerate,
            }))
        });
    }

    extern "C" fn unused_clone_date(_: u64, _: f64) -> HostTextResult {
        HostTextResult {
            value: std::ptr::null(),
            error: std::ptr::null(),
        }
    }
    extern "C" fn unused_clone_set(_: u64, _: *const c_char, _: *const c_char) -> HostHandleResult {
        HostHandleResult {
            value: 0,
            error: std::ptr::null(),
        }
    }
    extern "C" fn unused_clone_predicate(_: u64, _: *const c_char, _: u8) -> HostHandleResult {
        HostHandleResult {
            value: 0,
            error: std::ptr::null(),
        }
    }

    extern "C" fn clone_test_predicate(
        handle: u64,
        key: *const c_char,
        operation: u8,
    ) -> HostHandleResult {
        assert_eq!(handle, 711);
        assert_eq!(operation, 3);
        HostHandleResult {
            value: u64::from(to_str(key) == r#""0""#),
            error: std::ptr::null(),
        }
    }

    fn install_clone_test_host_operations() {
        HOST_ENUM_COUNTS.with(|counts| counts.borrow_mut().clear());
        HOST_RELEASE_COUNTS.with(|counts| counts.borrow_mut().clear());
        HOST_RETAIN_COUNTS.with(|counts| counts.borrow_mut().clear());
        HOST_ERROR.with(|error| error.borrow_mut().take());
        HOST_OPERATIONS.with(|slot| {
            slot.set(Some(HostOperations {
                retain: clone_test_retain,
                release: clone_test_release,
                get: clone_test_get,
                query: clone_test_query,
                date_set: unused_clone_date,
                set: unused_clone_set,
                predicate: clone_test_predicate,
                enumerate: clone_test_enumerate,
            }))
        });
    }

    fn host_value(handle: u64) -> Value {
        Value::Host(Rc::new(HostLease {
            handle,
            release: clone_test_release,
        }))
    }

    fn host_cycle_red_worker(test_name: &str, env: &str) -> bool {
        if std::env::var_os(env).is_some() {
            return false;
        }
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .env(env, "1")
            .status()
            .unwrap();
        assert!(status.success(), "Host cycle worker failed: {status}");
        true
    }

    fn destroy(value: Value) {
        unsafe { thaw_json_destroy(Box::into_raw(Box::new(value))) };
    }

    #[test]
    fn structured_clone_rebuilds_host_map_and_set_as_independent_native_collections() {
        install_clone_test_host_operations();
        let map = super::clone_value(&host_value(750), CloneMode::Structured).unwrap();
        assert!(
            is_branded_wrapper(&map),
            "a cloned Map keeps the native collection brand"
        );
        let entries = map
            .as_object()
            .unwrap()
            .get(b"__thaw_map_entries__".as_slice())
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(entries.len(), 2);
        let first = entries[0].as_array().unwrap();
        assert_eq!(first[0].as_f64(), Some(1.0));
        assert_eq!(first[1].as_str(), Some("a"));
        let set = super::clone_value(&host_value(751), CloneMode::Structured).unwrap();
        assert!(is_branded_wrapper(&set));
        let values = set
            .as_object()
            .unwrap()
            .get(b"__thaw_set_values__".as_slice())
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(
            values.iter().filter_map(Value::as_f64).collect::<Vec<_>>(),
            [3.0, 4.0]
        );
        destroy(map);
        destroy(set);
    }

    #[test]
    fn structured_clone_rejects_host_functions_with_a_data_clone_error() {
        install_clone_test_host_operations();
        let result = super::clone_value(&host_value(752), CloneMode::Structured);
        assert!(result.is_err());
        let error = HOST_ERROR.with(|error| error.borrow_mut().take()).unwrap();
        assert!(
            error.starts_with('\u{1}'),
            "the error carries a class tag: {error:?}"
        );
        assert!(error.contains("DataCloneError"));
        assert!(error.contains("function f() {} could not be cloned."));
    }

    #[test]
    fn structured_clone_rejects_host_arrays_beyond_dense_capacity_without_allocating() {
        install_clone_test_host_operations();
        assert!(super::clone_value(&host_value(761), CloneMode::Structured).is_err());
        let error = HOST_ERROR.with(|error| error.borrow_mut().take()).unwrap();
        assert!(error.contains("RangeError"), "{error:?}");
        assert!(error.contains("longer than 16777216 elements"), "{error:?}");
    }

    #[test]
    fn structured_clone_of_a_sparse_host_array_probes_only_present_indices() {
        install_clone_test_host_operations();
        // The mock panics on any property read other than the two present indices.
        let cloned = super::clone_value(&host_value(760), CloneMode::Structured).unwrap();
        let items = cloned.as_array().unwrap();
        assert_eq!(items.len(), 1_000_000);
        assert_eq!(items[7].as_str(), Some("x"));
        assert_eq!(items[999_999].as_str(), Some("x"));
        let holes = ARRAY_HOLES.with(|holes| {
            holes
                .borrow()
                .get(
                    &(Rc::as_ptr(match &cloned {
                        Value::Array(array) => array,
                        _ => unreachable!(),
                    }) as usize),
                )
                .map(|holes| (holes.len(), holes.contains(&0), holes.contains(&7)))
        });
        assert_eq!(holes, Some((999_998, true, false)));
        destroy(cloned);
    }

    #[test]
    fn structured_clone_materializes_each_plain_host_identity_once() {
        const CHILD: &str = "THAW_STRUCTURED_CLONE_HOST_CYCLE_CHILD";
        if host_cycle_red_worker("json::structured_clone::tests::structured_clone_materializes_each_plain_host_identity_once", CHILD) { return; }
        install_clone_test_host_operations();
        let source = host_value(700);
        let cloned = super::clone_value(&source, CloneMode::Structured).unwrap();
        let fields = cloned.as_object().unwrap();
        assert!(strict_identity(
            &cloned,
            fields.get(b"self".as_slice()).unwrap()
        ));
        let left = fields.get(b"left".as_slice()).unwrap();
        let right = fields.get(b"right".as_slice()).unwrap();
        assert!(strict_identity(left, right));
        assert!(matches!(left, Value::Object(_)));
        HOST_ENUM_COUNTS.with(|counts| {
            assert_eq!(counts.borrow().get(&700), Some(&1));
            assert_eq!(counts.borrow().get(&701), Some(&1));
        });
        let released_before_source_drop =
            HOST_RELEASE_COUNTS.with(|counts| counts.borrow().get(&700).copied().unwrap_or(0));
        destroy(source);
        assert_eq!(
            HOST_RELEASE_COUNTS.with(|counts| counts.borrow().get(&700).copied()),
            Some(released_before_source_drop + 1),
            "the source lease stays alive until source destruction"
        );
        destroy(cloned);
    }

    #[test]
    fn structured_clone_materializes_mutually_recursive_host_array_and_object() {
        const CHILD: &str = "THAW_STRUCTURED_CLONE_HOST_MUTUAL_CYCLE_CHILD";
        if host_cycle_red_worker("json::structured_clone::tests::structured_clone_materializes_mutually_recursive_host_array_and_object", CHILD) { return; }
        install_clone_test_host_operations();
        let source = host_value(710);
        let cloned = super::clone_value(&source, CloneMode::Structured).unwrap();
        let Value::Array(list) = cloned.as_object().unwrap().get(b"list".as_slice()).unwrap()
        else {
            panic!("expected native array")
        };
        assert!(strict_identity(&cloned, &shared_array_ref(list)[0]));
        assert!(matches!(cloned, Value::Object(_)));
        destroy(source);
        destroy(cloned);
    }

    #[test]
    fn structured_clone_host_failure_discards_uncommitted_shells_and_leases_once() {
        install_failure_clone_host_operations();
        let source = host_value(720);
        let result = super::clone_value(&source, CloneMode::Structured);
        assert!(result.is_err());
        let error = thaw_json_take_host_error();
        assert_eq!(
            unsafe { CStr::from_ptr(error) }.to_string_lossy(),
            "clone getter exploded"
        );
        unsafe {
            thaw_arena::destroy_string(error.cast_mut());
        }
        assert!(FAILURE_RELEASE_ARMED.with(Cell::get));
        HOST_RELEASE_COUNTS.with(|counts| {
            assert_eq!(counts.borrow().get(&721), Some(&1));
            assert_eq!(counts.borrow().get(&722), Some(&1));
            assert_eq!(counts.borrow().get(&723), Some(&1));
        });
        HOST_RETAIN_COUNTS.with(|counts| {
            assert_eq!(counts.borrow().get(&722), Some(&1));
            assert_eq!(counts.borrow().get(&723), Some(&1));
        });
        assert!(thaw_json_take_host_error().is_null());
        install_clone_test_host_operations();
        let good_source = host_value(701);
        let cloned = super::clone_value(&good_source, CloneMode::Structured).unwrap();
        assert_eq!(
            cloned
                .as_object()
                .unwrap()
                .get(b"x".as_slice())
                .unwrap()
                .as_f64(),
            Some(1.0)
        );
        FAILURE_RELEASE_ARMED.with(|armed| armed.set(false));
        destroy(source);
        destroy(good_source);
        destroy(cloned);
    }

    #[test]
    fn structured_clone_preserves_a_self_cycle() {
        const CHILD: &str = "THAW_STRUCTURED_CLONE_SELF_CYCLE_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let status = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "json::structured_clone::tests::structured_clone_preserves_a_self_cycle",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .status()
                .unwrap();
            assert!(status.success(), "self-cycle worker failed: {status}");
            return;
        }

        let source = Value::shared_object(indexmap::IndexMap::new());
        let Value::Object(source_fields) = &source else {
            unreachable!()
        };
        shared_object_ref_mut(source_fields).insert(b"self".to_vec(), source.clone());

        let cloned = super::clone_value(&source, CloneMode::Structured).unwrap();

        assert!(!strict_identity(&source, &cloned));
        let cloned_self = cloned.as_object().unwrap().get(b"self".as_slice()).unwrap();
        assert!(strict_identity(&cloned, cloned_self));
        destroy(source);
        destroy(cloned);
    }

    #[test]
    fn structured_clone_preserves_shared_children_but_not_source_identity() {
        let child = Value::shared_object(indexmap::IndexMap::from([(
            b"x".to_vec(),
            number_value(1.0),
        )]));
        let source = Value::shared_object(indexmap::IndexMap::from([
            (b"left".to_vec(), child.clone()),
            (b"right".to_vec(), child),
        ]));
        let cloned = super::clone_value(&source, CloneMode::Structured).unwrap();
        let source_fields = source.as_object().unwrap();
        let cloned_fields = cloned.as_object().unwrap();
        let source_left = source_fields.get(b"left".as_slice()).unwrap();
        let left = cloned_fields.get(b"left".as_slice()).unwrap();
        let right = cloned_fields.get(b"right".as_slice()).unwrap();
        assert!(strict_identity(left, right));
        assert!(!strict_identity(source_left, left));
        assert!(!strict_identity(&source, &cloned));
        let Value::Object(left_fields) = left else {
            unreachable!()
        };
        shared_object_ref_mut(left_fields).insert(b"x".to_vec(), number_value(2.0));
        assert_eq!(
            source_left
                .as_object()
                .unwrap()
                .get(b"x".as_slice())
                .unwrap()
                .as_f64(),
            Some(1.0)
        );
        assert_eq!(
            right
                .as_object()
                .unwrap()
                .get(b"x".as_slice())
                .unwrap()
                .as_f64(),
            Some(2.0)
        );
        destroy(source);
        destroy(cloned);
    }

    #[test]
    fn structured_clone_preserves_mixed_cycles_and_array_holes() {
        const CHILD: &str = "THAW_STRUCTURED_CLONE_MIXED_CYCLE_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let status = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "json::structured_clone::tests::structured_clone_preserves_mixed_cycles_and_array_holes", "--nocapture"])
                .env(CHILD, "1")
                .status()
                .unwrap();
            assert!(status.success(), "mixed-cycle worker failed: {status}");
            return;
        }
        let list = Value::shared_array(Vec::new());
        let source =
            Value::shared_object(indexmap::IndexMap::from([(b"list".to_vec(), list.clone())]));
        let Value::Array(items) = &list else {
            unreachable!()
        };
        shared_array_ref_mut(items).extend([
            source.clone(),
            napi_undefined_value(),
            napi_undefined_value(),
        ]);
        mark_array_holes(items, [1]);

        let cloned = super::clone_value(&source, CloneMode::Structured).unwrap();
        let Value::Array(cloned_list) =
            cloned.as_object().unwrap().get(b"list".as_slice()).unwrap()
        else {
            unreachable!()
        };
        assert!(!strict_identity(&list, &Value::Array(cloned_list.clone())));
        assert!(strict_identity(&cloned, &shared_array_ref(cloned_list)[0]));
        assert!(!array_has_index(cloned_list, 1));
        assert!(array_has_index(cloned_list, 2));
        assert!(is_napi_undefined(&shared_array_ref(cloned_list)[2]));
        destroy(source);
        destroy(cloned);
    }

    #[test]
    fn structured_clone_handles_a_4096_node_chain_without_recursion() {
        let mut source = Value::shared_object(indexmap::IndexMap::new());
        for _ in 0..4095 {
            source = Value::shared_object(indexmap::IndexMap::from([(b"next".to_vec(), source)]));
        }
        let cloned = super::clone_value(&source, CloneMode::Structured).unwrap();
        let mut cursor = &cloned;
        for _ in 0..4095 {
            cursor = cursor.as_object().unwrap().get(b"next".as_slice()).unwrap();
        }
        assert!(cursor.as_object().unwrap().is_empty());
        destroy(source);
        destroy(cloned);
    }
}
