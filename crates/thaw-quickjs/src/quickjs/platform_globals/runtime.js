  const thawAsyncContext = globalThis.__thaw_async_context_bootstrap;
  globalThis.global = globalThis;
  const thawDateConstructor = Date;
  const thawDateGetTime = Date.prototype.getTime;
  const thawDateApply = Reflect.apply;
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
  globalThis.__thaw_to_iterator = value => {
    const iterator =
      value != null && typeof value[Symbol.iterator] === 'function'
        ? value[Symbol.iterator]()
        : value;
    if (iterator == null || typeof iterator.next !== 'function') return iterator;
    return {
      next: () => iterator.next(),
      return: typeof iterator.return === 'function'
        ? () => iterator.return()
        : () => ({ value: undefined, done: true }),
      throw: typeof iterator.throw === 'function'
        ? error => iterator.throw(error)
        : () => ({ value: undefined, done: true }),
    };
  };
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
  const thawGraphObjectEntries = Object.entries;
  const thawGraphObjectValues = Object.values;
  const thawGraphObjectCreate = Object.create;
  const thawGraphDefineProperty = Object.defineProperty;
  const thawGraphOwnProperty = Object.prototype.hasOwnProperty;
  const thawGraphArrayIsArray = Array.isArray;
  const thawGraphArrayFrom = Array.from;
  const thawGraphArrayMap = Array.prototype.map;
  const thawGraphArrayForEach = Array.prototype.forEach;
  const thawGraphArrayEvery = Array.prototype.every;
  const thawGraphArrayPush = Array.prototype.push;
  const thawGraphMapSet = Map.prototype.set;
  const thawGraphMapGet = Map.prototype.get;
  const thawGraphSetAdd = Set.prototype.add;
  const thawGraphSetHas = Set.prototype.has;
  const thawGraphWeakMapGet = WeakMap.prototype.get;
  const thawGraphWeakMapSet = WeakMap.prototype.set;
  const thawGraphWeakMapHas = WeakMap.prototype.has;
  const thawGraphRetain = globalThis.__thaw_retain_dynamic_value;
  const thawGraphRelease = globalThis.__thaw_release_dynamic_value;
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
  // The native Json bridge sends an index-based graph only for the function
  // replacer route. Allocate every node first so back edges and sibling aliases
  // recover exactly one JS object; user keys live in pairs, outside metadata.
  const thawGraphApply = Reflect.apply;
  const thawGraphGetPrototypeOf = Reflect.getPrototypeOf;
  const thawGraphReflectOwnKeys = Reflect.ownKeys;
  const thawGraphObjectIs = Object.is;
  const thawGraphOwn = (object, key) => thawGraphApply(thawGraphOwnProperty, object, [key]);
  const thawGraphGetOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
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
  const thawNapiSymbolConstructor = Symbol;
  const thawNapiSymbolFor = Symbol.for;
  const thawNapiSymbolKeyFor = Symbol.keyFor;
  const thawNapiReflectGetOwnPropertyDescriptor = Reflect.getOwnPropertyDescriptor;
  const thawNapiReflectDefineProperty = Reflect.defineProperty;
  const thawNapiReflectGet = Reflect.get;
  const thawNapiReflectHas = Reflect.has;
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
    var __thaw_napi_symbol_values = thawNapiSafeMap(), __thaw_napi_symbol_ids = thawNapiSafeMap();
    var __thaw_napi_binary_values = thawNapiSafeMap();
    var __thaw_napi_binary_finalizers = thawNapiFinalizationRegistry ? thawNapiSafeFinalizer(function(held) { if (__thaw_napi_binary_values.get(held.id) !== held.entry) return; __thaw_napi_binary_values.delete(held.id); __thaw_napi_handle('release_handle', held.id, '', []); }) : null;
    var __thaw_napi_finalizers = thawNapiFinalizationRegistry ? thawNapiSafeFinalizer(function(id) { __thaw_napi_reference_values.delete(id); __thaw_napi_handle('release', id, '', []); }) : null;
    var __thaw_napi_proxy_finalizers = thawNapiFinalizationRegistry ? thawNapiSafeFinalizer(function(held) { if (__thaw_napi_proxies.get(held.id) !== held.entry) return; __thaw_napi_proxies.delete(held.id); __thaw_napi_handle('release_handle', held.id, '', []); }) : null;
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
    var __thaw_napi_decode = function(value, origins, path) { path = path || []; if (thawGraphArrayIsArray(origins)) origins = thawNapiSafeMap(thawNapiArrayMap(origins, function(entry) { return [thawGraphStringify(entry[0]), entry]; })); var origin = origins && origins.get(thawGraphStringify(path)); if (origin && origin[1] === 'date') return new thawGraphDateConstructor(origin[2] === null ? NaN : origin[2]); if (origin && origin[1] === 'nonfinite') return thawGraphNumber(origin[2]); var plain = origin && origin[1] === 'plain'; if (value && typeof value === 'object') { if (!plain && value['$__thaw_napi_undefined$'] === true) return undefined; if (!plain && thawGraphOwn(value, '__thaw_napi_error__')) { var ctor = value.name === 'TypeError' ? thawGraphTypeError : value.name === 'RangeError' ? thawNapiRangeErrorConstructor : thawNapiErrorConstructor, error = new ctor(value.__thaw_napi_error__); if (value.name && error.name !== value.name) error.name = value.name; return error; } if (!plain && value.__thaw_napi_symbol__) { var symbol = __thaw_napi_symbol_values.get(value.__thaw_napi_symbol__); if (!symbol) { symbol = value.global ? thawNapiSymbolFor(value.description) : thawNapiSymbolConstructor(value.description); __thaw_napi_symbol_values.set(value.__thaw_napi_symbol__, symbol); __thaw_napi_symbol_ids.set(symbol, value.__thaw_napi_symbol__); } return symbol; } if (!plain && value.__thaw_napi_ref__) { var stored = __thaw_napi_reference_values.get(value.__thaw_napi_ref__); return thawNapiDeref(stored); } if (!plain && value.__thaw_napi_handle__) return __thaw_napi_proxy(value.__thaw_napi_handle__); if (!plain && value.__thaw_napi_promise__) return __thaw_napi_result_value(value); if (!plain && value.__thaw_napi_binary__) { var id = value.__thaw_napi_binary__, stored = __thaw_napi_binary_values.get(id), existing = thawNapiDeref(stored); if (existing) return existing; var binary; if (value.kind === 'ArrayBuffer' || value.kind === 'SharedArrayBuffer') { binary = value.kind === 'SharedArrayBuffer' ? new thawNapiSharedArrayBufferConstructor(value.data.length) : new thawGraphArrayBufferConstructor(value.data.length); thawGraphApply(thawNapiUint8Set, new thawGraphUint8ArrayConstructor(binary), [value.data]); } else { var backing = __thaw_napi_decode(value.buffer, origins, thawGraphApply(thawNapiArrayConcat, path, ['buffer'])); if (value.kind === 'DataView') binary = new thawNapiDataViewConstructor(backing, value.byte_offset, value.length); else { var constructors = thawNapiTypedArrayConstructors, viewCtor = constructors[value.array_type]; if (!viewCtor) throw new thawGraphTypeError('unsupported native typed array kind'); binary = new viewCtor(backing, value.byte_offset, value.length); } } var entry = thawNapiWeakRef ? new thawNapiWeakRef(binary) : binary; __thaw_napi_binary_values.set(id, entry); __thaw_napi_handles.set(binary, id); __thaw_napi_proxy_owners.set(binary, owner); if (__thaw_napi_binary_finalizers) __thaw_napi_binary_finalizers.register(binary, { id: id, entry: entry }); return binary; } if (!plain && value.type === 'Buffer' && thawGraphArrayIsArray(value.data) && thawGraphBufferFrom) return thawGraphBufferFrom(value.data); if (thawGraphArrayIsArray(value)) return thawNapiArrayMap(value, function(child, index) { return __thaw_napi_decode(child, origins, thawGraphApply(thawNapiArrayConcat, path, [index])); }); var decoded = {}; thawNapiArrayEach(thawGraphObjectKeys(value), function(key) { thawGraphDefineProperty(decoded, key, { value: __thaw_napi_decode(value[key], origins, thawGraphApply(thawNapiArrayConcat, path, [key])), enumerable: true, configurable: true, writable: true }); }); return decoded; } return value; };
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
    var __thaw_napi_symbol_id = function(handle, symbol) { var id = __thaw_napi_symbol_ids.get(symbol), global = thawNapiSymbolKeyFor(symbol), created = __thaw_napi_handle('symbol', handle, global === undefined ? symbol.description || '' : global, [global !== undefined, id || null]); id = created.value; __thaw_napi_symbol_ids.set(symbol, id); __thaw_napi_symbol_values.set(id, symbol); return id; };
    var __thaw_napi_key = function(handle, name, operation, args) { return typeof name === 'symbol' ? __thaw_napi_handle(operation + '_symbol', handle, thawGraphString(__thaw_napi_symbol_id(handle, name)), args) : __thaw_napi_handle(operation, handle, thawGraphString(name), args); };
    var __thaw_napi_proxy = function(handle, prototype) { var stored = __thaw_napi_proxies.get(thawGraphString(handle)), existing = thawNapiDeref(stored); if (existing) return existing; __thaw_napi_handle('renew_handle', handle, '', []); var target = thawGraphObjectCreate(prototype || thawNapiObjectPrototype), methods = thawNapiSafeMap();
    var read = function(name) { var result = __thaw_napi_key(handle, name, 'get', []); if (result.kind !== 'method') return result.value; var method = methods.get(result.value); if (!method) { var captured = result.value; method = function() { var args = thawGraphApply(thawNapiArraySlice, arguments, []), value = __thaw_napi_handle('call_captured', captured, thawGraphString(__thaw_napi_handles.get(proxy)), args).value; __thaw_napi_sync_arguments(args); return value; }; methods.set(captured, method); } return method; };
    var descriptor = function(name) { var meta = __thaw_napi_key(handle, name, 'descriptor', []).value; if (!meta) return undefined; var desc = meta.accessor ? { configurable: meta.configurable, enumerable: meta.enumerable, get: function() { return read(name); }, set: meta.setter ? function(value) { __thaw_napi_key(handle, name, 'set', [value]); } : undefined } : { configurable: meta.configurable, enumerable: meta.enumerable, writable: meta.writable, value: read(name) }; if (!meta.configurable) { var own = thawNapiReflectGetOwnPropertyDescriptor(target, name); if (!own) thawNapiReflectDefineProperty(target, name, desc); else if (!meta.accessor && own.writable) thawNapiReflectDefineProperty(target, name, { value: desc.value }); return thawNapiReflectGetOwnPropertyDescriptor(target, name); } return desc; };
    var proxy = new thawNapiProxyConstructor(target, {
    get: function(_, name, receiver) { if (!__thaw_napi_key(handle, name, 'has', []).value) return thawNapiReflectGet(_, name, receiver); var own = thawNapiReflectGetOwnPropertyDescriptor(_, name); if (own && !own.configurable && !own.writable && thawGraphOwn(own, 'value')) return own.value; return read(name); },
    set: function(_, name, value) { __thaw_napi_key(handle, name, 'set', [value]); __thaw_napi_sync_arguments([value]); var own = thawNapiReflectGetOwnPropertyDescriptor(_, name); if (own && !own.configurable && own.writable) thawNapiReflectDefineProperty(_, name, { value: read(name) }); return true; },
    has: function(_, name) { return thawNapiReflectHas(_, name) || __thaw_napi_key(handle, name, 'has', []).value; },
    ownKeys: function(_) { var keys = __thaw_napi_handle('own_keys', handle, '', []).value; thawNapiArrayEach(thawGraphReflectOwnKeys(_), function(key) { if (thawGraphApply(thawNapiArrayIndexOf, keys, [key]) < 0) thawNapiArrayAppend(keys, key); }); thawNapiArrayEach(keys, function(key) { descriptor(key); }); return keys; },
    getOwnPropertyDescriptor: function(_, name) { return descriptor(name) || thawNapiReflectGetOwnPropertyDescriptor(_, name); },
    preventExtensions: function() { return false; }
    }); __thaw_napi_handles.set(proxy, handle); __thaw_napi_proxy_owners.set(proxy, owner); var entry = thawNapiWeakRef ? new thawNapiWeakRef(proxy) : proxy, id = thawGraphString(handle); __thaw_napi_proxies.set(id, entry); if (__thaw_napi_proxy_finalizers) __thaw_napi_proxy_finalizers.register(proxy, { id: id, entry: entry }); return proxy; };
    const state = {
      proxy: __thaw_napi_proxy,
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
  const thawGraphEncode = (root, handleMask = 0, live = false, plainResult = false) => {
    const handleCarriers = new thawGraphWeakMapConstructor();
    const retained = [];
    const retain = value => {
      const handle = thawGraphRetain(value);
      thawGraphApply(thawGraphArrayPush, retained, [handle]);
      return handle;
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
    const token = (value, key = '') => {
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
      if (live && typeof value === 'symbol') {
        const id = sources.length;
        thawGraphApply(thawGraphArrayPush, sources, [value]);
        thawGraphApply(thawGraphSetAdd, livePrimitives, [id]);
        return { r: id };
      }
      if (value === undefined) return { u: 1 };
      if (typeof value === 'number' && !thawGraphNumberIsFinite(value))
        return { nf: thawGraphNumberIsNaN(value) ? 'NaN' : value > 0 ? 'Infinity' : '-Infinity' };
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
        thawGraphApply(thawGraphArrayPush, nodes, [{ hdl: retain(value) }]);
      } else if (thawGraphApply(thawGraphWeakMapHas, handleCarriers, [value])) {
        thawGraphApply(thawGraphArrayPush, nodes, [{ hdl: thawGraphApply(thawGraphWeakMapGet, handleCarriers, [value]) }]);
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
        const entries = [];
        for (let slot = 0; slot < value.length; slot++)
          thawGraphApply(thawGraphArrayPush, entries, [thawGraphOwn(value, slot)
            ? token(value[slot], thawGraphString(slot)) : { h: 1 }]);
        thawGraphApply(thawGraphArrayPush, nodes, [{ a: entries }]);
      } else {
        thawGraphApply(thawGraphArrayPush, nodes, [{ o: thawGraphApply(thawGraphArrayMap, thawGraphObjectKeys(value), [key => [key, token(value[key], key)]]) }]);
      }
    }
    return thawGraphStringify({ root: rootToken, nodes, leases: retained });
    } catch (error) {
      for (let index = 0; index < retained.length; index++)
        thawGraphRelease(retained[index]);
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
  const thawGraphDecode = graph => {
    if (!graph || typeof graph !== 'object' || !thawGraphArrayIsArray(graph.nodes)
      || !thawGraphOwn(graph, 'root'))
      throw new thawGraphTypeError('Invalid native JSON graph');
    const own = (value, key) => thawGraphOwn(value, key);
    const nodes = thawGraphApply(thawGraphArrayMap, graph.nodes, [node => {
      if (!node || typeof node !== 'object' || thawGraphArrayIsArray(node) || thawGraphObjectKeys(node).length !== 1)
        throw new thawGraphTypeError('Invalid native JSON graph node');
      if (own(node, 'a') && thawGraphArrayIsArray(node.a)) return new thawGraphArrayConstructor(node.a.length);
      if (own(node, 'o') && thawGraphArrayIsArray(node.o)) return {};
      if (own(node, 'd') && (node.d === null || typeof node.d === 'number'))
        return new thawGraphDateConstructor(node.d === null ? NaN : node.d);
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
      if (keys.length !== 1) throw new thawGraphTypeError('Invalid native JSON graph token');
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
      if (own(token, 'bi') && typeof token.bi === 'string'
        && thawGraphApply(thawGraphRegExpTest, thawGraphBigIntDecimalPattern, [token.bi]))
        return thawGraphBigInt(token.bi);
      if (own(token, 'h') && token.h === 1) return undefined;
      if (own(token, 'nf') && (token.nf === 'NaN' || token.nf === 'Infinity' || token.nf === '-Infinity'))
        return thawGraphNumber(token.nf);
      throw new thawGraphTypeError('Invalid native JSON graph token');
    };
    thawGraphApply(thawGraphArrayForEach, graph.nodes, [(node, index) => {
      if (own(node, 'a')) {
        thawGraphApply(thawGraphArrayForEach, node.a, [(token, slot) => {
          if (!token || typeof token !== 'object' || token.h !== 1 || thawGraphObjectKeys(token).length !== 1)
            nodes[index][slot] = decode(token);
        }]);
      } else if (own(node, 'o')) {
        thawGraphApply(thawGraphArrayForEach, node.o, [pair => {
          if (!thawGraphArrayIsArray(pair) || pair.length !== 2 || typeof pair[0] !== 'string')
            throw new thawGraphTypeError('Invalid native JSON graph property');
          thawGraphDefineProperty(nodes[index], pair[0], {
            value: decode(pair[1]), writable: true, enumerable: true, configurable: true,
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
  globalThis.__thaw_object_with_native_getters = (keys, readable, ...getters) => {
    const object = {};
    for (let i = 0; i < keys.length; i++) {
      Object.defineProperty(object, keys[i], {
        get: readable[i] ? getters[i] : undefined,
        enumerable: true,
        configurable: true,
      });
    }
    return object;
  };
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
  globalThis.__thaw_accessor_descriptor = (readable, writable, getter, setter) => ({
    get: readable ? getter : undefined,
    set: writable ? setter : undefined,
    enumerable: true,
    configurable: true,
  });
  globalThis.__thaw_data_descriptor = value => ({
    value,
    writable: true,
    enumerable: true,
    configurable: true,
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
  const normalizeDelay = value => {
    const number = Number(value);
    if (!Number.isFinite(number) || number < 0) return 0;
    return Math.min(Math.trunc(number), 2147483647);
  };
  const timerHandle = id => ({
    ref() { const timer = timers.get(id); if (timer) timer.refed = true; return this; },
    unref() { const timer = timers.get(id); if (timer) timer.refed = false; return this; },
    hasRef() { const timer = timers.get(id); return Boolean(timer && timer.refed); },
    refresh() { const timer = timers.get(id); if (timer) timer.due = Date.now() + timer.milliseconds; return this; },
    close() { timers.delete(id); },
    [Symbol.toPrimitive]() { return id; }
  });
  const schedule = (callback, delay, repeat, args, refed = true) => {
    if (typeof callback !== 'function') {
      throw new TypeError('timer callback must be a function');
    }
    const id = nextTimerId++;
    const milliseconds = normalizeDelay(delay);
    timers.set(id, { callback, args, repeat, milliseconds, refed,
                     asyncContext: thawAsyncContext.get(),
                     due: Date.now() + milliseconds });
    return timerHandle(id);
  };
  globalThis.setTimeout = (callback, delay = 0, ...args) =>
    schedule(callback, delay, false, args);
  globalThis.clearTimeout = id => { timers.delete(Number(id)); };
  globalThis.setInterval = (callback, delay = 0, ...args) =>
    schedule(callback, delay, true, args);
  globalThis.clearInterval = globalThis.clearTimeout;
  globalThis.setImmediate = (callback, ...args) =>
    schedule(callback, 0, false, args);
  globalThis.clearImmediate = globalThis.clearTimeout;
  globalThis.__thaw_set_timeout_ref = (callback, delay, refed) =>
    schedule(callback, delay, false, [], Boolean(refed));
  globalThis.__thaw_set_timer_ref = (id, refed) => {
    const timer = timers.get(Number(id));
    if (timer) timer.refed = Boolean(refed);
  };
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
