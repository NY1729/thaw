  // Thaw represents a `Date` natively as a fixed object with a single
  // `timestamp` (milliseconds since epoch) field -- see thaw-hir's
  // `date_object_type` and thaw-bridge's matching `.d.ts` `Date`
  // classification. Crossing the `callDynamic` JSON boundary, a real JS
  // `Date` instance would otherwise serialize via its own default
  // `toJSON` (an ISO string) and a `{ timestamp }` argument would arrive
  // as a plain object not a `Date`, so neither side would recognize the
  // other's representation. These two hooks make the `{"timestamp": N}`
  // shape the shared wire format in both directions:
  //  - overriding `toJSON` here means every `JSON.stringify` in this
  //    runtime (the `callDynamic` return path, but also anything nested
  //    inside a returned array/object) emits `{"timestamp": N}` for any
  //    `Date`, at any depth, for free;
  //  - the reviver is applied explicitly by `invoke_impl` (thaw-quickjs's
  //    Rust side) when parsing incoming call arguments, reconstructing a
  //    real `Date` from that same shape so native functions doing
  //    `instanceof Date`/`typeof` checks (e.g. date-fns's `toDate`) see
  //    the value they expect.
  Date.prototype.toJSON = function () {
    return { timestamp: this.getTime() };
  };
  // This reviver also recognizes `{"__thaw_js_handle_id__": N}` -- a
  // `JsValue` (an opaque handle to a live QuickJS object, e.g. a schema
  // instance returned by a Fallback function like zod's `z.string()`)
  // crossing a `callDynamic` JSON argument boundary the same way a `Date`
  // does: there's no JSON encoding of "a live JS object", so
  // `compile_dynamic_value_placeholder` (thaw-llvm's `json_bridge.rs`)
  // encodes the handle's own (permanent, lookup-table) id as this shape
  // instead, wherever the value would otherwise sit in the argument JSON
  // -- bare, or nested inside an object/array literal, at any depth,
  // since the reviver runs bottom-up over the whole parsed value just
  // like it already does for `Date`. Reviving it back to the real value
  // here, rather than a second Rust-side walk, reuses the exact same
  // hook this file already wires into every JSON-argument parse.
  globalThis.__thaw_json_date_reviver = (key, value) => {
    if (typeof value === 'string' && value.charCodeAt(0) === 1) {
      const separator = value.indexOf('\u0001', 1);
      if (separator > 1) {
        const error = new Error(value.slice(separator + 1));
        error.name = value.slice(1, separator);
        return error;
      }
    }
    if (
      value &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      Object.keys(value).length === 1 &&
      Array.isArray(value.__thaw_map_entries__)
    ) {
      return new Map(value.__thaw_map_entries__);
    }
    if (
      value &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      Object.keys(value).length === 1 &&
      Array.isArray(value.__thaw_set_values__)
    ) {
      return new Set(value.__thaw_set_values__);
    }
    if (
      value &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      Object.keys(value).length === 1 &&
      value.__thaw_regexp__ &&
      typeof value.__thaw_regexp__.source === 'string'
    ) {
      const pattern = new RegExp(
        value.__thaw_regexp__.source,
        value.__thaw_regexp__.flags || '',
      );
      pattern.lastIndex = Number(value.__thaw_regexp__.lastIndex || 0);
      return pattern;
    }
    if (
      value &&
      value.type === 'Buffer' &&
      Array.isArray(value.data)
    ) {
      return Buffer.from(value.data);
    }
    if (
      value &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      Object.keys(value).length === 1 &&
      typeof value.timestamp === 'number'
    ) {
      return new Date(value.timestamp);
    }
    if (
      value &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      Object.keys(value).length === 1 &&
      typeof value.__thaw_js_handle_id__ === 'number'
    ) {
      const id = value.__thaw_js_handle_id__;
      const live = globalThis.__thaw_value_handle_live;
      if (!live || !live[id - 1]) {
        throw new Error('invalid or released dynamic value handle ' + id);
      }
      return globalThis.__thaw_value_handles[id - 1];
    }
    // A bare `undefined` argument (`schema.safeParse(undefined)`, real
    // zod's own `z.undefined()`) has no JSON encoding either -- `coerce_
    // to_declared` (thaw-hir) encodes it as this same sentinel shape
    // `compile_napi_undefined_json` already uses for a NAPI return
    // value, reused here rather than inventing a second one, even
    // though it crosses a different boundary (a `callDynamic` JSON
    // argument, not a NAPI result).
    if (
      value &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      value.type === 'Buffer' &&
      Array.isArray(value.data)
    ) {
      return Buffer.from(value.data);
    }
    if (
      value &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      Object.keys(value).length === 1 &&
      value.$__thaw_napi_undefined$ === true
    ) {
      return undefined;
    }
    return value;
  };
  globalThis.__thaw_json_binary_replacer = function (key, value) {
    const source = this[key];
    if (source instanceof ArrayBuffer) {
      return { type: 'Buffer', data: Array.from(new Uint8Array(source)) };
    }
    if (ArrayBuffer.isView(source)) {
      return {
        type: 'Buffer',
        data: Array.from(new Uint8Array(source.buffer, source.byteOffset, source.byteLength)),
      };
    }
    return value;
  };
  globalThis.__thaw_json_safe_stringify = function (value) {
    const ancestors = [];
    return JSON.stringify(value, function (key, nested) {
      nested = globalThis.__thaw_json_binary_replacer.call(this, key, nested);
      if (nested && typeof nested === 'object') {
        while (ancestors.length && ancestors[ancestors.length - 1] !== this) ancestors.pop();
        if (ancestors.includes(nested)) return undefined;
        ancestors.push(nested);
      }
      return nested;
    });
  };
