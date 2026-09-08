#[test]
fn registry_native_addon_builds_and_runs_end_to_end() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-native-addon-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("native-add");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function add(a: number, b: number): number;\n\
         export declare function combine(first: (value: number) => number, value: number, second: (value: number) => number): number;\n\
         export declare function maybe(value: number, callback?: (value: number) => number): number;\n\
         export declare function variadic(callback: (...values: number[]) => number): number;\n\
         export declare function optionalParameter(callback: (value: number, extra?: number) => number): number;\n",
    )
    .unwrap();
    let addon_c = dir.join("addon.c");
    std::fs::write(&addon_c, r#"
            #include <stddef.h>
            typedef void* napi_env; typedef void* napi_value; typedef void* napi_callback_info;
            typedef int napi_status;
            extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t*, napi_value*, napi_value*, void**);
            extern napi_status napi_get_value_double(napi_env, napi_value, double*);
            extern napi_status napi_create_double(napi_env, double, napi_value*);
            extern napi_status napi_get_undefined(napi_env, napi_value*);
            extern napi_status napi_call_function(napi_env, napi_value, napi_value, size_t, const napi_value*, napi_value*);
            extern napi_status napi_create_function(napi_env, const char*, size_t, napi_value (*)(napi_env,napi_callback_info), void*, napi_value*);
            extern napi_status napi_set_named_property(napi_env, napi_value, const char*, napi_value);
            static napi_value add(napi_env env, napi_callback_info info) {
                size_t argc = 2; napi_value argv[2]; double a, b; napi_value result;
                napi_get_cb_info(env, info, &argc, argv, 0, 0);
                napi_get_value_double(env, argv[0], &a); napi_get_value_double(env, argv[1], &b);
                napi_create_double(env, a + b, &result); return result;
            }
            static napi_value combine(napi_env env, napi_callback_info info) {
                size_t argc = 3; napi_value argv[3], receiver, left, right, result; double a, b;
                napi_get_cb_info(env, info, &argc, argv, 0, 0);
                napi_get_undefined(env, &receiver);
                napi_call_function(env, receiver, argv[0], 1, &argv[1], &left);
                napi_call_function(env, receiver, argv[2], 1, &argv[1], &right);
                napi_get_value_double(env, left, &a); napi_get_value_double(env, right, &b);
                napi_create_double(env, a + b, &result); return result;
            }
            static napi_value maybe(napi_env env, napi_callback_info info) {
                size_t argc = 2; napi_value argv[2], receiver, result;
                napi_get_cb_info(env, info, &argc, argv, 0, 0);
                if (argc == 1) return argv[0];
                napi_get_undefined(env, &receiver);
                napi_call_function(env, receiver, argv[1], 1, argv, &result);
                return result;
            }
            static napi_value variadic(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value callback, receiver, values[3], result;
                napi_get_cb_info(env, info, &argc, &callback, 0, 0);
                napi_get_undefined(env, &receiver);
                napi_create_double(env, 1, &values[0]); napi_create_double(env, 2, &values[1]); napi_create_double(env, 3, &values[2]);
                napi_call_function(env, receiver, callback, 3, values, &result); return result;
            }
            static napi_value optional_parameter(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value callback, receiver, value, result;
                napi_get_cb_info(env, info, &argc, &callback, 0, 0);
                napi_get_undefined(env, &receiver); napi_create_double(env, 9, &value);
                napi_call_function(env, receiver, callback, 1, &value, &result); return result;
            }
            __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
                napi_value fn;
                napi_create_function(env, "add", 3, add, 0, &fn);
                napi_set_named_property(env, exports, "add", fn);
                napi_create_function(env, "combine", 7, combine, 0, &fn);
                napi_set_named_property(env, exports, "combine", fn);
                napi_create_function(env, "maybe", 5, maybe, 0, &fn);
                napi_set_named_property(env, exports, "maybe", fn);
                napi_create_function(env, "variadic", 8, variadic, 0, &fn);
                napi_set_named_property(env, exports, "variadic", fn);
                napi_create_function(env, "optionalParameter", 17, optional_parameter, 0, &fn);
                napi_set_named_property(env, exports, "optionalParameter", fn);
                return exports;
            }
        "#).unwrap();
    assert!(Command::new("cc")
        .args(["-shared", "-fPIC"])
        .arg(&addon_c)
        .arg("-o")
        .arg(package.join("native.node"))
        .status()
        .unwrap()
        .success());
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
        &source,
        "import { add, combine, maybe, optionalParameter, variadic } from \"native-add\"; function main(): void { console.log(add(20, 22)); console.log(combine((value: number): number => value + 1, 10, (value: number): number => value * 2)); console.log(maybe(7)); console.log(maybe(7, (value: number): number => value * 3)); console.log(variadic((...values: number[]): number => values[0] + values[1] + values[2])); console.log(optionalParameter((value: number): number => value + 1)); }\n",
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n31\n7\n21\n6\n10\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_javascript_wrapper_calls_bundled_native_addon() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-native-wrapper-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("native-wrapper");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function addOne(value: number): number;\nexport declare function boxed(value: number): number;\nexport declare function nativeCallback(value: number): number;\nexport declare function preservesCallbackIdentity(): boolean;\nexport declare function passesObject(): number;\nexport declare function mutatesObject(): number;\nexport declare function roundTripsObject(): number;\nexport declare function passesNativeHandle(value: number): number;\nexport declare function prototypeRoundTrip(value: number): number;\nexport declare function watchObject(): number;\nexport declare function collectObjects(): number;\nexport declare function finalizedObjects(): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "const native = require.addon(); native.Box.prototype.read = function() { return this.get(); }; native.Box.prototype.mark = function(value) { return value + 1; }; module.exports.addOne = value => native.add(value, 1); module.exports.boxed = value => new native.Box(value).read(); module.exports.nativeCallback = value => native.invoke(item => item * 2, value); module.exports.preservesCallbackIdentity = () => { const callback = () => {}; return native.same(callback, callback); }; module.exports.passesObject = () => { const object = { value: 42 }; return native.same(object, object) ? native.objectValue(object) : 0; }; module.exports.mutatesObject = () => { const object = { value: 1 }; native.mutate(object); return object.value; }; module.exports.roundTripsObject = () => { const object = { value: 1 }; native.objectValue(object); object.value = 9; return native.objectValue(object); }; module.exports.passesNativeHandle = value => new native.Box(new native.Box(value)).get(); module.exports.prototypeRoundTrip = value => new native.Box(value).self().callMark(); const watcher = new native.Box(0); module.exports.watchObject = () => { watcher.watch({ value: 1 }, function() { this.marked = 41; }); return 1; }; module.exports.collectObjects = async () => { for (let index = 0; index < 4; index++) { __thaw_gc(); await Promise.resolve(); } return 1; }; module.exports.finalizedObjects = () => watcher.finalized() + watcher.marked;",
    )
    .unwrap();
    let addon_c = dir.join("addon.c");
    std::fs::write(
        &addon_c,
        r#"
            #include <stddef.h>
            #include <stdlib.h>
            typedef void* napi_env; typedef void* napi_value; typedef void* napi_callback_info;
            typedef int napi_status;
            typedef napi_value (*napi_callback)(napi_env,napi_callback_info);
            typedef struct { const char* utf8name; napi_value name; napi_callback method; napi_callback getter; napi_callback setter; napi_value value; unsigned attributes; void* data; } napi_property_descriptor;
            typedef struct { double value; } box;
            extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t*, napi_value*, napi_value*, void**);
            extern napi_status napi_get_value_double(napi_env, napi_value, double*);
            extern napi_status napi_create_double(napi_env, double, napi_value*);
            extern napi_status napi_create_function(napi_env, const char*, size_t, napi_value (*)(napi_env,napi_callback_info), void*, napi_value*);
            extern napi_status napi_call_function(napi_env, napi_value, napi_value, size_t, const napi_value*, napi_value*);
            extern napi_status napi_get_undefined(napi_env, napi_value*);
            extern napi_status napi_get_boolean(napi_env, _Bool, napi_value*);
            extern napi_status napi_get_named_property(napi_env, napi_value, const char*, napi_value*);
            extern napi_status napi_set_named_property(napi_env, napi_value, const char*, napi_value);
            extern napi_status napi_define_class(napi_env, const char*, size_t, napi_callback, void*, size_t, const napi_property_descriptor*, napi_value*);
            extern napi_status napi_wrap(napi_env, napi_value, void*, void (*)(napi_env,void*,void*), void*, void**);
            extern napi_status napi_unwrap(napi_env, napi_value, void**);
            extern napi_status napi_add_finalizer(napi_env, napi_value, void*, void (*)(napi_env,void*,void*), void*, void**);
            static double finalized_objects; static napi_value watched_callback, watched_receiver;
            static void finalize_object(napi_env env, void* data, void* hint) {
                napi_value result; (void)data; (void)hint; finalized_objects++;
                napi_call_function(env, watched_receiver, watched_callback, 0, 0, &result);
            }
            static napi_value add(napi_env env, napi_callback_info info) {
                size_t argc = 2; napi_value argv[2], result; double left, right;
                napi_get_cb_info(env, info, &argc, argv, 0, 0);
                napi_get_value_double(env, argv[0], &left); napi_get_value_double(env, argv[1], &right);
                napi_create_double(env, left + right, &result); return result;
            }
            static napi_value invoke(napi_env env, napi_callback_info info) {
                size_t argc = 2; napi_value argv[2], receiver, result;
                napi_get_cb_info(env, info, &argc, argv, 0, 0);
                napi_get_undefined(env, &receiver);
                napi_call_function(env, receiver, argv[0], 1, &argv[1], &result); return result;
            }
            static napi_value same(napi_env env, napi_callback_info info) {
                size_t argc = 2; napi_value argv[2], result;
                napi_get_cb_info(env, info, &argc, argv, 0, 0);
                napi_get_boolean(env, argv[0] == argv[1], &result); return result;
            }
            static napi_value object_value(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value object, result;
                napi_get_cb_info(env, info, &argc, &object, 0, 0);
                napi_get_named_property(env, object, "value", &result); return result;
            }
            static napi_value mutate(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value object, value;
                napi_get_cb_info(env, info, &argc, &object, 0, 0);
                napi_create_double(env, 42, &value); napi_set_named_property(env, object, "value", value);
                return object;
            }
            static napi_value box_new(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, self; double value; box *source = 0, *data = malloc(sizeof(*data));
                napi_get_cb_info(env, info, &argc, &arg, &self, 0);
                if (napi_unwrap(env, arg, (void**)&source) == 0) value = source->value; else napi_get_value_double(env, arg, &value);
                data->value = value; napi_wrap(env, self, data, 0, 0, 0); return self;
            }
            static napi_value box_get(napi_env env, napi_callback_info info) {
                size_t argc = 0; napi_value self, result; box* data;
                napi_get_cb_info(env, info, &argc, 0, &self, 0); napi_unwrap(env, self, (void**)&data);
                napi_create_double(env, data->value, &result); return result;
            }
            static napi_value box_self(napi_env env, napi_callback_info info) {
                size_t argc = 0; napi_value self;
                napi_get_cb_info(env, info, &argc, 0, &self, 0); return self;
            }
            static napi_value box_call_mark(napi_env env, napi_callback_info info) {
                size_t argc = 0; napi_value self, mark, value, result;
                napi_get_cb_info(env, info, &argc, 0, &self, 0);
                napi_get_named_property(env, self, "mark", &mark);
                napi_create_double(env, 42, &value);
                napi_call_function(env, self, mark, 1, &value, &result); return result;
            }
            static napi_value box_watch(napi_env env, napi_callback_info info) {
                size_t argc = 2; napi_value args[2], result;
                napi_get_cb_info(env, info, &argc, args, &watched_receiver, 0);
                watched_callback = args[1];
                napi_add_finalizer(env, args[0], 0, finalize_object, 0, 0);
                napi_get_undefined(env, &result); return result;
            }
            static napi_value box_finalized(napi_env env, napi_callback_info info) {
                napi_value result; (void)info;
                napi_create_double(env, finalized_objects, &result); return result;
            }
            __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
                napi_value fn, constructor; napi_property_descriptor properties[5] = {
                    { "get", 0, box_get, 0, 0, 0, 0, 0 },
                    { "self", 0, box_self, 0, 0, 0, 0, 0 },
                    { "callMark", 0, box_call_mark, 0, 0, 0, 0, 0 },
                    { "watch", 0, box_watch, 0, 0, 0, 0, 0 },
                    { "finalized", 0, box_finalized, 0, 0, 0, 0, 0 }
                };
                napi_create_function(env, "add", 3, add, 0, &fn); napi_set_named_property(env, exports, "add", fn);
                napi_create_function(env, "invoke", 6, invoke, 0, &fn); napi_set_named_property(env, exports, "invoke", fn);
                napi_create_function(env, "same", 4, same, 0, &fn); napi_set_named_property(env, exports, "same", fn);
                napi_create_function(env, "objectValue", 11, object_value, 0, &fn); napi_set_named_property(env, exports, "objectValue", fn);
                napi_create_function(env, "mutate", 6, mutate, 0, &fn); napi_set_named_property(env, exports, "mutate", fn);
                napi_define_class(env, "Box", 3, box_new, 0, 5, properties, &constructor);
                napi_set_named_property(env, exports, "Box", constructor); return exports;
            }
        "#,
    )
    .unwrap();
    assert!(Command::new("cc")
        .args(["-shared", "-fPIC"])
        .arg(&addon_c)
        .arg("-o")
        .arg(package.join("native.node"))
        .status()
        .unwrap()
        .success());
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
        &source,
        "import { addOne, boxed, nativeCallback, preservesCallbackIdentity, passesObject, mutatesObject, roundTripsObject, passesNativeHandle, prototypeRoundTrip, watchObject, collectObjects, finalizedObjects } from \"native-wrapper\"; function main(): void { console.log(addOne(41)); console.log(boxed(42)); console.log(nativeCallback(21)); console.log(preservesCallbackIdentity()); console.log(passesObject()); console.log(mutatesObject()); console.log(roundTripsObject()); console.log(passesNativeHandle(42)); console.log(prototypeRoundTrip(42)); watchObject(); collectObjects(); collectObjects(); console.log(finalizedObjects()); }\n",
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "42\n42\n42\ntrue\n42\n42\n9\n42\n43\n42\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_native_addon_returned_function_runs_end_to_end() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-native-factory-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("native-factory");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function multiplier(factor: number): (value: number) => number;\nexport declare function retain(callback: (value: number) => number): (value: number) => number;\n",
    )
    .unwrap();
    let addon_c = dir.join("addon.c");
    std::fs::write(
        &addon_c,
        r#"
            #include <stddef.h>
            typedef void* napi_env; typedef void* napi_value; typedef void* napi_callback_info;
            typedef int napi_status;
            static double factor;
            extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t*, napi_value*, napi_value*, void**);
            extern napi_status napi_get_value_double(napi_env, napi_value, double*);
            extern napi_status napi_create_double(napi_env, double, napi_value*);
            extern napi_status napi_create_function(napi_env, const char*, size_t, napi_value (*)(napi_env,napi_callback_info), void*, napi_value*);
            extern napi_status napi_set_named_property(napi_env, napi_value, const char*, napi_value);
            static napi_value multiply(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, result; double value; void* data;
                napi_get_cb_info(env, info, &argc, &arg, 0, &data);
                napi_get_value_double(env, arg, &value);
                napi_create_double(env, value * *(double*)data, &result); return result;
            }
            static napi_value multiplier(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, result;
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_get_value_double(env, arg, &factor);
                napi_create_function(env, "multiply", 8, multiply, &factor, &result); return result;
            }
            static napi_value retain(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value callback;
                napi_get_cb_info(env, info, &argc, &callback, 0, 0); return callback;
            }
            __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
                napi_value fn;
                napi_create_function(env, "multiplier", 10, multiplier, 0, &fn);
                napi_set_named_property(env, exports, "multiplier", fn);
                napi_create_function(env, "retain", 6, retain, 0, &fn);
                napi_set_named_property(env, exports, "retain", fn); return exports;
            }
        "#,
    )
    .unwrap();
    assert!(Command::new("cc")
        .args(["-shared", "-fPIC"])
        .arg(&addon_c)
        .arg("-o")
        .arg(package.join("native.node"))
        .status()
        .unwrap()
        .success());
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
        &source,
        "import { multiplier, retain } from \"native-factory\"; function double(value: number): number { return value * 2; } function main(): void { const triple = multiplier(3); console.log(triple(14)); const retained = retain(double); console.log(retained(21)); }\n",
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_native_addon_class_method_builds_and_runs_end_to_end() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-native-addon-class-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("native-box");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
            package.join("package.d.ts"),
            "export declare class NativeBox { constructor(value: number); constructor(value: string | undefined); static twice(value: number): number; static get version(): number; static set version(value: number); get value(): number; set value(value: number); get optional(): string | undefined; set optional(value: string | undefined); get(): number; add(delta?: number): number; sum(...values: number[]): number; isUndefined(value: string | undefined): boolean; returnUndefined(): string | undefined; echoNullish(value: string | null | undefined): string | null | undefined; echoOptionalArray(value: (string | undefined)[]): (string | undefined)[]; echoOptionalTuple(value: [string, string | undefined]): [string, string | undefined]; hasUndefinedProperty(value: { value?: string }): boolean; getLater(callback: (error: Json, result: Json) => void): number; }\n",
        )
        .unwrap();
    let addon_c = dir.join("addon.c");
    std::fs::write(
            &addon_c,
            r#"
            #include <stddef.h>
            #include <stdlib.h>
            typedef void* napi_env; typedef void* napi_value; typedef void* napi_callback_info;
            typedef int napi_status;
            typedef enum { napi_undefined = 0, napi_null = 1, napi_boolean = 2, napi_number = 3, napi_string = 4, napi_symbol = 5, napi_object = 6, napi_function = 7, napi_external = 8, napi_bigint = 9 } napi_valuetype;
            typedef napi_value (*napi_callback)(napi_env,napi_callback_info);
            typedef struct { const char* utf8name; napi_value name; napi_callback method; napi_callback getter; napi_callback setter; napi_value value; unsigned attributes; void* data; } napi_property_descriptor;
            typedef struct { double value; } native_box;
            static double box_version = 1;
            extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t*, napi_value*, napi_value*, void**);
            extern napi_status napi_get_value_double(napi_env, napi_value, double*);
            extern napi_status napi_create_double(napi_env, double, napi_value*);
            extern napi_status napi_get_null(napi_env, napi_value*);
            extern napi_status napi_get_undefined(napi_env, napi_value*);
            extern napi_status napi_get_boolean(napi_env, _Bool, napi_value*);
            extern napi_status napi_typeof(napi_env, napi_value, napi_valuetype*);
            extern napi_status napi_create_string_utf8(napi_env, const char*, size_t, napi_value*);
            extern napi_status napi_has_own_property(napi_env, napi_value, napi_value, _Bool*);
            extern napi_status napi_get_named_property(napi_env, napi_value, const char*, napi_value*);
            extern napi_status napi_call_function(napi_env, napi_value, napi_value, size_t, const napi_value*, napi_value*);
            extern napi_status napi_set_named_property(napi_env, napi_value, const char*, napi_value);
            extern napi_status napi_wrap(napi_env, napi_value, void*, void (*)(napi_env,void*,void*), void*, void**);
            extern napi_status napi_unwrap(napi_env, napi_value, void**);
            extern napi_status napi_define_class(napi_env, const char*, size_t, napi_callback, void*, size_t, const napi_property_descriptor*, napi_value*);
            static void finalize_box(napi_env env, void* data, void* hint) { (void)env; (void)hint; free(data); }
            static napi_value box_new(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, self; double value = -1; napi_valuetype type;
                napi_get_cb_info(env, info, &argc, &arg, &self, 0);
                napi_typeof(env, arg, &type); if (type != napi_undefined) napi_get_value_double(env, arg, &value);
                native_box* box = malloc(sizeof(*box)); box->value = value;
                napi_wrap(env, self, box, finalize_box, 0, 0); return self;
            }
            static napi_value box_get(napi_env env, napi_callback_info info) {
                size_t argc = 0; napi_value self, result; native_box* box;
                napi_get_cb_info(env, info, &argc, 0, &self, 0);
                napi_unwrap(env, self, (void**)&box);
                napi_create_double(env, box->value, &result); return result;
            }
            static napi_value box_twice(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, result; double value;
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_get_value_double(env, arg, &value);
                napi_create_double(env, value * 2, &result); return result;
            }
            static napi_value box_set(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, self; native_box* box; double value;
                napi_get_cb_info(env, info, &argc, &arg, &self, 0);
                napi_unwrap(env, self, (void**)&box);
                napi_get_value_double(env, arg, &value); box->value = value; return arg;
            }
            static napi_value box_get_version(napi_env env, napi_callback_info info) {
                napi_value result; (void)info;
                napi_create_double(env, box_version, &result); return result;
            }
            static napi_value box_set_version(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg; double value;
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_get_value_double(env, arg, &value); box_version = value; return arg;
            }
            static napi_value box_add(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, self, result; native_box* box; double delta = 0;
                napi_get_cb_info(env, info, &argc, &arg, &self, 0);
                napi_unwrap(env, self, (void**)&box);
                if (argc == 1) napi_get_value_double(env, arg, &delta);
                napi_create_double(env, box->value + delta, &result); return result;
            }
            static napi_value box_sum(napi_env env, napi_callback_info info) {
                size_t argc = 8; napi_value args[8], self, result; double sum = 0, value;
                napi_get_cb_info(env, info, &argc, args, &self, 0);
                for (size_t index = 0; index < argc; index++) {
                    napi_get_value_double(env, args[index], &value); sum += value;
                }
                napi_create_double(env, sum, &result); return result;
            }
            static napi_value box_is_undefined(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, result; napi_valuetype type;
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_typeof(env, arg, &type); napi_get_boolean(env, type == napi_undefined, &result); return result;
            }
            static napi_value box_return_undefined(napi_env env, napi_callback_info info) {
                (void)info; napi_value result; napi_get_undefined(env, &result); return result;
            }
            static napi_value box_echo_nullish(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg; napi_get_cb_info(env, info, &argc, &arg, 0, 0); return arg;
            }
            static napi_value box_has_undefined_property(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, key, property, result; _Bool has = 0; napi_valuetype type = napi_null;
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_create_string_utf8(env, "value", 5, &key);
                napi_has_own_property(env, arg, key, &has);
                if (has) { napi_get_named_property(env, arg, "value", &property); napi_typeof(env, property, &type); }
                napi_get_boolean(env, has && type == napi_undefined, &result); return result;
            }
            static napi_value box_get_later(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value callback, self, callback_args[2], ignored, queued; native_box* box;
                napi_get_cb_info(env, info, &argc, &callback, &self, 0);
                napi_unwrap(env, self, (void**)&box);
                napi_get_null(env, &callback_args[0]);
                napi_create_double(env, box->value, &callback_args[1]);
                napi_call_function(env, self, callback, 2, callback_args, &ignored);
                napi_create_double(env, 1, &queued); return queued;
            }
            __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
                napi_value constructor;
                napi_property_descriptor properties[14] = {
                    { "get", 0, box_get, 0, 0, 0, 0, 0 },
                    { "value", 0, 0, box_get, box_set, 0, 0, 0 },
                    { "add", 0, box_add, 0, 0, 0, 0, 0 },
                    { "sum", 0, box_sum, 0, 0, 0, 0, 0 },
                    { "getLater", 0, box_get_later, 0, 0, 0, 0, 0 },
                    { "isUndefined", 0, box_is_undefined, 0, 0, 0, 0, 0 },
                    { "returnUndefined", 0, box_return_undefined, 0, 0, 0, 0, 0 },
                    { "echoNullish", 0, box_echo_nullish, 0, 0, 0, 0, 0 },
                    { "echoOptionalArray", 0, box_echo_nullish, 0, 0, 0, 0, 0 },
                    { "echoOptionalTuple", 0, box_echo_nullish, 0, 0, 0, 0, 0 },
                    { "hasUndefinedProperty", 0, box_has_undefined_property, 0, 0, 0, 0, 0 },
                    { "optional", 0, 0, box_return_undefined, box_echo_nullish, 0, 0, 0 },
                    { "twice", 0, box_twice, 0, 0, 0, 1024, 0 },
                    { "version", 0, 0, box_get_version, box_set_version, 0, 1024, 0 }
                };
                napi_define_class(env, "NativeBox", 9, box_new, 0, 14, properties, &constructor);
                napi_set_named_property(env, exports, "NativeBox", constructor);
                return exports;
            }
        "#,
        )
        .unwrap();
    assert!(Command::new("cc")
        .args(["-shared", "-fPIC"])
        .arg(&addon_c)
        .arg("-o")
        .arg(package.join("native.node"))
        .status()
        .unwrap()
        .success());
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            "import { NativeBox } from \"native-box\"; function main(): void { console.log(NativeBox.twice(21)); console.log(NativeBox.version); console.log(NativeBox.version = 3); console.log(NativeBox.version); const box: JsValue = new NativeBox(42); console.log(box.value); console.log(box.value = 10); console.log(box.value); console.log(box.get()); console.log(box.add()); console.log(box.add(8)); console.log(box.sum()); console.log(box.sum(1, 2, 3)); console.log(box.isUndefined(undefined)); console.log(box.returnUndefined() === undefined); console.log(box.echoNullish(null) === null); console.log(box.echoNullish(undefined) === undefined); console.log(box.optional === undefined); console.log((box.optional = undefined) === undefined); console.log(box.echoOptionalArray([\"x\", undefined])[1] === undefined); const tuple: [string, string | undefined] = [\"x\", undefined]; const echoed = box.echoOptionalTuple(tuple); console.log(echoed[1] === undefined); console.log(box.hasUndefinedProperty({ value: undefined })); const optionalBox: JsValue = new NativeBox(undefined); console.log(optionalBox.get()); const callback = (error: Json, result: Json): void => { console.log(Number(result)); }; console.log(box.getLater(callback)); }\n",
        )
        .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "42\n1\n3\n3\n42\n10\n10\n10\n10\n18\n0\n6\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n-1\n10\n1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

