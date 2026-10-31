// Helper globals the compiler's dynamic-value paths look up by name (`getDynamicValue("__thaw_...")`),
// installed once per realm that serves compiled code. The blocks below are verbatim ports of the
// QuickJS bootstrap helpers (thaw-quickjs `platform_globals/runtime.js`), run against thaw-js with the
// captured intrinsics re-bound here.
(function () {
  const thawGraphApply = Reflect.apply;
  const thawDateApply = Reflect.apply;
  const thawGraphDefineProperty = Object.defineProperty;
  const thawGraphObjectCreate = Object.create;
  const thawGraphObjectSetPrototypeOf = Object.setPrototypeOf;
  const thawGraphObjectFreeze = Object.freeze;
  const thawGraphObjectIs = Object.is;
  const thawGraphObjectIsFrozen = Object.isFrozen;
  const thawGraphObjectIsSealed = Object.isSealed;
  const thawGraphArraySlice = Array.prototype.slice;
  const thawGraphArrayPush = Array.prototype.push;
  const thawStringSplit = String.prototype.split;
  const thawStringStartsWith = String.prototype.startsWith;
  const thawStringIndexOf = String.prototype.indexOf;
  const thawStringSlice = String.prototype.slice;
  const thawGraphMapConstructor = Map;
  const thawGraphSetConstructor = Set;
  const thawGraphMapSet = Map.prototype.set;
  const thawGraphMapGet = Map.prototype.get;
  const thawGraphGetPrototypeOf = Reflect.getPrototypeOf;
  const thawGraphReflectGet = Reflect.get;
  const thawGraphReflectSet = Reflect.set;
  const thawGraphReflectHas = Reflect.has;
  const thawGraphReflectDeleteProperty = Reflect.deleteProperty;
  const thawGraphReflectOwnKeys = Reflect.ownKeys;
  const thawGraphReflectDefineProperty = Reflect.defineProperty;
  const thawGraphReflectGetOwnPropertyDescriptor = Reflect.getOwnPropertyDescriptor;
  const thawGraphReflectPreventExtensions = Reflect.preventExtensions;
  const thawGraphNumber = Number;
  const thawGraphString = String;
  const thawGraphTypeError = TypeError;
  const thawGraphProxy = Proxy;
  const thawGraphParse = JSON.parse;
  const thawBootstrapIteratorKey = Symbol.iterator;
  const thawProjectedReceivers = new WeakSet();
  const thawNativeViews = new WeakSet();
  const thawLiveIterators = new WeakSet();
  const thawNativeWrapperSet = new WeakSet();
  const thawNativeCollections = new WeakMap();
  const thawNativeIteratorSteps = new WeakMap();
  const thawClassProjectionNames = new WeakMap();
  const thawProjectedReceiverHas = WeakSet.prototype.has;
  const thawProjectedReceiverAdd = WeakSet.prototype.add;
  const thawNapiWeakSetAdd = WeakSet.prototype.add;
  const thawOriginSerialGet = WeakMap.prototype.get;
  const thawOriginSerialSet = WeakMap.prototype.set;
  const thawClassProjectionSet = WeakMap.prototype.set;
  const thawMapIteratorPrototype = thawGraphGetPrototypeOf(new Map().keys());
  const thawSetIteratorPrototype = thawGraphGetPrototypeOf(new Set().values());
  const thawMapIteratorNext = thawMapIteratorPrototype.next;
  const thawSetIteratorNext = thawSetIteratorPrototype.next;
  // Symbol-keyed static properties are not encoded across this boundary yet.
  globalThis.__thaw_property_key = (key) => key;
  globalThis.__thaw_storage_value = (values) => values;
  globalThis.__thaw_json_host_instanceof = (value, constructor) => value instanceof constructor;
  thawGraphDefineProperty(globalThis, '__thaw_object_with_native_getters', {
    value: (keys, readable, writable, accessors, methods, ...rest) => {
      // A class instance passes its class name before the callbacks: its method
      // keys become non-enumerable members of a prototype that names the class.
      const ancestry = typeof rest[0] === 'string' ? rest[0] : undefined;
      const callbacks = ancestry === undefined ? rest : thawGraphApply(thawGraphArraySlice, rest, [1]);
      const isClass = ancestry !== undefined;
      const target = {};
      let displayName = ancestry;
      if (isClass) {
        const names = thawGraphApply(thawStringSplit, ancestry, ['\u001f']);
        const display = name => {
          // Module-scoped classes are mangled `__thawmod<N>_Name`.
          if (thawGraphApply(thawStringStartsWith, name, ['__thawmod'])) {
            const cut = thawGraphApply(thawStringIndexOf, name, ['_', 9]);
            if (cut > 9) return thawGraphApply(thawStringSlice, name, [cut + 1]);
          }
          return name;
        };
        displayName = display(names[0]);
        // A class that is also used as a value has a real JS counterpart: its
        // prototype keeps `instanceof` and `constructor` exact. A class without
        // one gets a named stand-in that inherits from the nearest real ancestor.
        let inherited = Object.prototype;
        let proto;
        for (let i = 0; i < names.length; i++) {
          const realClass = globalThis['__thaw_class_value_' + names[i]];
          if (typeof realClass === 'function' && realClass.prototype) {
            if (i === 0) proto = realClass.prototype; else inherited = realClass.prototype;
            break;
          }
        }
        if (proto === undefined) {
          proto = thawGraphObjectCreate(inherited);
          const constructor = thawGraphDefineProperty(function () {}, 'name', { value: displayName, configurable: true });
          thawGraphDefineProperty(proto, 'constructor', { value: constructor, writable: true, configurable: true });
          thawGraphDefineProperty(constructor, 'prototype', { value: proto });
        }
        thawGraphObjectSetPrototypeOf(target, proto);
      }
      const positions = new thawGraphMapConstructor();
      for (let i = 0; i < keys.length; i++) {
        thawGraphApply(thawGraphMapSet, positions, [keys[i], i]);
        // A class method is a prototype member in Node: it is reachable through `get`/`in`
        // below but is not an own property of the instance.
        if (isClass && methods[i]) {
          if (accessors[i]) {
            const proto = thawGraphGetPrototypeOf(target);
            const owner = globalThis['__thaw_class_value_' + methods[i]];
            const existing = typeof owner === 'function' && owner.prototype
              && thawGraphReflectGetOwnPropertyDescriptor(owner.prototype, keys[i]);
            if (existing) continue;
            thawGraphDefineProperty(proto, keys[i], {
              get: readable[i] ? callbacks[i * 2] : undefined,
              set: writable[i] ? callbacks[i * 2 + 1] : undefined,
              enumerable: false, configurable: true,
            });
          }
          continue;
        } else if (accessors[i]) {
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
      // A receiver-aware method reads as one stable function per wrapper. Each call rebinds to
      // the call's own receiver when that is another projected owner (`m.call(otherView)`).
      const methodFunctions = [];
      const method = index => methodFunctions[index] ??= thawGraphDefineProperty(function (...args) {
        const receiver = isClass || thawGraphApply(thawProjectedReceiverHas, thawProjectedReceivers, [this])
          ? this : wrapper;
        return thawGraphApply(thawGraphApply(callbacks[index * 2], receiver, []), undefined, args);
      }, 'name', { value: keys[index], configurable: true });
      const value = index => methods[index] ? method(index) : read(index);
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
          if (isClass && methods[i]) continue;
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
            if (!writableNow || !configurable) update.value = value(i);
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
          if (isClass && methods[index]) {
            // An own property assigned over the method shadows it.
            return thawGraphReflectGetOwnPropertyDescriptor(target, key)
              ? thawGraphReflectGet(target, key, receiver) : method(index);
          }
          const descriptor = thawGraphReflectGetOwnPropertyDescriptor(target, key);
          return descriptor.configurable || descriptor.writable ? value(index) : descriptor.value;
        },
        set(_target, key, value, receiver) {
          sync();
          const index = thawGraphApply(thawGraphMapGet, positions, [key]);
          // A key outside the fixed native layout lives on the JS shadow target only.
          if (index === undefined) return thawGraphReflectSet(target, key, value);
          if (accessors[index]) return thawGraphReflectSet(target, key, value, receiver);
          if (isClass && methods[index]) return thawGraphReflectSet(target, key, value);
          if (!(flags(key) & 1)) return false;
          write(index, value);
          return true;
        },
        deleteProperty(_target, key) {
          sync();
          // Native fixed-layout fields cannot be removed from the owner.
          // Leaving the Proxy default would delete only its shadow slot.
          const index = thawGraphApply(thawGraphMapGet, positions, [key]);
          if (index !== undefined && !(isClass && methods[index])) return false;
          return thawGraphReflectDeleteProperty(target, key);
        },
        has(_target, key) {
          sync();
          const index = thawGraphApply(thawGraphMapGet, positions, [key]);
          if (isClass && index !== undefined && methods[index]) return true;
          return thawGraphReflectHas(target, key);
        },
        ownKeys() { sync(); return thawGraphReflectOwnKeys(target); },
        getOwnPropertyDescriptor(_target, key) {
          sync();
          const descriptor = thawGraphReflectGetOwnPropertyDescriptor(target, key);
          const index = thawGraphApply(thawGraphMapGet, positions, [key]);
          if (index === undefined || accessors[index] || !descriptor || (isClass && methods[index])) return descriptor;
          return { value: descriptor.configurable || descriptor.writable ? value(index) : descriptor.value,
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
          if (index === undefined) return thawGraphReflectDefineProperty(target, key, descriptor);
          if (isClass && methods[index]) return thawGraphReflectDefineProperty(target, key, descriptor);
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
              if (!thawGraphObjectIs(descriptor.value, value(index))) return false;
            } else {
              if (!defineWrite(index, descriptor.value, requested)) return false;
            }
          }
          // A fixed field can coerce the written value. Once the target
          // becomes non-configurable and non-writable, Proxy invariants need
          // its actual stored value, including for an attributes-only update.
          if (!accessors[index]) updated.value = value(index);
          if (!thawGraphApply(setFlags, stateReceiver, [key, requested])) return false;
          if (!thawGraphReflectDefineProperty(target, key, updated)) return false;
          if (thawGraphObjectIsFrozen(target)) thawGraphApply(setState, stateReceiver, [3]);
          else if (thawGraphObjectIsSealed(target)) thawGraphApply(setState, stateReceiver, [2]);
          return true;
        },
      });
      let hasAccessor = false;
      for (let i = 0; i < keys.length; i++) if (accessors[i]) hasAccessor = true;
      if (!hasAccessor && accessors[keys.length] !== true) thawGraphApply(thawNapiWeakSetAdd, thawNativeWrapperSet, [wrapper]);
      thawGraphApply(thawProjectedReceiverAdd, thawProjectedReceivers, [wrapper]);
      if (isClass) thawGraphApply(thawClassProjectionSet, thawClassProjectionNames, [wrapper, displayName]);
      return wrapper;
    },
    writable: false, configurable: false,
  });
  thawGraphDefineProperty(globalThis, '__thaw_array_with_native_view', {
    value: (canWrite, lenCb, getCb, setCb, pushCb, resizeCb, holeCb, presentCb, rawCb) => {
      const target = [];
      const receiver = thawGraphObjectCreate(null);
      const len = () => thawGraphApply(lenCb, receiver, []);
      const at = index => thawGraphApply(getCb, receiver, [index]);
      const has = index => index < len() && thawGraphApply(presentCb, receiver, [index]);
      const indexOf = key => {
        if (typeof key !== 'string') return -1;
        const number = thawGraphNumber(key);
        return number >>> 0 === number && number !== 4294967295
          && thawGraphString(number) === key ? number : -1;
      };
      const write = (index, value) => {
        if (!canWrite) return false;
        const length = len();
        // An owner-backed element callback answers `false` for a value that is not storable.
        if (index < length) return thawGraphApply(setCb, receiver, [index, value]) !== false;
        if (index === length) return thawGraphApply(pushCb, receiver, [value]) !== false;
        thawGraphApply(resizeCb, receiver, [index + 1]);
        return thawGraphApply(setCb, receiver, [index, value]) !== false;
      };
      const resize = value => {
        const length = thawGraphNumber(value);
        if (length >>> 0 !== length) throw new RangeError('Invalid array length');
        thawGraphApply(resizeCb, receiver, [length]);
        return true;
      };
      // A tagged template's strings array: `.raw` is the registered sibling, frozen like the array.
      let raw;
      const rawArray = () => {
        if (raw === undefined) {
          const found = typeof rawCb === 'function' ? thawGraphApply(rawCb, receiver, []) : null;
          raw = found && found.length > 0 ? thawGraphObjectFreeze(found) : null;
        }
        return raw;
      };
      const view = new thawGraphProxy(target, {
        get(_target, key, self) {
          if (key === 'length') return len();
          const index = indexOf(key);
          if (index >= 0) return has(index) ? at(index) : undefined;
          if (key === 'raw' && !thawGraphReflectHas(target, key)) {
            const found = rawArray();
            if (found) return found;
          }
          return thawGraphReflectGet(target, key, self);
        },
        set(_target, key, value) {
          if (key === 'length') return resize(value);
          const index = indexOf(key);
          if (index < 0) return thawGraphReflectSet(target, key, value);
          return write(index, value);
        },
        has(_target, key) {
          if (key === 'length') return true;
          const index = indexOf(key);
          return index >= 0 ? has(index) : thawGraphReflectHas(target, key);
        },
        deleteProperty(_target, key) {
          const index = indexOf(key);
          if (index < 0) return thawGraphReflectDeleteProperty(target, key);
          if (index < len()) thawGraphApply(holeCb, receiver, [index]);
          return true;
        },
        ownKeys() {
          const keys = [];
          const length = len();
          for (let index = 0; index < length; index++)
            if (has(index)) thawGraphApply(thawGraphArrayPush, keys, [thawGraphString(index)]);
          thawGraphApply(thawGraphArrayPush, keys, ['length']);
          const extra = thawGraphReflectOwnKeys(target);
          for (let i = 0; i < extra.length; i++)
            if (extra[i] !== 'length') thawGraphApply(thawGraphArrayPush, keys, [extra[i]]);
          return keys;
        },
        getOwnPropertyDescriptor(_target, key) {
          if (key === 'length')
            return { value: len(), writable: true, enumerable: false, configurable: false };
          const index = indexOf(key);
          if (index < 0) return thawGraphReflectGetOwnPropertyDescriptor(target, key);
          if (!has(index)) return undefined;
          return { value: at(index), writable: canWrite, enumerable: true, configurable: true };
        },
        defineProperty(_target, key, descriptor) {
          if ('get' in descriptor || 'set' in descriptor) return false;
          if (key === 'length') return 'value' in descriptor ? resize(descriptor.value) : true;
          const index = indexOf(key);
          if (index < 0) return thawGraphReflectDefineProperty(target, key, descriptor);
          return 'value' in descriptor ? write(index, descriptor.value) : has(index);
        },
      });
      thawGraphApply(thawProjectedReceiverAdd, thawNativeViews, [view]);
      return view;
    },
    writable: false, configurable: false,
  });
  thawGraphDefineProperty(globalThis, '__thaw_collection_with_native_view', {
    value: (isMap, keyKind, valueKind, sizeCb, hasCb, getCb, setCb, deleteCb, clearCb, keysCb, nextKeyCb) => {
      const receiver = thawGraphObjectCreate(null);
      const keyOk = key => keyKind === 'any' || typeof key === keyKind;
      const valueOk = value => valueKind === 'any' || typeof value === valueKind;
      const keys = () => thawGraphApply(keysCb, receiver, []);
      const has = key => keyOk(key) && thawGraphApply(hasCb, receiver, [key]) === true;
      const get = key => keyOk(key) ? thawGraphApply(getCb, receiver, [key]) : undefined;
      const proto = thawGraphObjectCreate(isMap ? thawGraphMapConstructor.prototype : thawGraphSetConstructor.prototype);
      const view = thawGraphObjectCreate(proto);
      const define = (name, fn) => thawGraphDefineProperty(proto, name,
        { value: fn, writable: true, configurable: true, enumerable: false });
      const iterate = mode => {
        let cursor = 0;
        let done = false;
        const iterator = thawGraphObjectCreate(isMap ? thawMapIteratorPrototype : thawSetIteratorPrototype);
        const advance = () => {
          if (done) return { value: undefined, done: true };
          // Retain the view along with its callbacks while an iterator outlives its receiver.
          const step = thawGraphApply(nextKeyCb, view, [cursor]);
          if (step.length === 0) {
            done = true;
            return { value: undefined, done: true };
          }
          cursor = step[0];
          const key = step[1];
          const value = mode === 0 ? key : isMap ? get(key) : key;
          return { value: mode === 2 ? [key, value] : value, done: false };
        };
        thawGraphApply(thawOriginSerialSet, thawNativeIteratorSteps, [iterator, { isMap, advance }]);
        thawGraphDefineProperty(iterator, 'next', { value: function () {
          const state = thawGraphApply(thawOriginSerialGet, thawNativeIteratorSteps, [this]);
          if (state && state.isMap === isMap) return state.advance();
          return thawGraphApply(isMap ? thawMapIteratorNext : thawSetIteratorNext, this, []);
        }, writable: true, configurable: true });
        thawGraphApply(thawProjectedReceiverAdd, thawNativeViews, [iterator]);
        thawGraphApply(thawProjectedReceiverAdd, thawLiveIterators, [iterator]);
        return iterator;
      };
      const reject = () => new thawGraphTypeError(
        'Cannot store this value in a native ' + (isMap ? 'Map' : 'Set') + ' view');
      thawGraphDefineProperty(proto, 'size', {
        get: () => thawGraphApply(sizeCb, receiver, []), configurable: true, enumerable: false,
      });
      define('has', has);
      define('delete', key => keyOk(key) && thawGraphApply(deleteCb, receiver, [key]) === true);
      define('clear', () => { thawGraphApply(clearCb, receiver, []); });
      define('forEach', (callback, thisArg) => {
        if (typeof callback !== 'function') throw new thawGraphTypeError(String(callback) + ' is not a function');
        const iterator = iterate(2);
        for (let step = iterator.next(); !step.done; step = iterator.next()) {
          thawDateApply(callback, thisArg, [step.value[1], step.value[0], view]);
        }
      });
      if (isMap) {
        define('get', get);
        define('set', (key, value) => {
          if (!keyOk(key) || !valueOk(value)) throw reject();
          thawGraphApply(setCb, receiver, [key, value]);
          return view;
        });
        define('keys', () => iterate(0));
        define('values', () => iterate(1));
        define('entries', () => iterate(2));
        define(thawBootstrapIteratorKey, () => iterate(2));
      } else {
        define('add', value => {
          if (!keyOk(value)) throw reject();
          thawGraphApply(setCb, receiver, [value]);
          return view;
        });
        define('values', () => iterate(0));
        define('keys', () => iterate(0));
        define('entries', () => iterate(2));
        define(thawBootstrapIteratorKey, () => iterate(0));
      }
      thawGraphApply(thawOriginSerialSet, thawNativeCollections, [view, { isMap, keys, get }]);
      thawGraphApply(thawProjectedReceiverAdd, thawNativeViews, [view]);
      return view;
    },
    writable: false, configurable: false,
  });
  globalThis.__thaw_object_from_operations = (kinds, keysText, keyKinds, values, ...callbacks) => {
    const keys = thawGraphParse(keysText);
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
      if (kind === 'proto') {
        const prototype = values[i];
        if (prototype === null || typeof prototype === 'object' || typeof prototype === 'function') {
          Object.setPrototypeOf(object, prototype);
        }
        continue;
      }
      const key = keyKinds[i] ? globalThis.__thaw_property_key(keys[i]) : keys[i];
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
  // Host enumeration (`__thaw_json_host_enumerate`): own keys / values / entries, Map and Set contents,
  // a native collection view's live snapshot, and the prototype.
  const symbolKeyWire = (key) => /^\u0003R?\d+(:|$)/.test(key);
  globalThis.__thaw_json_host_enumerate = (value, operation) => {
    const view = value !== null && typeof value === 'object' ? thawGraphApply(thawOriginSerialGet, thawNativeCollections, [value]) : undefined;
    if (view && operation >= 4 && operation <= 6) {
      const keys = view.keys();
      if (operation !== 6) return keys;
      return keys.map((key) => view.get(key));
    }
    switch (operation) {
      case 0: return Object.entries(value);
      case 1: return Reflect.ownKeys(value);
      case 2: return Object.keys(value).filter((key) => !symbolKeyWire(key));
      case 3: { const keys = Object.keys(value); return Object.values(value).filter((_, index) => !symbolKeyWire(keys[index])); }
      case 4: return Array.from(Map.prototype.keys.call(value));
      case 5: return Array.from(Set.prototype.values.call(value));
      case 6: return Array.from(Map.prototype.values.call(value));
      case 7: return [...value];
      case 8: return [Reflect.getPrototypeOf(value)];
      case 10: return Array.from(Map.prototype.entries.call(value));
      case 11: return Array.from(Set.prototype.values.call(value));
      default: throw new TypeError('Invalid native host enumeration');
    }
  };
  // Host queries thaw-js answers in JS: native wrapper / integrity state, class projection name, collection kind.
  globalThis.__thaw_json_host_query = (value, operation) => {
    switch (operation) {
      case 17: return thawGraphApply(thawProjectedReceiverHas, thawNativeWrapperSet, [value]) ? '1' : '0';
      case 18: return Object.isFrozen(value) ? '1' : '0';
      case 19: return Object.isSealed(value) ? '1' : '0';
      case 20: return Reflect.isExtensible(value) ? '0' : '1';
      case 22: return Error.prototype.isPrototypeOf(value) ? '1' : '0';
      case 23: return thawGraphApply(thawOriginSerialGet, thawClassProjectionNames, [value]) ?? '';
      case 24: {
        const native = value !== null && typeof value === 'object' ? thawGraphApply(thawOriginSerialGet, thawNativeCollections, [value]) : undefined;
        if (native) return native.isMap ? 'map' : 'set';
        try { Map.prototype.has.call(value, undefined); return 'map'; } catch (_) {}
        try { Set.prototype.has.call(value, undefined); return 'set'; } catch (_) {}
        return '';
      }
      default: throw new TypeError('Invalid native host query');
    }
  };
})();
