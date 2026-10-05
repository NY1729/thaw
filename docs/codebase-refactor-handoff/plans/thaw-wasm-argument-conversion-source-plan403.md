# WebAssembly argument conversion source contract, HEAD403

Unresolved findings: 01a0f491-e520-7553-bc95-21aa1259f046:0 and 01a0f491-9095-7650-9b62-dbe7cd6a53a0:0.

All outgoing JS argument encoders are in platform_globals/webassembly.js: wasmPrepareFuncref bridge, wasmDecodeValue generated callable, and WasmInstance export callable. Each currently uses args.map(wasmEncodeValue); all have parameter types available from the exported-function metadata or reference parameters. Shared encoder must iterate parameter types (not supplied args), convert each argument once to the declared Wasm type, retain any externref value including numeric/BigInt primitives, and ignore excess args while supplying undefined for omitted args. Native BigInt.asIntN(64,value) and unary plus implement scalar type boundaries.

Host wasm_call_function requires exact encoded arity and forwards through wasm_instance_runtime_value/wasm_number. Thus producing encoded arguments from the signature also reaches the existing exact-length host contract without permitting reference handles as numeric values. Numeric JSON representation must preserve NaN and both infinities; current null conflates them, so handle that deliberately with tag validation in the shared host numeric decoder.

Do not confuse this JS-to-Wasm path with wasm_call_js_import: host-to-JS arguments are actual Value objects and import results are converted by wasm_from_js_value against output types. Table/Global externref writes have their own callers of value-based encoder and need source audit; no broad externref completion claim from export arguments alone.

No tests/build/parser/runtime execution. Global scalar shared conversion candidate is separately under review.
