// The native addon proxy layer (plan tj107 sections 2 and 4, PRs 6b-1 and 6b-2): the export listing, the owner
// lookup, and the object and array proxies of native handles. napi_proxy.rs runs this file as a script; its
// completion value is the factory below, called once per realm with `cap`, the private natives (exports, owner,
// invoker). The function proxies are a stub until PR 6b-3.
(function (cap) {
  'use strict';
  // The intrinsics the proxies use, captured before user code runs. ponytail: the Reflect and Object methods are
  // captured as functions, so a later replacement of the owning object does not change them.
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

  const notImplemented = name => () => {
    throw new TypeError(`Native addon ${name} is not implemented`);
  };

  // One bridge operation's reply, with its exception graph thrown and its error raised (RJ bridgeReply).
  const bridgeReply = (operation, handle, packet, message) => {
    const response = jsonParse(bridgeHandle(operation, handle, '', packet));
    if (response && hasOwn(response, '__thaw_exception_graph__')) {
      if (typeof response.__thaw_exception_graph__ !== 'string')
        throw new TypeError('Invalid native exception graph');
      throw decodeOwned(response.__thaw_exception_graph__);
    }
    if (!response || hasOwn(response, '__thaw_error__'))
      throw new TypeError(response && response.__thaw_error__ ? response.__thaw_error__ : message);
    return response;
  };

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

  return {
    exports,
    eachExport,
    ownerOf,
    proxyForHandle,
    functionForHandle: notImplemented('functions'),
  };
});
