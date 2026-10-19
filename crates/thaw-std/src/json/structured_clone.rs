use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CloneMode {
    Snapshot,
    Structured,
}

#[allow(dead_code)]
struct GraphCloner {
    mode: CloneMode,
    memo: HashMap<usize, Value>,
    jobs: Vec<CloneJob>,
    fills: Vec<PendingFill>,
}

#[allow(dead_code)]
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
        if let Value::Host(lease) = source {
            return Ok(if self.mode == CloneMode::Structured {
                host_deep_clone(lease)
            } else {
                Value::Host(Rc::clone(lease))
            });
        }
        let Some(identity) = object_identity_key(source) else {
            return Ok(source.clone());
        };
        if let Some(value) = self.memo.get(&identity) {
            return Ok(value.clone());
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
        let root = self.intern(source)?;
        while let Some(job) = self.jobs.pop() {
            self.process_job(job)?;
        }
        Ok((root, self.fills))
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
            CloneJob::HostArray { .. } | CloneJob::HostObject { .. } => {
                unreachable!("host jobs are handled by the structured clone extension")
            }
        }
        Ok(())
    }
}

fn source_value(source: &SharedObject) -> Value {
    Value::Object(source.clone())
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
                    mark_array_holes(&target, holes);
                }
            }
            PendingFill::Object { target, fields } => *shared_object_ref_mut(&target) = fields,
        }
    }
}

pub(super) fn clone_value(source: &Value, mode: CloneMode) -> Result<Value, ()> {
    let (root, fills) = GraphCloner::new(mode).discover(source)?;
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

    fn destroy(value: Value) {
        unsafe { thaw_json_destroy(Box::into_raw(Box::new(value))) };
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
        shared_object_ref_mut(&left_fields).insert(b"x".to_vec(), number_value(2.0));
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
        assert!(strict_identity(&cloned, &shared_array_ref(&cloned_list)[0]));
        assert!(!array_has_index(&cloned_list, 1));
        assert!(array_has_index(&cloned_list, 2));
        assert!(is_napi_undefined(&shared_array_ref(&cloned_list)[2]));
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
