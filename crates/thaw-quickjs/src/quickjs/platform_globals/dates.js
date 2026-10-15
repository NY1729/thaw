  // The native Date wire shape is `{timestamp: milliseconds}`. The
  // bridge's own stringify replacer emits it when crossing into native
  // JSON; user JSON.stringify keeps Date.prototype.toJSON (including an
  // instance's custom override) untouched.
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
  const thawSymbols = new Map();
  const nativeSymbol = Symbol;
  const nativeSymbolFor = Symbol.for.bind(Symbol);
  const thawPropertyKey = (value) => {
    if (typeof value !== 'string') return value;
    if (value.startsWith('\u001f@@')) return Symbol[value.slice(3)] || value;
    if (value.startsWith('__thaw_symbol_')) return Symbol[value.slice(14)] || value;
    if (value.charCodeAt(0) !== 3) return value;
    if (!thawSymbols.has(value)) {
      const separator = value.indexOf(':');
      const registered = value.charCodeAt(1) === 82; // native `\u0003R{id}:description`
      if (registered && !/^\u0003R[0-9]+:/.test(value)) return value;
      const description = separator < 0 ? undefined : value.slice(separator + 1);
      thawSymbols.set(value, registered ? nativeSymbolFor(description) : nativeSymbol(description));
    }
    return thawSymbols.get(value);
  };
  // Native graph and static-property callers share this bootstrap decoder.
  // User code may run before either caller's first lookup, so publish the
  // original function as an immutable property while the realm is starting.
  thawGraphDefineProperty(globalThis, '__thaw_property_key', {
    value: thawPropertyKey, writable: false, configurable: false, enumerable: true,
  });
  globalThis.__thaw_json_date_reviver = (key, value) => {
    if (
      typeof value === 'string' &&
      (value.charCodeAt(0) === 3 ||
        value.startsWith('\u001f@@') ||
        value.startsWith('__thaw_symbol_'))
    ) {
      return globalThis.__thaw_property_key(value);
    }
    if (typeof value === 'string' && value.charCodeAt(0) === 1) {
      const frame = globalThis.__thaw_error_frame_parts(value);
      if (frame) {
        const error = new Error(frame.original === null ? frame.display : frame.original);
        error.name = frame.name;
        return error;
      }
      const separator = value.indexOf('\u0001', 1);
      if (separator > 1) {
        const error = new Error(value.slice(separator + 1));
        error.name = value.slice(1, separator);
        return error;
      }
    }
    // shortcut: the reserved legacy tag shape can collide with user JSON; origins would be needed to disambiguate it.
    if (
      value &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      Object.keys(value).length === 1 &&
      Object.prototype.hasOwnProperty.call(value, '$__thaw_non_finite$')
    ) {
      const tag = value.$__thaw_non_finite$;
      if (tag === 'NaN') return NaN;
      if (tag === 'Infinity') return Infinity;
      if (tag === '-Infinity') return -Infinity;
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
    // A compiled object method keyed by a well-known symbol (`[Symbol.dispose]`,
    // `[Symbol.asyncDispose]`, ...) crosses as a `"\u001f@@<name>"` string key
    // (thaw-hir's `well_known_symbol_key`, which can't collide with a real
    // property name). Re-key it onto the real symbol here so a JS API that
    // looks it up (`DisposableStack.prototype.use`) finds it. The reviver runs
    // bottom-up, so nested objects are already revived by the time their
    // parent is processed.
    if (value && typeof value === 'object') {
      for (const key of Object.keys(value)) {
        if (key.startsWith('\u001f@@') || key.startsWith('__thaw_symbol_')) {
          const symbol = thawPropertyKey(key);
          if (typeof symbol === 'symbol') {
            Object.defineProperty(value, symbol, {
              value: value[key],
              writable: true,
              enumerable: false,
              configurable: true,
            });
            delete value[key];
          }
        }
      }
    }
    return value;
  };
  // `JsValue + JsValue`: a compiled `String(x) + String(y)` can't tell a
  // dynamic bigint from a dynamic string, so the compiled `+` delegates
  // here and lets JS pick numeric addition, concatenation, or BigInt
  // addition.
  globalThis.__thaw_dynamic_add = (left, right) => left + right;
  globalThis.__thaw_json_binary_replacer = function (key, value) {
    const source = this[key];
    if (source instanceof Date) return { timestamp: source.getTime() };
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
  // Legacy-wire parse: plain `JSON.parse` followed by a post-order restoration
  // pass that applies `__thaw_json_date_reviver`'s conversions. A reviver
  // cannot be used directly because returning `undefined` from it deletes the
  // property (`{x: undefined}` would lose `x`, `[undefined]` would become a
  // hole); here the restored value is always defined as an own data property.
  globalThis.__thaw_json_parse_legacy = function (text) {
    const revive = globalThis.__thaw_json_date_reviver;
    const define = (holder, key, value) => Object.defineProperty(holder, key, {
      value, writable: true, enumerable: true, configurable: true,
    });
    const walk = (holder, key) => {
      const value = holder[key];
      if (value !== null && typeof value === 'object') {
        for (const child of Object.keys(value)) walk(value, child);
      }
      const revived = revive.call(holder, key, value);
      if (revived !== value) define(holder, key, revived);
    };
    const root = {};
    define(root, '', JSON.parse(text));
    walk(root, '');
    return root[''];
  };
  // Legacy-wire stringify: an acquired-value snapshot. Every own property is
  // read exactly once (a getter runs once, and a getter that throws on a
  // second call still works); the Date / byte-view / non-finite / undefined
  // decisions are made from that acquired value, never from a re-read of
  // `holder[key]`. Nested `undefined` uses the same sentinel as the top-level
  // result (`null` and a missing property stay distinguishable).
  const OMIT = Symbol('omit');
  const snapshot = (holder, key, value, ancestors, mark) => {
    if (value instanceof Date) return { timestamp: value.getTime() };
    if (value instanceof ArrayBuffer) {
      return { type: 'Buffer', data: Array.from(new Uint8Array(value)) };
    }
    if (ArrayBuffer.isView(value)) {
      return {
        type: 'Buffer',
        data: Array.from(new Uint8Array(value.buffer, value.byteOffset, value.byteLength)),
      };
    }
    if (value !== null && (typeof value === 'object' || typeof value === 'bigint')) {
      const toJSON = value.toJSON;
      if (typeof toJSON === 'function') value = toJSON.call(value, key);
    }
    if (value === undefined) return mark ? { $__thaw_napi_undefined$: true } : OMIT;
    if (typeof value === 'function' || typeof value === 'symbol') return OMIT;
    if (typeof value === 'bigint') throw new TypeError('Do not know how to serialize a BigInt');
    if (typeof value === 'number') {
      if (Number.isFinite(value)) return value;
      return { $__thaw_non_finite$: Number.isNaN(value) ? 'NaN' : value > 0 ? 'Infinity' : '-Infinity' };
    }
    if (value === null || typeof value !== 'object') return value;
    if (value instanceof Number || value instanceof String || value instanceof Boolean) {
      return snapshot(holder, key, value.valueOf(), ancestors, mark);
    }
    if (ancestors.includes(value)) return OMIT;
    ancestors.push(value);
    let out;
    if (Array.isArray(value)) {
      out = [];
      for (let index = 0; index < value.length; index++) {
        const item = Object.prototype.hasOwnProperty.call(value, index)
          ? snapshot(value, String(index), value[index], ancestors, mark)
          : null;
        out.push(item === OMIT ? null : item);
      }
    } else {
      out = Object.create(null);
      for (const name of Object.keys(value)) {
        const item = snapshot(value, name, value[name], ancestors, mark);
        if (item !== OMIT) out[name] = item;
      }
    }
    ancestors.pop();
    return out;
  };
  globalThis.__thaw_json_safe_stringify = function (value, mark) {
    const encoded = snapshot({ '': value }, '', value, [], !!mark);
    if (value === undefined || encoded === OMIT) return undefined;
    const text = JSON.stringify(encoded);
    // Well-formed `JSON.stringify` escapes a lone surrogate as `\uXXXX`,
    // but if the engine left one raw it would break the native Rust
    // `String` conversion -- escape any that remain.
    let escaped = '';
    for (let i = 0; i < text.length; i++) {
      const code = text.charCodeAt(i);
      if (code >= 0xD800 && code <= 0xDFFF) {
        const paired = code <= 0xDBFF && i + 1 < text.length
          && text.charCodeAt(i + 1) >= 0xDC00 && text.charCodeAt(i + 1) <= 0xDFFF;
        if (paired) {
          escaped += text[i] + text[i + 1];
          i++;
        } else {
          escaped += '\\u' + code.toString(16).padStart(4, '0');
        }
      } else {
        escaped += text[i];
      }
    }
    return escaped;
  };
