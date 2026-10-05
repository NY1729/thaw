  const thawAsyncContext = globalThis.__thaw_async_context_bootstrap;
  globalThis.global = globalThis;
  const thawDateConstructor = Date;
  const thawDateGetTime = Date.prototype.getTime;
  const thawDateApply = Reflect.apply;
  const thawBootstrapDefineProperty = Object.defineProperty;
  const thawBootstrapIteratorKey = Symbol.iterator;
  Object.defineProperty(globalThis, '__thaw_date_from_live_value', {
    value: value => thawDateApply(thawDateGetTime, new thawDateConstructor(value), []),
    configurable: false, writable: false,
  });
  // Used by thaw-bridge's generated module-bootstrap glue
  // (`crates/thaw-bridge/src/bridge/generation.rs`) wherever a package
  // export gets `.bind()`-captured onto a bare/qualified global -- a
  // plain `fn.bind(receiver)` was originally added so a *stateful
  // namespace object*'s method keeps its receiver when called
  // detached (real example: joi's `Root.string()`, handlebars'
  // `registerHelper`), but a bound function is a genuinely different
  // function object that carries none of the original's own
  // properties -- for a real ES6 class/constructor export (real
  // example: luxon's `DateTime`), that silently drops every static
  // method (`DateTime.fromISO`, ...), since `static` methods are
  // (correctly, per spec) non-enumerable own properties of the class
  // itself, invisible to a plain `Object.assign`. This copies every
  // own property descriptor (enumerable or not) from the original
  // onto the bound wrapper, skipping the handful `Function.prototype.
  // bind` already gives the wrapper its own, correct versions of
  // (`length`/`name`/`prototype`) or that would throw if copied
  // (`arguments`/`caller`, non-configurable on a bound function).
  globalThis.__thaw_bind_preserving_statics = (fn, receiver) => {
    if (typeof fn !== 'function') return fn;
    const bound = Function.prototype.bind.call(fn, receiver);
    for (const prop of Object.getOwnPropertyNames(fn)) {
      if (prop === 'length' || prop === 'name' || prop === 'prototype'
        || prop === 'arguments' || prop === 'caller') continue;
      try {
        Object.defineProperty(bound, prop, Object.getOwnPropertyDescriptor(fn, prop));
      } catch (error) { /* non-configurable on `fn` itself -- leave unset */ }
    }
    return bound;
  };
  const nativeBigInt = BigInt;
  globalThis.__thaw_native_loose_equal = (left, right, leftKind, rightKind) => {
    const decode = (value, kind) => kind === 1 ? nativeBigInt(value)
      : kind === 2 ? globalThis.__thaw_property_key(value) : value;
    if ((leftKind === 3 || rightKind === 3) &&
        globalThis.__thaw_same_native_callback(left, right)) return true;
    return decode(left, leftKind) == decode(right, rightKind);
  };
  globalThis.__thaw_strict_equal_dynamic = (left, right, leftKind, rightKind) =>
    ((leftKind === 3 || rightKind === 3) &&
      globalThis.__thaw_same_native_callback(left, right)) || left === right;
  globalThis.__thaw_strict_equal_bigint_dynamic = (digits, value) =>
    typeof value === 'bigint' && value === BigInt(digits);
  globalThis.__thaw_strict_equal_number_dynamic = (text, value) =>
    typeof value === 'number' && value === Number(text);
  globalThis.__thaw_strict_equal_null_dynamic = value => value === null;
  globalThis.__thaw_strict_equal_undefined_dynamic = value => value === undefined;
  globalThis.__thaw_typeof_dynamic_value = value => typeof value;
  globalThis.__thaw_is_undefined_dynamic_value = value => value === undefined;
  globalThis.__thaw_is_null_dynamic_value = value => value === null;
  globalThis.__thaw_is_nullish_dynamic_value = value => value == null;
  globalThis.__thaw_is_array_dynamic_value = value => Array.isArray(value);
  thawBootstrapDefineProperty(globalThis, '__thaw_to_iterator', {
    value: value => {
      if (value == null) return value;
      const method = value[thawBootstrapIteratorKey];
      const iterator = typeof method === 'function'
        ? thawDateApply(method, value, []) : value;
      if (iterator == null) return iterator;
      const next = iterator.next;
      if (typeof next !== 'function') return iterator;
      return {
        next: () => thawDateApply(next, iterator, []),
        return: () => {
          const close = iterator.return;
          return typeof close === 'function'
            ? thawDateApply(close, iterator, []) : { value: undefined, done: true };
        },
        throw: error => {
          const resume = iterator.throw;
          return typeof resume === 'function'
            ? thawDateApply(resume, iterator, [error])
            : { value: undefined, done: true };
        },
      };
    }, enumerable: true, configurable: false, writable: false,
  });
  globalThis.__thaw_json_stringify_replacer = (value, space, replacer) =>
    JSON.stringify(value, function (key, item) {
      return thawDateApply(replacer, this, [key, item]);
    }, space);
  const thawGraphDateConstructor = Date;
  const thawGraphMapConstructor = Map;
  const thawGraphWeakMapConstructor = WeakMap;
  const thawGraphArrayConstructor = Array;
  const thawGraphArrayBufferConstructor = ArrayBuffer;
  const thawGraphUint8ArrayConstructor = Uint8Array;
  const thawGraphSetConstructor = Set;
  const thawGraphRegExpConstructor = RegExp;
  const thawGraphBigInt = BigInt;
  const thawGraphNumber = Number;
  const thawGraphNumberIsFinite = Number.isFinite;
  const thawGraphNumberIsNaN = Number.isNaN;
  const thawGraphNumberIsInteger = Number.isInteger;
  const thawGraphNumberIsSafeInteger = Number.isSafeInteger;
  const thawGraphString = String;
  const thawGraphTypeError = TypeError;
  const thawGraphParse = JSON.parse;
  const thawGraphStringify = JSON.stringify;
  const thawGraphObjectKeys = Object.keys;
  const thawGraphObjectGetOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
  const thawGraphObjectEntries = Object.entries;
  const thawGraphObjectValues = Object.values;
  const thawGraphObjectCreate = Object.create;
  const thawGraphDefineProperty = Object.defineProperty;
  const thawGraphProxy = Proxy;
  const thawNapiIntrinsicObject = Object;
  const thawGraphReflectGet = Reflect.get;
  const thawGraphReflectSet = Reflect.set;
  const thawGraphStringConcat = String.prototype.concat;
  const thawGraphStringCodeUnitForCoercion = String.prototype.charCodeAt;
  thawGraphDefineProperty(globalThis, '__thaw_json_host_abstract_to_string_units', {
    value: value => {
      // String(value) accepts Symbol; the abstract ToString used by N-API
      // does not.  concat performs ToString with the string hint.
      const string = thawGraphApply(thawGraphStringConcat, '', [value]);
      const units = [];
      for (let index = 0; index < string.length; index++)
        thawGraphApply(thawGraphArrayPush, units,
          [thawGraphApply(thawGraphStringCodeUnitForCoercion, string, [index])]);
      return thawGraphStringify(units);
    }, configurable: false, writable: false,
  });
  thawGraphDefineProperty(globalThis, '__thaw_json_host_set_property', {
    value: (object, key, value) => thawGraphReflectSet(object, key, value),
    configurable: false, writable: false,
  });
  thawGraphDefineProperty(globalThis, '__thaw_json_host_set_receiver_data', {
    value: (receiver, key, value) => {
      // Ordinary [[Set]] for a writable data descriptor tests the receiver's
      // own descriptor. An own accessor must fail rather than invoke its setter.
      const target = {};
      thawGraphDefineProperty(target, key,
        { value: undefined, writable: true, configurable: true });
      return thawGraphReflectSet(target, key, value, receiver);
    }, configurable: false, writable: false,
  });
  thawGraphDefineProperty(globalThis, '__thaw_json_host_parse_property_key', {
    value: text => {
      const key = thawGraphParse(text);
      if (typeof key !== 'string')
        throw new thawGraphTypeError('Invalid host property key');
      return key;
    }, configurable: false, writable: false,
  });
  const thawGraphReflectDefineProperty = Reflect.defineProperty;
  const thawNapiBootstrapDeleteProperty = Reflect.deleteProperty;
  const thawGraphReflectPreventExtensions = Reflect.preventExtensions;
  const thawGraphReflectGetOwnPropertyDescriptor = Reflect.getOwnPropertyDescriptor;
  const thawGraphObjectIsSealed = Object.isSealed;
  const thawGraphObjectIsFrozen = Object.isFrozen;
  const thawHostObjectSeal = Object.seal;
  const thawHostObjectFreeze = Object.freeze;
  const thawHostNativeSymbolFor = Symbol.for;
  const thawHostNativeSymbol = Symbol;
  const thawHostNativeSymbols = new thawGraphMapConstructor();
  const thawHostWeakRef = typeof WeakRef === 'function' ? WeakRef : null;
  const thawHostWeakDeref = thawHostWeakRef && thawHostWeakRef.prototype.deref;
  const thawHostFinalizationRegistry = typeof FinalizationRegistry === 'function'
    ? FinalizationRegistry : null;
  const thawHostFinalizerRegister = thawHostFinalizationRegistry
    && thawHostFinalizationRegistry.prototype.register;
  const thawHostWeakSymbols = !!thawHostWeakRef
    && !!thawHostFinalizationRegistry;
  const thawHostNativeSymbolIDs = new thawGraphWeakMapConstructor();
  const thawHostRegisteredSymbolIDs = new thawGraphMapConstructor();
  const thawHostSymbolUnpin = globalThis.__thaw_napi_graph_symbol_unpin;
  const thawHostNativeSymbolPin = globalThis.__thaw_napi_graph_symbol_pin;
  const thawHostSymbolFinalizer = thawHostWeakSymbols
    ? new thawHostFinalizationRegistry(held => {
        if (thawGraphApply(thawGraphMapGet, thawHostNativeSymbols, [held.id])
          !== held.entry) return;
        thawGraphApply(thawGraphMapDelete, thawHostNativeSymbols, [held.id]);
        thawHostSymbolUnpin(held.id);
      }) : null;
  const thawHostNativeSymbolMetadata = handle => {
    if (typeof handle !== 'string'
      || !thawGraphApply(thawGraphRegExpTest,
        thawGraphPositiveDecimalPattern, [handle]))
      throw new thawGraphTypeError('Invalid native Symbol handle');
    const reply = thawGraphParse(thawGraphNapiBridgeHandle('symbol_metadata',
      handle, '', '[]'));
    if (!reply || reply.__thaw_error__ || !reply.value
      || typeof reply.value.id !== 'string'
      || !thawGraphApply(thawGraphRegExpTest,
        thawGraphPositiveDecimalPattern, [reply.value.id])
      || reply.value.handle !== handle
      || !thawGraphArrayIsArray(reply.value.units)
      || typeof reply.value.registered !== 'boolean'
      || typeof reply.value.jsOrigin !== 'boolean')
      throw new thawGraphTypeError('Invalid native Symbol metadata');
    return reply.value;
  };
  thawGraphDefineProperty(globalThis, '__thaw_json_host_native_symbol_needs_pin', {
    value: handle => {
      const metadata = thawHostNativeSymbolMetadata(handle);
      return !metadata.jsOrigin && !metadata.registered;
    },
    configurable: false, writable: false,
  });
  const thawHostNativeSymbolID = symbol =>
    thawGraphApply(thawGraphWeakMapGet, thawHostNativeSymbolIDs, [symbol])
      ?? thawGraphApply(thawGraphMapGet, thawHostRegisteredSymbolIDs, [symbol]);
  thawGraphDefineProperty(globalThis, '__thaw_json_host_native_symbol', {
    value: handle => {
      const metadata = thawHostNativeSymbolMetadata(handle);
      const id = metadata.id;
      if (metadata.jsOrigin) {
        const owner = thawGraphNapiOwnerOfHandle(handle);
        const state = owner && thawGraphNapiSymbolState(owner);
        const original = state && state.symbolForNative(id);
        if (typeof original !== 'symbol')
          throw new thawGraphTypeError('Original JavaScript Symbol is no longer live');
        return original;
      }
      const priorEntry = thawGraphApply(thawGraphMapGet, thawHostNativeSymbols, [id]);
      const previous = priorEntry && priorEntry.weak
        ? thawGraphApply(thawHostWeakDeref, priorEntry.weak, [])
        : priorEntry && priorEntry.symbol;
      if (previous !== undefined) return previous;
      const description = globalThis.__thaw_json_host_utf16_string(
        thawGraphStringify(metadata.units));
      if (!metadata.registered && !thawHostWeakSymbols)
        throw new thawGraphTypeError('Local native Symbol requires weak identity support');
      const symbol = metadata.registered ? thawHostNativeSymbolFor(description)
        : thawHostNativeSymbol(description);
      if (metadata.registered) {
        thawGraphApply(thawGraphMapSet, thawHostNativeSymbols,
          [id, { symbol }]);
        thawGraphApply(thawGraphMapSet, thawHostRegisteredSymbolIDs, [symbol, id]);
      } else {
        const entry = { weak: new thawHostWeakRef(symbol) };
        thawGraphApply(thawGraphMapSet, thawHostNativeSymbols, [id, entry]);
        thawGraphApply(thawGraphWeakMapSet, thawHostNativeSymbolIDs, [symbol, id]);
        thawGraphApply(thawHostFinalizerRegister, thawHostSymbolFinalizer,
          [symbol, { id, entry }]);
      }
      return symbol;
    }, configurable: false, writable: false,
  });
  thawGraphDefineProperty(globalThis, '__thaw_json_host_object_integrity', {
    value: (object, freeze) => {
      thawGraphApply(freeze ? thawHostObjectFreeze : thawHostObjectSeal, undefined, [object]);
      return true;
    }, configurable: false, writable: false,
  });
  thawGraphDefineProperty(globalThis, '__thaw_json_host_define_data_property', {
    value: (object, key, value, attributes) => {
      thawGraphDefineProperty(object, key, {
        value,
        writable: (attributes & 1) !== 0,
        enumerable: (attributes & 2) !== 0,
        configurable: (attributes & 4) !== 0,
      });
      return true;
    }, configurable: false, writable: false,
  });
  thawGraphDefineProperty(globalThis, '__thaw_json_host_define_callback_property', {
    value: (object, key, getterId, setterId, methodId, attributes, invoke) => {
      const callback = id => function (...args) {
        const packet = new thawGraphArrayConstructor();
        thawGraphApply(thawGraphArrayPush, packet, [this]);
        for (let index = 0; index < args.length; index++)
          thawGraphApply(thawGraphArrayPush, packet, [args[index]]);
        const input = thawGraphEncode(packet);
        const answer = thawGraphParse(invoke(String(id), input));
        if (answer && answer.__thaw_error__)
          throw new thawGraphTypeError(String(answer.__thaw_error__));
        if (!answer || answer.kind !== 'graph' || typeof answer.value !== 'string')
          throw new thawGraphTypeError('Invalid native callback result');
        return thawGraphDecodeOwned(answer.value);
      };
      if (methodId !== 0) {
        thawGraphDefineProperty(object, key, {
          value: callback(methodId), writable: (attributes & 1) !== 0,
          enumerable: (attributes & 2) !== 0,
          configurable: (attributes & 4) !== 0,
        });
      } else {
        thawGraphDefineProperty(object, key, {
          get: getterId !== 0 ? callback(getterId) : undefined,
          set: setterId !== 0 ? callback(setterId) : undefined,
          enumerable: (attributes & 2) !== 0,
          configurable: (attributes & 4) !== 0,
        });
      }
      return true;
    }, configurable: false, writable: false,
  });
  const thawGraphOwnProperty = Object.prototype.hasOwnProperty;
  const thawGraphArrayIsArray = Array.isArray;
  const thawGraphArrayFrom = Array.from;
  const thawGraphArrayMap = Array.prototype.map;
  const thawGraphArraySome = Array.prototype.some;
  const thawGraphArrayForEach = Array.prototype.forEach;
  const thawGraphArrayEvery = Array.prototype.every;
  const thawGraphArrayPush = Array.prototype.push;
  const thawGraphArraySlice = Array.prototype.slice;
  const thawGraphArrayJoin = Array.prototype.join;
  const thawGraphStringFromCharCode = String.fromCharCode;
  const thawGraphMapSet = Map.prototype.set;
  const thawGraphMapGet = Map.prototype.get;
  const thawGraphMapDelete = Map.prototype.delete;
  const thawGraphSetAdd = Set.prototype.add;
  const thawGraphSetHas = Set.prototype.has;
  const thawGraphWeakMapGet = WeakMap.prototype.get;
  const thawGraphWeakMapSet = WeakMap.prototype.set;
  const thawGraphWeakMapHas = WeakMap.prototype.has;
  const thawGraphRetain = globalThis.__thaw_retain_dynamic_value;
  const thawGraphRelease = globalThis.__thaw_release_dynamic_value;
  thawGraphDefineProperty(globalThis, '__thaw_json_host_strict_scalar_equal', {
    value: (left, kind, number, text) => {
      const right = kind === 0 ? undefined : kind === 1 ? null
        : kind === 2 ? number !== 0 : kind === 3 ? number
        : kind === 4 ? thawGraphParse(text) : kind === 5 ? thawGraphBigInt(text)
        : kind === 6 ? globalThis.__thaw_json_host_utf16_string(text)
        : (() => { throw new thawGraphTypeError('Invalid native scalar kind'); })();
      return left === right;
    }, configurable: false, writable: false,
  });
  thawGraphDefineProperty(globalThis, '__thaw_json_host_strict_handles_equal', {
    value: (left, right) => left === right,
    configurable: false, writable: false,
  });
  thawGraphDefineProperty(globalThis, '__thaw_json_host_utf16_string', {
    value: encoded => {
      const units = thawGraphParse(encoded);
      if (!thawGraphArrayIsArray(units)) throw new thawGraphTypeError('Expected UTF-16 units');
      const chunks = [];
      for (let start = 0; start < units.length; start += 4096) {
        const part = [];
        for (let index = start; index < units.length && index < start + 4096; index++) {
          const unit = units[index];
          if (!thawGraphNumberIsInteger(unit) || unit < 0 || unit > 65535)
            throw new thawGraphTypeError('Invalid UTF-16 unit');
          thawGraphApply(thawGraphArrayPush, part, [unit]);
        }
        thawGraphApply(thawGraphArrayPush, chunks,
          [thawGraphApply(thawGraphStringFromCharCode, thawGraphString, part)]);
      }
      return thawGraphApply(thawGraphArrayJoin, chunks, ['']);
    }, configurable: false, writable: false,
  });
  let thawGraphBufferFrom;
  let thawGraphBufferIsBuffer;
  const nativeDateGetTime = thawGraphDateConstructor.prototype.getTime;
  const nativeDateSetTime = thawGraphDateConstructor.prototype.setTime;
  globalThis.__thaw_json_host_date_set = (value, timestamp) =>
    thawGraphString(thawGraphApply(nativeDateSetTime, value, [timestamp]));
  globalThis.__thaw_json_host_query = (value, operation) => {
    switch (operation) {
      case 0: return typeof value;
      case 1: return value ? '1' : '0';
      case 2: return thawGraphString(thawGraphNumber(value));
      case 3: return thawGraphStringify(thawGraphString(value));
      case 4: return thawGraphArrayIsArray(value) ? '1' : '0';
      case 5: return value === undefined ? '1' : '0';
      case 6: return value == null ? '1' : '0';
      case 7: return value === null ? '1' : '0';
      case 8: return thawGraphStringify(value) ?? 'null';
      case 9: return value !== null && (typeof value === 'object' || typeof value === 'function') ? '1' : '0';
      case 10: return thawGraphObjectIs(value, -0) ? '-0' : thawGraphString(value);
      case 11: return thawGraphBufferIsBuffer(value) ? '1' : '0';
      case 12: {
        if (value === null || (typeof value !== 'object' && typeof value !== 'function')) return '0';
        // A revoked Proxy must propagate its error rather than be mistaken
        // for an ordinary object rejected by the Date internal-slot check.
        thawGraphGetPrototypeOf(value);
        // The captured native method checks the Date internal slot; a
        // matching prototype or `timestamp` property cannot impersonate it.
        try { thawGraphApply(nativeDateGetTime, value, []); return '1'; }
        catch (error) { if (error instanceof thawGraphTypeError) return '0'; throw error; }
      }
      case 13: return thawGraphString(thawGraphApply(nativeDateGetTime, value, []));
      // Slot 14 is reserved by the native API for captured JS_IsFunction.
      // Number conversion is deliberately separate because ToPrimitive can
      // execute user code, unlike a callable-kind query.
      case 16: {
        const number = thawGraphNumber(value);
        return thawGraphObjectIs(number, -0) ? '-0' : thawGraphString(number);
      }
      case 15: return thawHostStringUnits(value);
      default: throw new thawGraphTypeError('Invalid native host query');
    }
  };
  globalThis.__thaw_json_host_enumerate = (value, operation) => {
    const entries = operation === 0 ? thawGraphObjectEntries(value)
      : operation === 1 ? thawGraphReflectOwnKeys(value)
      : operation === 2 ? thawGraphObjectKeys(value)
      : operation === 3 ? thawGraphObjectValues(value)
      : null;
    if (entries === null) throw new thawGraphTypeError('Invalid native host enumeration');
    return thawGraphEncode(entries, 0, true);
  };
  thawGraphDefineProperty(globalThis, '__thaw_json_host_property_names', {
    value: (value, keyMode, keyFilter, keyConversion) => {
    const result = [];
    const seen = new thawGraphSetConstructor();
    let current = value;
    while (current !== null) {
      for (const key of thawGraphReflectOwnKeys(current)) {
        const symbol = typeof key === 'symbol';
        if (symbol ? (keyFilter & 16) !== 0 : (keyFilter & 8) !== 0) continue;
        const descriptor = thawGraphObjectGetOwnPropertyDescriptor(current, key);
        if (descriptor === undefined) continue;
        if (thawGraphApply(thawGraphSetHas, seen, [key])) continue;
        thawGraphApply(thawGraphSetAdd, seen, [key]);
        const attributes = (descriptor.writable ? 1 : 0)
          | (descriptor.enumerable ? 2 : 0)
          | (descriptor.configurable ? 4 : 0);
        const required = keyFilter & 7;
        if ((attributes & required) !== required) continue;
        let outputKey = key;
        if (!symbol && keyConversion === 0) {
          const number = thawGraphNumber(key);
          if (thawGraphNumberIsInteger(number) && number >= 0
            && number < 4294967295 && thawGraphString(number) === key)
            outputKey = number;
        }
        thawGraphApply(thawGraphArrayPush, result, [outputKey]);
      }
      if (keyMode === 1) break;
      current = thawGraphGetPrototypeOf(current);
    }
    return thawGraphEncode(result, 0, true);
    }, configurable: false, writable: false,
  });
  // The native Json bridge sends an index-based graph only for the function
  // replacer route. Allocate every node first so back edges and sibling aliases
  // recover exactly one JS object; user keys live in pairs, outside metadata.
  const thawGraphApply = Reflect.apply;
  const thawGraphStringCodeUnit = String.prototype.charCodeAt;
  const thawHostStringUnits = value => {
    if (typeof value !== 'string') throw new thawGraphTypeError('Expected string');
    const units = [];
    for (let index = 0; index < value.length; index++)
      thawGraphApply(thawGraphArrayPush, units,
        [thawGraphApply(thawGraphStringCodeUnit, value, [index])]);
    return thawGraphStringify(units);
  };
  const thawMalformedStringUnits = value => {
    const units = [];
    let malformed = false;
    for (let index = 0; index < value.length; index++) {
      const unit = thawGraphApply(thawGraphStringCodeUnit, value, [index]);
      thawGraphApply(thawGraphArrayPush, units, [unit]);
      if (unit >= 0xd800 && unit <= 0xdbff) {
        const next = index + 1 < value.length
          ? thawGraphApply(thawGraphStringCodeUnit, value, [index + 1]) : -1;
        if (next >= 0xdc00 && next <= 0xdfff) {
          thawGraphApply(thawGraphArrayPush, units, [next]);
          index++;
        } else malformed = true;
      } else if (unit >= 0xdc00 && unit <= 0xdfff) malformed = true;
    }
    return malformed ? units : null;
  };
  const thawGraphGetPrototypeOf = Reflect.getPrototypeOf;
  const thawGraphSetPrototypeOf = Reflect.setPrototypeOf;
  globalThis.__thaw_json_host_get_prototype = value => thawGraphGetPrototypeOf(value);
  globalThis.__thaw_json_host_set_prototype = (value, prototype) =>
    thawGraphSetPrototypeOf(value, prototype);
  globalThis.__thaw_json_host_instanceof = (value, constructor) =>
    value instanceof constructor;
  const thawGraphReflectHas = Reflect.has;
  const thawGraphReflectDeleteProperty = Reflect.deleteProperty;
  thawGraphDefineProperty(globalThis, '__thaw_json_host_key_predicate', {
    value: (object, keyPayload, operation, encoded) => {
      const key = encoded ? thawGraphParse(keyPayload) : keyPayload;
      if (encoded && typeof key !== 'string')
        throw new thawGraphTypeError('Invalid host property key');
      if (operation === 0) return thawGraphApply(thawGraphReflectHas, Reflect, [object, key]);
      if (operation === 1) return thawGraphOwn(object, key);
      if (operation === 2)
        return thawGraphApply(thawGraphReflectDeleteProperty, Reflect, [object, key]);
      throw new thawGraphTypeError('Invalid native property operation');
    },
    writable: false, configurable: false,
  });
  const thawGraphReflectOwnKeys = Reflect.ownKeys;
  const thawGraphObjectIs = Object.is;
  const thawGraphOwn = (object, key) => thawGraphApply(thawGraphOwnProperty, object, [key]);
  const thawGraphGetOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
  // Keep the accepted enumerable predicate's parser and descriptor read
  // captured even when the public JSON or Reflect objects are replaced.
  thawGraphDefineProperty(globalThis, '__thaw_host_property_is_enumerable', {
    value: (object, keyJson) => {
      const descriptor = thawGraphReflectGetOwnPropertyDescriptor(object, thawGraphParse(keyJson));
      return descriptor !== undefined && descriptor.enumerable;
    },
    writable: false, configurable: false,
  });
  const thawGraphDateTime = thawGraphDateConstructor.prototype.getTime;
  const thawGraphArrayBufferLength = thawGraphGetOwnPropertyDescriptor(thawGraphArrayBufferConstructor.prototype, 'byteLength').get;
  const thawGraphIsView = thawGraphArrayBufferConstructor.isView;
  const thawGraphMapSize = thawGraphGetOwnPropertyDescriptor(thawGraphMapConstructor.prototype, 'size').get;
  const thawGraphSetSize = thawGraphGetOwnPropertyDescriptor(thawGraphSetConstructor.prototype, 'size').get;
  const thawGraphRegExpSource = thawGraphGetOwnPropertyDescriptor(thawGraphRegExpConstructor.prototype, 'source').get;
  const thawGraphRegExpFlags = thawGraphGetOwnPropertyDescriptor(thawGraphRegExpConstructor.prototype, 'flags').get;
  const thawGraphRegExpTest = thawGraphRegExpConstructor.prototype.test;
  const thawGraphBigIntDecimalPattern = /^(?:0|-?[1-9][0-9]*)(?![\s\S])/;
  const thawGraphPositiveDecimalPattern = /^[1-9][0-9]*(?![\s\S])/;
  const thawGraphNapiOwner = globalThis.__thaw_napi_graph_owner;
  const thawGraphNapiRelease = globalThis.__thaw_napi_graph_release;
  thawGraphDefineProperty(globalThis, '__thaw_json_graph_owner_of_handle', {
    value: handle => typeof thawGraphNapiOwner === 'function' ? thawGraphNapiOwner(handle) : '',
    writable: false, configurable: false,
  });
  // The native owner decides which proxy cache owns a handle. Factory code
  // lives in this bootstrap closure: user scripts cannot pre-register a
  // factory for a guessed future owner ID.
  const thawNapiProxyConstructor = Proxy;
  const thawNapiFunctionBind = Function.prototype.bind;
  const thawNapiSymbolConstructor = Symbol;
  const thawNapiSymbolFor = Symbol.for;
  const thawNapiSymbolKeyFor = Symbol.keyFor;
  const thawNapiSymbolDescription = thawGraphGetOwnPropertyDescriptor(
    Symbol.prototype, 'description').get;
  const thawNapiReflectGetOwnPropertyDescriptor = Reflect.getOwnPropertyDescriptor;
  const thawNapiReflectDefineProperty = Reflect.defineProperty;
  const thawNapiReflectIsExtensible = Reflect.isExtensible;
  const thawNapiReflectPreventExtensions = Reflect.preventExtensions;
  const thawNapiReflectDeleteProperty = Reflect.deleteProperty;
  const thawNapiReflectGetPrototypeOf = Reflect.getPrototypeOf;
  const thawNapiReflectSetPrototypeOf = Reflect.setPrototypeOf;
  const thawNapiReflectGet = Reflect.get;
  const thawNapiReflectHas = Reflect.has;
  const thawNapiReflectConstruct = Reflect.construct;
  const thawNapiArraySlice = Array.prototype.slice;
  const thawNapiArrayShift = Array.prototype.shift;
  const thawNapiArraySome = Array.prototype.some;
  const thawNapiArrayIndexOf = Array.prototype.indexOf;
  const thawNapiArrayConcat = Array.prototype.concat;
  const thawNapiStringStartsWith = String.prototype.startsWith;
  const thawNapiStringSlice = String.prototype.slice;
  const thawNapiStringIndexOf = String.prototype.indexOf;
  const thawNapiMathMin = Math.min;
  const thawNapiUint8Set = Uint8Array.prototype.set;
  const thawNapiArrayMap = (array, callback) => thawGraphApply(thawGraphArrayMap, array, [callback]);
  const thawNapiArrayEach = (array, callback) => thawGraphApply(thawGraphArrayForEach, array, [callback]);
  const thawNapiArraySomeOf = (array, callback) => thawGraphApply(thawNapiArraySome, array, [callback]);
  const thawNapiArrayAppend = (array, value) => thawGraphApply(thawGraphArrayPush, array, [value]);
  const thawNapiObjectPrototype = Object.prototype;
  const thawNapiSharedArrayBufferConstructor = typeof SharedArrayBuffer === 'function'
    ? SharedArrayBuffer : null;
  const thawNapiDataViewConstructor = DataView;
  const thawNapiTypedArrayConstructors = [Int8Array, Uint8Array, Uint8ClampedArray,
    Int16Array, Uint16Array, Int32Array, Uint32Array, Float32Array,
    Float64Array, globalThis.BigInt64Array, globalThis.BigUint64Array];
  const thawNapiFunctionHasInstance = Function.prototype[Symbol.hasInstance];
  const thawNapiHasInstance = (ctor, value) => ctor &&
    thawGraphApply(thawNapiFunctionHasInstance, ctor, [value]);
  const thawNapiTypedArrayKind = value => {
    for (let kind = 0; kind < thawNapiTypedArrayConstructors.length; kind++) {
      if (thawNapiHasInstance(thawNapiTypedArrayConstructors[kind], value)) return kind;
    }
    return -1;
  };
  const thawNapiWeakRef = typeof WeakRef === 'function' ? WeakRef : null;
  const thawNapiWeakRefDeref = thawNapiWeakRef ? thawNapiWeakRef.prototype.deref : null;
  const thawNapiDeref = stored => {
    if (!stored || !thawNapiWeakRefDeref) return stored;
    try { return thawGraphApply(thawNapiWeakRefDeref, stored, []); }
    catch (_) { return stored; }
  };
  // A failed weak dereference is unknown, never evidence for native scope
  // retirement. This path uses the captured intrinsic and lets the trusted
  // bridge report an error instead of manufacturing a dead Symbol.
  const thawNapiSymbolDeref = stored =>
    typeof stored === 'symbol' || stored === undefined ? stored
      : thawGraphApply(thawNapiWeakRefDeref, stored, []);
  const thawNapiErrorConstructor = Error;
  const thawNapiRangeErrorConstructor = RangeError;
  const thawNapiPromiseConstructor = Promise;
  const thawNapiFinalizationRegistry = typeof FinalizationRegistry === 'function'
    ? FinalizationRegistry : null;
  const thawNapiFinalizationRegister = thawNapiFinalizationRegistry
    ? thawNapiFinalizationRegistry.prototype.register : null;
  const thawNapiMapDelete = thawGraphMapConstructor.prototype.delete;
  const thawNapiWeakSetConstructor = WeakSet;
  const thawNapiWeakSetAdd = WeakSet.prototype.add;
  const thawNapiWeakSetHas = WeakSet.prototype.has;
  const thawNapiWeakSetDelete = WeakSet.prototype.delete;
  const thawNapiSafeMap = entries => {
    const map = new thawGraphMapConstructor(entries);
    return {
      get: key => thawGraphApply(thawGraphMapGet, map, [key]),
      set: (key, value) => thawGraphApply(thawGraphMapSet, map, [key, value]),
      delete: key => thawGraphApply(thawNapiMapDelete, map, [key]),
    };
  };
  const thawNapiSafeWeakMap = () => {
    const map = new thawGraphWeakMapConstructor();
    return {
      get: key => thawGraphApply(thawGraphWeakMapGet, map, [key]),
      set: (key, value) => thawGraphApply(thawGraphWeakMapSet, map, [key, value]),
      has: key => thawGraphApply(thawGraphWeakMapHas, map, [key]),
    };
  };
  const thawNapiSafeSet = entries => {
    const set = new thawGraphSetConstructor(entries);
    return { has: value => thawGraphApply(thawGraphSetHas, set, [value]) };
  };
  // Capture the well-known Symbol set before user scripts can replace fields.
  const thawNapiWellKnownSymbols = thawNapiSafeSet(
    Object.getOwnPropertyNames(thawNapiSymbolConstructor)
      .map(name => thawNapiSymbolConstructor[name])
      .filter(value => typeof value === 'symbol'));
  const thawNapiSafeWeakSet = () => {
    const set = new thawNapiWeakSetConstructor();
    return {
      add: value => thawGraphApply(thawNapiWeakSetAdd, set, [value]),
      has: value => thawGraphApply(thawNapiWeakSetHas, set, [value]),
      delete: value => thawGraphApply(thawNapiWeakSetDelete, set, [value]),
    };
  };
  const thawNapiSafeFinalizer = callback => {
    const finalizer = new thawNapiFinalizationRegistry(callback);
    return { register: (value, held) =>
      thawGraphApply(thawNapiFinalizationRegister, finalizer, [value, held]) };
  };
  const thawGraphNapiBridgeHandle = globalThis.__thaw_napi_bridge_handle;
  const thawGraphNapiInvoker = globalThis.__thaw_napi_graph_invoker;
  const thawGraphNapiBridgeCall = globalThis.__thaw_napi_bridge_call;
  const thawGraphNapiBridgeExports = globalThis.__thaw_napi_bridge_exports;
  const thawGraphNapiBridgeAvailable = globalThis.__thaw_napi_bridge_available;
  thawGraphDefineProperty(globalThis, '__thaw_json_graph_napi_bridge_available', {
    value: () => thawGraphNapiBridgeAvailable(), writable: false, configurable: false,
  });
  thawGraphDefineProperty(globalThis, '__thaw_json_graph_napi_exports', {
    value: () => thawGraphParse(thawGraphNapiBridgeExports()),
    writable: false, configurable: false,
  });
  thawGraphDefineProperty(globalThis, '__thaw_json_graph_napi_each_export', {
    value: (info, prefix, packageName, install) => {
      const qualified = thawGraphApply(thawNapiArrayIndexOf,
        info.qualifiedPackages, [packageName]) !== -1;
      thawNapiArrayEach(info.names, name => {
        if (qualified) {
          if (!thawGraphApply(thawNapiStringStartsWith, name, [prefix])) return;
          install(name, thawGraphApply(thawNapiStringSlice, name, [prefix.length]));
        } else if (thawGraphApply(thawNapiStringIndexOf, name, ['::']) === -1) {
          install(name, name);
        }
      });
    },
    writable: false, configurable: false,
  });
  const thawGraphNapiOwnerOfHandle = handle =>
    typeof thawGraphNapiOwner === 'function' ? thawGraphNapiOwner(handle) : '';
  const thawGraphNapiReferenceAllocator = { next: 0 };
  const thawGraphNapiHandles = thawNapiSafeWeakMap();
  const thawGraphNapiFunctionHandles = thawNapiSafeWeakMap();
  const thawGraphForeignReceiver = value => value !== null
    && (typeof value === 'object' || typeof value === 'function')
    && thawGraphApply(thawGraphWeakMapGet, thawGraphNapiHandles, [value]) === undefined
    && thawGraphApply(thawGraphWeakMapGet, thawGraphNapiFunctionHandles, [value]) === undefined
    ? value : undefined;
  thawGraphDefineProperty(globalThis, '__thaw_json_graph_native_pair_matches', {
    value: (value, nativeHandle, kind) => {
      if (typeof nativeHandle !== 'string'
        || !thawGraphApply(thawGraphRegExpTest,
          thawGraphPositiveDecimalPattern, [nativeHandle])) return false;
      if (kind === 1) // Native Function wrapper identity.
        return thawGraphNapiFunctionHandles.get(value) === nativeHandle;
      if (kind === 2 && typeof value === 'symbol') {
        const id = thawHostNativeSymbolID(value);
        if (id === undefined) return false;
        const reply = thawGraphParse(thawGraphNapiBridgeHandle('symbol_metadata',
          nativeHandle, '', '[]'));
        return !!reply && !reply.__thaw_error__ && !!reply.value
          && reply.value.id === id && reply.value.handle === nativeHandle;
      }
      return false;
    }, writable: false, configurable: false,
  });
  const thawGraphNapiFunctions = new thawGraphMapConstructor();
  const thawGraphNapiFunctionFinalizer = thawNapiFinalizationRegistry
    ? thawNapiSafeFinalizer(held => {
        if (thawGraphApply(thawGraphMapGet, thawGraphNapiFunctions, [held.id]) === held.entry)
          thawGraphApply(thawGraphMapDelete, thawGraphNapiFunctions, [held.id]);
      }) : null;
  const thawGraphNapiProxyOwners = thawNapiSafeWeakMap();
  const thawGraphNapiStates = new thawGraphMapConstructor();
  const thawGraphNapiState = owner => {
    if (typeof owner !== 'string'
      || !thawGraphApply(thawGraphRegExpTest, thawGraphPositiveDecimalPattern, [owner]))
      throw new thawGraphTypeError('Invalid native addon graph owner');
    const previous = thawGraphApply(thawGraphMapGet, thawGraphNapiStates, [owner]);
    if (previous) return previous;
    var __thaw_napi_owner_metadata = true;
    var __thaw_napi_reference_allocator = thawGraphNapiReferenceAllocator;
    var __thaw_napi_proxy_owners = thawGraphNapiProxyOwners;
    var __thaw_napi_reference_ids = thawNapiSafeWeakMap();
    var __thaw_napi_reference_values = thawNapiSafeMap();
    var __thaw_napi_handles = thawGraphNapiHandles;
    var __thaw_napi_owner_ids = thawNapiSafeSet([owner]);
    var __thaw_napi_proxies = thawNapiSafeMap();
    var __thaw_napi_symbol_values = thawNapiSafeMap();
    var __thaw_napi_symbol_ids = thawNapiSafeWeakMap();
    var __thaw_napi_global_symbol_ids = thawNapiSafeMap();
    var __thaw_napi_symbol_pins = thawNapiSafeMap();
    var __thaw_napi_symbol_finalizers = thawNapiFinalizationRegistry
      ? thawNapiSafeFinalizer(function(held) {
          if (__thaw_napi_symbol_values.get(held.id) !== held.entry) return;
          __thaw_napi_symbol_values.delete(held.id);
          __thaw_napi_symbol_pins.delete(held.id);
          // The native owner validates ID and generation before scope sweep.
          try { __thaw_napi_handle('symbol_collected', owner, held.id, []); } catch (_) {}
        }) : null;
    var __thaw_napi_symbol_store = function(id, symbol, permanent) {
      if (!permanent && (!thawNapiWeakRef || !__thaw_napi_symbol_finalizers))
        throw new thawGraphTypeError('Local JavaScript Symbol requires weak identity support');
      if (permanent) {
        __thaw_napi_symbol_values.set(id, symbol);
        __thaw_napi_global_symbol_ids.set(symbol, id);
      } else {
        var entry = new thawNapiWeakRef(symbol);
        __thaw_napi_symbol_values.set(id, entry);
        __thaw_napi_symbol_ids.set(symbol, id);
        __thaw_napi_symbol_finalizers.register(symbol, { id: id, entry: entry });
      }
    };
    var __thaw_napi_binary_values = thawNapiSafeMap();
    var __thaw_napi_binary_finalizers = thawNapiFinalizationRegistry ? thawNapiSafeFinalizer(function(held) { if (!held.active) return; held.active = false; if (__thaw_napi_binary_values.get(held.id) === held.entry) __thaw_napi_binary_values.delete(held.id); __thaw_napi_handle('release_handle', held.id, '', []); }) : null;
    var __thaw_napi_finalizers = thawNapiFinalizationRegistry ? thawNapiSafeFinalizer(function(id) { __thaw_napi_reference_values.delete(id); __thaw_napi_handle('release', id, '', []); }) : null;
    var __thaw_napi_proxy_finalizers = thawNapiFinalizationRegistry ? thawNapiSafeFinalizer(function(held) { if (!held.active) return; held.active = false; if (__thaw_napi_proxies.get(held.id) === held.entry) __thaw_napi_proxies.delete(held.id); __thaw_napi_handle('release_handle', held.id, '', []); }) : null;
    var __thaw_napi_reference_value = function(id, active) { var stored = __thaw_napi_reference_values.get(id), value = thawNapiDeref(stored), properties = {}; if (!value) return properties; thawNapiArrayEach(thawGraphObjectKeys(value), function(key) { properties[key] = __thaw_napi_argument(value[key], active); }); return properties; };
    var __thaw_napi_argument = function(value, active) {
    if (value === null || (typeof value !== 'function' && typeof value !== 'object')) return value;
    var handle = __thaw_napi_handles.get(value); if (handle) { var owner = thawGraphNapiOwnerOfHandle(thawGraphString(handle)); if (__thaw_napi_owner_metadata && !__thaw_napi_owner_ids.has(owner)) throw new thawGraphTypeError('native object belongs to another addon'); return { __thaw_napi_handle__: handle }; } if (__thaw_napi_proxy_owners.has(value)) throw new thawGraphTypeError('native object belongs to another addon');
    active = active || thawNapiSafeWeakSet();
    var id = __thaw_napi_reference_ids.get(value);
    if (id && active.has(value)) return { __thaw_napi_ref__: id };
    var fresh = !id; if (fresh) { if (__thaw_napi_reference_allocator.next >= 9007199254740991) throw new thawNapiRangeErrorConstructor('native reference limit exceeded'); id = ++__thaw_napi_reference_allocator.next; __thaw_napi_reference_ids.set(value, id); }
    if (typeof value === 'function') { if (fresh) { __thaw_napi_reference_values.set(id, thawNapiWeakRef ? new thawNapiWeakRef(value) : value); thawGraphDefineProperty(globalThis, '__thaw_napi_reference_' + id, { value: function() { var args = thawGraphApply(thawNapiArraySlice, arguments, []), meta = thawGraphApply(thawNapiArrayShift, args, []), receiver; if (meta && meta.__thaw_napi_argument_handles__) thawNapiArrayEach(meta.__thaw_napi_argument_handles__, function(entry) { var path = entry[0], parent = args; for (var i = 0; i + 1 < path.length; i++) { var own = thawGraphGetOwnPropertyDescriptor(parent, path[i]); if (!own) throw new thawGraphTypeError('invalid native callback path'); parent = own.value; } thawGraphDefineProperty(parent, path[path.length - 1], { value: __thaw_napi_proxy(entry[1]), writable: true, enumerable: true, configurable: true }); }); if (meta && meta.__thaw_napi_this_handle__) { var stored = __thaw_napi_proxies.get(meta.__thaw_napi_this_handle__); receiver = thawNapiDeref(stored); if (!receiver) receiver = __thaw_napi_proxy(meta.__thaw_napi_this_handle__); } var returned = thawGraphApply(value, receiver, args); return typeof returned === 'function' ? __thaw_napi_argument(returned) : returned; }, writable: false, configurable: false }); } return { __thaw_napi_function__: id }; }
    __thaw_napi_reference_values.set(id, thawNapiWeakRef ? new thawNapiWeakRef(value) : value);
    if (fresh && __thaw_napi_finalizers) __thaw_napi_finalizers.register(value, id);
    active.add(value);
    var view = thawGraphIsView(value), buffer = thawNapiHasInstance(thawGraphArrayBufferConstructor, value) || (thawNapiSharedArrayBufferConstructor !== null && thawNapiHasInstance(thawNapiSharedArrayBufferConstructor, value)), isBuffer = thawGraphBufferIsBuffer && thawGraphBufferIsBuffer(value);
    var dataView = view && thawNapiHasInstance(thawNapiDataViewConstructor, value);
    var typedKind = view && !isBuffer && !dataView ? thawNapiTypedArrayKind(value) : -1;
    if (view && !isBuffer && !dataView && typedKind < 0) throw new thawGraphTypeError('unsupported native typed array kind');
    var encoded = isBuffer ? { __thaw_napi_buffer__: id, value: thawGraphApply(thawNapiArraySlice, value, []) } : buffer ? { __thaw_napi_arraybuffer__: id, shared: thawNapiSharedArrayBufferConstructor !== null && thawNapiHasInstance(thawNapiSharedArrayBufferConstructor, value), value: thawGraphApply(thawNapiArraySlice, new thawGraphUint8ArrayConstructor(value), []) } : view ? { __thaw_napi_view__: id, kind: dataView ? -1 : typedKind, length: dataView ? value.byteLength : value.length, byte_offset: value.byteOffset, buffer: __thaw_napi_argument(value.buffer, active) } : thawGraphArrayIsArray(value) ? { __thaw_napi_array__: id, value: thawNapiArrayMap(value, function(item) { return __thaw_napi_argument(item, active); }) } : { __thaw_napi_object__: id, value: __thaw_napi_reference_value(id, active) };
    active.delete(value); return encoded;
    };
    var __thaw_napi_arguments = function(args) { var active = thawNapiSafeWeakSet(); return thawGraphApply(thawGraphArrayMap, args, [function(value) { return __thaw_napi_argument(value, active); }]); };
    // A plain object argument can carry a getter-only accessor
    // property (real trigger: better-sqlite3's own `Database`
    // instance, passed as `prepare(sql, this, ...)`'s second
    // argument, whose `name`/`open`/`inTransaction`/`readonly`/
    // `memory` are all getter-only via `Object.defineProperties`)
    // -- syncing the native side's echoed-back value onto it
    // with a plain `=` assignment threw a real engine
    // `TypeError: no setter for property` the moment any such
    // key came back, even though the native round trip never
    // actually changed it. A getter-only property is always
    // freshly computed on the next real read anyway, so there is
    // nothing meaningful to "sync" for one -- skip a key the
    // assignment itself rejects instead of crashing the whole
    // call.
    var __thaw_napi_decode = function(value, origins, path) { path = path || []; if (thawGraphArrayIsArray(origins)) origins = thawNapiSafeMap(thawNapiArrayMap(origins, function(entry) { return [thawGraphStringify(entry[0]), entry]; })); var origin = origins && origins.get(thawGraphStringify(path)); if (origin && origin[1] === 'date') return new thawGraphDateConstructor(origin[2] === null ? NaN : origin[2]); if (origin && origin[1] === 'nonfinite') return thawGraphNumber(origin[2]); var plain = origin && origin[1] === 'plain'; if (value && typeof value === 'object') { if (!plain && value['$__thaw_napi_undefined$'] === true) return undefined; if (!plain && thawGraphOwn(value, '__thaw_napi_error__')) { var ctor = value.name === 'TypeError' ? thawGraphTypeError : value.name === 'RangeError' ? thawNapiRangeErrorConstructor : thawNapiErrorConstructor, error = new ctor(value.__thaw_napi_error__); if (value.name && error.name !== value.name) error.name = value.name; return error; } if (!plain && value.__thaw_napi_symbol__) { var stored = __thaw_napi_symbol_values.get(value.__thaw_napi_symbol__), symbol = thawNapiDeref(stored); if (!symbol) { symbol = value.global ? thawNapiSymbolFor(value.description) : thawNapiSymbolConstructor(value.description); __thaw_napi_symbol_store(value.__thaw_napi_symbol__, symbol, value.global); } return symbol; } if (!plain && value.__thaw_napi_ref__) { var stored = __thaw_napi_reference_values.get(value.__thaw_napi_ref__); return thawNapiDeref(stored); } if (!plain && value.__thaw_napi_handle__) return __thaw_napi_proxy(value.__thaw_napi_handle__); if (!plain && value.__thaw_napi_promise__) return __thaw_napi_result_value(value); if (!plain && value.__thaw_napi_binary__) { var id = value.__thaw_napi_binary__, stored = __thaw_napi_binary_values.get(id), existing = thawNapiDeref(stored); if (existing) return existing; var binary; if (value.kind === 'ArrayBuffer' || value.kind === 'SharedArrayBuffer') { binary = value.kind === 'SharedArrayBuffer' ? new thawNapiSharedArrayBufferConstructor(value.data.length) : new thawGraphArrayBufferConstructor(value.data.length); thawGraphApply(thawNapiUint8Set, new thawGraphUint8ArrayConstructor(binary), [value.data]); } else { var backing = __thaw_napi_decode(value.buffer, origins, thawGraphApply(thawNapiArrayConcat, path, ['buffer'])); if (value.kind === 'DataView') binary = new thawNapiDataViewConstructor(backing, value.byte_offset, value.length); else { var constructors = thawNapiTypedArrayConstructors, viewCtor = constructors[value.array_type]; if (!viewCtor) throw new thawGraphTypeError('unsupported native typed array kind'); binary = new viewCtor(backing, value.byte_offset, value.length); } } var entry = thawNapiWeakRef && __thaw_napi_binary_finalizers ? new thawNapiWeakRef(binary) : binary; __thaw_napi_handles.set(binary, id); __thaw_napi_proxy_owners.set(binary, owner); __thaw_napi_handle('renew_handle', id, '', []); try { __thaw_napi_binary_values.set(id, entry); if (__thaw_napi_binary_finalizers) { var held = { id: id, entry: entry, active: true }; try { __thaw_napi_binary_finalizers.register(binary, held); } catch (error) { held.active = false; throw error; } } } catch (error) { if (__thaw_napi_binary_values.get(id) === entry) __thaw_napi_binary_values.delete(id); try { __thaw_napi_handle('release_handle', id, '', []); } catch (_) {} throw error; } return binary; } if (!plain && value.type === 'Buffer' && thawGraphArrayIsArray(value.data) && thawGraphBufferFrom) return thawGraphBufferFrom(value.data); if (thawGraphArrayIsArray(value)) return thawNapiArrayMap(value, function(child, index) { return __thaw_napi_decode(child, origins, thawGraphApply(thawNapiArrayConcat, path, [index])); }); var decoded = {}; thawNapiArrayEach(thawGraphObjectKeys(value), function(key) { thawGraphDefineProperty(decoded, key, { value: __thaw_napi_decode(value[key], origins, thawGraphApply(thawNapiArrayConcat, path, [key])), enumerable: true, configurable: true, writable: true }); }); return decoded; } return value; };
    var __thaw_napi_sync_arguments = function(args) { var seen = thawNapiSafeWeakSet(); var sync = function(value) { if (value === null || typeof value !== 'object' || seen.has(value)) return; seen.add(value); if (true && thawGraphIsView(value) && !(thawGraphBufferIsBuffer && thawGraphBufferIsBuffer(value))) { sync(value.buffer); return; } var handle = __thaw_napi_handles.get(value), id = __thaw_napi_reference_ids.get(value); if (handle && __thaw_napi_owner_metadata && !__thaw_napi_owner_ids.has(thawGraphNapiOwnerOfHandle(thawGraphString(handle)))) throw new thawGraphTypeError('native object belongs to another addon'); if (!handle && !id) return; var response = __thaw_napi_handle(handle ? 'sync_handle' : 'sync_reference', handle || id, '', []), updated = response.value, originMap = thawNapiSafeMap(thawNapiArrayMap((response.origins || []), function(entry) { return [thawGraphStringify(entry[0]), entry]; })); if (true && (thawNapiHasInstance(thawGraphArrayBufferConstructor, value) || (thawNapiSharedArrayBufferConstructor !== null && thawNapiHasInstance(thawNapiSharedArrayBufferConstructor, value)))) { thawGraphApply(thawNapiUint8Set, new thawGraphUint8ArrayConstructor(value), [thawGraphApply(thawNapiArraySlice, updated, [0, value.byteLength])]); return; } if (thawGraphBufferIsBuffer && thawGraphBufferIsBuffer(value)) { for (var i = 0; i < thawNapiMathMin(value.length, updated.length); i++) value[i] = updated[i]; return; } if (handle) return; if (thawGraphArrayIsArray(value)) value.length = updated.length; else thawNapiArrayEach(thawGraphObjectKeys(value), function(key) { if (!thawGraphOwn(updated, key)) { try { delete value[key]; } catch (e) {} } }); thawNapiArrayEach(thawGraphObjectKeys(updated), function(key) { var next = handle ? updated[key] : __thaw_napi_decode(updated[key], originMap, [thawGraphArrayIsArray(updated) ? thawGraphNumber(key) : key]); if (key === '__proto__') thawGraphDefineProperty(value, key, { value: next, writable: true, enumerable: true, configurable: true }); else if (value[key] !== next) { try { value[key] = next; } catch (e) {} } if (next && typeof next === 'object') sync(next); }); }; thawGraphApply(thawGraphArrayForEach, args, [sync]); };
    var __thaw_napi_promises = thawNapiSafeMap();
    var __thaw_napi_promise_finalizers = thawNapiFinalizationRegistry ? thawNapiSafeFinalizer(function(held) { if (__thaw_napi_promises.get(held.id) === held.entry) __thaw_napi_promises.delete(held.id); }) : null;
    var __thaw_napi_result_value = function(value, origins) {
    if (value && typeof value === 'object' && !thawNapiArraySomeOf((origins || []), function(entry) { return entry[0].length === 0 && entry[1] === 'plain'; }) && thawGraphOwn(value, '__thaw_napi_promise__')) { var id = value.__thaw_napi_promise__, stored = __thaw_napi_promises.get(id), existing = thawNapiDeref(stored); if (existing) return existing; var promise = new thawNapiPromiseConstructor(function(resolve, reject) { var check = function() { var state; try { state = __thaw_napi_handle('promise_state', id, '', []); } catch (error) { reject(error); return; } if (state.kind === 'pending') { setTimeout(check, 0); return; } if (state.kind === 'rejected') { reject(state.value); return; } resolve(state.value); }; setTimeout(check, 0); }); var entry = thawNapiWeakRef ? new thawNapiWeakRef(promise) : promise; __thaw_napi_promises.set(id, entry); if (__thaw_napi_promise_finalizers) __thaw_napi_promise_finalizers.register(promise, { id: id, entry: entry }); return promise; }
    return __thaw_napi_decode(value, origins);
    };
    var __thaw_napi_handle = function(operation, target, name, args) {
    var result = thawGraphParse(thawGraphNapiBridgeHandle(operation, thawGraphString(target), name || '', thawGraphStringify(__thaw_napi_arguments(args || []))));
    if (result && thawGraphOwn(result, '__thaw_error__')) throw new thawNapiErrorConstructor(result.__thaw_error__);
    if (result && result.value && !thawNapiArraySomeOf((result.origins || []), function(entry) { return entry[0].length === 0 && entry[1] === 'plain'; }) && thawGraphOwn(result.value, '$__thaw_napi_undefined$') && result.value['$__thaw_napi_undefined$'] === true) result.value = undefined;
    else if (result && thawGraphOwn(result, 'value') && result.kind !== 'method' && operation !== 'sync_reference') result.value = __thaw_napi_result_value(result.value, result.origins);
    return result;
    };
    var __thaw_napi_symbol_id = function(handle, symbol, graph) {
      var id = __thaw_napi_symbol_ids.get(symbol)
          || __thaw_napi_global_symbol_ids.get(symbol);
      var global = thawNapiSymbolKeyFor(symbol);
      var permanent = global !== undefined || thawNapiWellKnownSymbols.has(symbol);
      var created = __thaw_napi_handle(graph ? 'symbol_graph' : 'symbol',
        graph ? owner : handle,
        global === undefined ? thawGraphApply(thawNapiSymbolDescription, symbol, []) || '' : global,
        [global !== undefined, id || null, permanent]);
      id = created.value;
      try {
        __thaw_napi_symbol_store(id, symbol, permanent);
        if (!permanent) __thaw_napi_handle('symbol_pin', owner, id, []);
        return id;
      } catch (error) {
        if (!permanent) {
          try { __thaw_napi_handle('symbol_rollback', owner, id, []); }
          catch (_) { /* Preserve the original JS cache/pin failure. */ }
        }
        throw error;
      }
    };
    var __thaw_napi_key = function(handle, name, operation, args) { return typeof name === 'symbol' ? __thaw_napi_handle(operation + '_symbol', handle, thawGraphString(__thaw_napi_symbol_id(handle, name)), args) : __thaw_napi_handle(operation, handle, thawGraphString(name), args); };
    var __thaw_napi_proxy = function(handle, prototype) { var stored = __thaw_napi_proxies.get(thawGraphString(handle)), existing = thawNapiDeref(stored); if (existing) return existing; var arrayKind = thawGraphParse(thawGraphNapiBridgeHandle('array_kind_graph', thawGraphString(handle), '', '[]')); if (!arrayKind || thawGraphOwn(arrayKind, '__thaw_error__') || typeof arrayKind.value !== 'boolean') throw new thawGraphTypeError('Invalid native array kind'); var target = arrayKind.value ? [] : thawGraphObjectCreate(prototype || thawNapiObjectPrototype);
    var graphProperty = function(operation, values) { var receiver = operation === 'get_graph' ? values[1] : operation === 'set_graph' ? values[2] : undefined, packet = thawGraphEncode(values, 0, true, false, thawGraphForeignReceiver(receiver)), result = thawGraphBridgeReply(thawGraphParse(thawGraphNapiBridgeHandle(operation, thawGraphString(handle), '', packet))); if (!result || thawGraphOwn(result, '__thaw_error__')) throw new thawGraphTypeError(result && result.__thaw_error__ ? result.__thaw_error__ : 'Native property operation failed'); return result.kind === 'graph' && typeof result.value === 'string' ? thawGraphDecodeOwned(result.value) : result.value; };
    var read = function(name, receiver) { return graphProperty('get_graph', [name, receiver]); };
    var syncArrayLength = function() {
      if (!arrayKind.value) return;
      var length = read('length');
      if (!thawGraphNumberIsSafeInteger(length) || length < 0 || length > 4294967295
        || !thawNapiReflectDefineProperty(target, 'length', { value: length }))
        throw new thawGraphTypeError('Cannot synchronize native array length');
      var nativeDescriptor = graphProperty('descriptor_graph', ['length']);
      var targetDescriptor = thawNapiReflectGetOwnPropertyDescriptor(target, 'length');
      if (!nativeDescriptor || nativeDescriptor.accessor
        || nativeDescriptor.configurable || nativeDescriptor.enumerable
        || typeof nativeDescriptor.writable !== 'boolean'
        || !targetDescriptor || (nativeDescriptor.writable && !targetDescriptor.writable)
        || (!nativeDescriptor.writable && targetDescriptor.writable
          && !thawNapiReflectDefineProperty(target, 'length', { writable: false })))
        throw new thawGraphTypeError('Cannot synchronize native array length attributes');
    };
    var fallbackGetters = thawNapiSafeMap(), fallbackSetters = thawNapiSafeMap();
    var descriptor = function(name) {
      if (arrayKind.value && name === 'length') {
        syncArrayLength();
        return thawNapiReflectGetOwnPropertyDescriptor(target, 'length');
      }
      var meta = graphProperty('descriptor_graph', [name]);
      if (!meta) { var stale = thawNapiReflectGetOwnPropertyDescriptor(target, name); if (stale && stale.configurable) thawNapiReflectDeleteProperty(target, name); fallbackGetters.delete(name); fallbackSetters.delete(name); return undefined; }
      var getter = meta.getter_source ? thawGraphDescriptorSource(meta.getter_source) : undefined;
      var setter = meta.setter_source ? thawGraphDescriptorSource(meta.setter_source) : undefined;
      if (meta.accessor && meta.getter && !getter) {
        getter = fallbackGetters.get(name);
        if (!getter) { getter = function() { return read(name, this); }; fallbackGetters.set(name, getter); }
      }
      if (meta.accessor && meta.setter && !setter) {
        setter = fallbackSetters.get(name);
        if (!setter) { setter = function(value) { graphProperty('set_graph', [name, value, this]); }; fallbackSetters.set(name, setter); }
      }
      var desc = meta.accessor ? { configurable: !!meta.configurable, enumerable: !!meta.enumerable, get: getter, set: setter } : { configurable: !!meta.configurable, enumerable: !!meta.enumerable, writable: !!meta.writable, value: read(name) };
      if (!meta.configurable) { if (!thawNapiReflectDefineProperty(target, name, desc)) throw new thawGraphTypeError('Cannot materialize native descriptor'); return thawNapiReflectGetOwnPropertyDescriptor(target, name); }
      return desc;
    };
    var proxy = new thawNapiProxyConstructor(target, {
    get: function(_, name, receiver) { if (arrayKind.value && name === 'length') syncArrayLength(); if (!graphProperty('has_graph', [name])) { descriptor(name); return thawNapiReflectGet(_, name, receiver); } var own = thawNapiReflectGetOwnPropertyDescriptor(_, name); if (own && !own.configurable && !own.writable && thawGraphOwn(own, 'value')) return own.value; if (own && !own.configurable && thawGraphOwn(own, 'get') && own.get === undefined) return undefined; return read(name, receiver); },
    set: function(_, name, value, receiver) { var own = thawNapiReflectGetOwnPropertyDescriptor(_, name); if (own && !own.configurable && ((thawGraphOwn(own, 'value') && !own.writable) || (thawGraphOwn(own, 'set') && own.set === undefined))) return false; if (graphProperty('set_graph', [name, value, receiver]) !== true) return false; if (arrayKind.value) syncArrayLength(); if (own && !own.configurable && thawGraphOwn(own, 'value') && own.writable) thawNapiReflectDefineProperty(_, name, { value: read(name) }); return true; },
    has: function(_, name) { var native = graphProperty('has_graph', [name]); if (!native) descriptor(name); return native || thawNapiReflectHas(_, name); },
    ownKeys: function(_) { if (arrayKind.value) syncArrayLength(); var keys = graphProperty('own_keys_graph', []); if (!thawGraphArrayIsArray(keys)) throw new thawGraphTypeError('Invalid native ownKeys graph'); thawNapiArrayEach(thawGraphReflectOwnKeys(_), function(key) { if (thawGraphApply(thawNapiArrayIndexOf, keys, [key]) < 0) { var stale = thawNapiReflectGetOwnPropertyDescriptor(_, key); if (stale && stale.configurable) { thawNapiReflectDeleteProperty(_, key); fallbackGetters.delete(key); fallbackSetters.delete(key); } else thawNapiArrayAppend(keys, key); } }); thawNapiArrayEach(keys, function(key) { descriptor(key); }); return keys; },
    getOwnPropertyDescriptor: function(_, name) { return descriptor(name) || thawNapiReflectGetOwnPropertyDescriptor(_, name); },
    defineProperty: function(target, key, description) {
      if (!thawGraphDefineNativeProperty(handle, target, key, description)) return false;
      if (arrayKind.value) syncArrayLength();
      return true;
    },
    deleteProperty: function(target, key) { var own = thawNapiReflectGetOwnPropertyDescriptor(target, key); if (own && !own.configurable) return false; if (graphProperty('delete_graph', [key]) !== true) return false; return thawNapiReflectDeleteProperty(target, key); },
    getPrototypeOf: function(target) { var native = graphProperty('get_prototype_graph', []); return native === undefined ? thawNapiReflectGetPrototypeOf(target) : native; },
    setPrototypeOf: function(target, next) { if (!thawNapiReflectIsExtensible(target)) return thawNapiReflectGetPrototypeOf(target) === next; var previous = thawNapiReflectGetPrototypeOf(target); if (!thawNapiReflectSetPrototypeOf(target, next)) return false; try { if (graphProperty('set_prototype_graph', [next]) === true) return true; } catch (error) { thawNapiReflectSetPrototypeOf(target, previous); throw error; } thawNapiReflectSetPrototypeOf(target, previous); return false; },
    isExtensible: function(target) { if (graphProperty('is_extensible_graph', []) === false && thawNapiReflectIsExtensible(target) && !thawGraphPreventNativeExtensions(graphProperty, proxy, target)) throw new thawGraphTypeError('Cannot synchronize native integrity'); return thawNapiReflectIsExtensible(target); },
    preventExtensions: function(target) { return thawGraphPreventNativeExtensions(graphProperty, proxy, target); }
    }); __thaw_napi_handles.set(proxy, handle); __thaw_napi_proxy_owners.set(proxy, owner); var entry = thawNapiWeakRef && __thaw_napi_proxy_finalizers ? new thawNapiWeakRef(proxy) : proxy, id = thawGraphString(handle); __thaw_napi_handle('renew_handle', handle, '', []); try { __thaw_napi_proxies.set(id, entry); if (__thaw_napi_proxy_finalizers) { var held = { id: id, entry: entry, active: true }; try { __thaw_napi_proxy_finalizers.register(proxy, held); } catch (error) { held.active = false; throw error; } } } catch (error) { if (__thaw_napi_proxies.get(id) === entry) __thaw_napi_proxies.delete(id); try { __thaw_napi_handle('release_handle', handle, '', []); } catch (_) {} throw error; } return proxy; };
    const state = {
      proxy: __thaw_napi_proxy,
      registerSymbol(value) {
        if (typeof value !== 'symbol') throw new thawGraphTypeError('Expected a JavaScript Symbol');
        return __thaw_napi_symbol_id(owner, value, true);
      },
      symbolForNative(id) {
        const value = thawNapiSymbolDeref(__thaw_napi_symbol_values.get(id));
        return typeof value === 'symbol' ? value : undefined;
      },
      adoptSymbol(id, value) {
        __thaw_napi_symbol_store(id, value,
          thawNapiSymbolKeyFor(value) !== undefined || thawNapiWellKnownSymbols.has(value));
      },
      symbolLive(id) {
        return typeof thawNapiSymbolDeref(__thaw_napi_symbol_values.get(id)) === 'symbol';
      },
      symbolPin(id, pin) {
        if (!pin) { __thaw_napi_symbol_pins.delete(id); return true; }
        const value = thawNapiSymbolDeref(__thaw_napi_symbol_values.get(id));
        if (typeof value !== 'symbol') return false;
        __thaw_napi_symbol_pins.set(id, value);
        return true;
      },
      callExport(name, args) {
        const result = thawGraphParse(thawGraphNapiBridgeCall(name,
          thawGraphStringify(__thaw_napi_arguments(args))));
        if (result && thawGraphOwn(result, '__thaw_error__'))
          throw new thawGraphTypeError(result.__thaw_error__);
        __thaw_napi_sync_arguments(args);
        return __thaw_napi_result_value(result.value, result.origins);
      },
      construct(name, args, prototype) {
        const created = __thaw_napi_handle('construct', name, '', args);
        thawNapiArrayEach(thawGraphObjectKeys(prototype), key => {
          __thaw_napi_handle('set', created.value, key, [prototype[key]]);
        });
        return __thaw_napi_proxy(created.value, prototype);
      },
    };
    thawGraphApply(thawGraphMapSet, thawGraphNapiStates, [owner, state]);
    return state;
  };
  const thawGraphNapiSymbolState = owner =>
    thawGraphApply(thawGraphMapGet, thawGraphNapiStates, [owner]);
  const thawGraphNapiSymbolForNative = id => {
    const owner = thawGraphNapiOwnerOfHandle(id);
    const state = owner ? thawGraphNapiState(owner) : null;
    const existing = state && state.symbolForNative(id);
    if (existing !== undefined) return existing;
    const value = globalThis.__thaw_json_host_native_symbol(id);
    if (state) state.adoptSymbol(id, value);
    return value;
  };
  const thawNapiSymbolRegister = (owner, value) => {
      const state = thawGraphNapiSymbolState(owner);
      if (!state) throw new thawGraphTypeError('Unknown native Symbol owner');
      return state.registerSymbol(value);
    };
  const thawNapiSymbolLive = (owner, id) => {
      const state = thawGraphNapiSymbolState(owner);
      if (!state) throw new thawGraphTypeError('Unknown native Symbol owner');
      return state.symbolLive(id);
    };
  const thawNapiSymbolPin = (owner, id, pin) => {
      const state = thawGraphNapiSymbolState(owner);
      return !!state && state.symbolPin(id, pin);
    };
  // Register the owner operations once with Rust userdata, then remove every
  // bootstrap capability from the realm before user code can run. The native
  // positive root remains owned by the captured finalizer callback only.
  globalThis.__thaw_napi_graph_symbol_install(
    thawNapiSymbolRegister, thawNapiSymbolLive, thawNapiSymbolPin);
  for (const name of ['__thaw_napi_graph_symbol_install',
    '__thaw_napi_graph_symbol_pin', '__thaw_napi_graph_symbol_unpin']) {
    if (!thawNapiBootstrapDeleteProperty(globalThis, name))
      throw new thawGraphTypeError('Cannot hide native Symbol bridge');
  }
  const thawGraphNapiProxyForHandle = (handle, prototype) => {
    if (typeof handle !== 'string'
      || !thawGraphApply(thawGraphRegExpTest, thawGraphPositiveDecimalPattern, [handle]))
      throw new thawGraphTypeError('Invalid native addon graph handle');
    const owner = thawGraphNapiOwnerOfHandle(handle);
    return thawGraphNapiState(owner).proxy(handle, prototype);
  };
  thawGraphDefineProperty(globalThis, '__thaw_json_graph_proxy_for_handle', {
    value: thawGraphNapiProxyForHandle, writable: false, configurable: false,
  });
  const thawGraphNapiOwnerForExport = name => {
    const info = thawGraphParse(thawGraphNapiBridgeExports());
    const owner = info && info.ownerByExportName && info.ownerByExportName[name];
    if (typeof owner !== 'string')
      throw new thawGraphTypeError('Native addon export has no live owner');
    return owner;
  };
  thawGraphDefineProperty(globalThis, '__thaw_json_graph_napi_call_export', {
    value: (name, args) => thawGraphNapiState(thawGraphNapiOwnerForExport(name)).callExport(name, args),
    writable: false, configurable: false,
  });
  thawGraphDefineProperty(globalThis, '__thaw_json_graph_napi_construct', {
    value: (name, args, prototype) =>
      thawGraphNapiState(thawGraphNapiOwnerForExport(name)).construct(name, args, prototype),
    writable: false, configurable: false,
  });
  const thawGraphMapEntries = thawGraphMapConstructor.prototype.entries;
  const thawGraphSetValues = thawGraphSetConstructor.prototype.values;
  const thawGraphKind = (getter, value) => {
    if (value === null || typeof value !== 'object') return false;
    try { thawGraphApply(getter, value, []); return true; }
    catch (_) { return false; }
  };
  const thawGraphDate = value => {
    if (value === null || typeof value !== 'object') return null;
    try { return { time: thawGraphApply(thawGraphDateTime, value, []) }; }
    catch (_) { return null; }
  };
  const thawGraphArrayBuffer = value => {
    if (value === null || typeof value !== 'object') return false;
    try { thawGraphApply(thawGraphArrayBufferLength, value, []); return true; }
    catch (_) { return false; }
  };
  const thawGraphNapiRetainReference = handle => {
    const result = thawGraphParse(thawGraphNapiBridgeHandle('retain_graph_handle',
      thawGraphString(handle), '', '[]'));
    if (!result || result.__thaw_error__ || typeof result.value !== 'string'
      || !thawGraphApply(thawGraphRegExpTest, thawGraphPositiveDecimalPattern, [result.value]))
      throw new thawGraphTypeError('Cannot retain native graph handle');
    return result.value;
  };
  const thawGraphEncode = (root, handleMask = 0, live = false, plainResult = false,
    forceHandle = undefined) => {
    const handleCarriers = new thawGraphWeakMapConstructor();
    const symbolHandles = new thawGraphMapConstructor();
    const retained = [];
    const retainedNapi = [];
    const retain = value => {
      const handle = thawGraphRetain(value);
      thawGraphApply(thawGraphArrayPush, retained, [handle]);
      return handle;
    };
    const retainSymbol = value => {
      let handle = thawGraphApply(thawGraphMapGet, symbolHandles, [value]);
      if (handle === undefined) {
        handle = retain(value);
        thawGraphApply(thawGraphMapSet, symbolHandles, [value, handle]);
      }
      return handle;
    };
    const retainNative = handle => {
      const token = thawGraphNapiRetainReference(handle);
      thawGraphApply(thawGraphArrayPush, retainedNapi, [token]);
      return thawGraphString(handle);
    };
    try {
    if (thawGraphArrayIsArray(root) && handleMask && !live) {
      thawGraphApply(thawGraphArrayForEach, root, [(item, index) => {
        if ((handleMask & (1 << index)) !== 0) {
          const carrier = thawGraphObjectCreate(null);
          thawGraphApply(thawGraphWeakMapSet, handleCarriers, [carrier, retain(item)]);
          root[index] = carrier;
        }
      }]);
    }
    const ids = new thawGraphWeakMapConstructor();
    const sources = [];
    const nodes = [];
    const livePrimitives = new thawGraphSetConstructor();
    const symbolNodes = new thawGraphMapConstructor();
    const token = (value, key = '') => {
      if (forceHandle !== undefined && value === forceHandle)
        return { hdl: retain(value) };
      // JSON.stringify invokes an object's toJSON before its replacer. The
      // existing binary replacer then restores real Date/Buffer identity, so
      // keep those native kinds but honor user toJSON on ordinary objects.
      if (plainResult && value !== null
        && (typeof value === 'object' || typeof value === 'function')
        && !thawGraphDate(value) && !thawGraphArrayBuffer(value)
        && !thawGraphIsView(value) && typeof value.toJSON === 'function')
        value = value.toJSON(key);
      // Compiled result calls return JSON data, not live function handles.
      // Preserve JSON's undefined-like treatment without creating a lease
      // that the ordinary result path has no registered Host owner for.
      if (plainResult && (typeof value === 'function' || typeof value === 'symbol'))
        return { u: 1 };
      if (plainResult && typeof value === 'bigint')
        throw new thawGraphTypeError('Do not know how to serialize a BigInt');
      // Ordinary primitives, including BigInt, have exact scalar tokens.
      // Symbol needs a live handle; objects/functions stay live below so getters
      // and Proxies are not traversed before the replacer reads them.
      if (typeof value === 'bigint') return { bi: thawGraphString(value) };
      if (typeof value === 'symbol') {
        const nativeID = thawHostNativeSymbolID(value);
        if (nativeID !== undefined) {
          const resolved = thawGraphParse(thawGraphNapiBridgeHandle('symbol_handle',
            nativeID, '', '[]'));
          if (!resolved || resolved.__thaw_error__ || !resolved.value
            || typeof resolved.value !== 'string'
            || !thawGraphApply(thawGraphRegExpTest, thawGraphPositiveDecimalPattern,
              [resolved.value]))
            throw new thawGraphTypeError('Native Symbol owner is unavailable');
          return { nsy: retainNative(resolved.value), hdl: retainSymbol(value) };
        }
      }
      if (live && typeof value === 'symbol') {
        let id = thawGraphApply(thawGraphMapGet, symbolNodes, [value]);
        if (id === undefined) {
          id = sources.length;
          thawGraphApply(thawGraphMapSet, symbolNodes, [value, id]);
          thawGraphApply(thawGraphArrayPush, sources, [value]);
          thawGraphApply(thawGraphSetAdd, livePrimitives, [id]);
        }
        return { r: id };
      }
      if (value === undefined) return { u: 1 };
      if (typeof value === 'number' && thawGraphObjectIs(value, -0)) return { nf: '-0' };
      if (typeof value === 'number' && !thawGraphNumberIsFinite(value))
        return { nf: thawGraphNumberIsNaN(value) ? 'NaN' : value > 0 ? 'Infinity' : '-Infinity' };
      if (typeof value === 'string') {
        const units = thawMalformedStringUnits(value);
        if (units !== null) return { su: units };
      }
      if (value === null || typeof value === 'string' || typeof value === 'number'
        || typeof value === 'boolean') return { v: value };
      if ((typeof value !== 'object' && typeof value !== 'function'))
        throw new thawGraphTypeError('Unsupported native callback graph value');
      let id = thawGraphApply(thawGraphWeakMapGet, ids, [value]);
      if (id === undefined) {
        id = sources.length;
        thawGraphApply(thawGraphWeakMapSet, ids, [value, id]);
        thawGraphApply(thawGraphArrayPush, sources, [value]);
      }
      return { r: id };
    };
    const rootToken = token(root, '');
    for (let index = 0; index < sources.length; index++) {
      const value = sources[index];
      if (thawGraphApply(thawGraphSetHas, livePrimitives, [index])) {
        thawGraphApply(thawGraphArrayPush, nodes, [{ hdl: retainSymbol(value) }]);
      } else if (thawGraphApply(thawGraphWeakMapHas, handleCarriers, [value])) {
        thawGraphApply(thawGraphArrayPush, nodes, [{ hdl: thawGraphApply(thawGraphWeakMapGet, handleCarriers, [value]) }]);
      } else if (thawGraphApply(thawGraphWeakMapHas, thawGraphNapiFunctionHandles, [value])) {
        thawGraphApply(thawGraphArrayPush, nodes,
          [{ nfn: retainNative(thawGraphApply(thawGraphWeakMapGet,
            thawGraphNapiFunctionHandles, [value])), hdl: retain(value) }]);
      } else if (thawGraphApply(thawGraphWeakMapHas, thawGraphNapiHandles, [value])) {
        thawGraphApply(thawGraphArrayPush, nodes,
          [{ nh: retainNative(thawGraphApply(thawGraphWeakMapGet, thawGraphNapiHandles, [value])) }]);
      } else if (live && index !== 0) {
        // Only the wrapper-created argument array is copied. Replacer
        // holders and values stay live: a Proxy/getter must not be visited
        // until compiled code actually reads it.
        thawGraphApply(thawGraphArrayPush, nodes, [{ hdl: retain(value) }]);
      } else if (typeof value === 'function') {
        thawGraphApply(thawGraphArrayPush, nodes, [{ hdl: retain(value) }]);
      } else if (thawGraphArrayBuffer(value) || thawGraphIsView(value)) {
        const bytes = thawGraphArrayBuffer(value)
          ? new thawGraphUint8ArrayConstructor(value)
          : new thawGraphUint8ArrayConstructor(value.buffer, value.byteOffset, value.byteLength);
        thawGraphApply(thawGraphArrayPush, nodes, [{ b: thawGraphArrayFrom(bytes) }]);
      } else if (thawGraphDate(value)) {
        const time = thawGraphDate(value).time;
        thawGraphApply(thawGraphArrayPush, nodes, [{ d: thawGraphNumberIsFinite(time) ? time : null }]);
      } else if (thawGraphKind(thawGraphMapSize, value)) {
        thawGraphApply(thawGraphArrayPush, nodes, [{ m: token(thawGraphArrayFrom(thawGraphApply(thawGraphMapEntries, value, []))) }]);
      } else if (thawGraphKind(thawGraphSetSize, value)) {
        thawGraphApply(thawGraphArrayPush, nodes, [{ s: token(thawGraphArrayFrom(thawGraphApply(thawGraphSetValues, value, []))) }]);
      } else if (thawGraphKind(thawGraphRegExpSource, value)) {
        thawGraphApply(thawGraphArrayPush, nodes, [{ re: [thawGraphApply(thawGraphRegExpSource, value, []),
          thawGraphApply(thawGraphRegExpFlags, value, []), value.lastIndex] }]);
      } else if (thawGraphArrayIsArray(value)) {
        const keys = plainResult ? [] : thawGraphApply(thawGraphReflectOwnKeys, Reflect, [value]);
        const descriptors = thawGraphApply(thawGraphArrayMap, keys,
          [key => thawGraphApply(thawGraphGetOwnPropertyDescriptor, Object, [value, key])]);
        if (!plainResult && thawGraphApply(thawGraphArraySome, descriptors,
          [descriptor => descriptor && !thawGraphOwn(descriptor, 'value')])) {
          thawGraphApply(thawGraphArrayPush, nodes, [{ hdl: retain(value) }]);
        } else {
          const entries = [];
          for (let slot = 0; slot < value.length; slot++)
            thawGraphApply(thawGraphArrayPush, entries, [thawGraphOwn(value, slot)
              ? token(value[slot], thawGraphString(slot)) : { h: 1 }]);
          const properties = thawGraphApply(thawGraphArrayMap, keys, [(key, slot) => {
            const descriptor = descriptors[slot];
            if (!descriptor) throw new thawGraphTypeError('Graph array property disappeared');
            const keyToken = token(key);
            const wireKey = typeof key === 'symbol' || thawGraphOwn(keyToken, 'su')
              ? keyToken : key;
            const flags = (descriptor.writable ? 1 : 0) | (descriptor.enumerable ? 2 : 0)
              | (descriptor.configurable ? 4 : 0);
            return [wireKey, token(descriptor.value, key), flags];
          }]);
          thawGraphApply(thawGraphArrayPush, nodes, [plainResult
            ? { a: entries } : { a: entries, p: properties }]);
        }
      } else {
        const keys = plainResult ? thawGraphObjectKeys(value)
          : thawGraphApply(thawGraphReflectOwnKeys, Reflect, [value]);
        const descriptors = thawGraphApply(thawGraphArrayMap, keys,
          [key => thawGraphApply(thawGraphGetOwnPropertyDescriptor, Object, [value, key])]);
        if (!plainResult && thawGraphApply(thawGraphArraySome, descriptors,
          [descriptor => descriptor && !thawGraphOwn(descriptor, 'value')])) {
          // Keep an accessor-bearing object live. A data snapshot would call
          // its getter early and replace a getter/setter with a data field.
          thawGraphApply(thawGraphArrayPush, nodes, [{ hdl: retain(value) }]);
        } else {
          thawGraphApply(thawGraphArrayPush, nodes, [{ o: thawGraphApply(thawGraphArrayMap, keys,
            [(key, slot) => {
              const descriptor = descriptors[slot];
              if (!descriptor) throw new thawGraphTypeError('Graph property disappeared');
              const keyToken = token(key);
              const wireKey = typeof key === 'symbol' || thawGraphOwn(keyToken, 'su')
                ? keyToken : key;
              const flags = plainResult ? 7 : ((descriptor.writable ? 1 : 0)
                | (descriptor.enumerable ? 2 : 0)
                | (descriptor.configurable ? 4 : 0));
              return [wireKey, token(plainResult ? value[key] : descriptor.value, key), flags];
            }]) }]);
        }
      }
    }
    return thawGraphStringify({ root: rootToken, nodes, leases: retained,
      napiLeases: retainedNapi });
    } catch (error) {
      for (let index = 0; index < retained.length; index++)
        thawGraphRelease(retained[index]);
      for (let index = 0; index < retainedNapi.length; index++)
        thawGraphNapiRelease(retainedNapi[index]);
      throw error;
    }
  };
  globalThis.__thaw_json_graph_encode_js = thawGraphEncode;
  // The result ABI trusts node kinds from this encoder. Loaded user scripts
  // may call it, but cannot replace it with a forged metadata producer.
  thawGraphDefineProperty(globalThis, '__thaw_json_graph_encode_js', {
    value: globalThis.__thaw_json_graph_encode_js,
    writable: false,
    configurable: false,
  });
  const thawGraphDescriptorSource = source => thawGraphDecode({
    root: { r: 0 }, nodes: [source],
  });
  // ToPropertyDescriptor reads each field in the specified order. Keep the
  // descriptor functions themselves live and transfer their graph handles to
  // the native descriptor owner; do not call accessors while defining them.
  const thawGraphDefineNativeProperty = (handle, target, key, input) => {
    const own = thawNapiReflectGetOwnPropertyDescriptor(target, key);
    if (!own && !thawNapiReflectIsExtensible(target)) return false;
    if (own && !own.configurable) {
      // Let the engine apply the standard compatibility rules before any
      // native mutation. A nonconfigurable writable data property may still
      // change value or become readonly, and identical definitions are valid.
      const probe = thawGraphObjectCreate(null);
      if (!thawNapiReflectDefineProperty(probe, key, own)
        || !thawNapiReflectDefineProperty(probe, key, input)) return false;
      // Accessor shadows delegate to the native callback. The getter or
      // setter returned from this Proxy is that stable shadow function, not
      // the original native callback handle. Reapplying an identical
      // nonconfigurable descriptor is a no-op and must not register the
      // shadow as a different native callback.
      if (thawGraphOwn(own, 'get')) {
        const updated = thawNapiReflectGetOwnPropertyDescriptor(probe, key);
        if (updated.get === own.get && updated.set === own.set
          && updated.enumerable === own.enumerable
          && updated.configurable === own.configurable) return true;
      }
    }
    const has = field => thawNapiReflectHas(input, field);
    const get = field => thawNapiReflectGet(input, field);
    const metadataPacket = thawGraphEncode([key], 0, true);
    const metadataReply = thawGraphBridgeReply(thawGraphParse(thawGraphNapiBridgeHandle(
      'descriptor_graph', thawGraphString(handle), '', metadataPacket)));
    if (!metadataReply || thawGraphOwn(metadataReply, '__thaw_error__'))
      throw new thawGraphTypeError(metadataReply && metadataReply.__thaw_error__
        ? metadataReply.__thaw_error__ : 'Native descriptor lookup failed');
    const previous = metadataReply.value;
    const enumerable = has('enumerable') ? !!get('enumerable')
      : previous ? !!previous.enumerable : false;
    const configurable = has('configurable') ? !!get('configurable')
      : previous ? !!previous.configurable : false;
    const hasValue = has('value');
    const value = hasValue ? get('value') : undefined;
    const hasWritable = has('writable');
    const writable = hasWritable ? !!get('writable')
      : previous && !previous.accessor ? !!previous.writable : false;
    const hasGetter = has('get');
    const getter = hasGetter ? get('get') : undefined;
    const hasSetter = has('set');
    const setter = hasSetter ? get('set') : undefined;
    if ((hasGetter || hasSetter) && (hasValue || hasWritable))
      throw new thawGraphTypeError('Invalid property descriptor');
    if ((getter !== undefined && typeof getter !== 'function')
      || (setter !== undefined && typeof setter !== 'function'))
      throw new thawGraphTypeError('Invalid accessor callback');
    const accessor = hasGetter || hasSetter;
    const flags = (writable ? 1 : 0) | (enumerable ? 2 : 0)
      | (configurable ? 4 : 0);
    const operation = accessor ? 'define_accessor_graph'
      : !hasValue && !hasWritable && previous ? 'reconfigure_graph'
      : 'define_data_graph';
    const packet = thawGraphEncode(operation === 'define_accessor_graph'
      ? [key, getter, setter, flags, (hasGetter ? 1 : 0) | (hasSetter ? 2 : 0)]
      : operation === 'reconfigure_graph' ? [key, flags]
      : [key, value, flags, hasValue ? 1 : 0], 0, true);
    const result = thawGraphBridgeReply(thawGraphParse(thawGraphNapiBridgeHandle(operation,
      thawGraphString(handle), '', packet)));
    if (!result || thawGraphOwn(result, '__thaw_error__'))
      throw new thawGraphTypeError(result && result.__thaw_error__
        ? result.__thaw_error__ : 'Native descriptor definition failed');
    if (result.value !== true) return false;
    if (own && !own.configurable) {
      if (!thawNapiReflectDefineProperty(target, key, input))
        throw new thawGraphTypeError('Cannot synchronize native descriptor');
    } else if (!configurable) {
      // An attribute-only update must keep an accessor an accessor. The
      // proxy target is only a shadow for Proxy invariants; the native
      // descriptor remains the authority for reads and writes.
      const reflectedAccessor = accessor
        || (operation === 'reconfigure_graph' && previous && previous.accessor);
      const readNativeValue = receiver => {
        const response = thawGraphBridgeReply(thawGraphParse(thawGraphNapiBridgeHandle('get_graph',
          thawGraphString(handle), '', thawGraphEncode([key, receiver], 0, true,
            false, thawGraphForeignReceiver(receiver)))));
        if (!response || thawGraphOwn(response, '__thaw_error__'))
          throw new thawGraphTypeError(response && response.__thaw_error__
            ? response.__thaw_error__ : 'Native accessor read failed');
        return response.kind === 'graph' && typeof response.value === 'string'
          ? thawGraphDecodeOwned(response.value) : response.value;
      };
      const reflectedGet = reflectedAccessor
        ? accessor && hasGetter ? getter
          : previous && previous.getter
            ? function() { return readNativeValue(this); } : undefined
        : undefined;
      const reflectedSet = accessor && hasSetter ? setter : previous && previous.setter
        ? function (next) {
            const response = thawGraphBridgeReply(thawGraphParse(thawGraphNapiBridgeHandle('set_graph',
              thawGraphString(handle), '', thawGraphEncode([key, next, this], 0, true,
                false, thawGraphForeignReceiver(this)))));
            if (!response || thawGraphOwn(response, '__thaw_error__'))
              throw new thawGraphTypeError(response && response.__thaw_error__
                ? response.__thaw_error__ : 'Native accessor write failed');
          } : undefined;
      const reflected = reflectedAccessor
        ? { get: reflectedGet, set: reflectedSet, enumerable, configurable }
        : { value: operation === 'reconfigure_graph' && previous
            ? readNativeValue() : value, writable, enumerable, configurable };
      if (!thawNapiReflectDefineProperty(target, key, reflected))
        throw new thawGraphTypeError('Cannot materialize native descriptor');
    }
    return true;
  };
  const thawGraphPreventNativeExtensions = (graphProperty, proxy, target) => {
    const keys = graphProperty('own_keys_graph', []);
    if (!thawGraphArrayIsArray(keys))
      throw new thawGraphTypeError('Invalid native integrity key list');
    // A nonextensible Proxy target must have every native own key. Build its
    // shadow descriptors before changing native authority, while the target
    // is still extensible; descriptor reads use the same live graph route.
    for (const key of keys) {
      const descriptor = thawNapiReflectGetOwnPropertyDescriptor(proxy, key);
      if (descriptor && !thawNapiReflectDefineProperty(target, key, descriptor))
        return false;
    }
    const prototype = thawNapiReflectGetPrototypeOf(proxy);
    if (!thawNapiReflectSetPrototypeOf(target, prototype)) return false;
    if (graphProperty('prevent_extensions_graph', []) !== true) return false;
    return thawNapiReflectPreventExtensions(target);
  };
  const thawGraphNapiFunctionForHandle = handle => {
    if (typeof handle !== 'string'
      || !thawGraphApply(thawGraphRegExpTest, thawGraphPositiveDecimalPattern, [handle]))
      throw new thawGraphTypeError('Invalid native Function graph handle');
    const owner = thawGraphNapiOwnerOfHandle(handle);
    thawGraphNapiState(owner); // Validate the owning live addon before cache use.
    const stored = thawGraphApply(thawGraphMapGet, thawGraphNapiFunctions, [handle]);
    const existing = thawNapiDeref(stored);
    if (existing) return existing;
    // The private Rust factory acquires and owns its positive native root.
    // No transferable token is left behind if the engine cannot enter it.
    const invoke = thawGraphNapiInvoker(handle);
    const invokeNative = (values, construct, forceHandle) => {
      const packet = thawGraphEncode(values, 0, true, false, forceHandle);
      const result = thawGraphBridgeReply(thawGraphParse(invoke(packet, construct)));
      if (!result || thawGraphOwn(result, '__thaw_error__'))
        throw new thawGraphTypeError(result && result.__thaw_error__
          ? result.__thaw_error__ : 'Native Function call failed');
      if (result.kind !== 'graph' || typeof result.value !== 'string')
        throw new thawGraphTypeError('Invalid native Function graph result');
      return thawGraphDecodeOwned(result.value);
    };
    const withReceiver = (receiver, args) => {
      const values = [receiver];
      // Do not consult a user-replaced Array iterator while packing the
      // engine-supplied new.target or the call-time this value.
      for (let index = 0; index < args.length; index++)
        thawNapiArrayAppend(values, args[index]);
      return values;
    };
    const nativeFunction = function() {
      'use strict';
      const args = thawGraphApply(thawNapiArraySlice, arguments, []);
      return invokeNative(withReceiver(this, args), false);
    };
    const graphProperty = (operation, values) => {
      const receiver = operation === 'get_graph' ? values[1]
        : operation === 'set_graph' ? values[2] : undefined;
      const packet = thawGraphEncode(values, 0, true, false,
        thawGraphForeignReceiver(receiver));
      const result = thawGraphBridgeReply(thawGraphParse(thawGraphNapiBridgeHandle(operation, handle, '', packet)));
      if (!result || thawGraphOwn(result, '__thaw_error__'))
        throw new thawGraphTypeError(result && result.__thaw_error__
          ? result.__thaw_error__ : 'Native Function property operation failed');
      return result.kind === 'graph' && typeof result.value === 'string'
        ? thawGraphDecodeOwned(result.value) : result.value;
    };
    const nativeDescriptor = key => graphProperty('descriptor_graph', [key]);
    const fallbackGetters = thawNapiSafeMap();
    const fallbackSetters = thawNapiSafeMap();
    // A bound callable has configurable name/length and no intrinsic
    // nonconfigurable prototype. Its apply/construct traps below preserve the
    // actual call receiver and native constructor route.
    const proxyTarget = thawGraphApply(thawNapiFunctionBind, nativeFunction,
      [undefined]);
    // The bound target has intrinsic name/length of its own. Native
    // configurable properties may temporarily shadow them, particularly
    // after preventExtensions materializes every native descriptor. If the
    // addon later deletes such a property, restore the target's original
    // intrinsic descriptor (or remove a native-only shadow) before another
    // Proxy trap observes the now-absent native key.
    const initialTargetOwn = thawNapiSafeMap();
    thawNapiArrayEach(thawGraphReflectOwnKeys(proxyTarget), key => {
      initialTargetOwn.set(key,
        thawNapiReflectGetOwnPropertyDescriptor(proxyTarget, key));
    });
    const reconcileMissingNativeKey = key => {
      const own = thawNapiReflectGetOwnPropertyDescriptor(proxyTarget, key);
      if (own && own.configurable) {
        const initial = initialTargetOwn.get(key);
        const restored = initial
          ? thawNapiReflectDefineProperty(proxyTarget, key, initial)
          : thawNapiReflectDeleteProperty(proxyTarget, key);
        if (!restored)
          throw new thawGraphTypeError('Cannot synchronize deleted native Function property');
      }
      fallbackGetters.delete(key);
      fallbackSetters.delete(key);
    };
    const proxy = new thawNapiProxyConstructor(proxyTarget, {
      apply(_target, thisArg, args) {
        return thawGraphApply(nativeFunction, thisArg, args);
      },
      construct(_target, args, newTarget) {
        // The engine supplies the actual new.target. Carry it in the same
        // owned graph as the arguments; no mutable side channel may be
        // overwritten by a reentrant constructor/prototype getter.
        // QuickJS's GetFunctionRealm walks through a Proxy to its target.
        // This temporary Proxy suppresses only the probe's prototype Get;
        // the native constructor below performs the one observable Get on
        // the real newTarget. Object construction then yields the fallback
        // Object.prototype belonging to newTarget's actual realm.
        const realmProbe = new thawGraphProxy(newTarget, {
          get(target, key, receiver) {
            return key === 'prototype' ? undefined
              : thawGraphReflectGet(target, key, receiver);
          },
        });
        const realmObject = thawNapiReflectConstruct(thawNapiIntrinsicObject,
          [], realmProbe);
        const fallback = thawNapiReflectGetPrototypeOf(realmObject);
        const values = [newTarget, fallback];
        for (let index = 0; index < args.length; index++)
          thawNapiArrayAppend(values, args[index]);
        return invokeNative(values, true, fallback);
      },
      get(target, key, receiver) {
        const native = graphProperty('has_graph', [key]);
        if (!native) reconcileMissingNativeKey(key);
        const own = thawNapiReflectGetOwnPropertyDescriptor(target, key);
        if (own && !own.configurable && thawGraphOwn(own, 'value') && !own.writable)
          return own.value;
        // A nonconfigurable target accessor without a getter also fixes the
        // Proxy's observable Get result, even if native state changed during
        // an earlier reentrant descriptor callback.
        if (own && !own.configurable && thawGraphOwn(own, 'get')
          && own.get === undefined) return undefined;
        return native ? graphProperty('get_graph', [key, receiver])
          : thawNapiReflectGet(target, key, receiver);
      },
      set(target, key, value, receiver) {
        if (!graphProperty('has_graph', [key])) reconcileMissingNativeKey(key);
        const own = thawNapiReflectGetOwnPropertyDescriptor(target, key);
        // A callable target already owns immutable function properties. A
        // Proxy cannot report a successful write to one of those properties,
        // and native mutation must not happen before that invariant check.
        if (own && !own.configurable
          && ((thawGraphOwn(own, 'value') && !own.writable)
            || (thawGraphOwn(own, 'set') && own.set === undefined)))
          return false;
        if (graphProperty('set_graph', [key, value, receiver]) !== true) return false;
        if (own && !own.configurable && own.writable)
          thawNapiReflectDefineProperty(target, key,
            { value: graphProperty('get_graph', [key]) });
        return true;
      },
      has(target, key) {
        const native = graphProperty('has_graph', [key]);
        if (!native) reconcileMissingNativeKey(key);
        return native || thawNapiReflectHas(target, key);
      },
      ownKeys(target) {
        const keys = graphProperty('own_keys_graph', []);
        if (!thawGraphArrayIsArray(keys))
          throw new thawGraphTypeError('Invalid native Function ownKeys result');
        thawNapiArrayEach(thawGraphReflectOwnKeys(target), key => {
          if (thawGraphApply(thawNapiArrayIndexOf, keys, [key]) < 0) {
            reconcileMissingNativeKey(key);
            if (thawNapiReflectGetOwnPropertyDescriptor(target, key))
              thawNapiArrayAppend(keys, key);
          }
        });
        return keys;
      },
      getOwnPropertyDescriptor(target, key) {
        const own = thawNapiReflectGetOwnPropertyDescriptor(target, key);
        const meta = nativeDescriptor(key);
        if (!meta) {
          reconcileMissingNativeKey(key);
          return thawNapiReflectGetOwnPropertyDescriptor(target, key);
        }
        if (own && !own.configurable) return own;
        let getter = meta.getter_source
          ? thawGraphDescriptorSource(meta.getter_source) : undefined;
        let setter = meta.setter_source
          ? thawGraphDescriptorSource(meta.setter_source) : undefined;
        if (meta.accessor && meta.getter && !getter) {
          getter = fallbackGetters.get(key);
          if (!getter) {
            getter = function() { return graphProperty('get_graph', [key, this]); };
            fallbackGetters.set(key, getter);
          }
        }
        if (meta.accessor && meta.setter && !setter) {
          setter = fallbackSetters.get(key);
          if (!setter) {
            setter = function(value) { graphProperty('set_graph', [key, value, this]); };
            fallbackSetters.set(key, setter);
          }
        }
        const descriptor = meta.accessor
          ? { configurable: !!meta.configurable, enumerable: !!meta.enumerable,
              get: getter, set: setter }
          : { configurable: !!meta.configurable, enumerable: !!meta.enumerable,
              writable: !!meta.writable, value: graphProperty('get_graph', [key]) };
        if (!meta.configurable) {
          // A bound callable's name and length begin configurable. Before a
          // nonconfigurable native descriptor can be reported through Proxy,
          // the target must hold the same descriptor; merely returning the
          // bound function's original metadata would lose native authority.
          if (!thawNapiReflectDefineProperty(target, key, descriptor))
            throw new thawGraphTypeError('Cannot materialize native Function descriptor');
          return thawNapiReflectGetOwnPropertyDescriptor(target, key);
        }
        return descriptor;
      },
      defineProperty(target, key, description) {
        if (!graphProperty('has_graph', [key])) reconcileMissingNativeKey(key);
        return thawGraphDefineNativeProperty(handle, target, key, description);
      },
      deleteProperty(target, key) {
        if (!graphProperty('has_graph', [key])) reconcileMissingNativeKey(key);
        const own = thawNapiReflectGetOwnPropertyDescriptor(target, key);
        if (own && !own.configurable) return false;
        if (graphProperty('delete_graph', [key]) !== true) return false;
        fallbackGetters.delete(key);
        fallbackSetters.delete(key);
        return thawNapiReflectDeleteProperty(target, key);
      },
      getPrototypeOf(target) {
        const native = graphProperty('get_prototype_graph', []);
        // The N-API prototype API returns Undefined for a value without an
        // explicit native prototype. Retain the callable target's ordinary
        // Function.prototype in that case.
        return native === undefined ? thawNapiReflectGetPrototypeOf(target) : native;
      },
      setPrototypeOf(target, prototype) {
        if (!thawNapiReflectIsExtensible(target))
          return thawNapiReflectGetPrototypeOf(target) === prototype;
        // The private bound target can reject a cycle that the native
        // prototype table cannot see (for example a JS Proxy in the chain).
        // Check that target-specific rule before committing native state.
        const previous = thawNapiReflectGetPrototypeOf(target);
        if (!thawNapiReflectSetPrototypeOf(target, prototype)) return false;
        try {
          if (graphProperty('set_prototype_graph', [prototype]) === true) return true;
        } catch (error) {
          thawNapiReflectSetPrototypeOf(target, previous);
          throw error;
        }
        thawNapiReflectSetPrototypeOf(target, previous);
        return false;
      },
      isExtensible(target) {
        if (graphProperty('is_extensible_graph', []) === false
          && thawNapiReflectIsExtensible(target))
          if (!thawGraphPreventNativeExtensions(graphProperty, proxy, target))
            throw new thawGraphTypeError('Cannot synchronize native integrity');
        return thawNapiReflectIsExtensible(target);
      },
      preventExtensions(target) {
        return thawGraphPreventNativeExtensions(graphProperty, proxy, target);
      },
    });
    thawGraphApply(thawGraphWeakMapSet, thawGraphNapiFunctionHandles, [proxy, handle]);
    const entry = thawNapiWeakRef ? new thawNapiWeakRef(proxy) : proxy;
    thawGraphApply(thawGraphMapSet, thawGraphNapiFunctions, [handle, entry]);
    if (thawGraphNapiFunctionFinalizer)
      thawGraphNapiFunctionFinalizer.register(proxy, { id: handle, entry });
    return proxy;
  };
  const thawGraphDecode = graph => {
    if (!graph || typeof graph !== 'object' || !thawGraphArrayIsArray(graph.nodes)
      || !thawGraphOwn(graph, 'root'))
      throw new thawGraphTypeError('Invalid native JSON graph');
    const own = (value, key) => thawGraphOwn(value, key);
    const nodes = thawGraphApply(thawGraphArrayMap, graph.nodes, [node => {
      if (!node || typeof node !== 'object' || thawGraphArrayIsArray(node)
        || (thawGraphObjectKeys(node).length !== 1
          && !(thawGraphObjectKeys(node).length === 2
            && ((own(node, 'a') && own(node, 'p'))
              || (own(node, 'nfn') && own(node, 'hdl'))))))
        throw new thawGraphTypeError('Invalid native JSON graph node');
      if (own(node, 'a') && thawGraphArrayIsArray(node.a)
        && (!own(node, 'p') || thawGraphArrayIsArray(node.p)))
        return new thawGraphArrayConstructor(node.a.length);
      if (own(node, 'o') && thawGraphArrayIsArray(node.o)) return {};
      if (own(node, 'd') && (node.d === null || typeof node.d === 'number'))
        return new thawGraphDateConstructor(node.d === null ? NaN : node.d);
      if (own(node, 'nfn') && own(node, 'hdl')) {
        if (typeof node.nfn !== 'string'
          || !thawGraphApply(thawGraphRegExpTest,
            thawGraphPositiveDecimalPattern, [node.nfn])
          || !thawGraphNumberIsSafeInteger(node.hdl) || node.hdl <= 0
          || !globalThis.__thaw_value_handle_live?.[node.hdl - 1])
          throw new thawGraphTypeError('Invalid paired native Function graph node');
        const value = globalThis.__thaw_value_handles[node.hdl - 1];
        if (!globalThis.__thaw_json_graph_native_pair_matches(value, node.nfn, 1))
          throw new thawGraphTypeError('Mismatched native Function graph node');
        return value;
      }
      if (own(node, 'hdl') && thawGraphNumberIsSafeInteger(node.hdl) && node.hdl > 0) {
        const id = node.hdl;
        if (!globalThis.__thaw_value_handle_live || !globalThis.__thaw_value_handle_live[id - 1])
          throw new thawGraphTypeError('Invalid native JSON graph handle');
        return globalThis.__thaw_value_handles[id - 1];
      }
      if (own(node, 'nh') && typeof node.nh === 'string'
        && thawGraphApply(thawGraphRegExpTest, thawGraphPositiveDecimalPattern, [node.nh])) {
        return thawGraphNapiProxyForHandle(node.nh);
      }
      if (own(node, 'nfn') && typeof node.nfn === 'string'
        && thawGraphApply(thawGraphRegExpTest, thawGraphPositiveDecimalPattern, [node.nfn])) {
        return thawGraphNapiFunctionForHandle(node.nfn);
      }
      if (own(node, 'b') && thawGraphArrayIsArray(node.b)
        && thawGraphApply(thawGraphArrayEvery, node.b, [byte => thawGraphNumberIsInteger(byte) && byte >= 0 && byte <= 255]))
        return thawGraphBufferFrom(node.b);
      if (own(node, 'm')) return new thawGraphMapConstructor();
      if (own(node, 's')) return new thawGraphSetConstructor();
      if (own(node, 're') && thawGraphArrayIsArray(node.re) && node.re.length === 3
        && typeof node.re[0] === 'string' && typeof node.re[1] === 'string') {
        const pattern = new thawGraphRegExpConstructor(node.re[0], node.re[1]);
        pattern.lastIndex = thawGraphNumber(node.re[2] || 0);
        return pattern;
      }
      throw new thawGraphTypeError('Invalid native JSON graph node');
    }]);
    const decode = token => {
      if (!token || typeof token !== 'object' || thawGraphArrayIsArray(token))
        throw new thawGraphTypeError('Invalid native JSON graph token');
      const keys = thawGraphObjectKeys(token);
      if (keys.length !== 1
        && !(keys.length === 2 && own(token, 'nsy') && own(token, 'hdl')))
        throw new thawGraphTypeError('Invalid native JSON graph token');
      if (own(token, 'r')) {
        if (!thawGraphNumberIsSafeInteger(token.r) || token.r < 0 || token.r >= nodes.length)
          throw new thawGraphTypeError('Invalid native JSON graph reference');
        return nodes[token.r];
      }
      if (own(token, 'v')) {
        if (token.v !== null && typeof token.v === 'object')
          throw new thawGraphTypeError('Invalid native JSON graph scalar');
        return token.v;
      }
      if (own(token, 'u') && token.u === 1) return undefined;
      if (own(token, 'su') && thawGraphArrayIsArray(token.su)
        && thawGraphApply(thawGraphArrayEvery, token.su,
          [unit => thawGraphNumberIsInteger(unit) && unit >= 0 && unit <= 65535]))
        return globalThis.__thaw_json_host_utf16_string(thawGraphStringify(token.su));
      if (own(token, 'bi') && typeof token.bi === 'string'
        && thawGraphApply(thawGraphRegExpTest, thawGraphBigIntDecimalPattern, [token.bi]))
        return thawGraphBigInt(token.bi);
      if (own(token, 'nsy') && own(token, 'hdl')) {
        if (typeof token.nsy !== 'string'
          || !thawGraphApply(thawGraphRegExpTest,
            thawGraphPositiveDecimalPattern, [token.nsy])
          || !thawGraphNumberIsSafeInteger(token.hdl) || token.hdl <= 0
          || !globalThis.__thaw_value_handle_live?.[token.hdl - 1])
          throw new thawGraphTypeError('Invalid paired native Symbol graph token');
        const symbol = globalThis.__thaw_value_handles[token.hdl - 1];
        if (!globalThis.__thaw_json_graph_native_pair_matches(symbol, token.nsy, 2))
          throw new thawGraphTypeError('Mismatched native Symbol graph token');
        return symbol;
      }
      if (own(token, 'nsy') && typeof token.nsy === 'string'
        && thawGraphApply(thawGraphRegExpTest, thawGraphPositiveDecimalPattern, [token.nsy])) {
        const symbol = thawGraphNapiSymbolForNative(token.nsy);
        if (globalThis.__thaw_json_host_native_symbol_needs_pin(token.nsy))
          thawHostNativeSymbolPin(token.nsy);
        return symbol;
      }
      if (own(token, 'h') && token.h === 1) return undefined;
      if (own(token, 'nf') && (token.nf === 'NaN' || token.nf === 'Infinity' || token.nf === '-Infinity' || token.nf === '-0'))
        return thawGraphNumber(token.nf);
      throw new thawGraphTypeError('Invalid native JSON graph token');
    };
    thawGraphApply(thawGraphArrayForEach, graph.nodes, [(node, index) => {
      if (own(node, 'a')) {
        thawGraphApply(thawGraphArrayForEach, node.a, [(token, slot) => {
          if (!token || typeof token !== 'object' || token.h !== 1 || thawGraphObjectKeys(token).length !== 1)
            nodes[index][slot] = decode(token);
        }]);
        if (own(node, 'p')) thawGraphApply(thawGraphArrayForEach, node.p, [pair => {
          if (!thawGraphArrayIsArray(pair) || pair.length !== 3)
            throw new thawGraphTypeError('Invalid native JSON graph array property');
          const key = typeof pair[0] === 'string' ? pair[0] : decode(pair[0]);
          if (typeof key !== 'string' && typeof key !== 'symbol')
            throw new thawGraphTypeError('Invalid native JSON graph array property key');
          const flags = pair[2];
          if (!thawGraphNumberIsInteger(flags) || flags < 0 || flags > 7)
            throw new thawGraphTypeError('Invalid native JSON graph array property attributes');
          thawGraphDefineProperty(nodes[index], key, {
            value: decode(pair[1]), writable: !!(flags & 1),
            enumerable: !!(flags & 2), configurable: !!(flags & 4),
          });
        }]);
      } else if (own(node, 'o')) {
        thawGraphApply(thawGraphArrayForEach, node.o, [pair => {
          if (!thawGraphArrayIsArray(pair) || (pair.length !== 2 && pair.length !== 3))
            throw new thawGraphTypeError('Invalid native JSON graph property');
          const key = typeof pair[0] === 'string' ? pair[0] : decode(pair[0]);
          if (typeof key !== 'string' && typeof key !== 'symbol')
            throw new thawGraphTypeError('Invalid native JSON graph property');
          const flags = pair.length === 2 ? 7 : pair[2];
          if (!thawGraphNumberIsInteger(flags) || flags < 0 || flags > 7)
            throw new thawGraphTypeError('Invalid native JSON graph property attributes');
          thawGraphDefineProperty(nodes[index], key, {
            value: decode(pair[1]), writable: !!(flags & 1),
            enumerable: !!(flags & 2), configurable: !!(flags & 4),
          });
        }]);
      }
    }]);
    // Container nodes are fully populated before Map/Set entries are read.
    // Their entries can themselves refer back to the Map/Set being filled.
    thawGraphApply(thawGraphArrayForEach, graph.nodes, [(node, index) => {
      if (own(node, 'm')) {
        const entries = decode(node.m);
        if (!thawGraphArrayIsArray(entries)) throw new thawGraphTypeError('Invalid native Map entries');
        thawGraphApply(thawGraphArrayForEach, entries, [pair => {
          if (!thawGraphArrayIsArray(pair) || pair.length < 2)
            throw new thawGraphTypeError('Invalid native Map entry');
          thawGraphApply(thawGraphMapSet, nodes[index], [pair[0], pair[1]]);
        }]);
      } else if (own(node, 's')) {
        const values = decode(node.s);
        if (!thawGraphArrayIsArray(values)) throw new thawGraphTypeError('Invalid native Set values');
        thawGraphApply(thawGraphArrayForEach, values, [value => thawGraphApply(thawGraphSetAdd, nodes[index], [value])]);
      }
    }]);
    return decode(graph.root);
  };
  thawGraphDefineProperty(globalThis, '__thaw_json_graph_decode', {
    value: thawGraphDecode, writable: false, configurable: false,
  });
  // Native result Json can be destroyed before this decoder runs. The graph
  // owns one extra registry reference per live host node until reconstruction
  // finishes, including when a malformed graph or getter throws.
  const thawGraphDecodeOwned = text => {
    const graph = thawGraphParse(text);
    try {
      return thawGraphDecode(graph);
    } finally {
      try {
        if (thawGraphArrayIsArray(graph?.leases)) {
          for (let index = 0; index < graph.leases.length; index++) {
            const handle = graph.leases[index];
            if (thawGraphNumberIsSafeInteger(handle) && handle > 0)
              thawGraphRelease(handle);
          }
        }
      } finally {
        if (thawGraphArrayIsArray(graph?.napiLeases)) {
          for (let index = 0; index < graph.napiLeases.length; index++) {
            const reference = graph.napiLeases[index];
            if (typeof reference === 'string'
              && thawGraphApply(thawGraphRegExpTest, thawGraphPositiveDecimalPattern, [reference])
              && typeof thawGraphNapiRelease === 'function')
              thawGraphNapiRelease(reference);
          }
        }
      }
    }
  };
  thawGraphDefineProperty(globalThis, '__thaw_json_graph_decode_owned', {
    value: thawGraphDecodeOwned, writable: false, configurable: false,
  });
  const thawGraphBridgeReply = response => {
    if (response && thawGraphOwn(response, '__thaw_exception_graph__')) {
      if (typeof response.__thaw_exception_graph__ !== 'string')
        throw new thawGraphTypeError('Invalid native exception graph');
      // Decoding releases every transfer lease even if the thrown value is
      // itself an object whose reconstruction fails.
      throw thawGraphDecodeOwned(response.__thaw_exception_graph__);
    }
    return response;
  };
  thawGraphDefineProperty(globalThis, '__thaw_object_with_native_getters', {
    value: (keys, readable, writable, accessors, ...callbacks) => {
      const target = {};
      const positions = new thawGraphMapConstructor();
      for (let i = 0; i < keys.length; i++) {
        thawGraphApply(thawGraphMapSet, positions, [keys[i], i]);
        if (accessors[i]) {
          thawGraphDefineProperty(target, keys[i], {
            get: readable[i] ? callbacks[i * 2] : undefined,
            set: writable[i] ? callbacks[i * 2 + 1] : undefined,
            enumerable: true, configurable: true,
          });
        } else {
          thawGraphDefineProperty(target, keys[i], {
            value: undefined, writable: true, enumerable: true, configurable: true,
          });
        }
      }
      const state = thawGraphApply(thawGraphArraySlice, callbacks, [keys.length * 2]);
      const [extensible, sealed, frozen, setState, propertyFlags, setFlags, canFlags] = state;
      const defineWrites = thawGraphApply(thawGraphArraySlice, state, [7]);
      const stateReceiver = thawGraphObjectCreate(null);
      let wrapper;
      const read = index => thawGraphApply(callbacks[index * 2], wrapper, []);
      const write = (index, value) => thawGraphApply(callbacks[index * 2 + 1], wrapper, [value]);
      const defineWrite = (index, value, requested) =>
        thawGraphApply(defineWrites[index], wrapper, [value, requested]);
      // State callbacks capture the native pointer and have no receiver.
      // Supplying this wrapper as their JS `this` would make the graph
      // callback encode the whole Proxy holder and recursively enter sync().
      // The generated callback is non-strict: `null` would become globalThis,
      // so use a private empty holder instead.
      const flags = key => thawGraphNumber(thawGraphApply(propertyFlags, stateReceiver, [key]));
      let syncing = false;
      const sync = () => {
        if (syncing) return;
        syncing = true;
        try {
        // A direct native Object.freeze/seal/preventExtensions can run after
        // this wrapper was created. Bring the Proxy target into the same
        // state before reporting descriptors or extensibility; Proxy
        // invariants require the target to carry non-configurable flags.
        const nativeFrozen = thawGraphApply(frozen, stateReceiver, []);
        const nativeSealed = nativeFrozen || thawGraphApply(sealed, stateReceiver, []);
        for (let i = 0; i < keys.length; i++) {
          const key = keys[i];
          const current = thawGraphReflectGetOwnPropertyDescriptor(target, key);
          const bits = flags(key);
          const configurable = !!(bits & 4) && !nativeSealed;
          const enumerable = !!(bits & 2);
          if (accessors[i]) {
            if (current.configurable && (!configurable || current.enumerable !== enumerable)) {
              thawGraphReflectDefineProperty(target, key, { configurable, enumerable });
            }
          } else if (current.configurable || current.writable) {
            const writableNow = !!(bits & 1) && !nativeFrozen;
            const update = { configurable, enumerable, writable: writableNow };
            if (!writableNow || !configurable) update.value = read(i);
            thawGraphReflectDefineProperty(target, key, update);
          }
        }
        if (!thawGraphApply(extensible, stateReceiver, [])) {
          thawGraphReflectPreventExtensions(target);
        }
        } finally { syncing = false; }
      };
      wrapper = new thawGraphProxy(target, {
        get(_target, key, receiver) {
          sync();
          const index = thawGraphApply(thawGraphMapGet, positions, [key]);
          if (index === undefined) return thawGraphReflectGet(target, key, receiver);
          if (accessors[index]) return thawGraphReflectGet(target, key, receiver);
          const descriptor = thawGraphReflectGetOwnPropertyDescriptor(target, key);
          return descriptor.configurable || descriptor.writable ? read(index) : descriptor.value;
        },
        set(_target, key, value, receiver) {
          sync();
          const index = thawGraphApply(thawGraphMapGet, positions, [key]);
          if (index === undefined) return false;
          if (accessors[index]) return thawGraphReflectSet(target, key, value, receiver);
          if (!(flags(key) & 1)) return false;
          write(index, value);
          return true;
        },
        deleteProperty(_target, key) {
          sync();
          // Native fixed-layout fields cannot be removed from the owner.
          // Leaving the Proxy default would delete only its shadow slot.
          if (thawGraphApply(thawGraphMapGet, positions, [key]) !== undefined) return false;
          return thawGraphReflectDeleteProperty(target, key);
        },
        ownKeys() { sync(); return thawGraphReflectOwnKeys(target); },
        getOwnPropertyDescriptor(_target, key) {
          sync();
          const descriptor = thawGraphReflectGetOwnPropertyDescriptor(target, key);
          const index = thawGraphApply(thawGraphMapGet, positions, [key]);
          if (index === undefined || accessors[index] || !descriptor) return descriptor;
          return { value: descriptor.configurable || descriptor.writable ? read(index) : descriptor.value,
            writable: descriptor.writable, enumerable: descriptor.enumerable,
            configurable: descriptor.configurable };
        },
        isExtensible() { sync(); return thawGraphApply(extensible, stateReceiver, []); },
        preventExtensions() {
          sync();
          if (!thawGraphApply(setState, stateReceiver, [keys.length ? 1 : 3])) return false;
          return thawGraphReflectPreventExtensions(target);
        },
        defineProperty(_target, key, descriptor) {
          sync();
          const index = thawGraphApply(thawGraphMapGet, positions, [key]);
          if (index === undefined) return false;
          const before = thawGraphReflectGetOwnPropertyDescriptor(target, key);
          const updated = { ...descriptor };
          if (!accessors[index]) {
            if ('get' in descriptor || 'set' in descriptor) return false;
          } else if ('get' in descriptor || 'set' in descriptor || 'value' in descriptor
              || 'writable' in descriptor) {
            // Replacing the native accessor itself needs a distinct owner
            // metadata mutation, not merely a change to this JS target.
            return false;
          }
          const requested = ((updated.writable ?? before.writable) ? 1 : 0)
            | ((updated.enumerable ?? before.enumerable) ? 2 : 0)
            | ((updated.configurable ?? before.configurable) ? 4 : 0);
          if (!thawGraphApply(canFlags, stateReceiver, [key, requested])) return false;
          // Check the JS target transition without changing it. The native
          // value write may still throw; committing target attributes first
          // would leave the alias with a descriptor the owner never accepted.
          const probe = thawGraphObjectCreate(null);
          if (!thawGraphReflectDefineProperty(probe, key, before)
              || !thawGraphReflectDefineProperty(probe, key, updated)) return false;
          if (!accessors[index] && 'value' in descriptor) {
            const current = flags(key);
            // A frozen, non-writable field accepts the same value without a
            // write. Configurable non-writable fields accept a new value via
            // DefineOwnProperty, unlike ordinary assignment.
            if (!(current & 1) && !(current & 4)) {
              if (!thawGraphObjectIs(descriptor.value, read(index))) return false;
            } else {
              if (!defineWrite(index, descriptor.value, requested)) return false;
            }
          }
          // A fixed field can coerce the written value. Once the target
          // becomes non-configurable and non-writable, Proxy invariants need
          // its actual stored value, including for an attributes-only update.
          if (!accessors[index]) updated.value = read(index);
          if (!thawGraphApply(setFlags, stateReceiver, [key, requested])) return false;
          if (!thawGraphReflectDefineProperty(target, key, updated)) return false;
          if (thawGraphObjectIsFrozen(target)) thawGraphApply(setState, stateReceiver, [3]);
          else if (thawGraphObjectIsSealed(target)) thawGraphApply(setState, stateReceiver, [2]);
          return true;
        },
      });
      return wrapper;
    },
    writable: false, configurable: false,
  });
  globalThis.__thaw_object_from_operations = (kinds, keys, values, ...callbacks) => {
    const object = {};
    let callback = 0;
    for (let i = 0; i < kinds.length; i++) {
      const kind = kinds[i];
      if (kind === 'spread') {
        const source = values[i];
        if (source == null) continue;
        for (const key of Reflect.ownKeys(Object(source))) {
          if (!Object.getOwnPropertyDescriptor(source, key)?.enumerable) continue;
          Object.defineProperty(object, key, {
            value: source[key], writable: true, enumerable: true, configurable: true,
          });
        }
        continue;
      }
      const key = keys[i];
      if (kind === 'data') {
        Object.defineProperty(object, key, {
          value: values[i], writable: true, enumerable: true, configurable: true,
        });
        continue;
      }
      const fn = callbacks[callback++];
      if (kind === 'method') {
        Object.defineProperty(object, key, {
          value: function (...args) { return fn(this, ...args); },
          writable: true, enumerable: true, configurable: true,
        });
        continue;
      }
      const previous = Object.getOwnPropertyDescriptor(object, key);
      const accessor = previous && !('value' in previous) ? previous : {};
      Object.defineProperty(object, key, {
        get: kind === 'getter' ? function () { return fn(this); } : accessor.get,
        set: kind === 'setter' ? function (value) { fn(this, value); } : accessor.set,
        enumerable: true,
        configurable: true,
      });
    }
    return object;
  };
  globalThis.__thaw_accessor_descriptor = (readable, writable, getter, setter, flags = 7) => ({
    get: readable ? getter : undefined,
    set: writable ? setter : undefined,
    enumerable: !!(flags & 2),
    configurable: !!(flags & 4),
  });
  globalThis.__thaw_data_descriptor = (value, flags = 7) => ({
    value,
    writable: !!(flags & 1),
    enumerable: !!(flags & 2),
    configurable: !!(flags & 4),
  });
  globalThis.__thaw_json_stringify_native_accessors = (replacer, space, value) =>
    JSON.stringify(value, replacer, space);
  globalThis.__thaw_iterator_from = source => {
    if (source?.__thawNativeIterator) {
      return Object.assign(Object.create(Iterator.prototype), source, {
        next: (...args) => args.length ? source.__thawNext(args[0]) : source.next(),
        return: (...args) => args.length ? source.__thawReturn(args[0]) : source.return(),
        throw: value => source.__thawThrow(String(value))
      });
    }
    let iterator = source[Symbol.iterator]?.() ?? source;
    if (typeof iterator.return !== 'function') {
      const sourceIterator = iterator;
      iterator = Object.assign(Object.create(Iterator.prototype), {
        next: sourceIterator.next.bind(sourceIterator),
        return: value => ({ value, done: true })
      });
    }
    return Iterator.from(iterator);
  };
  // Generic `instanceof` check for a live `JsValue` (opaque QuickJS
  // handle) left operand against *any* named global constructor --
  // `dynamic_value_check_by_name` (thaw-hir's `expressions/
  // coercions.rs`) passes the class name across the boundary as a
  // positional JSON argument rather than needing one hardcoded
  // `value => value instanceof X` global per class name. A name that
  // doesn't resolve to a callable (a typo, or a global this runtime
  // doesn't define) answers `false` rather than throwing, matching how
  // every native-builtin-specific check this replaced already degraded.
  globalThis.__thaw_instanceof_dynamic_value_by_name = (name, value) => {
    const ctor = globalThis[name];
    return typeof ctor === 'function' && value instanceof ctor;
  };
  // General sibling of the Date-only check above, for `value instanceof C`
  // where both sides are live handles (e.g. a decorated class's own
  // "class token", class-transformer's `plainToInstance(User, ...)`
  // result). A non-callable right operand (a plain object) is `false`
  // rather than a thrown TypeError, matching the compile-time path this
  // replaces.
  // Serializes an Error's own *and inherited* enumerable scalar/object
  // properties (everything but `name`/`message`/`stack`) to JSON, for the
  // exception ABI. `JSON.stringify` alone would miss `http-errors`'
  // `status`/`statusCode`/`expose`, which live on the error's *prototype*;
  // `for...in` walks the prototype chain. Used by `describe_tagged_
  // exception` (thaw-quickjs) and replayed by `__thaw_error_from_tagged`.
  globalThis.__thaw_error_properties_json = function (error) {
    if (!error || (typeof error !== 'object' && typeof error !== 'function')) return '';
    const out = {};
    for (const key in error) {
      if (key === 'message' || key === 'name' || key === 'stack') continue;
      const value = error[key];
      const type = typeof value;
      if (
        type === 'string' ||
        type === 'number' ||
        type === 'boolean' ||
        value === null ||
        (type === 'object' && value !== null)
      ) {
        out[key] = value;
      }
    }
    return JSON.stringify(out);
  };
  globalThis.__thaw_instanceof_dynamic_value = (value, target) =>
    typeof target === 'function' ? value instanceof target : false;
  // Rebuilds a real Error from the tagged exception string a native
  // boundary carries (`describe_tagged_exception`, thaw-quickjs): an
  // optional `\u0001name\u0001message` prefix, an optional trailing
  // `\u0005<json>` bag of the error's own extra properties (`http-errors`'
  // `status`/`statusCode`/`expose`/`headers`, which koa's own error
  // handler reads), and the native name-override/cause/code tags
  // (`\u0004`/`\u0002`/`\u0003`) stripped from the message. Shared by both
  // the synchronous callback-error and rejected-native-Promise paths of
  // the native-callback wrapper.
  // Native frames count UTF-16 units, exactly the units String.slice uses.
  globalThis.__thaw_error_frame_parts = function (raw) {
    if (raw.charCodeAt(0) !== 1) return null;
    const separator = raw.indexOf('\u0001', 1);
    if (separator < 1) return null;
    const body = raw.slice(separator + 1);
    if (!body.startsWith('\u001eE1:')) return null;
    let cursor = 4;
    const length = () => {
      const end = body.indexOf(':', cursor);
      if (end <= cursor) return null;
      const digits = body.slice(cursor, end);
      if (!/^[0-9]+$/.test(digits)) return null;
      const value = Number(digits);
      if (!Number.isSafeInteger(value)) return null;
      cursor = end + 1;
      return value;
    };
    const displayLength = length();
    if (displayLength === null) return null;
    let originalLength = null;
    if (body.slice(cursor, cursor + 2) === '-:') cursor += 2;
    else {
      originalLength = length();
      if (originalLength === null) return null;
    }
    if (cursor + displayLength > body.length) return null;
    const display = body.slice(cursor, cursor + displayLength);
    cursor += displayLength;
    let original = null;
    if (originalLength !== null) {
      if (cursor + originalLength > body.length) return null;
      original = body.slice(cursor, cursor + originalLength);
      cursor += originalLength;
    }
    const chain = raw.slice(1, separator);
    const name = chain.charCodeAt(0) === 30
      ? chain.slice(1).split('\u001f')[0] : chain.split('$')[0];
    return { name, display, original, suffix: body.slice(cursor) };
  };
  globalThis.__thaw_error_from_tagged = function (raw) {
    const frame = globalThis.__thaw_error_frame_parts(raw);
    if (frame) {
      const error = new Error(frame.original === null ? frame.display : frame.original);
      error.name = frame.name;
      const propertiesIndex = frame.suffix.indexOf('\u0005');
      if (propertiesIndex >= 0) {
        try {
          const properties = JSON.parse(frame.suffix.slice(propertiesIndex + 1));
          for (const key in properties) error[key] = properties[key];
        } catch (ignored) {}
      }
      return error;
    }
    let propertiesJson = null;
    const propertiesIndex = raw.indexOf('\u0005');
    if (propertiesIndex >= 0) {
      propertiesJson = raw.slice(propertiesIndex + 1);
      raw = raw.slice(0, propertiesIndex);
    }
    let name = 'Error';
    let message = raw;
    if (raw.charCodeAt(0) === 1) {
      const separator = raw.indexOf('\u0001', 1);
      if (separator > 1) {
        const chain = raw.slice(1, separator);
        name = chain.charCodeAt(0) === 30
          ? chain.slice(1).split('\u001f')[0]
          : chain.split('$')[0];
        message = raw.slice(separator + 1);
      }
    }
    message = message.split('\u0004')[0].split('\u0002')[0].split('\u0003')[0];
    const error = new Error(message);
    error.name = name;
    if (propertiesJson) {
      try {
        const properties = JSON.parse(propertiesJson);
        for (const key in properties) error[key] = properties[key];
      } catch (ignored) {}
    }
    return error;
  };
  globalThis.__thaw_is_buffer_dynamic_value = value => Buffer.isBuffer(value);
  const timers = new Map();
  const nativeDateNow = Date.now.bind(Date);
  const nativeDate = Date;
  const nativeReflectConstruct = Reflect.construct;
  const nativeReflectApply = Reflect.apply;
  const nativeDateToString = Date.prototype.toString;
  const mockClocks = [];
  const activeMockClock = api => {
    for (let index = mockClocks.length - 1; index >= 0; index--) {
      const clock = mockClocks[index];
      if (clock.enabled && clock.apis.has(api)) return clock;
    }
    return null;
  };
  const refreshMockDate = () => {
    const clock = activeMockClock('Date');
    globalThis.Date = clock ? clock.date : nativeDate;
  };
  const mockDateFor = clock => {
    function MockDate(...args) {
      if (!new.target) return nativeReflectApply(nativeDateToString, new nativeDate(clock.now), []);
      return nativeReflectConstruct(nativeDate, args.length ? args : [clock.now], new.target);
    }
    Object.setPrototypeOf(MockDate, nativeDate);
    MockDate.prototype = nativeDate.prototype;
    MockDate.now = () => clock.now;
    return MockDate;
  };
  const normalizeDelay = value => {
    const number = Number(value);
    if (!Number.isFinite(number) || number < 0) return 0;
    return Math.min(Math.trunc(number), 2147483647);
  };
  const timerHandle = id => ({
    ref() { const timer = timers.get(id); if (timer && !timer.clock) timer.refed = true; return this; },
    unref() { const timer = timers.get(id); if (timer && !timer.clock) timer.refed = false; return this; },
    hasRef() { const timer = timers.get(id); return Boolean(timer && (timer.clock || timer.refed)); },
    refresh() { const timer = timers.get(id); if (timer && !timer.clock) timer.due = nativeDateNow() + timer.milliseconds; return this; },
    close() { timers.delete(id); },
    [Symbol.toPrimitive]() { return id; }
  });
  const schedule = (callback, delay, repeat, args, refed = true, api = 'setTimeout', forceReal = false) => {
    if (typeof callback !== 'function') {
      throw new TypeError('timer callback must be a function');
    }
    const id = nextTimerId++;
    const milliseconds = normalizeDelay(delay);
    const clock = forceReal ? null : activeMockClock(api);
    timers.set(id, { callback, args, repeat, milliseconds, refed,
                     asyncContext: thawAsyncContext.get(),
                     clock, due: (clock ? clock.now : nativeDateNow()) + (clock && api === 'setImmediate' ? -1 : milliseconds) });
    return timerHandle(id);
  };
  globalThis.setTimeout = (callback, delay = 0, ...args) =>
    schedule(callback, delay, false, args, true, 'setTimeout');
  globalThis.clearTimeout = id => { timers.delete(Number(id)); };
  globalThis.setInterval = (callback, delay = 0, ...args) =>
    schedule(callback, delay, true, args, true, 'setInterval');
  globalThis.clearInterval = globalThis.clearTimeout;
  globalThis.setImmediate = (callback, ...args) =>
    schedule(callback, 0, false, args, true, 'setImmediate');
  globalThis.clearImmediate = globalThis.clearTimeout;
  globalThis.__thaw_set_timeout_ref = (callback, delay, refed) =>
    schedule(callback, delay, false, [], Boolean(refed), 'setTimeout', true);
  globalThis.__thaw_set_timer_ref = (id, refed) => {
    const timer = timers.get(Number(id));
    if (timer) timer.refed = Boolean(refed);
  };
  const invokeTimer = (timer, propagate = false) => {
    const previousAsyncContext = thawAsyncContext.swap(timer.asyncContext);
    try {
      if (propagate) return timer.callback(...timer.args);
      try { timer.callback(...timer.args); }
      catch (error) {
        if (typeof process === 'undefined' || !process.emit) throw error;
        process.emit('uncaughtExceptionMonitor', error, 'uncaughtException');
        if (!process.emit('uncaughtException', error, 'uncaughtException')) throw error;
      }
    } finally { thawAsyncContext.swap(previousAsyncContext); }
  };
  const nextMockTimer = (clock, through) => {
    let selected = null;
    for (const entry of timers) {
      const [id, timer] = entry;
      if (timer.clock !== clock || timer.due > through) continue;
      if (!selected || timer.due < selected[1].due ||
          (timer.due === selected[1].due && id < selected[0])) selected = entry;
    }
    return selected;
  };
  const fireMockTimer = (clock, id, timer) => {
    if (timers.get(id) !== timer) return;
    invokeTimer(timer, true);
    if (timers.get(id) !== timer) return;
    if (timer.repeat) timer.due += timer.milliseconds;
    else timers.delete(id);
  };
  const requireMockClock = clock => {
    if (!clock || !clock.enabled || !mockClocks.includes(clock)) throw new Error('mock timers are not enabled');
  };
  globalThis.__thaw_test_mock_timers_enable = options => {
    options = options === undefined ? {} : options;
    if (!options || typeof options !== 'object') throw new TypeError('mock timer options must be an object');
    const supported = ['setTimeout', 'setInterval', 'setImmediate', 'Date'];
    const requestedApis = options.apis;
    const apis = requestedApis === undefined ? supported : requestedApis;
    if (!Array.isArray(apis) || apis.some(api => !supported.includes(api))) throw new TypeError('invalid mock timer API');
    const requestedNow = options.now;
    const now = requestedNow === undefined ? 0 : Number(requestedNow);
    if (!Number.isFinite(now) || now < 0) throw new RangeError('invalid mock timer time');
    const clock = { enabled: true, now, apis: new Set(apis) };
    clock.date = mockDateFor(clock);
    mockClocks.push(clock);
    refreshMockDate();
    return clock;
  };
  globalThis.__thaw_test_mock_timers_tick = (clock, milliseconds) => {
    requireMockClock(clock);
    const amount = Number(milliseconds === undefined ? 1 : milliseconds);
    if (!Number.isFinite(amount) || amount < 0) throw new RangeError('invalid mock timer advance');
    const through = clock.now + amount;
    if (!Number.isFinite(through)) throw new RangeError('invalid mock timer advance');
    clock.now = through;
    let entry;
    while ((entry = nextMockTimer(clock, through))) fireMockTimer(clock, entry[0], entry[1]);
  };
  globalThis.__thaw_test_mock_timers_run_all = clock => {
    requireMockClock(clock);
    let lastDue = -Infinity;
    for (const timer of timers.values()) if (timer.clock === clock) lastDue = Math.max(lastDue, timer.due);
    if (lastDue !== -Infinity) globalThis.__thaw_test_mock_timers_tick(clock, lastDue - clock.now);
  };
  globalThis.__thaw_test_mock_timers_set_time = (clock, milliseconds) => {
    requireMockClock(clock);
    const now = Number(milliseconds === undefined ? 0 : milliseconds);
    if (!Number.isFinite(now) || now < 0) throw new RangeError('invalid mock timer time');
    clock.now = now;
  };
  globalThis.__thaw_test_mock_timers_reset = clock => {
    requireMockClock(clock);
    clock.enabled = false;
    for (const [id, timer] of timers) if (timer.clock === clock) timers.delete(id);
    mockClocks.splice(mockClocks.indexOf(clock), 1);
    refreshMockDate();
  };
  globalThis.__thaw_test_real_now = nativeDateNow;
  globalThis.queueMicrotask = callback => {
    if (typeof callback !== 'function') {
      throw new TypeError('microtask callback must be a function');
    }
    Promise.resolve().then(callback);
  };
  const consoleInspect = value => {
    if (typeof value === 'string') return value;
    if (typeof value === 'symbol' || typeof value === 'bigint') return String(value);
    try { const encoded = JSON.stringify(value); return encoded === undefined ? String(value) : encoded; }
    catch (_) { return '[Circular]'; }
  };
  const consoleFormat = (...args) => {
    if (args.length === 0) return '';
    if (typeof args[0] !== 'string') return args.map(consoleInspect).join(' ');
    let index = 1;
    let output = args[0].replace(/%[sdifjoOc%]/g, token => {
      if (token === '%%') return '%';
      if (index >= args.length) return token;
      const value = args[index++];
      if (token === '%s') return String(value);
      if (token === '%d' || token === '%f') return String(Number(value));
      if (token === '%i') return String(parseInt(value, 10));
      if (token === '%c') return '';
      return consoleInspect(value);
    });
    while (index < args.length) output += ' ' + consoleInspect(args[index++]);
    return output;
  };
  globalThis.Console = class Console {
    constructor(stdout, stderr) {
      const options = stdout && stdout.stdout ? stdout : { stdout, stderr };
      // Node exposes `console._stdout`/`_stderr` as the real
      // `process.stdout`/`process.stderr` *streams* (objects with a
      // `.write(text)` method), not the raw writer this realm starts
      // with. Code that reaches past the `console` API and writes
      // directly to them -- real trigger: winston's own Console transport
      // (`console._stdout.write(...)`) -- got `undefined` for `.write`
      // and threw "not a function". Wrapped in a minimal stream object
      // here, while `_write` below keeps accepting either shape (a raw
      // writer function, or an object with `.write`).
      const asStream = sink => {
        if (sink && typeof sink.write === 'function') return sink;
        const writer = typeof sink === 'function' ? sink : () => {};
        return { write: writer };
      };
      this._stdout = asStream(options.stdout);
      this._stderr = asStream(options.stderr || options.stdout);
      this._counts = new Map();
      this._timers = new Map();
      this._indent = '';
      for (const name of ['log', 'info', 'debug', 'warn', 'error']) {
        this[name] = this[name].bind(this);
      }
    }
    _write(stream, args) {
      const text = this._indent + consoleFormat(...args) + '\n';
      if (typeof stream === 'function') stream(text);
      else if (stream && typeof stream.write === 'function') stream.write(text);
    }
    log(...args) { this._write(this._stdout, args); }
    info(...args) { this.log(...args); }
    debug(...args) { this.log(...args); }
    warn(...args) { this._write(this._stderr, args); }
    error(...args) { this.warn(...args); }
    dir(value, options) { this.log(consoleInspect(value)); }
    dirxml(...args) { this.log(...args); }
    table(value) { this.log(consoleInspect(value)); }
    assert(condition, ...args) {
      if (!condition) this.error('Assertion failed' + (args.length ? ': ' + consoleFormat(...args) : ''));
    }
    count(label = 'default') {
      const name = String(label); const value = (this._counts.get(name) || 0) + 1;
      this._counts.set(name, value); this.log(`${name}: ${value}`);
    }
    countReset(label = 'default') { this._counts.delete(String(label)); }
    time(label = 'default') { this._timers.set(String(label), Date.now()); }
    timeLog(label = 'default', ...args) {
      const name = String(label); const started = this._timers.get(name);
      if (started !== undefined) this.log(`${name}: ${Date.now() - started}ms`, ...args);
    }
    timeEnd(label = 'default') { const name = String(label); this.timeLog(name); this._timers.delete(name); }
    group(...args) { if (args.length) this.log(...args); this._indent += '  '; }
    groupCollapsed(...args) { this.group(...args); }
    groupEnd() { this._indent = this._indent.substring(0, Math.max(0, this._indent.length - 2)); }
    trace(...args) {
      const error = new Error(consoleFormat(...args));
      this.error(`Trace: ${error.message}` + (error.stack ? `\n${error.stack}` : ''));
    }
    clear() {}
    profile() {}
    profileEnd() {}
    timeStamp() {}
  };
  if (typeof globalThis.console === 'undefined') {
    globalThis.console = new Console(globalThis.__thaw_console_stdout,
                                     globalThis.__thaw_console_stderr);
  }
  // Real Node fully drains its own separate nextTick queue before
  // running *any* pending Promise microtask, at every checkpoint --
  // routing through `queueMicrotask` (itself `Promise.resolve().then
  // (callback)`, see above) would only ever give plain FIFO ordering
  // against whatever `.then()` calls already happened to be scheduled
  // first, never the "nextTick always wins" guarantee real code relies
  // on. `nextTickQueue` is a plain array, drained by
  // `__thaw_drain_next_tick_queue` (thaw-quickjs's Rust host calls this
  // immediately before every single native Promise-job execution, not
  // just once per batch, so a nextTick queued from inside one
  // microtask still runs before the *next* one).
  let nextTickQueue = [];
  const nextTick = (callback, ...args) => {
    if (typeof callback !== 'function') {
      throw new TypeError('process.nextTick callback must be a function');
    }
    nextTickQueue.push({ callback, args, asyncContext: thawAsyncContext.get() });
  };
  globalThis.__thaw_next_tick_queue_pending = () => nextTickQueue.length > 0;
  // Returns whether it actually ran anything -- callers that poll "is
  // some condition met yet, else give up" (`finish_with_platform_events`,
  // `thaw_js_run_until_native_resolved` in api.rs) need to re-check that
  // condition immediately after a nextTick callback runs (it may be the
  // very thing that resolves the promise they're waiting on), not fall
  // through to their own "nothing left to do" exhaustion check first.
  globalThis.__thaw_drain_next_tick_queue = () => {
    let ranAny = false;
    while (nextTickQueue.length) {
      ranAny = true;
      const batch = nextTickQueue;
      nextTickQueue = [];
      let index = 0;
      try {
        for (; index < batch.length; index++) {
          const { callback, args, asyncContext } = batch[index];
          const previousAsyncContext = thawAsyncContext.swap(asyncContext);
          try {
            try {
              callback(...args);
            } catch (error) {
              // Matches `__thaw_run_due_timers`'s uncaught handling.
              if (typeof process === 'undefined' || !process.emit) throw error;
              process.emit('uncaughtExceptionMonitor', error, 'uncaughtException');
              if (!process.emit('uncaughtException', error, 'uncaughtException')) throw error;
            }
          } finally {
            thawAsyncContext.swap(previousAsyncContext);
          }
        }
      } catch (error) {
        // The batch was removed from the queue before dispatch. Keep its
        // unrun callbacks ahead of newly queued ticks for the next pass.
        nextTickQueue = batch.slice(index + 1).concat(nextTickQueue);
        throw error;
      }
    }
    return ranAny;
  };
  if (typeof globalThis.process === 'undefined') globalThis.process = {};
  const hostInfo = typeof globalThis.__thaw_os_identity === 'function'
    ? JSON.parse(globalThis.__thaw_os_identity()) : {};
  const processStart = Date.now();
  const processListeners = new Map();
  const processOn = (event, listener, once = false) => {
    if (typeof listener !== 'function') throw new TypeError('listener must be a function');
    const name = String(event);
    const listeners = processListeners.get(name) || [];
    if (listeners.length === 0 && (name === 'SIGINT' || name === 'SIGTERM')) {
      globalThis.__thaw_process_configure_signal(name, true);
    }
    listeners.push({ listener, once });
    processListeners.set(name, listeners);
    return globalThis.process;
  };
  const processOff = (event, listener) => {
    const name = String(event);
    const listeners = processListeners.get(name) || [];
    const remaining = listeners.filter(entry => entry.listener !== listener);
    processListeners.set(name, remaining);
    if (remaining.length === 0 && (name === 'SIGINT' || name === 'SIGTERM')) {
      globalThis.__thaw_process_configure_signal(name, false);
    }
    return globalThis.process;
  };
  const processEmit = (event, ...args) => {
    const name = String(event);
    const listeners = (processListeners.get(name) || []).slice();
    for (const entry of listeners) {
      if (entry.once) processOff(name, entry.listener);
      entry.listener(...args);
    }
    return listeners.length !== 0;
  };
  const hrtime = previous => {
    const nanoseconds = BigInt(Date.now() - processStart) * 1000000n;
    let seconds = Number(nanoseconds / 1000000000n);
    let remainder = Number(nanoseconds % 1000000000n);
    if (previous !== undefined) {
      seconds -= Number(previous[0]);
      remainder -= Number(previous[1]);
      if (remainder < 0) { seconds--; remainder += 1000000000; }
    }
    return [seconds, remainder];
  };
  hrtime.bigint = () => BigInt(Date.now() - processStart) * 1000000n;
  const processStream = (fd, writer) => ({
    fd,
    isTTY: globalThis.__thaw_process_is_tty(fd) ? true : undefined,
    write(value, encoding, callback) {
      if (typeof encoding === 'function') callback = encoding;
      writer(String(value));
      if (typeof callback === 'function') queueMicrotask(callback);
      return true;
    },
    on() { return this; },
    once() { return this; },
    off() { return this; },
    removeListener() { return this; },
    setEncoding() { return this; },
    ref() { return this; },
    unref() { return this; }
  });
  const stdinListeners = new Map();
  let stdinEncoding;
  let stdinStarted = false;
  let stdinPaused = false;
  let stdinPending = [];
  const drainStdin = () => {
    if (stdinPaused) return;
    while (stdinPending.length && !stdinPaused) {
      const event = stdinPending.shift();
      if (event.type === 'data') {
        const value = Buffer.from(event.value, 'hex');
        const chunk = stdinEncoding ? value.toString(stdinEncoding) : value;
        for (const listener of (stdinListeners.get('data') || []).slice()) listener(chunk);
      } else {
        for (const listener of (stdinListeners.get('end') || []).slice()) listener();
      }
    }
  };
  const startStdin = () => {
    if (stdinStarted) return;
    stdinStarted = true;
    globalThis.__thaw_process_start_stdin();
  };
  const stdin = {
    fd: 0,
    isTTY: globalThis.__thaw_process_is_tty(0) ? true : undefined,
    setEncoding(encoding) { stdinEncoding = String(encoding); return this; },
    on(name, listener) {
      const key = String(name), listeners = stdinListeners.get(key) || [];
      listeners.push(listener);
      stdinListeners.set(key, listeners);
      if (key === 'data' || key === 'end') startStdin();
      return this;
    },
    once(name, listener) {
      const wrapped = value => { this.off(name, wrapped); listener(value); };
      wrapped.listener = listener;
      return this.on(name, wrapped);
    },
    off(name, listener) {
      const key = String(name), listeners = stdinListeners.get(key) || [];
      stdinListeners.set(key, listeners.filter(entry => entry !== listener && entry.listener !== listener));
      return this;
    },
    removeListener(name, listener) { return this.off(name, listener); },
    resume() { stdinPaused = false; startStdin(); nextTick(drainStdin); return this; },
    pause() { stdinPaused = true; return this; },
    ref() { return this; },
    unref() { return this; }
  };
  if (stdin.isTTY) {
    stdin.isRaw = false;
    stdin.setRawMode = enabled => {
      const raw = Boolean(enabled);
      if (!globalThis.__thaw_process_set_raw_mode(raw)) throw new Error('failed to set stdin raw mode');
      stdin.isRaw = raw;
      return stdin;
    };
  }
  const previousPlatformPoll = globalThis.__thaw_poll_platform_events;
  globalThis.__thaw_poll_platform_events = () => {
    if (previousPlatformPoll) previousPlatformPoll();
    stdinPending.push(...JSON.parse(globalThis.__thaw_process_poll_stdin()));
    drainStdin();
  };
  globalThis.__thaw_run_main_file = filename => {
    const builtinRequire = globalThis.require, cache = Object.create(null);
    const normalize = path => {
      const absolute = path.startsWith('/'), parts = [];
      path.split('/').forEach(part => {
        if (!part || part === '.') return;
        if (part === '..') parts.pop(); else parts.push(part);
      });
      return (absolute ? '/' : '') + parts.join('/');
    };
    const load = (base, request) => {
      request = String(request);
      if (!request.startsWith('./') && !request.startsWith('../') && !request.startsWith('/')) return builtinRequire(request);
      const target = normalize(request.startsWith('/') ? request : base + '/' + request);
      const candidates = /\.[^/]+$/.test(target) ? [target] : [target, target + '.js', target + '.cjs', target + '.json', target + '/index.js', target + '/index.cjs', target + '/index.json'];
      let source, resolved;
      for (const candidate of candidates) {
        try { source = globalThis.__thaw_worker_read_source(candidate); resolved = candidate; break; } catch (_) {}
      }
      if (!resolved) throw new Error("Cannot find module '" + request + "'");
      if (cache[resolved]) return cache[resolved].exports;
      const module = cache[resolved] = { exports: {} }, slash = resolved.lastIndexOf('/'), dirname = slash < 0 ? '.' : resolved.slice(0, slash);
      if (resolved.endsWith('.json')) module.exports = JSON.parse(source);
      else Function('module', 'exports', 'require', '__filename', '__dirname', source)(module, module.exports, name => load(dirname, name), resolved, dirname);
      return module.exports;
    };
    const slash = String(filename).lastIndexOf('/');
    return load(slash < 0 ? '.' : String(filename).slice(0, slash), String(filename));
  };
  Object.assign(globalThis.process, {
    argv: globalThis.process.argv || JSON.parse(globalThis.__thaw_host_argv_json || '[]'),
    env: Object.assign({}, JSON.parse(globalThis.__thaw_host_env_json || '{}'),
                       globalThis.process.env || {}),
    platform: globalThis.process.platform || hostInfo.platform || 'linux',
    arch: globalThis.process.arch || hostInfo.arch || '',
    version: globalThis.process.version || '',
    execPath: globalThis.process.execPath || JSON.parse(globalThis.__thaw_host_argv_json || '["node"]')[0],
    execArgv: globalThis.process.execArgv || JSON.parse(globalThis.__thaw_host_exec_argv_json || '[]'),
    config: globalThis.process.config || { variables: {} },
    versions: Object.assign({ node: '', modules: '', uv: '' },
                            globalThis.process.versions || {}),
    stdin: globalThis.process.stdin || stdin,
    stdout: globalThis.process.stdout || processStream(1, globalThis.__thaw_console_stdout),
    stderr: globalThis.process.stderr || processStream(2, globalThis.__thaw_console_stderr),
    cwd: () => globalThis.__thaw_process_cwd(),
    chdir: directory => globalThis.__thaw_process_chdir(String(directory)),
    exit: code => {
      const status = code === undefined ? Number(globalThis.process.exitCode || 0) : Number(code);
      globalThis.process.exitCode = status;
      processEmit('exit', status);
      globalThis.__thaw_process_exit(status);
    },
    nextTick,
    uptime: () => (Date.now() - processStart) / 1000,
    hrtime,
    memoryUsage: () => ({ rss: 0, heapTotal: 0, heapUsed: 0, external: 0, arrayBuffers: 0 }),
    cpuUsage: () => ({ user: 0, system: 0 }),
    on: (event, listener) => processOn(event, listener),
    once: (event, listener) => processOn(event, listener, true),
    off: processOff,
    removeListener: processOff,
    emit: processEmit,
    listenerCount: event => (processListeners.get(String(event)) || []).length,
    emitWarning: (warning, options) => {
      const value = warning instanceof Error ? warning : new Error(String(warning));
      value.name = options && options.type ? String(options.type) : 'Warning';
      if (options && options.code) value.code = String(options.code);
      if (!processEmit('warning', value) && globalThis.console
          && typeof globalThis.console.warn === 'function') globalThis.console.warn(value);
    },
    exitCode: globalThis.process.exitCode,
    title: globalThis.process.title || 'thaw',
    // Real Node's diagnostic report API -- only `header.
    // glibcVersionRuntime` is filled in (the one field a native-addon
    // loader's own musl-vs-glibc runtime cross-check reads, real trigger:
    // `better-sqlite3`), matching real Node's own `undefined` value on a
    // musl build.
    report: globalThis.process.report || {
      getReport: () => ({ header: { glibcVersionRuntime: hostInfo.glibcVersionRuntime } })
    }
  });

  // V8 exposes structured CallSite objects while QuickJS exposes stack
  // frames as strings. Packages using Error.prepareStackTrace (notably
  // deprecation helpers) only need this small common subset.
  if (typeof Error.captureStackTrace === 'function') {
    const captureStackTrace = Error.captureStackTrace;
    Error.captureStackTrace = (target, constructor) => {
      const prepare = Error.prepareStackTrace;
      Error.prepareStackTrace = undefined;
      captureStackTrace(target, constructor);
      const raw = target.stack;
      Error.prepareStackTrace = prepare;
      if (typeof prepare !== 'function') return;
      const lines = Array.isArray(raw) ? raw.map(String) : String(raw || '').split('\n').slice(1);
      const frames = lines.map(line => {
        const text = line.trim().replace(/^at\s+/, '');
        const match = /^(?:(.*?)\s+\()?(.+?):(\d+):(\d+)\)?$/.exec(text);
        const name = match && match[1] || null;
        const file = match && match[2] || '<anonymous>';
        const row = match ? Number(match[3]) : 0;
        const column = match ? Number(match[4]) : 0;
        return {
          getFileName: () => file,
          getLineNumber: () => row,
          getColumnNumber: () => column,
          getFunctionName: () => name,
          getMethodName: () => name,
          getThis: () => undefined,
          getTypeName: () => null,
          getEvalOrigin: () => undefined,
          isEval: () => false,
          isNative: () => false,
          toString: () => text
        };
      });
      target.stack = prepare(target, frames);
    };
  }

  if (typeof globalThis.DOMException !== 'function') {
    const legacyCodes = {
      IndexSizeError: 1, HierarchyRequestError: 3, WrongDocumentError: 4,
      InvalidCharacterError: 5, NoModificationAllowedError: 7,
      NotFoundError: 8, NotSupportedError: 9, InUseAttributeError: 10,
      InvalidStateError: 11, SyntaxError: 12, InvalidModificationError: 13,
      NamespaceError: 14, InvalidAccessError: 15, TypeMismatchError: 17,
      SecurityError: 18, NetworkError: 19, AbortError: 20,
      URLMismatchError: 21, QuotaExceededError: 22, TimeoutError: 23,
      InvalidNodeTypeError: 24, DataCloneError: 25
    };
    class DOMException extends Error {
      constructor(message = '', name = 'Error') {
        super(String(message));
        this.name = String(name);
        this.code = legacyCodes[this.name] || 0;
      }
    }
    for (const [name, code] of Object.entries(legacyCodes)) {
      const constant = name.replace(/Error$/, '')
        .replace(/([a-z])([A-Z])/g, '$1_$2').toUpperCase() + '_ERR';
      Object.defineProperty(DOMException, constant, { value: code });
      Object.defineProperty(DOMException.prototype, constant, { value: code });
    }
    globalThis.DOMException = DOMException;
  }
