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
  globalThis.__thaw_json_date_reviver = (key, value) => {
    if (
      value &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      Object.keys(value).length === 1 &&
      typeof value.timestamp === 'number'
    ) {
      return new Date(value.timestamp);
    }
    return value;
  };
