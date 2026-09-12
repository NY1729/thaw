# A practical `Intl` (ECMA-402) polyfill, for luxon

**Status: done.**

## Scope

Thaw's embedded QuickJS-NG has no `Intl` (ECMA-402) support at all --
confirmed by inspecting the vendored engine source (`rquickjs-sys`'s
bundled `quickjs/`), no built-in support and no build flag for it. A
**full**, spec-complete `Intl` (real ICU locale data, every calendar
system, full BCP-47 locale negotiation, every numbering system) is not
attempted here. Instead, matching the precedent already set by thaw's
own `node:crypto` shim (honestly supports HMAC/hashing but not
RSA/ECDSA, documented and accepted), this implements a **practical,
English/Latin-numeral-only `Intl`** covering exactly what real luxon
needs to behave correctly:

- `Intl.DateTimeFormat` -- real, `jiff`-backed IANA timezone offset
  computation (with correct, historically-accurate DST transitions, not
  a hand-rolled rule), English month/weekday/era/day-period names,
  every field style (`numeric`/`2-digit`/`long`/`short`/`narrow`) real
  luxon's own presets (`impl/formats.js`) and internal `Locale`
  extraction (`impl/locale.js`) use.
- `Intl.NumberFormat` -- plain decimal grouping/padding
  (`PolyNumberFormatter`'s actual usage), plus `style: 'unit'` (found
  while getting `Duration.toHuman()` to match real Node -- the initial
  plan only anticipated the plain-decimal shape).
- `Intl.ListFormat` -- English conjunction/disjunction joining
  (`Duration.toHuman()`'s list-style option).
- `Intl.RelativeTimeFormat`, `Intl.Locale`, `Intl.PluralRules`,
  `Intl.Collator`, `Intl.Segmenter` are **not implemented at all** --
  confirmed real luxon's own feature-detection (`impl/util.js`'s
  `hasRelative()`/`hasLocaleWeekInfo()`) degrades gracefully to its own
  English-only internal tables when these are `undefined`, for any
  locale, not just English (a real, honest limitation: a non-English
  locale gets English relative-time phrases and default week settings
  from this polyfill, not real ones).
- The system's default timezone (`Intl.DateTimeFormat().resolvedOptions
  ().timeZone` with no explicit `timeZone` given) is always `"UTC"` --
  matches thaw's actual Lambda deployment target (which *is* UTC),
  rather than trying to detect the build machine's own zone.

## Why `Intl.DateTimeFormat` alone was enough to fix luxon's whole
timezone system

Real luxon's `zones/IANAZone.js#offset(ts)` -- the single mechanism
*every* IANA-zone-aware operation in the library is built on -- doesn't
call any dedicated "get the UTC offset for this zone" API at all. It
constructs `new Intl.DateTimeFormat("en-US", { hour12: false, timeZone,
year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit",
minute: "2-digit", second: "2-digit", era: "short" })`, calls
`.formatToParts(date)`, and derives the offset by treating the reported
*local* fields as if they were UTC and diffing against the real
timestamp. A correct, DST-aware `formatToParts` was therefore
sufficient to fix the whole gap -- no changes to luxon itself, and no
separate offset API needed on thaw's side either.
`IANAZone.isValidZone` similarly just constructs an `Intl.
DateTimeFormat` in a try/catch and expects a `RangeError` for an unknown
zone, which the constructor validates eagerly (one call to the native
primitive below, using the current instant).

## Native primitive: `__thaw_intl_zoned_parts` (Rust, `jiff`)

