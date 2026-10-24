//! ArrayBuffer and the typed arrays (`Uint8Array` & co); the methods live in a JS prelude on top of
//! the native storage, `Buffer` / `TextEncoder` / `TextDecoder` in `buffer.rs`.

use crate::*;
use std::cell::RefCell;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Ty {
    I8,
    U8,
    U8C,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Ty {
    pub(crate) fn size(self) -> usize {
        match self {
            Ty::I8 | Ty::U8 | Ty::U8C => 1,
            Ty::I16 | Ty::U16 => 2,
            Ty::I32 | Ty::U32 | Ty::F32 => 4,
            Ty::F64 => 8,
        }
    }

    pub(crate) fn tag(self) -> &'static str {
        match self {
            Ty::I8 => "[object Int8Array]",
            Ty::U8 => "[object Uint8Array]",
            Ty::U8C => "[object Uint8ClampedArray]",
            Ty::I16 => "[object Int16Array]",
            Ty::U16 => "[object Uint16Array]",
            Ty::I32 => "[object Int32Array]",
            Ty::U32 => "[object Uint32Array]",
            Ty::F32 => "[object Float32Array]",
            Ty::F64 => "[object Float64Array]",
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Ty::I8 => "Int8Array",
            Ty::U8 => "Uint8Array",
            Ty::U8C => "Uint8ClampedArray",
            Ty::I16 => "Int16Array",
            Ty::U16 => "Uint16Array",
            Ty::I32 => "Int32Array",
            Ty::U32 => "Uint32Array",
            Ty::F32 => "Float32Array",
            Ty::F64 => "Float64Array",
        }
    }
}

const ALL: [Ty; 9] = [
    Ty::I8,
    Ty::U8,
    Ty::U8C,
    Ty::I16,
    Ty::U16,
    Ty::I32,
    Ty::U32,
    Ty::F32,
    Ty::F64,
];

pub(crate) type Bytes = Rc<RefCell<Vec<u8>>>;

#[derive(Clone)]
pub(crate) struct TypedArr {
    pub(crate) abuf: Obj,
    pub(crate) data: Bytes,
    pub(crate) ty: Ty,
    pub(crate) off: usize,
    pub(crate) len: usize,
}

impl TypedArr {
    pub(crate) fn get(&self, i: usize) -> Option<f64> {
        if i >= self.len {
            return None;
        }
        let d = self.data.borrow();
        let p = self.off + i * self.ty.size();
        let b = &d[p..p + self.ty.size()];
        Some(match self.ty {
            Ty::I8 => b[0] as i8 as f64,
            Ty::U8 | Ty::U8C => b[0] as f64,
            Ty::I16 => i16::from_le_bytes([b[0], b[1]]) as f64,
            Ty::U16 => u16::from_le_bytes([b[0], b[1]]) as f64,
            Ty::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Ty::U32 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Ty::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Ty::F64 => f64::from_le_bytes(b.try_into().unwrap()),
        })
    }

    pub(crate) fn set(&self, i: usize, v: f64) {
        if i >= self.len {
            return;
        }
        let mut d = self.data.borrow_mut();
        let p = self.off + i * self.ty.size();
        let wrap = |bits: u32| -> u64 {
            if !v.is_finite() {
                0
            } else {
                (v.trunc().rem_euclid(2f64.powi(bits as i32))) as u64
            }
        };
        match self.ty {
            Ty::I8 | Ty::U8 => d[p] = wrap(8) as u8,
            Ty::U8C => {
                d[p] = if v.is_nan() {
                    0
                } else {
                    // round half to even, clamped
                    let c = v.clamp(0.0, 255.0);
                    let f = c.floor();
                    let r = if c - f > 0.5 || (c - f == 0.5 && f % 2.0 != 0.0) {
                        f + 1.0
                    } else {
                        f
                    };
                    r as u8
                }
            }
            Ty::I16 | Ty::U16 => d[p..p + 2].copy_from_slice(&(wrap(16) as u16).to_le_bytes()),
            Ty::I32 | Ty::U32 => d[p..p + 4].copy_from_slice(&(wrap(32) as u32).to_le_bytes()),
            Ty::F32 => d[p..p + 4].copy_from_slice(&(v as f32).to_le_bytes()),
            Ty::F64 => d[p..p + 8].copy_from_slice(&v.to_le_bytes()),
        }
    }

    pub(crate) fn to_vec(&self) -> Vec<f64> {
        (0..self.len).filter_map(|i| self.get(i)).collect()
    }
}

