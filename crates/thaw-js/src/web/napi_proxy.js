// The native addon proxy layer (plan tj107 sections 2 and 4, PRs 6b-1, 6b-2 and 6b-4a): the export listing, the owner
// lookup, and the object and array proxies of native handles. napi_proxy.rs runs this file as a script; its
// completion value is the factory below, called once per realm with `cap`, the private natives (exports, owner,
// invoker). Object, array and function proxies of the native handles (PR 6b-2 and 6b-3).
(function (cap) {
  'use strict';
  // The intrinsics the proxies use, captured before user code runs. ponytail: the Reflect and Object methods are
  // captured as functions, so a later replacement of the owning object does not change them.
  const intrinsicObject = Object;
  const reflectConstruct = Reflect.construct;
  const reflectGet = Reflect.get;
  const reflectSet = Reflect.set;
  const reflectHas = Reflect.has;
  const reflectDefine = Reflect.defineProperty;
  const reflectDelete = Reflect.deleteProperty;
  const reflectOwnKeys = Reflect.ownKeys;
  const reflectGetOwn = Reflect.getOwnPropertyDescriptor;
  const reflectGetProto = Reflect.getPrototypeOf;
  const reflectSetProto = Reflect.setPrototypeOf;
  const reflectIsExt = Reflect.isExtensible;
  const reflectPreventExt = Reflect.preventExtensions;
  const objectCreate = Object.create;
  const hasOwnProperty = Object.prototype.hasOwnProperty;
  const isArrayOf = Array.isArray;
  const jsonParse = JSON.parse;
  const jsonStringify = JSON.stringify;
  const decodeGraph = __thaw_json_graph_decode;
  const decodeOwned = __thaw_json_graph_decode_owned;
  const encodeGraph = __thaw_json_graph_encode_js;
  const bridgeHandle = __thaw_napi_bridge_handle;
  const tagNative = __thaw_napi_graph_tag;
  const positiveDecimal = /^[1-9][0-9]*$/;

  const hasOwn = (object, key) => hasOwnProperty.call(object, key);

  const exports = () => jsonParse(cap.exports());

  // RJ runtime.js each_export: qualified packages list only their own prefixed names.
  const eachExport = (info, prefix, packageName, install) => {
    const qualified = info.qualifiedPackages.indexOf(packageName) !== -1;
    info.names.forEach(name => {
      if (qualified) {
        if (!name.startsWith(prefix)) return;
        install(name, name.slice(prefix.length));
      } else if (name.indexOf('::') === -1) {
        install(name, name);
      }
    });
  };

  const ownerOf = handle => cap.owner(handle);

  // A parsed bridge reply, with its exception graph thrown and its error raised (RJ bridgeReply).
  const checkReply = (response, message) => {
    if (response && hasOwn(response, '__thaw_exception_graph__')) {
      if (typeof response.__thaw_exception_graph__ !== 'string')
        throw new TypeError('Invalid native exception graph');
      throw decodeOwned(response.__thaw_exception_graph__);
    }
    if (!response || hasOwn(response, '__thaw_error__'))
      throw new TypeError(response && response.__thaw_error__ ? response.__thaw_error__ : message);
    return response;
  };

  // One bridge operation's reply.
  const bridgeReply = (operation, handle, packet, message) =>
    checkReply(jsonParse(bridgeHandle(operation, handle, '', packet)), message);

  // The value of a bridge reply: a graph is decoded (and its leases released), anything else is its value.
  const replyValue = (operation, handle, packet, message) => {
    const response = bridgeReply(operation, handle, packet, message);
    return response.kind === 'graph' && typeof response.value === 'string'
      ? decodeOwned(response.value) : response.value;
  };

  // A foreign object is passed by value; a native-handle proxy of this layer travels as its handle.
  const handleOf = new WeakMap();
  const foreignReceiver = value => value !== null
    && (typeof value === 'object' || typeof value === 'function')
    && !handleOf.has(value) ? value : undefined;

  const graphProperty = (handle, operation, values) => {
    const receiver = operation === 'get_graph' ? values[1]
      : operation === 'set_graph' ? values[2] : undefined;
    const packet = encodeGraph(values, 0, true, false, foreignReceiver(receiver));
    return replyValue(operation, handle, packet, 'Native property operation failed');
  };

  // Native handles are never released: a proxy lives until the process ends (ponytail: FinalizationRegistry and
  // WeakRef hold strongly in thaw-js, so a handle->proxy cache keyed by the handle is the whole lifetime model).
  const proxies = new Map();

  const preventNativeExtensions = (handle, proxy, target) => {
    const keys = graphProperty(handle, 'own_keys_graph', []);
    if (!isArrayOf(keys)) throw new TypeError('Invalid native integrity key list');
    // A nonextensible target must hold every native own key first, while it is still extensible.
    for (let index = 0; index < keys.length; index++) {
      const descriptor = reflectGetOwn(proxy, keys[index]);
      if (descriptor && !reflectDefine(target, keys[index], descriptor)) return false;
    }
    const prototype = reflectGetProto(proxy);
    if (!reflectSetProto(target, prototype)) return false;
    if (graphProperty(handle, 'prevent_extensions_graph', []) !== true) return false;
    return reflectPreventExt(target);
  };

  const defineNativeProperty = (handle, target, key, input) => {
    const own = reflectGetOwn(target, key);
    if (!own && !reflectIsExt(target)) return false;
    if (own && !own.configurable) {
      // Let the engine apply the standard compatibility rules before any native mutation.
      const probe = objectCreate(null);
      if (!reflectDefine(probe, key, own) || !reflectDefine(probe, key, input)) return false;
      // An identical redefinition of a nonconfigurable accessor is a no-op: its shadow functions stay.
      if (hasOwn(own, 'get')) {
        const updated = reflectGetOwn(probe, key);
        if (updated.get === own.get && updated.set === own.set
          && updated.enumerable === own.enumerable
          && updated.configurable === own.configurable) return true;
      }
    }
    const has = field => reflectHas(input, field);
    const get = field => reflectGet(input, field);
    const previous = bridgeReply('descriptor_graph', handle,
      encodeGraph([key], 0, true), 'Native descriptor lookup failed').value;
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
      throw new TypeError('Invalid property descriptor');
    if ((getter !== undefined && typeof getter !== 'function')
      || (setter !== undefined && typeof setter !== 'function'))
      throw new TypeError('Invalid accessor callback');
    const accessor = hasGetter || hasSetter;
    const flags = (writable ? 1 : 0) | (enumerable ? 2 : 0) | (configurable ? 4 : 0);
    const operation = accessor ? 'define_accessor_graph'
      : !hasValue && !hasWritable && previous ? 'reconfigure_graph'
      : 'define_data_graph';
    const packet = encodeGraph(operation === 'define_accessor_graph'
      ? [key, getter, setter, flags, (hasGetter ? 1 : 0) | (hasSetter ? 2 : 0)]
      : operation === 'reconfigure_graph' ? [key, flags]
      : [key, value, flags, hasValue ? 1 : 0], 0, true);
    const result = bridgeReply(operation, handle, packet, 'Native descriptor definition failed');
    if (result.value !== true) return false;
    if (own && !own.configurable) {
      if (!reflectDefine(target, key, input))
        throw new TypeError('Cannot synchronize native descriptor');
    } else if (!configurable) {
      // An attribute-only update keeps an accessor an accessor. The target is only a shadow for the
      // Proxy invariants; the native descriptor stays the authority for reads and writes.
      const reflectedAccessor = accessor
        || (operation === 'reconfigure_graph' && previous && previous.accessor);
      const readNativeValue = receiver => replyValue('get_graph', handle,
        encodeGraph([key, receiver], 0, true, false, foreignReceiver(receiver)),
        'Native accessor read failed');
      const reflectedGet = reflectedAccessor
        ? accessor && hasGetter ? getter
          : previous && previous.getter
            ? function () { return readNativeValue(this); } : undefined
        : undefined;
      const reflectedSet = accessor && hasSetter ? setter
        : previous && previous.setter
          ? function (next) {
            bridgeReply('set_graph', handle,
              encodeGraph([key, next, this], 0, true, false, foreignReceiver(this)),
              'Native accessor write failed');
          } : undefined;
      const reflected = reflectedAccessor
        ? { get: reflectedGet, set: reflectedSet, enumerable, configurable }
        : { value: operation === 'reconfigure_graph' && previous
            ? readNativeValue() : value, writable, enumerable, configurable };
      if (!reflectDefine(target, key, reflected))
        throw new TypeError('Cannot materialize native descriptor');
    }
    return true;
  };

  // The object or array proxy of the native handle `handle` (a decimal string). Its target is `[]` for an array
  // and an object with `prototype` otherwise; every trap reads or writes the native value through the bridge.
  const proxyForHandle = (handle, prototype) => {
    if (typeof handle !== 'string' || !positiveDecimal.test(handle))
      throw new TypeError('Invalid native addon graph handle');
    const owner = ownerOf(handle);
    if (typeof owner !== 'string' || !positiveDecimal.test(owner))
      throw new TypeError('Invalid native addon graph owner');
    const existing = proxies.get(handle);
    if (existing !== undefined) return existing;
    const arrayKind = jsonParse(bridgeHandle('array_kind_graph', handle, '', '[]'));
    if (!arrayKind || typeof arrayKind.value !== 'boolean')
      throw new TypeError('Invalid native array kind');
    const isArray = arrayKind.value;
    const target = isArray ? [] : objectCreate(prototype || Object.prototype);
    const read = (name, receiver) => graphProperty(handle, 'get_graph', [name, receiver]);

    const syncArrayLength = () => {
      if (!isArray) return;
      const length = read('length');
      if (!Number.isSafeInteger(length) || length < 0 || length > 4294967295
        || !reflectDefine(target, 'length', { value: length }))
        throw new TypeError('Cannot synchronize native array length');
      const nativeDescriptor = graphProperty(handle, 'descriptor_graph', ['length']);
      const targetDescriptor = reflectGetOwn(target, 'length');
      if (!nativeDescriptor || nativeDescriptor.accessor
        || nativeDescriptor.configurable || nativeDescriptor.enumerable
        || typeof nativeDescriptor.writable !== 'boolean'
        || !targetDescriptor || (nativeDescriptor.writable && !targetDescriptor.writable)
        || (!nativeDescriptor.writable && targetDescriptor.writable
          && !reflectDefine(target, 'length', { writable: false })))
        throw new TypeError('Cannot synchronize native array length attributes');
    };

    // A native accessor keeps one getter and setter identity across descriptor reads: a nonconfigurable one
    // materialized on the target may not change them later.
    const fallbackGetters = new Map();
    const fallbackSetters = new Map();
    const sourceFunctions = new Map();
    const sourceFunction = source => {
      const cacheKey = jsonStringify(source);
      const cached = sourceFunctions.get(cacheKey);
      if (cached !== undefined) return cached;
      const decoded = decodeGraph({ root: { r: 0 }, nodes: [source] });
      if (typeof decoded === 'function') sourceFunctions.set(cacheKey, decoded);
      return decoded;
    };
    const descriptor = name => {
      if (isArray && name === 'length') {
        syncArrayLength();
        return reflectGetOwn(target, 'length');
      }
      const meta = graphProperty(handle, 'descriptor_graph', [name]);
      if (!meta) {
        const stale = reflectGetOwn(target, name);
        if (stale && stale.configurable) reflectDelete(target, name);
        fallbackGetters.delete(name);
        fallbackSetters.delete(name);
        return undefined;
      }
      let getter = meta.getter_source ? sourceFunction(meta.getter_source) : undefined;
      let setter = meta.setter_source ? sourceFunction(meta.setter_source) : undefined;
      if (meta.accessor && meta.getter && !getter) {
        getter = fallbackGetters.get(name);
        if (!getter) {
          getter = function () { return read(name, this); };
          fallbackGetters.set(name, getter);
        }
      }
      if (meta.accessor && meta.setter && !setter) {
        setter = fallbackSetters.get(name);
        if (!setter) {
          setter = function (next) { graphProperty(handle, 'set_graph', [name, next, this]); };
          fallbackSetters.set(name, setter);
        }
      }
      if (meta.accessor) {
        const current = reflectGetOwn(target, name);
        if (current && !current.configurable && !hasOwn(current, 'value')) {
          getter = meta.getter ? current.get : undefined;
          setter = meta.setter ? current.set : undefined;
        }
      }
      const desc = meta.accessor
        ? { configurable: !!meta.configurable, enumerable: !!meta.enumerable, get: getter, set: setter }
        : { configurable: !!meta.configurable, enumerable: !!meta.enumerable,
          writable: !!meta.writable, value: read(name) };
      if (!meta.configurable) {
        if (!reflectDefine(target, name, desc))
          throw new TypeError('Cannot materialize native descriptor');
        return reflectGetOwn(target, name);
      }
      return desc;
    };

    const proxy = new Proxy(target, {
      get(shadow, name, receiver) {
        if (isArray && name === 'length') syncArrayLength();
        if (!graphProperty(handle, 'has_graph', [name])) {
          descriptor(name);
          return reflectGet(shadow, name, receiver);
        }
        const own = reflectGetOwn(shadow, name);
        if (own && !own.configurable && !own.writable && hasOwn(own, 'value')) return own.value;
        if (own && !own.configurable && hasOwn(own, 'get') && own.get === undefined) return undefined;
        return read(name, receiver);
      },
      set(shadow, name, value, receiver) {
        const own = reflectGetOwn(shadow, name);
        if (own && !own.configurable && ((hasOwn(own, 'value') && !own.writable)
          || (hasOwn(own, 'set') && own.set === undefined))) return false;
        if (graphProperty(handle, 'set_graph', [name, value, receiver]) !== true) return false;
        if (isArray) syncArrayLength();
        if (own && !own.configurable && hasOwn(own, 'value') && own.writable)
          reflectDefine(shadow, name, { value: read(name) });
        return true;
      },
      has(shadow, name) {
        const native = graphProperty(handle, 'has_graph', [name]);
        if (!native) descriptor(name);
        return native || reflectHas(shadow, name);
      },
      ownKeys(shadow) {
        if (isArray) syncArrayLength();
        const keys = graphProperty(handle, 'own_keys_graph', []);
        if (!isArrayOf(keys)) throw new TypeError('Invalid native ownKeys graph');
        reflectOwnKeys(shadow).forEach(key => {
          if (keys.indexOf(key) < 0) {
            const stale = reflectGetOwn(shadow, key);
            if (stale && stale.configurable) {
              reflectDelete(shadow, key);
              fallbackGetters.delete(key);
              fallbackSetters.delete(key);
            } else {
              keys.push(key);
            }
          }
        });
        keys.forEach(key => { descriptor(key); });
        return keys;
      },
      getOwnPropertyDescriptor(shadow, name) {
        return descriptor(name) || reflectGetOwn(shadow, name);
      },
      defineProperty(shadow, key, description) {
        if (!defineNativeProperty(handle, shadow, key, description)) return false;
        if (isArray) syncArrayLength();
        return true;
      },
      deleteProperty(shadow, key) {
        const own = reflectGetOwn(shadow, key);
        if (own && !own.configurable) return false;
        if (graphProperty(handle, 'delete_graph', [key]) !== true) return false;
        return reflectDelete(shadow, key);
      },
      getPrototypeOf(shadow) {
        const native = graphProperty(handle, 'get_prototype_graph', []);
        return native === undefined ? reflectGetProto(shadow) : native;
      },
      setPrototypeOf(shadow, next) {
        if (!reflectIsExt(shadow)) return reflectGetProto(shadow) === next;
        const previous = reflectGetProto(shadow);
        if (!reflectSetProto(shadow, next)) return false;
        try {
          if (graphProperty(handle, 'set_prototype_graph', [next]) === true) return true;
        } catch (error) {
          reflectSetProto(shadow, previous);
          throw error;
        }
        reflectSetProto(shadow, previous);
        return false;
      },
      isExtensible(shadow) {
        if (graphProperty(handle, 'is_extensible_graph', []) === false && reflectIsExt(shadow)
          && !preventNativeExtensions(handle, proxy, shadow))
          throw new TypeError('Cannot synchronize native integrity');
        return reflectIsExt(shadow);
      },
      preventExtensions(shadow) {
        return preventNativeExtensions(handle, proxy, shadow);
      },
    });
    handleOf.set(proxy, handle);
    tagNative(proxy, 2, handle);
    bridgeReply('renew_handle', handle, '[]', 'Native handle renewal failed');
    proxies.set(handle, proxy);
    return proxy;
  };

  // ---- Plain typed ABI, structured part (PR 6b-4a; RJ runtime.js 1049-1096). Binary values and promises are
  // PR 6b-4b and throw a TypeError until then. Symbols are a per-realm Map, not a native registry (ponytail).
  const referenceIds = new WeakMap();
  const referenceValues = new Map();
  const symbolValues = new Map();
  const binaryMarkers = ['__thaw_napi_buffer__', '__thaw_napi_arraybuffer__', '__thaw_napi_view__',
    '__thaw_napi_binary__', '__thaw_napi_promise__'];
  let lastReference = 0;

  // A handle marker is only sent for an object whose native owner is one of this realm's export owners.
  const ownsHandle = handle => {
    const owners = exports().ownerByExportName || {};
    const owner = ownerOf(handle);
    return Object.keys(owners).some(name => owners[name] === owner);
  };

  const snapshotOf = (value, active) => {
    const properties = {};
    Object.keys(value).forEach(key => {
      Object.defineProperty(properties, key, {
        value: argument(value[key], active), enumerable: true, configurable: true, writable: true,
      });
    });
    return properties;
  };

  // The `__thaw_napi_reference_<id>` global the native side calls: its first argument is the metadata
  // (`true` with a receiver, or `{__thaw_napi_argument_handles__, __thaw_napi_this_handle__}`), then the arguments.
  const defineCallback = (id, target) => {
    Object.defineProperty(globalThis, '__thaw_napi_reference_' + id, {
      value: function () {
        const args = Array.prototype.slice.call(arguments);
        const meta = args.shift();
        let receiver;
        if (meta && meta.__thaw_napi_argument_handles__) {
          meta.__thaw_napi_argument_handles__.forEach(entry => {
            const path = entry[0];
            let parent = args;
            for (let i = 0; i + 1 < path.length; i++) {
              const own = Reflect.getOwnPropertyDescriptor(parent, path[i]);
              if (!own) throw new TypeError('invalid native callback path');
              parent = own.value;
            }
            Object.defineProperty(parent, path[path.length - 1], {
              value: proxyForHandle(entry[1]), writable: true, enumerable: true, configurable: true,
            });
          });
        }
        if (meta === true) receiver = args.shift();
        if (meta && meta.__thaw_napi_this_handle__) {
          const stored = proxies.get(meta.__thaw_napi_this_handle__);
          receiver = stored !== undefined ? stored : proxyForHandle(meta.__thaw_napi_this_handle__);
        }
        const returned = Reflect.apply(target, receiver, args);
        return typeof returned === 'function' ? argument(returned) : returned;
      },
      writable: false, configurable: false,
    });
  };

  // The wire form of one JS value passed to native code (RJ __thaw_napi_argument). `active` holds the objects being
  // encoded, so a cycle becomes a reference marker.
  const argument = (value, active) => {
    if (value === null || (typeof value !== 'function' && typeof value !== 'object')) return value;
    if (handleOf.has(value)) {
      const handle = handleOf.get(value);
      if (!ownsHandle(handle)) throw new TypeError('native object belongs to another addon');
      return { __thaw_napi_handle__: handle };
    }
    if (ArrayBuffer.isView(value) || value instanceof ArrayBuffer)
      throw new TypeError('native binary values are not implemented yet');
    active = active || new WeakSet();
    let id = referenceIds.get(value);
    if (id !== undefined && active.has(value)) return { __thaw_napi_ref__: id };
    const fresh = id === undefined;
    if (fresh) {
      lastReference += 1;
      id = lastReference;
      referenceIds.set(value, id);
      referenceValues.set(id, value);
    }
    if (typeof value === 'function') {
      if (fresh) defineCallback(id, value);
      return { __thaw_napi_function__: id };
    }
    active.add(value);
    const encoded = Array.isArray(value)
      ? { __thaw_napi_array__: id, value: value.map(item => argument(item, active)) }
      : { __thaw_napi_object__: id, value: snapshotOf(value, active) };
    active.delete(value);
    return encoded;
  };

  const argumentsOf = args => {
    const active = new WeakSet();
    return args.map(value => argument(value, active));
  };

  // The JS value of one wire value (RJ __thaw_napi_decode). `origins` lists the paths that need a type the JSON
  // cannot carry (dates, non-finite numbers, plain values); `path` is this value's path.
  const decodeValue = (value, origins, path) => {
    path = path || [];
    if (Array.isArray(origins)) origins = new Map(origins.map(entry => [jsonStringify(entry[0]), entry]));
    const origin = origins ? origins.get(jsonStringify(path)) : undefined;
    if (origin && origin[1] === 'date') return new Date(origin[2] === null ? NaN : origin[2]);
    if (origin && origin[1] === 'nonfinite') return Number(origin[2]);
    const plain = !!origin && origin[1] === 'plain';
    if (value && typeof value === 'object') {
      if (!plain && value['$__thaw_napi_undefined$'] === true) return undefined;
      if (!plain && hasOwn(value, '__thaw_napi_error__')) {
        const Constructor = value.name === 'TypeError' ? TypeError
          : value.name === 'RangeError' ? RangeError : Error;
        const error = new Constructor(value.__thaw_napi_error__);
        if (value.name && error.name !== value.name) error.name = value.name;
        return error;
      }
      if (!plain && value.__thaw_napi_symbol__) {
        const id = value.__thaw_napi_symbol__;
        if (!symbolValues.has(id)) {
          symbolValues.set(id, value.global ? Symbol.for(value.description) : Symbol(value.description));
        }
        return symbolValues.get(id);
      }
      if (!plain && value.__thaw_napi_ref__) return referenceValues.get(value.__thaw_napi_ref__);
      if (!plain && value.__thaw_napi_handle__) return proxyForHandle(value.__thaw_napi_handle__);
      if (!plain && binaryMarkers.some(key => hasOwn(value, key)))
        throw new TypeError('native binary and promise values are not implemented yet');
      if (Array.isArray(value)) return value.map((child, index) => decodeValue(child, origins, path.concat([index])));
      const decoded = {};
      Object.keys(value).forEach(key => {
        Object.defineProperty(decoded, key, {
          value: decodeValue(value[key], origins, path.concat([key])),
          enumerable: true, configurable: true, writable: true,
        });
      });
      return decoded;
    }
    return value;
  };

  // One bridge operation with its arguments encoded and its reply decoded (RJ __thaw_napi_handle). `sync_reference`
  // keeps its raw reply: the caller reads its origins itself.
  const handle = (operation, target, name, args) => {
    const result = jsonParse(bridgeHandle(operation, String(target), name || '',
      jsonStringify(argumentsOf(args || []))));
    if (result && hasOwn(result, '__thaw_error__')) throw new Error(result.__thaw_error__);
    if (result && hasOwn(result, 'value') && result.kind !== 'method' && operation !== 'sync_reference')
      result.value = decodeValue(result.value, result.origins);
    return result;
  };

  // After a call, the native side's values replace the JS arguments in place (RJ __thaw_napi_sync_arguments).
  const syncArguments = args => {
    const seen = new WeakSet();
    const sync = value => {
      if (value === null || typeof value !== 'object' || seen.has(value)) return;
      seen.add(value);
      const proxyHandle = handleOf.get(value);
      const id = referenceIds.get(value);
      if (proxyHandle !== undefined && !ownsHandle(proxyHandle))
        throw new TypeError('native object belongs to another addon');
      if (proxyHandle === undefined && id === undefined) return;
      const response = proxyHandle !== undefined
        ? handle('sync_handle', proxyHandle, '', [])
        : handle('sync_reference', id, '', []);
      if (proxyHandle !== undefined) return;
      const updated = response.value;
      const originMap = new Map((response.origins || []).map(entry => [jsonStringify(entry[0]), entry]));
      if (Array.isArray(value)) value.length = updated.length;
      else {
        Object.keys(value).forEach(key => {
          if (!hasOwn(updated, key)) {
            try { delete value[key]; } catch (error) { /* A non-configurable property stays. */ }
          }
        });
      }
      Object.keys(updated).forEach(key => {
        const next = decodeValue(updated[key], originMap, [Array.isArray(updated) ? Number(key) : key]);
        if (key === '__proto__') {
          Object.defineProperty(value, key, { value: next, writable: true, enumerable: true, configurable: true });
        } else if (value[key] !== next) {
          try { value[key] = next; } catch (error) { /* A getter-only property keeps its value. */ }
        }
        if (next && typeof next === 'object') sync(next);
      });
    };
    args.forEach(sync);
  };

  // The function proxy of the native Function handle `handle`. Its target is a plain function without `prototype`
  // (ponytail: thaw-js enforces no Proxy invariants, so the bound target of the QuickJS runtime is not needed).
  // apply and construct send the captured graph to the native Function through the invoker.
  const functions = new Map();

  const functionForHandle = handle => {
    if (typeof handle !== 'string' || !positiveDecimal.test(handle))
      throw new TypeError('Invalid native addon graph handle');
    const owner = ownerOf(handle);
    if (typeof owner !== 'string' || !positiveDecimal.test(owner))
      throw new TypeError('Invalid native addon graph owner');
    const existing = functions.get(handle);
    if (existing !== undefined) return existing;
    // The invoker factory retains the native Function root for the process lifetime (summary 3 of the plan).
    const invoke = cap.invoker(handle);
    const invokeNative = (values, construct, forceHandle) => {
      const packet = encodeGraph(values, 0, true, false, forceHandle);
      const result = checkReply(jsonParse(invoke(packet, construct)), 'Native Function call failed');
      if (result.kind !== 'graph' || typeof result.value !== 'string')
        throw new TypeError('Invalid native Function graph result');
      return decodeOwned(result.value);
    };
    // Appends `args` to `values` (the engine's argument list is not iterated through a user-replaceable method).
    const pack = (args, values) => {
      for (let index = 0; index < args.length; index++) values.push(args[index]);
      return values;
    };
    const target = function () {};
    // A function's own `prototype` is non-configurable; this target must lose it.
    Object.defineProperty(target, 'prototype', { configurable: true });
    delete target.prototype;
    const initialTargetOwn = new Map();
    reflectOwnKeys(target).forEach(key => { initialTargetOwn.set(key, reflectGetOwn(target, key)); });
    const fallbackGetters = new Map();
    const fallbackSetters = new Map();
    const sourceFunctions = new Map();

    // A native key that is missing again: an intrinsic target key the native side deleted is restored to its
    // initial descriptor, a native-only shadow is removed.
    const reconcileMissingNativeKey = key => {
      const own = reflectGetOwn(target, key);
      if (own && own.configurable) {
        const initial = initialTargetOwn.get(key);
        const restored = initial ? reflectDefine(target, key, initial) : reflectDelete(target, key);
        if (!restored) throw new TypeError('Cannot synchronize deleted native Function property');
      }
      fallbackGetters.delete(key);
      fallbackSetters.delete(key);
    };
    const sourceFunction = source => {
      const cacheKey = jsonStringify(source);
      const cached = sourceFunctions.get(cacheKey);
      if (cached !== undefined) return cached;
      const decoded = decodeGraph({ root: { r: 0 }, nodes: [source] });
      if (typeof decoded === 'function') sourceFunctions.set(cacheKey, decoded);
      return decoded;
    };
    const read = (name, receiver) => graphProperty(handle, 'get_graph', [name, receiver]);
    const nativeHas = key => graphProperty(handle, 'has_graph', [key]);

    const proxy = new Proxy(target, {
      apply(shadow, thisArg, args) {
        return invokeNative(pack(args, [thisArg]), false);
      },
      construct(shadow, args, newTarget) {
        // The engine supplies the actual new.target; it travels in the same graph as the arguments.
        const realmObject = reflectConstruct(intrinsicObject, [], newTarget);
        const fallback = reflectGetProto(realmObject);
        return invokeNative(pack(args, [newTarget, fallback]), true, fallback);
      },
      get(shadow, key, receiver) {
        const native = nativeHas(key);
        if (!native) reconcileMissingNativeKey(key);
        const own = reflectGetOwn(shadow, key);
        if (own && !own.configurable && hasOwn(own, 'value') && !own.writable) return own.value;
        // A nonconfigurable target accessor without a getter fixes the Get result, as in proxyForHandle.
        if (own && !own.configurable && hasOwn(own, 'get') && own.get === undefined) return undefined;
        return native ? read(key, receiver) : reflectGet(shadow, key, receiver);
      },
      set(shadow, key, value, receiver) {
        if (!nativeHas(key)) reconcileMissingNativeKey(key);
        const own = reflectGetOwn(shadow, key);
        // A callable target already owns immutable function properties: a Proxy cannot report a successful write
        // to one, and the native mutation must not happen first.
        if (own && !own.configurable
          && ((hasOwn(own, 'value') && !own.writable) || (hasOwn(own, 'set') && own.set === undefined)))
          return false;
        if (graphProperty(handle, 'set_graph', [key, value, receiver]) !== true) return false;
        if (own && !own.configurable && own.writable) reflectDefine(shadow, key, { value: read(key) });
        return true;
      },
      has(shadow, key) {
        const native = nativeHas(key);
        if (!native) reconcileMissingNativeKey(key);
        return native || reflectHas(shadow, key);
      },
      ownKeys(shadow) {
        const keys = graphProperty(handle, 'own_keys_graph', []);
        if (!isArrayOf(keys)) throw new TypeError('Invalid native Function ownKeys result');
        reflectOwnKeys(shadow).forEach(key => {
          if (keys.indexOf(key) < 0) {
            reconcileMissingNativeKey(key);
            if (reflectGetOwn(shadow, key)) keys.push(key);
          }
        });
        return keys;
      },
      getOwnPropertyDescriptor(shadow, key) {
        const own = reflectGetOwn(shadow, key);
        const meta = graphProperty(handle, 'descriptor_graph', [key]);
        if (!meta) {
          reconcileMissingNativeKey(key);
          return reflectGetOwn(shadow, key);
        }
        if (own && !own.configurable) return own;
        let getter = meta.getter_source ? sourceFunction(meta.getter_source) : undefined;
        let setter = meta.setter_source ? sourceFunction(meta.setter_source) : undefined;
        if (meta.accessor && meta.getter && !getter) {
          getter = fallbackGetters.get(key);
          if (!getter) {
            getter = function () { return read(key, this); };
            fallbackGetters.set(key, getter);
          }
        }
        if (meta.accessor && meta.setter && !setter) {
          setter = fallbackSetters.get(key);
          if (!setter) {
            setter = function (next) { graphProperty(handle, 'set_graph', [key, next, this]); };
            fallbackSetters.set(key, setter);
          }
        }
        const descriptor = meta.accessor
          ? { configurable: !!meta.configurable, enumerable: !!meta.enumerable, get: getter, set: setter }
          : { configurable: !!meta.configurable, enumerable: !!meta.enumerable,
            writable: !!meta.writable, value: read(key) };
        if (!meta.configurable) {
          // A nonconfigurable native descriptor must be on the target before a Proxy may report it.
          if (!reflectDefine(shadow, key, descriptor))
            throw new TypeError('Cannot materialize native Function descriptor');
          return reflectGetOwn(shadow, key);
        }
        return descriptor;
      },
      defineProperty(shadow, key, description) {
        if (!nativeHas(key)) reconcileMissingNativeKey(key);
        return defineNativeProperty(handle, shadow, key, description);
      },
      deleteProperty(shadow, key) {
        if (!nativeHas(key)) reconcileMissingNativeKey(key);
        const own = reflectGetOwn(shadow, key);
        if (own && !own.configurable) return false;
        if (graphProperty(handle, 'delete_graph', [key]) !== true) return false;
        fallbackGetters.delete(key);
        fallbackSetters.delete(key);
        return reflectDelete(shadow, key);
      },
      getPrototypeOf(shadow) {
        const native = graphProperty(handle, 'get_prototype_graph', []);
        // Undefined: no explicit native prototype, so the target keeps its own (Function.prototype).
        return native === undefined ? reflectGetProto(shadow) : native;
      },
      setPrototypeOf(shadow, next) {
        if (!reflectIsExt(shadow)) return reflectGetProto(shadow) === next;
        const previous = reflectGetProto(shadow);
        if (!reflectSetProto(shadow, next)) return false;
        try {
          if (graphProperty(handle, 'set_prototype_graph', [next]) === true) return true;
        } catch (error) {
          reflectSetProto(shadow, previous);
          throw error;
        }
        reflectSetProto(shadow, previous);
        return false;
      },
      isExtensible(shadow) {
        if (graphProperty(handle, 'is_extensible_graph', []) === false && reflectIsExt(shadow)
          && !preventNativeExtensions(handle, proxy, shadow))
          throw new TypeError('Cannot synchronize native integrity');
        return reflectIsExt(shadow);
      },
      preventExtensions(shadow) {
        return preventNativeExtensions(handle, proxy, shadow);
      },
    });
    handleOf.set(proxy, handle);
    tagNative(proxy, 1, handle);
    functions.set(handle, proxy);
    return proxy;
  };

  return {
    exports,
    eachExport,
    ownerOf,
    proxyForHandle,
    functionForHandle,
    argument,
    arguments: argumentsOf,
    decode: decodeValue,
    handle,
    syncArguments,
  };
});