`crates/thaw-quickjs/src/quickjs/intl.rs`'s `intl_zoned_parts_json(tz_name,
timestamp_ms)` is the *only* native function this polyfill needs. Given
an IANA zone name and an instant, it resolves `jiff::tz::TimeZone::get`
(bundled tzdata via the `tzdb-bundle-always` feature -- compiled
directly into the binary, no dependency on the host's own
`/usr/share/zoneinfo`, keeping the binary self-contained and portable
across x64/arm64 Lambda targets) and returns a JSON object:
`{"valid":true,"year":Y,"month":M,"day":D,"hour":H,"minute":Mi,
"second":S,"millisecond":Ms,"weekday":1-7 (ISO, Monday=1),
"offsetMinutes":N,"abbreviation":"EDT","dst":bool}`, or
`{"valid":false}` for an unrecognized zone name or non-finite
timestamp. `jiff::Zoned`'s own accessors give the zone-adjusted local
date/time fields directly -- no date arithmetic needed on the JS side
at all. Registered onto QuickJS-NG globals as `__thaw_intl_zoned_parts`
in `context.rs`, the exact same `rquickjs::Function::new` + `ctx.
globals().set(...)` pattern every other native builtin (crypto, random,
...) already uses.

## JS polyfill: `platform_globals/intl.js`

English name tables (months/weekdays/eras/day-periods), a `jiff`-backed
`formatToParts` that renders each requested field plus the real en-US
literal punctuation/ordering for every field combination luxon uses
(confirmed against real Node's own output, not guessed -- including the
`" at "` vs `", "` date/time connector, which depends on whether `month`
is `'long'`, and the `h11`/`h12` vs `h23`/`h24` hour-padding difference:
`hourCycle` `h23`/`h24` always zero-pads even under the `'numeric'`
style). `timeZoneName` prefers `jiff`'s own abbreviation (`"EDT"`) for
`short`/`shortGeneric` when it's a plain alphabetic code, always
synthesizes a `"GMT+H[:MM]"` offset string for `*Offset`, and for
`long`/`longGeneric` looks up a real per-zone English name (e.g.
"Eastern Daylight Time"/"Eastern Time") from a static table
(`intl_time_zone_names.rs`) covering all 418 real IANA zones --
mechanically extracted from a real `node`/ICU run (`Intl.
DateTimeFormat` itself, at reference instants clear of any recent
zone-rule change like Kazakhstan's 2024 restructuring), not typed from
memory, since CLDR's ~150+ metazone names are exactly the kind of data
real hallucination risk applies to. Falls back to the same synthesized
offset string for a zone the table has no entry for (shouldn't happen
for a real IANA name). `dateStyle`/`timeStyle` shorthand and
`fractionalSecondDigits` are accepted but not rendered (confirmed
luxon's own presets, `impl/formats.js`, never use them -- always plain
per-field options).

`Intl.NumberFormat`'s `style: 'unit'` (found via `Duration.toHuman()`)
uses a small per-unit English form table (`INTL_UNITS`) covering
exactly the units luxon's own `orderedUnits` iterates
(year/month/week/day/hour/minute/second/millisecond) -- `narrow` never
pluralizes in English, `short` only pluralizes year/month/week (`"3 hr"`
not `"3 hrs"`), `long` always pluralizes normally; confirmed against
real Node for every unit/style/count combination, not guessed.
`Intl.ListFormat` does plain English `"a"`/`"a and b"`/`"a, b, and c"`
joining (no narrow/short/long distinction worth modeling for English).

## Four general compiler/bridge bugs found and fixed getting luxon's
own `DateTime.fromISO(...)` and `Settings.defaultLocale` to actually run

None of these are Intl-specific; all four are general gaps any package
with the same shape can hit.

1. **`new <Namespace>.<Class>(...)` didn't compile at all**
   (`"only 'new Promise<T>(...)' is supported"`). thaw-hir's special-
   cased list of constructible globals (`TextEncoder`/
   `AbortController`/typed arrays/...) only ever matched a bare
   `Expr::Ident` callee (`crates/thaw-hir/src/lower/expressions/
   lowering.rs`), never `Expr::Member` -- so `new Intl.DateTimeFormat
   (...)` fell straight through to the Promise-only fallback. Fixed by
   reusing the exact same `getDynamicValue`/`constructDynamicValue`
   mechanism the existing list already uses, just with a dotted name
   (`"Intl.DateTimeFormat"`); `thaw_js_get_global` (`crates/
   thaw-quickjs/src/quickjs/api.rs`) tries `name` as a single, literal
   global property *first* (unchanged from before -- required, since a
   real package name can itself contain a literal `.`, e.g. `socket.io`,
   making a runtime key derived from one contain a `.` that was never
   meant as a namespace separator; confirmed this the hard way, a first
   attempt that split unconditionally broke `socket.io-client`'s own
   `io(...)` factory call at runtime with "invalid JavaScript value
   handle 0") and only walks nested object properties for a dotted path
   when no such literal global exists. Scoped to `Intl.DateTimeFormat`/
   `Intl.NumberFormat`/`Intl.ListFormat` specifically at the thaw-hir
   level, not every possible dotted global -- a deliberately narrow
   allow-list, matching the existing bare-identifier list's own style.
2. **A class with no public constructor lost its bare-identifier value
   binding entirely.** Real luxon's `DateTime`/`Duration`/`Interval`
   each declare `private constructor(...)` -- built only via static
   factories like `DateTime.fromISO(...)`. `generate_napi_class_
   constructors` correctly bails out on `!class.constructible` (`new
   DateTime()` really shouldn't compile) -- but that's also the *only*
   thing that ever populates `class_targets`, which every exported
   class name's bare-identifier resolution depends on. With no entry,
   "DateTime" fell back to the type-only `JsValue` alias every exported
   class gets (correct for `let x: DateTime` as a *type*, but not a
   *value* -- any non-call use, like a static method call's own callee
   or a static property read, failed to build: "unknown variable
   `__thaw_type_luxon_DateTime`"). Fixed by treating such a class like
   any other named package value once `class_targets` has nothing for
   it (`shim_generation.rs`'s `bare_value_classes`): bound via
   the exact same `$value$`-keyed runtime-getter mechanism a `Str`/
   `F64`/`JsValue` constant export already uses (confirmed pre-existing
   and already working via a direct synthetic test, `constant-kit`),
   reading the *real* class value straight off the bundle's own live JS
   exports.
3. **A static method call's receiver was never bound, and once it was,
   lost its own static methods.** `compile_typed_napi_method`
   (`crates/thaw-llvm/src/hir_codegen/dynamic_host/napi.rs`) looks up a
   static method's receiver via `thaw_js_get_global` against the
   class's *plain* name -- nothing had ever bound that bare global for
   a Fallback (QuickJS-NG-backed) class before (a real N-API module's
   export lookup is a different, already-working mechanism), so this
   whole code path was apparently never exercised by any real package
   until luxon. Fixed by also capturing the bare name in `generate_
   module_init`'s glue (`crates/thaw-bridge/src/bridge/generation.rs`),
   reusing the same read-`module.exports`-with-a-`default`-fallback
   expression the `$value$` getter above already uses. Once the
   receiver *was* correctly found, its `.fromISO` (and every other
   static method) still came back `undefined`: the generic "copy every
   `module.exports` property onto `globalThis`" bootstrap glue (and the
   separate `qualified_aliases` capture) `.bind()`s every function-typed
   property to preserve `this` for a *stateful namespace object*'s
   methods (real example: joi's `Root.string()`, round 6's fix) -- but
   a bound function is a genuinely different function object carrying
   none of the original's own properties, silently dropping every
   `static` method a real class has (non-enumerable by spec, so
   invisible to a plain property copy too -- confirmed via `fn_name ==
   "bound DateTime"` and zero own keys in a direct debug trace). Fixed
   by a shared `__thaw_bind_preserving_statics` helper
   (`platform_globals/runtime.js`) that binds *and* copies every own
   property descriptor (enumerable or not) from the original onto the
   bound wrapper, used at every `.bind()` call site in `generation.rs`.
4. **A constructible-but-never-`new`'d class's bare-identifier value
   binding was shadowed by its own unused constructor symbol.** Found
   after bug 2 above shipped: luxon's `Settings` is a purely static
   config object with *no* declared constructor at all (unlike
   `DateTime`/`Duration`/`Interval`, which all declare `private
   constructor(...)`) -- so `constructible` defaults to `true`, and
   `generate_napi_class_constructors`'s fallback path happily synthesizes
   a placeholder zero-arg constructor (`$new$Settings$arity0`) and a
   real `class_targets` entry for it, even though nothing ever calls
   `new Settings()`. `package_exports`'s class-name mapping preferred
   `class_targets` unconditionally, so `Settings`'s bare name resolved
   to that placeholder constructor symbol instead of a usable value --
   and since the Fallback (non-N-API) branch never generates a static
   property *setter* at all (only the N-API branch does), `Settings.
   defaultLocale = "en-US"` was never text-rewritten away first either;
   it reached thaw-hir raw, with `Settings` resolved to an
   unrecognizable constructor-invoking symbol: "needs a monomorphic
   native implementation" against `$new$Settings$arity0`. Fixed by
   broadening bug 2's mechanism to cover *every* exported class (not
   just non-constructible ones), scoped by a new, more general AST walk
   (`observed_bare_member_object_identifiers`, `shim_support.rs` --
   every `Expr::Member`, so a property read, an assignment target, and a
   method call's own callee are all covered uniformly) rather than the
   narrower "has an observed static method call" check bug 2 used, and
   making `package_exports` prefer this value binding over `class_
   targets`'s constructor symbol whenever both exist. `class_targets`/
   `class_rewrites`'s own, separate `new ClassName(...)` support is
   completely untouched -- a `new` call site never consults `package_
   exports` at all.

## Verified end to end against real luxon@3.7.2

`DateTime.fromISO(...)`, `.setZone("America/New_York")` across a real
DST boundary (July vs. January, confirmed via `jiff`'s bundled tzdata,
not a hand-rolled rule -- `-04:00`/`EDT` vs. `-05:00`/`EST`),
`.toFormat(...)`, `.toLocaleString(DateTime.DATE_FULL)`, `.toLocaleString
(DateTime.DATETIME_FULL)`, `.diff(...).toHuman()`, and `Settings.
defaultLocale = "en-US"` (a bare static property assignment, bug 4
above) -- every result matches real Node running the same luxon version
for the same fixed instants exactly (this polyfill always renders
English regardless of what locale string luxon itself requests, so
setting `Settings.defaultLocale` doesn't actually change any of this
test's output -- it's exercised here purely to cover bug 4, not because
luxon's own results depend on it).

## Tests

- `crates/thaw-quickjs/src/tests.rs`: `intl_date_time_format_matches_
  real_node_for_every_field_and_style_luxon_uses`, `intl_number_format_
  and_list_format_match_real_node` (network-free, raw JS via the
  existing `load`/`call` harness).
- `crates/thaw-llvm/src/hir_codegen/tests/values/dynamic_and_json.rs`:
  `constructs_a_namespaced_global_value_via_new` (network-free, via
  `compile_and_run` -- covers bug 1 above).
- `crates/thaw-cli/src/tests/registry_fallback/classes_and_values.rs`:
  `a_non_constructible_classs_static_methods_and_bare_name_both_work`
  (network-free synthetic registry package, mimicking luxon's `DateTime`
  shape exactly -- covers bugs 2 and 3 above); `a_constructible_but_
  never_newed_class_can_have_its_static_property_assigned` (mimicking
  luxon's `Settings` shape -- covers bug 4 above).
- `crates/thaw-cli/src/tests/native_addons/javascript_packages.rs`: a
  real luxon pinned integration test (`THAW_RUN_NPM_INTEGRATION=1`-
  gated), using fixed historical dates (never `DateTime.now()`, for
  determinism), including `Settings.defaultLocale = "en-US"` (bug 4).