impl Interp {
    pub(crate) fn typed_of(v: &Value) -> Option<TypedArr> {
        match v {
            Value::Object(o) => match &*o.kind.borrow() {
                Kind::Typed(t) => Some(t.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    pub(crate) fn bytes_of_buffer(v: &Value) -> Option<Bytes> {
        match v {
            Value::Object(o) => match &*o.kind.borrow() {
                Kind::ArrayBuffer(b) => Some(b.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    fn new_array_buffer(&self, data: Bytes) -> Obj {
        new_obj(self.rt.arraybuffer_proto.clone(), Kind::ArrayBuffer(data))
    }

    pub(crate) fn new_typed(
        &self,
        ty: Ty,
        abuf: Obj,
        data: Bytes,
        off: usize,
        len: usize,
    ) -> Value {
        let proto = self.rt.typed_protos.get(ty as usize).cloned();
        Value::Object(new_obj(
            proto,
            Kind::Typed(TypedArr {
                abuf,
                data,
                ty,
                off,
                len,
            }),
        ))
    }

    /// A fresh zero-filled typed array.
    pub(crate) fn typed_with_len(&self, ty: Ty, len: usize) -> Value {
        let data: Bytes = Rc::new(RefCell::new(vec![0; len * ty.size()]));
        let abuf = self.new_array_buffer(data.clone());
        self.new_typed(ty, abuf, data, 0, len)
    }

    /// A `Uint8Array` over a copy of `bytes`.
    pub(crate) fn u8_from_vec(&self, bytes: Vec<u8>) -> Value {
        let len = bytes.len();
        let data: Bytes = Rc::new(RefCell::new(bytes));
        let abuf = self.new_array_buffer(data.clone());
        self.new_typed(Ty::U8, abuf, data, 0, len)
    }

    fn construct_typed(&mut self, ty: Ty, args: &[Value]) -> R<Value> {
        let a0 = arg(args, 0);
        match &a0 {
            Value::Undefined | Value::Null => Ok(self.typed_with_len(ty, 0)),
            Value::Number(n) => {
                if !(*n >= 0.0 && n.is_finite()) || *n > 2f64.powi(32) {
                    return self.throw_error(
                        "RangeError",
                        &format!("Invalid typed array length: {}", number_to_string(*n)),
                    );
                }
                Ok(self.typed_with_len(ty, *n as usize))
            }
            Value::Object(o) => {
                if let Some(data) = Self::bytes_of_buffer(&a0) {
                    let total = data.borrow().len();
                    let off = match arg(args, 1) {
                        Value::Undefined => 0,
                        v => self.num(&v) as usize,
                    };
                    if off % ty.size() != 0 {
                        return self.throw_error(
                            "RangeError",
                            &format!(
                                "start offset of {} should be a multiple of {}",
                                ty.name(),
                                ty.size()
                            ),
                        );
                    }
                    if off > total {
                        return self.throw_error(
                            "RangeError",
                            &format!("Start offset {off} is outside the bounds of the buffer"),
                        );
                    }
                    let len = match arg(args, 2) {
                        Value::Undefined => {
                            if (total - off) % ty.size() != 0 {
                                return self.throw_error(
                                    "RangeError",
                                    &format!(
                                        "byte length of {} should be a multiple of {}",
                                        ty.name(),
                                        ty.size()
                                    ),
                                );
                            }
                            (total - off) / ty.size()
                        }
                        v => self.num(&v) as usize,
                    };
                    if off + len * ty.size() > total {
                        return self.throw_error(
                            "RangeError",
                            &format!("Invalid typed array length: {len}"),
                        );
                    }
                    return Ok(self.new_typed(ty, o.clone(), data, off, len));
                }
                let items: Vec<Value> = if let Some(t) = Self::typed_of(&a0) {
                    t.to_vec().into_iter().map(Value::Number).collect()
                } else if let Some(items) = self.iter_protocol(&a0)? {
                    items
                } else {
                    let n = self.get(&a0, "length")?;
                    let n = self.num(&n);
                    let mut v = Vec::new();
                    for i in 0..(if n.is_nan() { 0 } else { n as usize }) {
                        v.push(self.get(&a0, &i.to_string())?);
                    }
                    v
                };
                let out = self.typed_with_len(ty, items.len());
                let t = Self::typed_of(&out).unwrap();
                for (i, v) in items.iter().enumerate() {
                    let n = self.num(v);
                    t.set(i, n);
                }
                Ok(out)
            }
            other => {
                let n = self.num(other);
                Ok(self.typed_with_len(ty, if n.is_nan() { 0 } else { n as usize }))
            }
        }
    }

    pub(crate) fn install_typed(&mut self) {
        let g = self.global.clone();
        // ----- ArrayBuffer -----
        let ab_proto = self.plain();
        self.rt.arraybuffer_proto = Some(ab_proto.clone());
        let ab = self.native(|it, _, args| {
            let n = it.num(&arg(args, 0));
            let n = if n.is_nan() { 0.0 } else { n };
            if !(0.0..=2f64.powi(32)).contains(&n) {
                return it.throw_error("RangeError", "Array buffer allocation failed");
            }
            let data: Bytes = Rc::new(RefCell::new(vec![0; n as usize]));
            Ok(Value::Object(it.new_array_buffer(data)))
        });
        if let Value::Object(c) = &ab {
            put_hidden(c, "prototype", Value::Object(ab_proto.clone()));
            put_hidden(&ab_proto, "constructor", ab.clone());
            self.set_native(c, "isView", |_, _, a| {
                Ok(Value::Bool(Self::typed_of(&arg(a, 0)).is_some()))
            });
        }
        self.set_native(&ab_proto, "slice", |it, this, a| {
            let Some(data) = Self::bytes_of_buffer(this) else {
                return it.throw_error("TypeError", "Receiver is not an ArrayBuffer");
            };
            let n = data.borrow().len() as f64;
            let rel = |it: &mut Interp, v: Value, d: f64| {
                let x = if matches!(v, Value::Undefined) {
                    d
                } else {
                    it.num(&v).trunc()
                };
                if x < 0.0 {
                    (n + x).max(0.0)
                } else {
                    x.min(n)
                }
            };
            let (s, e) = (rel(it, arg(a, 0), 0.0), rel(it, arg(a, 1), n));
            let out = if e > s {
                data.borrow()[s as usize..e as usize].to_vec()
            } else {
                Vec::new()
            };
            Ok(Value::Object(
                it.new_array_buffer(Rc::new(RefCell::new(out))),
            ))
        });
        g.declare("ArrayBuffer", ab, true);

        // ----- typed arrays -----
        let ta_proto = self.plain(); // %TypedArray%.prototype: methods come from the JS prelude
        self.rt.typed_base_proto = Some(ta_proto.clone());
        for ty in ALL {
            let proto = new_obj(Some(ta_proto.clone()), Kind::Plain);
            self.rt.typed_protos.push(proto.clone());
            let ctor = self.native(move |it, _, args| it.construct_typed(ty, args));
            if let Value::Object(c) = &ctor {
                put_hidden(c, "prototype", Value::Object(proto.clone()));
                put_hidden(&proto, "constructor", ctor.clone());
                put_hidden(c, "name", Value::str(ty.name()));
                put_hidden(c, "BYTES_PER_ELEMENT", Value::Number(ty.size() as f64));
                put_hidden(&proto, "BYTES_PER_ELEMENT", Value::Number(ty.size() as f64));
            }
            g.declare(ty.name(), ctor, true);
        }
        self.set_native(&ta_proto, "set", |it, this, a| {
            let Some(t) = Self::typed_of(this) else {
                return it.throw_error("TypeError", "this is not a typed array.");
            };
            let off = match arg(a, 1) {
                Value::Undefined => 0,
                v => it.num(&v) as usize,
            };
            let src: Vec<f64> = if let Some(s) = Self::typed_of(&arg(a, 0)) {
                s.to_vec()
            } else {
                let items = match it.iter_items(&arg(a, 0)) {
                    Some(i) => i,
                    None => {
                        let n = it.get(&arg(a, 0), "length")?;
                        let n = it.num(&n) as usize;
                        let mut v = Vec::new();
                        for i in 0..n {
                            v.push(it.get(&arg(a, 0), &i.to_string())?);
                        }
                        v
                    }
                };
                items.iter().map(|v| it.num(v)).collect()
            };
            if off + src.len() > t.len {
                return it.throw_error("RangeError", "offset is out of bounds");
            }
            for (i, v) in src.into_iter().enumerate() {
                t.set(off + i, v);
            }
            Ok(Value::Undefined)
        });
        self.set_native(&ta_proto, "subarray", |it, this, a| {
            let Some(t) = Self::typed_of(this) else {
                return it.throw_error("TypeError", "this is not a typed array.");
            };
            let n = t.len as f64;
            let rel = |it: &mut Interp, v: Value, d: f64| {
                let x = if matches!(v, Value::Undefined) {
                    d
                } else {
                    it.num(&v).trunc()
                };
                if x < 0.0 {
                    (n + x).max(0.0)
                } else {
                    x.min(n)
                }
            };
            let (s, e) = (rel(it, arg(a, 0), 0.0), rel(it, arg(a, 1), n));
            let len = if e > s { (e - s) as usize } else { 0 };
            let view = it.new_typed(
                t.ty,
                t.abuf.clone(),
                t.data.clone(),
                t.off + s as usize * t.ty.size(),
                len,
            );
            // keep the receiver's prototype (Buffer subclasses)
            if let (Value::Object(v), Value::Object(r)) = (&view, this) {
                *v.proto.borrow_mut() = r.proto.borrow().clone();
            }
            Ok(view)
        });
        self.set_native(&ta_proto, "slice", |it, this, a| {
            let Some(t) = Self::typed_of(this) else {
                return it.throw_error("TypeError", "this is not a typed array.");
            };
            let n = t.len as f64;
            let rel = |it: &mut Interp, v: Value, d: f64| {
                let x = if matches!(v, Value::Undefined) {
                    d
                } else {
                    it.num(&v).trunc()
                };
                if x < 0.0 {
                    (n + x).max(0.0)
                } else {
                    x.min(n)
                }
            };
            let (s, e) = (rel(it, arg(a, 0), 0.0), rel(it, arg(a, 1), n));
            let (s, e) = (s as usize, (e as usize).max(s as usize));
            let out = it.typed_with_len(t.ty, e - s);
            let o = Self::typed_of(&out).unwrap();
            for i in s..e {
                o.set(i - s, t.get(i).unwrap_or(0.0));
            }
            if let (Value::Object(v), Value::Object(r)) = (&out, this) {
                *v.proto.borrow_mut() = r.proto.borrow().clone();
            }
            Ok(out)
        });
        self.run(TYPED_PRELUDE)
            .unwrap_or_else(|_| panic!("thaw-js typed array prelude must evaluate"));
    }
}

/// Generic methods are the Array ones (they only use `length` and indexing); the ones that must
/// build a typed result are written out.
const TYPED_PRELUDE: &str = r#"
(function () {
  function def(o, name, f) {
    Object.defineProperty(o, name, { value: f, writable: true, enumerable: false, configurable: true });
  }
  var AP = Array.prototype;
  var TP = Object.getPrototypeOf(Uint8Array.prototype);
  ['forEach', 'some', 'every', 'find', 'findIndex', 'findLast', 'findLastIndex', 'reduce', 'reduceRight', 'fill', 'at', 'toString', 'keys', 'values', 'entries'].forEach(function (n) {
    if (AP[n]) def(TP, n, AP[n]);
  });
  def(TP, 'toLocaleString', AP.toString);
  def(TP, 'indexOf', function (v, from) {
    from = from === undefined ? 0 : (from < 0 ? Math.max(this.length + from, 0) : from);
    for (var i = from; i < this.length; i++) if (this[i] === v) return i;
    return -1;
  });
  def(TP, 'lastIndexOf', function (v, from) {
    from = from === undefined ? this.length - 1 : (from < 0 ? this.length + from : Math.min(from, this.length - 1));
    for (var i = from; i >= 0; i--) if (this[i] === v) return i;
    return -1;
  });
  def(TP, 'join', function (sep) {
    sep = sep === undefined ? ',' : String(sep);
    var out = '';
    for (var i = 0; i < this.length; i++) out += (i ? sep : '') + this[i];
    return out;
  });
  def(TP, 'includes', function (v, from) {
    for (var i = from || 0; i < this.length; i++) if (this[i] === v || (v !== v && this[i] !== this[i])) return true;
    return false;
  });
  def(TP, 'reverse', function () {
    for (var i = 0, j = this.length - 1; i < j; i++, j--) { var t = this[i]; this[i] = this[j]; this[j] = t; }
    return this;
  });
  def(TP, 'map', function (f, t) {
    var r = new this.constructor(this.length);
    for (var i = 0; i < this.length; i++) r[i] = f.call(t, this[i], i, this);
    return r;
  });
  def(TP, 'filter', function (f, t) {
    var kept = [];
    for (var i = 0; i < this.length; i++) if (f.call(t, this[i], i, this)) kept.push(this[i]);
    return new this.constructor(kept);
  });
  def(TP, 'sort', function (cmp) {
    var a = Array.from(this);
    a.sort(cmp || function (x, y) { return x < y ? -1 : x > y ? 1 : 0; });
    for (var i = 0; i < a.length; i++) this[i] = a[i];
    return this;
  });
  def(TP, 'toSorted', function (cmp) { return this.slice().sort(cmp); });
  def(TP, 'toReversed', function () { return this.slice().reverse(); });
  def(TP, 'copyWithin', function (target, start, end) {
    var n = this.length, vals = Array.from(this);
    target = target < 0 ? Math.max(n + target, 0) : Math.min(target, n);
    start = start === undefined ? 0 : (start < 0 ? Math.max(n + start, 0) : Math.min(start, n));
    end = end === undefined ? n : (end < 0 ? Math.max(n + end, 0) : Math.min(end, n));
    for (var i = 0; i < end - start && target + i < n; i++) this[target + i] = vals[start + i];
    return this;
  });
  [Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array, Uint32Array, Float32Array, Float64Array].forEach(function (C) {
    def(C, 'from', function (src, mapFn, thisArg) {
      var items = Array.from(src);
      if (mapFn) items = items.map(function (v, i) { return mapFn.call(thisArg, v, i); });
      return new this(items);
    });
    def(C, 'of', function () { return new this(Array.from(arguments)); });
  });
})();
"#;
