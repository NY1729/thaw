#[test]
fn registry_native_addon_builds_and_runs_end_to_end() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-native-addon-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("native-add");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function add(a: number, b: number): number;\n",
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
            extern napi_status napi_create_function(napi_env, const char*, size_t, napi_value (*)(napi_env,napi_callback_info), void*, napi_value*);
            static napi_value add(napi_env env, napi_callback_info info) {
                size_t argc = 2; napi_value argv[2]; double a, b; napi_value result;
                napi_get_cb_info(env, info, &argc, argv, 0, 0);
                napi_get_value_double(env, argv[0], &a); napi_get_value_double(env, argv[1], &b);
                napi_create_double(env, a + b, &result); return result;
            }
            __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
                (void)exports;
                napi_value fn; napi_create_function(env, "add", 3, add, 0, &fn);
                return fn;
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
        "import { add } from \"native-add\"; function main(): void { console.log(add(20, 22)); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
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
        "export declare function addOne(value: number): number;\nexport declare function boxed(value: number): number;\nexport declare function nativeCallback(value: number): number;\nexport declare function preservesCallbackIdentity(): boolean;\nexport declare function passesObject(): number;\nexport declare function passesNativeHandle(value: number): number;\nexport declare function watchObject(): number;\nexport declare function collectObjects(): number;\nexport declare function finalizedObjects(): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "const native = require.addon(); native.Box.prototype.read = function() { return this.get(); }; module.exports.addOne = value => native.add(value, 1); module.exports.boxed = value => new native.Box(value).read(); module.exports.nativeCallback = value => native.invoke(item => item * 2, value); module.exports.preservesCallbackIdentity = () => { const callback = () => {}; return native.same(callback, callback); }; module.exports.passesObject = () => { const object = { value: 42 }; return native.same(object, object) ? native.objectValue(object) : 0; }; module.exports.passesNativeHandle = value => new native.Box(new native.Box(value)).get(); const watcher = new native.Box(0); module.exports.watchObject = () => { watcher.watch({ value: 1 }, function() { this.marked = 41; }); return 1; }; module.exports.collectObjects = async () => { for (let index = 0; index < 4; index++) { __thaw_gc(); await Promise.resolve(); } return 1; }; module.exports.finalizedObjects = () => watcher.finalized() + watcher.marked;",
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
                napi_value fn, constructor; napi_property_descriptor properties[3] = {
                    { "get", 0, box_get, 0, 0, 0, 0, 0 },
                    { "watch", 0, box_watch, 0, 0, 0, 0, 0 },
                    { "finalized", 0, box_finalized, 0, 0, 0, 0, 0 }
                };
                napi_create_function(env, "add", 3, add, 0, &fn); napi_set_named_property(env, exports, "add", fn);
                napi_create_function(env, "invoke", 6, invoke, 0, &fn); napi_set_named_property(env, exports, "invoke", fn);
                napi_create_function(env, "same", 4, same, 0, &fn); napi_set_named_property(env, exports, "same", fn);
                napi_create_function(env, "objectValue", 11, object_value, 0, &fn); napi_set_named_property(env, exports, "objectValue", fn);
                napi_define_class(env, "Box", 3, box_new, 0, 3, properties, &constructor);
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
        "import { addOne, boxed, nativeCallback, preservesCallbackIdentity, passesObject, passesNativeHandle, watchObject, collectObjects, finalizedObjects } from \"native-wrapper\"; function main(): void { console.log(addOne(41)); console.log(boxed(42)); console.log(nativeCallback(21)); console.log(preservesCallbackIdentity()); console.log(passesObject()); console.log(passesNativeHandle(42)); watchObject(); collectObjects(); collectObjects(); console.log(finalizedObjects()); }\n",
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
        "42\n42\n42\ntrue\n42\n42\n42\n"
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

#[test]
fn registry_runs_utf8_validate_prebuild_when_supplied() {
    let Ok(prebuild) = std::env::var("THAW_UTF8_VALIDATE_NODE") else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("thaw-cli-utf8-validate-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("utf-8-validate");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "declare function isValidUTF8(buffer: any): boolean;\n",
    )
    .unwrap();
    std::fs::copy(prebuild, package.join("native.node")).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            r#"function main(): void {
                console.log(Boolean(isValidUTF8(JSON.parse("[{\"type\":\"Buffer\",\"data\":[240,144,128,128]}]"))));
                console.log(Boolean(isValidUTF8(JSON.parse("[{\"type\":\"Buffer\",\"data\":[255]}]"))));
            }
            "#,
        )
        .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["utf-8-validate".into()],
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "true\nfalse\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_runs_bcrypt_async_callbacks_when_supplied() {
    let Ok(prebuild) = std::env::var("THAW_BCRYPT_NODE") else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("thaw-cli-bcrypt-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("bcrypt");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "declare function gen_salt(argsArray: Json): Json;\n\
             declare function encrypt(argsArray: Json): Json;\n\
             declare function compare(argsArray: Json): Json;\n",
    )
    .unwrap();
    std::fs::copy(prebuild, package.join("native.node")).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            r#"function main(): void {
                const saltQueued = callNativeAddonWithCallback(
                    "gen_salt",
                    JSON.parse("[\"b\",4,{\"type\":\"Buffer\",\"data\":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]}]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const encryptQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const compareQueued = callNativeAddonWithCallback(
                    "compare",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(Boolean(result));
                        return result;
                    }
                );
                const invalidQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"invalid\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(error));
                        return error;
                    }
                );
            }"#,
        )
        .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["bcrypt".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.lines().any(|line| line.starts_with("$2b$04$")),
        "{stdout}"
    );
    assert!(stdout.lines().any(|line| line == "false"), "{stdout}");
    assert!(
        stdout.lines().any(|line| line.contains("Invalid salt")),
        "{stdout}"
    );
    assert_eq!(stdout.lines().count(), 4, "{stdout}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_fetches_and_runs_bcrypt_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-bcrypt-{}", std::process::id()));
    let registry = dir.join("modules");
    let added = thaw_registry::add(&registry, "bcrypt@6.0.0").unwrap();
    let native = added.native_addon.expect("a matching prebuild is bundled");
    assert_eq!(native.platform, "linux");
    assert_eq!(native.arch, "x64");
    assert_eq!(native.libc, "glibc");
    assert!(registry.join("bcrypt/native.node").is_file());
    assert!(registry.join("bcrypt/native-addon.json").is_file());

    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            r#"function main(): void {
                const salt: Json = callNativeAddon(
                    "gen_salt_sync",
                    JSON.parse("[\"b\",4,{\"type\":\"Buffer\",\"data\":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]}]")
                );
                console.log(String(salt));
                const hash: Json = callNativeAddon(
                    "encrypt_sync",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]")
                );
                console.log(String(hash));
                console.log(Boolean(callNativeAddon(
                    "compare_sync",
                    JSON.parse("[\"wrong-password\",\"$2b$04$abcdefghijklmnopqrstuu\"]")
                )));
                const saltQueued = callNativeAddonWithCallback(
                    "gen_salt",
                    JSON.parse("[\"b\",4,{\"type\":\"Buffer\",\"data\":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]}]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const encryptQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const compareQueued = callNativeAddonWithCallback(
                    "compare",
                    JSON.parse("[\"wrong-password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(Boolean(result));
                        return result;
                    }
                );
                const invalidQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"invalid\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(error));
                        return error;
                    }
                );
            }"#,
        )
        .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["bcrypt".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 7, "{stdout}");
    assert!(lines[0].starts_with("$2b$04$"), "{stdout}");
    assert!(lines[1].starts_with("$2b$04$"), "{stdout}");
    assert_eq!(lines[2], "false", "{stdout}");
    assert!(
        lines[3..].iter().any(|line| line.starts_with("$2b$04$")),
        "{stdout}"
    );
    assert!(lines[3..].contains(&"false"), "{stdout}");
    assert!(
        lines[3..].iter().any(|line| line.contains("Invalid salt")),
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_fetches_and_loads_sqlite3_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-sqlite3-{}", std::process::id()));
    let registry = dir.join("modules");
    let added = thaw_registry::add(&registry, "sqlite3@5.1.7").unwrap();
    let native = added
        .native_addon
        .expect("the official GitHub prebuild should be downloaded");
    assert_eq!(native.platform, "linux");
    assert_eq!(native.arch, "x64");
    assert_eq!(native.libc, "glibc");
    assert!(native.source.contains("TryGhost/node-sqlite3/releases"));

    // Keep this fixture focused on construction and one error-first
    // callback method rather than sqlite3's full overloaded surface.
    std::fs::write(
            registry.join("sqlite3/package.d.ts"),
            "export declare class Database { constructor(filename: string); close(callback: (error: Error | null) => void): void; }\n",
        )
        .unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
        &source,
        "import { Database } from \"sqlite3\";\n\
             function main(): void {\n\
                 const database: JsValue = new Database(\":memory:\");\n\
                 const onClose = (error: Json): void => { console.log(\"sqlite3-closed\"); };\n\
                 database.close(onClose);\n\
                 console.log(\"sqlite3-constructed\");\n\
             }\n",
    )
    .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["sqlite3".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "sqlite3-constructed\nsqlite3-closed\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_fetches_and_runs_utf8_validate_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-utf8-validate-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let added = thaw_registry::add(&registry, "utf-8-validate@6.0.6").unwrap();
    let native = added.native_addon.expect("a matching prebuild is bundled");
    assert_eq!(native.platform, "linux");
    assert_eq!(native.arch, "x64");
    assert_eq!(native.libc, "glibc");
    assert!(registry.join("utf-8-validate/native.node").is_file());
    assert!(registry.join("utf-8-validate/native-addon.json").is_file());

    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            r#"function main(): void {
                console.log(Boolean(isValidUTF8(JSON.parse("[{\"type\":\"Buffer\",\"data\":[240,144,128,128]}]"))));
                console.log(Boolean(isValidUTF8(JSON.parse("[{\"type\":\"Buffer\",\"data\":[255]}]"))));
            }
            "#,
        )
        .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["utf-8-validate".into()],
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "true\nfalse\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_runs_real_p_limit_promise_workload_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-p-limit-{}", std::process::id()));
    let registry = dir.join("modules");
    let added = thaw_registry::add(&registry, "p-limit@2.3.0").unwrap();
    assert_eq!(
        added.dependency_versions.get("p-try").map(String::as_str),
        Some("2.2.0")
    );
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import pLimit from "p-limit";
                function main(): void {
                    const limit: JsValue = pLimit(2);
                    const task: JsValue = getDynamicValue("Number");
                    const result: Json = callDynamicValueWithValue(limit, task);
                    console.log(Number(result));
                }"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "0\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_builds_and_runs_a_real_esm_package_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-has-flag-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "has-flag@5.0.1").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import hasFlag from "has-flag";
                function main(): void {
                    const present: Json = hasFlag(JSON.parse("[\"--thaw-parser-backed-bundler\"]"));
                    console.log(Boolean(present));
                }"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "false\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_builds_and_runs_yaml_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-yaml-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "yaml@2.8.1").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { parse, stringify } from "yaml";
                function main(): void {
                    const args: Json = JSON.parse("[\"name: thaw\\nitems:\\n  - 20\\n  - 22\\n\"]");
                    const value: Json = parse(args);
                    console.log(String(value.name));
                    console.log(Number(value.items[0]) + Number(value.items[1]));
                    const output: Json = stringify(JSON.parse("[{\"enabled\":true}]"));
                    console.log(String(output));
                }"#,
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
        "thaw\n42\nenabled: true\n\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_builds_and_runs_date_fns_root_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-date-fns-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "date-fns@4.1.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { weeksToDays } from "date-fns";
                function main(): void {
                    console.log(weeksToDays(6));
                }"#,
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_builds_and_runs_nanoid_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-nanoid-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "nanoid@5.1.5").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { customAlphabet, nanoid } from "nanoid";
                function main(): void {
                    const first: string = nanoid();
                    const second: string = nanoid(12);
                    const makeId = customAlphabet("ab", 8);
                    const third: string = makeId();
                    const fourth: string = makeId(5);
                    const defaultMaker = customAlphabet("ab");
                    const fifth: string = defaultMaker();
                    console.log(first.length);
                    console.log(second.length);
                    console.log(first === second);
                    console.log(third.length);
                    console.log(fourth.length);
                    console.log(fifth.length);
                }"#,
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
        "21\n12\nfalse\n8\n5\n21\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_fetches_and_runs_parcel_watcher_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-parcel-watcher-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let watched = dir.join("watched");
    let snapshot = dir.join("snapshot.bin");
    std::fs::create_dir_all(&watched).unwrap();
    std::fs::write(watched.join("before.txt"), "before").unwrap();
    let added = thaw_registry::add(&registry, "@parcel/watcher@2.5.1").unwrap();
    let native = added
        .native_addon
        .expect("the platform optional dependency contains a prebuild");
    assert_eq!(native.platform, "linux");
    assert_eq!(native.arch, "x64");
    assert_eq!(native.libc, "glibc");
    assert!(native
        .source
        .contains("watcher-linux-x64-glibc/watcher.node"));

    let args = serde_json::to_string(&serde_json::json!([
        watched.to_string_lossy(),
        snapshot.to_string_lossy(),
        {}
    ]))
    .unwrap();
    let args_literal = serde_json::to_string(&args).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            format!(
                "function main(): void {{ const result: Json = writeSnapshot(JSON.parse({args_literal})); console.log(\"snapshot-created\"); }}\n"
            ),
        )
        .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["@parcel/watcher".into()],
    )
    .unwrap();
    let event_file = watched.join("event.txt");
    let subscribe_args =
        serde_json::to_string(&serde_json::json!([watched.to_string_lossy()])).unwrap();
    let subscribe_literal = serde_json::to_string(&subscribe_args).unwrap();
    let event_source = dir.join("events.ts");
    let event_output = dir.join("events-app");
    std::fs::write(
            &event_source,
            format!(
                r#"import * as fs from "node:fs";
                function main(): void {{
                    let received: number = 0;
                    const callback = (error: Json, result: Json): Json => {{
                        received = 1;
                        return result;
                    }};
                    const subscribed: Json = callNativeAddonWithCallback("subscribe", JSON.parse({subscribe_literal}), callback);
                    fs.writeFileSync("{}", "event");
                    while (received < 1) {{
                        const count: number = pollNativeAddonEvents();
                    }}
                    const unsubscribed: Json = callNativeAddonWithCallback("unsubscribe", JSON.parse({subscribe_literal}), callback);
                    console.log("watch-event");
                }}
                "#,
                event_file.to_string_lossy()
            ),
        )
        .unwrap();
    build(
        &event_source,
        &event_output,
        &[],
        &[],
        &[],
        &registry,
        &["@parcel/watcher".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "snapshot-created\n"
    );
    assert!(snapshot.is_file());
    let result = Command::new(&event_output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "watch-event\n");
    assert!(event_file.is_file());
    let _ = std::fs::remove_dir_all(dir);
}

/// One of the Phase-4 validation targets (see [[project_thaw_overview]]):
/// a real, `thaw registry add`-fetched zod builds and validates schemas
/// end-to-end, including a fully inline method chain
/// (`z.string().min(2).max(10).safeParse(...)`, no intermediate binding
/// at all) -- exercises `a_method_can_be_chained_directly_onto_another_
/// methods_call_result`'s fix against the real package, not just a
/// synthetic double. Pins down the whole "make zod work" arc from
/// [[project_npm_interop_gaps_2]] as a permanent regression test instead
/// of leaving it as one-off manual verification nobody would notice
/// regress.
///
/// Also covers `.refine()`/`.transform()` -- a real compiled (native)
/// closure passed as a dynamic-call argument -- see thaw-cli's own
/// `a_native_closure_can_be_passed_as_an_argument_to_a_dynamic_method_call`
/// for the synthetic, network-free version of the same mechanism -- and
/// `.pipe(z.string().min(3))`, a dynamic-call argument that's itself a
/// method call chained off a `JsValue` receiver with no intermediate
/// binding at all (see `a_dynamic_call_argument_that_is_itself_a_
/// chained_method_call_stays_live` for the synthetic version) -- and
/// `.superRefine((val, ctx) => { ctx.addIssue(...); })`, a `JsValue`-
/// typed callback parameter whose own method call reenters the dynamic-
/// call machinery from inside a native callback (see thaw-cli's own
/// `a_superrefine_shaped_native_callback_with_a_jsvalue_context_
/// parameter_works` for the synthetic version, and its own doc comment
/// for the four gaps this exercises together).
#[test]
fn registry_add_validates_zod_schemas_end_to_end_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-zod-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "zod").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import * as z from "zod";
function main(): void {
    console.log(z.string().min(2).max(10).safeParse("hello").success);
    console.log(z.string().min(2).max(10).safeParse("h").success);
    const schema = z.object({ name: z.string(), age: z.number() });
    console.log(schema.safeParse({ name: "Alice", age: 30 }).success);
    console.log(schema.safeParse({ name: "Alice", age: "thirty" }).success);
    console.log(z.number().refine((n: number) => n > 0, { message: "must be positive" }).safeParse(5).success);
    console.log(z.number().refine((n: number) => n > 0, { message: "must be positive" }).safeParse(-5).success);
    console.log(JSON.stringify(z.string().transform((s: string) => s.length).parse("hello")));
    console.log(z.string().pipe(z.string().min(3)).safeParse("hi").success);
    console.log(z.string().pipe(z.string().min(3)).safeParse("hello").success);
    const withCtx: JsValue = z.object({ a: z.number(), b: z.number() }).superRefine((val: Json, ctx: JsValue) => {
        if (Number(val.a) > Number(val.b)) {
            ctx.addIssue({ code: "custom", message: "a must be <= b" });
        }
    });
    console.log(withCtx.safeParse({ a: 1, b: 2 }).success);
    console.log(withCtx.safeParse({ a: 3, b: 2 }).success);
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["zod".to_string()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "true\nfalse\ntrue\nfalse\ntrue\nfalse\n5\nfalse\ntrue\ntrue\nfalse\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A real, fetched dayjs -- another Phase-4 validation target -- parses
/// and formats dates through both a single-hop chain (`dayjs(date).
/// format(...)`) and a double-hop one (`dayjs(date).add(...).format(
/// ...)`), fully inline with no intermediate `const` anywhere. Both
/// used to fail to build ("unsupported member call target") before this
/// session's two chained-call fixes; only UTC-based formatting is
/// asserted (never a local-time token) so this doesn't depend on the
/// host's time zone.
#[test]
fn registry_add_formats_dates_with_dayjs_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-dayjs-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "dayjs").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import dayjs from "dayjs";
function main(): void {
    console.log(dayjs("2024-01-15T00:00:00Z").format("YYYY-MM-DD"));
    console.log(dayjs("2024-01-15T00:00:00Z").add(10, "day").format("YYYY-MM-DD"));
}"#,
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
        "\"2024-01-15\"\n\"2024-01-25\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A real, fetched hono -- the last of the Phase-4 validation targets
/// exercised this session -- constructs its `Hono` class via `new`.
/// Pins the constructor half of [[project_npm_interop_gaps_2]]'s hono
/// arc (commit `9addc760`) against the real package. Deliberately scoped
/// to construction only, not route registration -- see `registry_add_
/// routes_and_serves_a_real_hono_app_when_enabled` below for that half,
/// which turned out not to need the separate callable-interface-typing
/// feature this comment used to describe: hono's real, flattened
/// `package.d.ts` never actually extracts `HonoBase`'s own methods
/// (`.get`/`.post`/`.fetch`/`.request`), so every call to them already
/// goes through the untyped/dynamic dispatch path built for zod, which
/// doesn't care what `Handler`'s own declared type is at all.
#[test]
fn registry_add_constructs_a_hono_app_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-hono-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "hono").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { Hono } from "hono";
function main(): void {
    const app = new Hono();
    console.log("hono app created");
}"#,
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
        "hono app created\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// The route-registration half of the real hono arc left unattempted by
/// the constructor-only test above: `app.get(path, handler)` followed by
/// `app.request(path)` (hono's own synchronous testing helper) reading
/// `.status`/`.text()` off the resulting real `Response`.
///
/// This used to read `.status` back as `undefined` with no error at all
/// -- the handler's `return c.text(...)` silently lowered its dynamic
/// method call through the untyped/JSON-decoding dispatch instead of
/// keeping the real handle, so hono's router received a content-free
/// `{}` snapshot in place of the actual `Response` object. See the
/// synthetic, network-free reproduction and full root-cause writeup at
/// `a_dynamic_callback_argument_returning_a_jsvalue_keeps_it_live_even_
/// when_unannotated` in `registry_modules.rs` -- this test just confirms
/// the same fix holds against the real, unmodified npm package.
///
/// Exercises both an unannotated block-body handler and an unannotated
/// implicit-return expression-body handler, matching how real hono
/// handlers are actually written (`Handler`'s own declared return type
/// is never surfaced through the flattened `.d.ts`, so nobody writing
/// real hono code annotates a handler's return type at all). Also reads
/// the response body back as a plain `string` (`await res.text()`, not
/// `JsValue`) -- a separate, narrower gap noted while finishing this
/// arc and fixed alongside it: `coerce_to_declared` had no path for
/// decoding a dynamic call's default `Json` result into a declared
/// scalar type at all (see `a_dynamic_method_calls_json_result_decodes_
/// into_a_declared_scalar_type` in `registry_modules.rs` for the
/// synthetic, network-free reproduction).
#[test]
fn registry_add_routes_and_serves_a_real_hono_app_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-hono-route-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "hono").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { Hono, Context } from "hono";
async function main(): Promise<void> {
    const appA = new Hono();
    appA.get('/', (c: Context) => { return c.text('hello world'); });
    const resA: JsValue = appA.request('/');
    console.log(resA.status);

    const appB = new Hono();
    appB.get('/', (c: JsValue) => c.text('hello again'));
    const resB: JsValue = appB.request('/');
    console.log(resB.status);
    const bodyB: string = await resB.text();
    console.log(bodyB);
}"#,
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
        "200\n200\nhello again\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Real, fetched uuid -- the first breadth-first bug-hunting target after
/// zod/dayjs/hono all closed out (per [[project_thaw_overview]]'s Phase-4
/// roadmap). `v4`/`v1`/`v3`/`v5`/`v6`/`v7`/`validate`/`version` all build
/// and run correctly with no changes needed (both real and synthetic
/// `v3`/`v5` outputs were cross-checked against a direct `node` run of
/// uuid's own bundle, byte-for-byte identical).
///
/// `parse(id)` (returns `NonSharedArrayBuffer`) piped straight into
/// `stringify(bytes)` (parameter `Uint8Array`, second parameter `offset`
/// omitted) crashed LLVM's own module verifier outright -- both
/// unresolved reference types classify the same "unclassified npm type"
/// way thaw-bridge falls back to for real, but land on *opposite* sides
/// of the return-value/parameter divide (`JsValue` vs `Json`), and
/// `stringify`'s own omitted-second-parameter dispatch routes the call
/// through an *ordinary* compiled function call, not a dynamic-call
/// intrinsic -- see the synthetic, network-free reproduction and full
/// root-cause writeup at `a_jsvalue_returning_functions_result_passed_
/// into_another_functions_json_parameter_works` in `registry_modules.rs`.
/// This test just confirms the same fix holds against the real,
/// unmodified npm package.
///
/// One known, deliberately-unattempted gap found along the way, scoped
/// as a separate, larger feature rather than a narrow bug: uuid's
/// `NIL`/`MAX` constant exports (`export { default as NIL } from
/// './nil.js'`, where the target file's default export is a bare
/// `declare const`, not a function) -- thaw's whole package-export
/// pipeline (thaw-bridge's parser, thaw-cli's shim generation) only ever
/// recognizes a function or class/interface export, never a plain data
/// constant, so this would need a genuinely new export kind threaded
/// through several layers, not a flattening fix alone.
///
/// `v4`'s own buffer-output overload (`v4<TBuf extends Uint8Array =
/// Uint8Array>(options, buf, offset?): TBuf`) -- previously unreachable,
/// always resolving to `v4`'s first (`buf?: undefined`) overload no
/// matter what a real call passed -- is now fixed (registry Fallback
/// functions pick an overload by real call-site arity/argument-type
/// scoring, the same mechanism external class methods already used; see
/// `fallback_function_overload_is_picked_by_call_site_arity` in
/// `registry_modules.rs` for the synthetic reproduction) and exercised
/// below.
#[test]
fn registry_add_generates_and_parses_uuids_with_real_uuid_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-uuid-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "uuid").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { v4, validate, version, parse, stringify } from "uuid";
function main(): void {
    const id: string = v4();
    console.log(validate(id));
    console.log(version(id));

    const ns = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";
    const bytes = parse(ns);
    console.log(bytes.length);
    const roundTripped: string = stringify(bytes);
    console.log(roundTripped);

    const filled: JsValue = v4(undefined, bytes as JsValue);
    console.log(filled);
}"#,
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
    let stdout = String::from_utf8_lossy(&result.stdout);
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("true"));
    assert_eq!(lines.next(), Some("4"));
    assert_eq!(lines.next(), Some("16"));
    assert_eq!(lines.next(), Some("6ba7b810-9dad-11d1-80b4-00c04fd430c8"));
    // If the buffer-output overload had stayed unreachable (always
    // resolving to `v4`'s first, `string`-returning overload instead),
    // this call wouldn't even have compiled -- `const filled: JsValue =
    // v4(...)` would be a type mismatch against a `string` result.
    let filled_line = lines.next().expect("v4's buffer overload result");
    let filled_bytes: Vec<&str> = filled_line.split(',').collect();
    assert_eq!(
        filled_bytes.len(),
        16,
        "expected a 16-byte filled buffer: {filled_line}"
    );
    assert_ne!(
        filled_line, "107,167,184,16,157,173,17,209,128,180,0,192,79,212,48,200",
        "buffer wasn't actually filled with random bytes by the buffer-output overload"
    );
    assert_eq!(lines.next(), None);
    let _ = std::fs::remove_dir_all(dir);
}

/// Real, fetched `uuid@8.3.2` -- a legacy major version predating the
/// currently-fixed latest uuid (the test above), with a completely
/// different `.d.ts` export shape: `uuid@8.3.2` itself ships no `.d.ts`
/// at all (types come from the separately-versioned `@types/uuid`),
/// whose real `index.d.mts` is `import uuid from "./index.js"; export
/// import v1 = uuid.v1; export import v4 = uuid.v4; export import
/// validate = uuid.validate; ...` -- a TS import-equals declaration whose
/// module reference is a qualified *entity name* (`uuid.v1`, a property
/// access into an already-imported value), not a `require(...)` call.
/// thaw-registry's flattening had no code recognizing this AST shape at
/// all, so none of uuid v8's functions were reachable.
///
/// Root cause, two parts: (1) nothing in `install.rs` matched
/// `TsModuleRef::TsEntityName` at all (only `TsExternalModuleRef`, the
/// `require(...)` shape `import_equals_targets` already handles) -- fixed
/// by a new loop resolving the qualifier (`uuid`) via the existing
/// `named_import_targets`, then reusing `reexported_function_
/// declarations` to look the member (`v1`) up in `uuid`'s own target
/// file exactly like a named re-export already does. (2) even once
/// found, `uuid.v1`'s own declared type (`export const v1: v1;`) is
/// typed through a *local, unexported* type-alias chain
/// (`type v1 = v1Buffer & v1String;`, each side itself a further local
/// alias resolving to a direct function type) -- a shape neither
/// `callable_const_declaration_snippet` (thaw-registry, text inclusion)
/// nor `extract_const_call_signature_decls` (thaw-bridge, classification)
/// recognized, both only handling a same-file call-signature interface or
/// a *direct* inline function type. Fixed on the thaw-registry side by
/// carrying the const's own snippet together with *every* type alias/
/// interface declared in the same file (harmless when unrelated -- `.d.ts`
/// type declarations are erasable); on the thaw-bridge side by a new
/// `resolve_local_callable_fn_types`, following a local alias chain and
/// unwrapping an intersection into each of its own operands, yielding one
/// `DtsFunction` per resolved overload (`v1Buffer`'s generic
/// buffer-output overload and `v1String`'s simple string-returning one),
/// the same "first overload wins" convention already documented as a
/// separate, scoped-out limitation for the latest uuid's own `v4`
/// (confirmed present here too, for the exact same reason -- not
/// attempted here either).
#[test]
fn registry_add_reexports_uuid_v8s_import_equals_functions_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-uuid-v8-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "uuid@8.3.2").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { validate, version, parse, stringify } from "uuid";
function main(): void {
    const ns = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";
    console.log(validate(ns));
    console.log(version(ns));
    const bytes = parse(ns);
    console.log(bytes.length);
    const roundTripped: string = stringify(bytes);
    console.log(roundTripped);
}"#,
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
    let stdout = String::from_utf8_lossy(&result.stdout);
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("true"));
    assert_eq!(lines.next(), Some("1"));
    assert_eq!(lines.next(), Some("16"));
    assert_eq!(lines.next(), Some("6ba7b810-9dad-11d1-80b4-00c04fd430c8"));
    assert_eq!(lines.next(), None);
    let _ = std::fs::remove_dir_all(dir);
}

/// Real, fetched `lodash@4.17.21` -- both entry-point shapes
/// (`import _ from "lodash"` and `import * as _ from "lodash"`), plus a
/// named import of a function with an *optional* callback parameter
/// (`filter`). Found via the earlier lodash `default`-import fix's own
/// verification against real lodash's full `.d.ts`: even
/// `import * as _ from "lodash"; _.chunk(...)` (calling a function with
/// no callback parameter at all) crashed with "unsupported dynamic
/// object field Function(...)".
///
/// Root cause: an *optional* callback parameter (real example: lodash's
/// `filter(collection: string | null | undefined, predicate?:
/// StringIterator<boolean>): string[];`) classifies as `Native(Optional
/// (Function(...)))`, a different `Native` variant from the bare
/// `Native(Function(...))` case this session's earlier closure-as-
/// plain-argument fix (commit `170f6a23`) covered, so it fell through
/// `typed_dynamic_declaration`'s (thaw-cli's `shims.rs`) widening match
/// arm unwidened. Separately, `typed_dynamic_bare_alias` (the wrapper
/// generated under a function's bare name) had its own, independent,
/// entirely unwidened parameter-type computation -- so a plain named
/// import of any function with such a parameter crashed the moment it
/// was called with the parameter omitted, regardless of the const-
/// export-shape fix above. Both fixed the same way -- widened to `Json`
/// for the non-`napi` backend, matching the existing bare-`Function`
/// case -- and `typed_dynamic_bare_alias` additionally needed `napi`
/// threaded through as a new parameter (it previously had no napi-
/// awareness at all).
#[test]
fn registry_add_builds_and_calls_real_lodash_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-lodash-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "lodash@4.17.21").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import _ from "lodash";
import * as ns from "lodash";
import { every, filter, find, map, reduce, some, sortBy } from "lodash";
function main(): void {
    let sortTotal: number = 0;
    let objectSortTotal: number = 0;
    console.log(_.chunk([1, 2, 3, 4], 2).length);
    console.log(_.capitalize("hello"));
    console.log(ns.chunk([1, 2, 3, 4], 2).length);
    console.log(_.filter("hello").length);
    console.log(map([1, 2, 3], value => value * 2).length);
    console.log(filter([1, 2, 3], value => value > 1).length);
    console.log(reduce([1, 2, 3], (sum, value) => sum + value, 0));
    console.log(find([1, 2, 3], value => value > 1));
    console.log(some([1, 2, 3], value => value === 2));
    console.log(every([1, 2, 3], value => value > 0));
    const sorted = sortBy([3, 1, 2], value => { sortTotal = sortTotal + value; return value; });
    console.log(sorted[0]);
    console.log(sorted.join(','));
    console.log(sortTotal);
    const sortedObject = sortBy({ a: 4, b: 2 }, value => { objectSortTotal = objectSortTotal + value; return value; });
    console.log(sortedObject[0]);
    console.log(sortedObject.join(','));
    console.log(objectSortTotal);
}"#,
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
    let stdout = String::from_utf8_lossy(&result.stdout);
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("2"));
    assert_eq!(lines.next(), Some("Hello"));
    assert_eq!(lines.next(), Some("2"));
    assert_eq!(lines.next(), Some("5"));
    assert_eq!(lines.next(), Some("3"));
    assert_eq!(lines.next(), Some("2"));
    assert_eq!(lines.next(), Some("6"));
    assert_eq!(lines.next(), Some("2"));
    assert_eq!(lines.next(), Some("true"));
    assert_eq!(lines.next(), Some("true"));
    assert_eq!(lines.next(), Some("1"));
    assert_eq!(lines.next(), Some("1,2,3"));
    assert_eq!(lines.next(), Some("6"));
    assert_eq!(lines.next(), Some("2"));
    assert_eq!(lines.next(), Some("2,4"));
    assert_eq!(lines.next(), Some("6"));
    assert_eq!(lines.next(), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_builds_and_calls_real_chalk_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-chalk-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "chalk@5.4.1").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import chalk from "chalk";
function main(): void {
    console.log(chalk.red.bold("hello"));
}"#,
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
    assert!(String::from_utf8_lossy(&result.stdout).contains("hello"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Real, fetched drizzle-orm -- the next breadth-first bug-hunting target
/// after uuid. drizzle-orm uses npm subpath exports extensively (the root
/// package exposes only SQL-operator helpers; the actual `sqliteTable`/
/// `pgTable` factories and column builders live under dialect-specific
/// subpaths like `drizzle-orm/sqlite-core`), which thaw-registry already
/// supports: `thaw registry add drizzle-orm` alone processes every subpath
/// at install time, and `--use drizzle-orm/sqlite-core` at build time
/// resolves against the already-installed `subpaths/sqlite-core/`
/// directory (`thaw registry add drizzle-orm/sqlite-core` directly does
/// *not* work -- it's treated as a separate package spec and fails trying
/// to `npm install` it).
///
/// `sqliteTable` (and `pgTable`, `mysqlTable`, ...) -- the central
/// primitive essentially all real drizzle schema-definition code depends
/// on -- was completely absent from the flattened `package.d.ts`, unlike
/// every previously-supported npm export shape. Root cause: drizzle-orm
/// declares it `export declare const sqliteTable: SQLiteTableFn;`
/// (`SQLiteTableFn` an interface with one call signature per overload) --
/// a `Decl::Var` binding, not `declare function`. Thaw's whole export
/// pipeline (thaw-registry's re-export flattening, thaw-bridge's `.d.ts`
/// parser) only ever recognized a function or class/interface export;
/// a plain const was silently dropped, same underlying gap as uuid's
/// `NIL`/`MAX` (see the doc comment above) but blocking rather than
/// cosmetic, since without it no real drizzle schema can be written at
/// all.
///
/// Fixed by teaching thaw-registry's flattening (`callable_const_
/// declarations` in `install.rs`) to recognize a `declare const NAME:
/// TypeRef;` whose `TypeRef` is a same-file interface with a call
/// signature, carrying both the const's and the interface's own
/// declaration text into the flattened output, and teaching thaw-bridge's
/// `parse_dts` (`extract_const_call_signature_decls`/
/// `lower_dts_call_signature`) to synthesize a `DtsFunction` per overload
/// from such an interface's call signatures -- exactly like an ordinary
/// ambient function declaration. No thaw-hir or thaw-cli shim-generation
/// changes were needed: once `sqliteTable` is just another entry in
/// `pkg.functions`, it flows through the entirely existing
/// `Classification::Fallback` call path (every overload here classifies
/// Fallback, since every parameter/return type involves unresolvable
/// generics/callbacks), identical to any other already-working npm
/// function.
///
/// Scope: this only covers making `sqliteTable`/`integer`/`text` etc.
/// *callable*. Using the *returned* table's columns
/// (`usersTable.id`) or passing it into a query builder
/// (`db.select().from(usersTable)`) is a separate, later concern, not
/// attempted here.
#[test]
fn registry_add_builds_a_sqlite_schema_with_real_drizzle_orm_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-drizzle-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "drizzle-orm").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { sqliteTable, integer, text } from "drizzle-orm/sqlite-core";
function main(): void {
    const users = sqliteTable("users", {
        id: integer(),
        name: text(),
    });
    console.log("ok");
}"#,
    )
    .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["drizzle-orm/sqlite-core".to_string()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "ok\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// Real drizzle-orm's query-builder chain (`db.select().from(t).where(cond)`,
/// via the `sqlite-proxy` driver) -- the scenario explicitly scoped out of
/// the test above. Every `.select()`/`.from()`/`.where()` link is a
/// "thenable" (drizzle's `QueryPromise` base class implements `.then` by
/// calling `execute()`), and `resolve_promise_value`
/// (`crates/thaw-quickjs/src/quickjs/api.rs`) used to call
/// `Promise.resolve()` unconditionally on every dynamic method call's
/// result -- which, per real JS semantics, eagerly invokes any thenable's
/// `.then`. This executed the query as soon as `.from(t)` returned, with
/// no `WHERE` clause, before `.where(...)` was ever applied.
///
/// Fixed by threading a `chain_intermediate` flag from thaw-llvm's
/// `compile_call_dynamic_method_handle` (`dynamic_host.rs`) through to
/// `thaw_js_call_method_handle_result`: a receiver that is itself a
/// chained `.method()` call is structurally detectable in the HIR (see
/// `lower_dynamic_value_method_call`, thaw-hir's `invocations.rs`, which
/// always lowers a method call's receiver with an expected type of
/// `JsValue`), so only the terminal link in a chain resolves its result --
/// matching real JS semantics, where only an explicit `await`/consumption
/// of the final value would ever settle a pending promise or thenable.
/// See `a_chained_methods_intermediate_thenable_result_is_not_resolved_early`
/// in `registry_modules.rs` for the synthetic, network-free reproduction.
#[test]
fn registry_add_runs_a_real_drizzle_query_builder_chain_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-drizzle-chain-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "drizzle-orm").unwrap();
    thaw_registry::add(&registry, "better-sqlite3").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { sqliteTable, integer, text } from "drizzle-orm/sqlite-core";
import { drizzle } from "drizzle-orm/sqlite-proxy";
import { eq } from "drizzle-orm";

const users = sqliteTable("users", {
    id: integer(),
    name: text(),
});

async function callback(sql: string, params: JsValue, method: string): Promise<{ rows: number[] }> {
    console.log(sql);
    return { rows: [] };
}

async function main(): Promise<void> {
    const db = drizzle(callback);
    await db.select().from(users).where(eq(users.id, 1));
}"#,
    )
    .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["drizzle-orm/sqlite-core".to_string(), "drizzle-orm/sqlite-proxy".to_string()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    let mut lines = stdout.lines();
    let sql = lines.next().unwrap();
    assert!(sql.contains("where"), "expected a WHERE clause: {sql}");
    assert_eq!(lines.next(), None, "query executed more than once: {stdout}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Real, fetched `zod@3.23.0` -- a version-diversity check prompted by the
/// user's own question of whether earlier zod work (this session, against
/// whatever `zod` resolved to as latest -- zod v4) actually generalizes,
/// rather than being overfit to that one observed version. It doesn't:
/// `import { object, string } from "zod"` failed outright (`` `zod` has
/// no export named `object` ``) against v3.
///
/// Root cause: zod v3's actual primitives (`object`, `string`, `number`,
/// ... -- essentially its whole public API) are declared in
/// `lib/types.d.ts` as **bare, non-exported** consts with a **direct**
/// inline function type (no interface involved at all):
/// `declare const objectType: <T extends ZodRawShape>(shape: T, params?)
/// => ZodObject<...>;`, one per primitive, later locally rename-exported
/// (`export { ..., objectType as object, ... };`). This session's earlier
/// `declare const`-export fix (drizzle-orm's `sqliteTable`, commit
/// `c3da7164`) only covered a const whose type is a `TsTypeRef` to a
/// same-file interface with a call signature -- neither a *direct*
/// inline function type, nor a *bare* (non-exported) const reached only
/// through a local rename-export, were covered.
///
/// Fixed by extending exactly that machinery: thaw-bridge's
/// `extract_const_call_signature_decls` gained a `CallableConstSignature::
/// Direct` case (alongside the existing `Interface` one) for a `TsFnType`
/// annotation directly, with a new `lower_dts_fn_type` sibling to
/// `lower_dts_call_signature` synthesizing the `DtsFunction`; thaw-
/// registry's `all_reexported_function_declarations` gained a bare-
/// `Decl::Var` scan feeding its existing `local_declarations` map (the
/// same one a bare `Decl::Fn` already uses), so a same-file rename-export
/// resolves a callable const exactly like it already does a function; and
/// `rename_declared_function` gained `"const "` to its rename-keyword
/// list (previously only `"function "`/`"class "`/`"interface "`), so the
/// renamed binding (`objectType` -> `object`) actually lands in the
/// flattened output under its real, public name.
///
/// `import { z } from "zod"` (zod v3's alternate, namespace-style entry
/// point) needed a separate, follow-on fix -- see
/// `registry_add_builds_a_schema_with_real_zod_v3s_z_namespace_when_enabled`
/// below.
#[test]
fn registry_add_builds_a_schema_with_real_zod_v3_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-zod-v3-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "zod@3.23.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { object, string, number } from "zod";
function main(): void {
    const schema = object({ name: string(), age: number() });
    console.log("ok");
}"#,
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "ok\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// Real, fetched `zod@3.23.0`'s alternate, namespace-style entry point:
/// `import { z } from "zod"` and `import zod from "zod"` (both
/// `z.object(...)`/`zod.object(...)`), the shape essentially as common in
/// real zod v3 code as the bare-name form the test above covers. Was a
/// separate, still-open gap after that fix: `z` is a self-referential
/// namespace alias (`import * as z from "./external"; export { z };
/// export default z;`) declared in `lib/index.d.ts`, one file *below* the
/// package's own entry point (`index.d.ts`, just `export * from
/// "./lib";`) -- unlike zod v4, whose own entry point declares this
/// alias directly, so it was already visible to `self_referential_
/// namespace_aliases` (thaw-bridge) without any special handling.
/// thaw-registry's flattening only ever pulled individual function/const
/// *declarations* through a wildcard re-export chain, never arbitrary
/// top-level import/export-alias statements from a file reached only
/// transitively, so `z`'s own alias declaration never reached the
/// flattened output at all.
///
/// Fixed with a new `self_referential_namespace_alias_snippets`
/// (thaw-registry's `install.rs`), recursing through wildcard re-exports
/// the same way `collect_namespace_reexports` already does, splicing the
/// exact `import`/`export` snippet text verbatim into the flattened
/// output wherever found (its import source doesn't need to resolve to
/// anything real -- `self_referential_namespace_aliases` only checks the
/// AST shape). `export default z;` also needed
/// `self_referential_namespace_aliases` itself (thaw-bridge) to recognize
/// a *separate* `export default X;` statement as an alias under the
/// implicit name `"default"` -- previously it only matched `export { X as
/// default }`, the form zod v4 happens to use instead.
#[test]
fn registry_add_builds_a_schema_with_real_zod_v3s_z_namespace_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-zod-v3-z-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "zod@3.23.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { z } from "zod";
import zod from "zod";
function main(): void {
    const schema = z.object({ name: z.string(), age: z.number() });
    const schema2 = zod.object({ ok: zod.boolean() });
    console.log("ok");
}"#,
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "ok\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_runs_a_real_hono_route_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-hono-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "hono@4.13.7").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { Hono, Context } from "hono";
async function main(): Promise<void> {
    const app = new Hono();
    app.get("/", (c: Context) => c.text("Hello Thaw"));
    const response = await app.request("/");
    const body: string = await response.text();
    console.log(response.status);
    console.log(body);
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["hono".into()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "200\nHello Thaw\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_runs_a_real_hapi_route_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-hapi-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "@hapi/hapi@21.4.3").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import * as Hapi from "@hapi/hapi";
async function main(): Promise<void> {
    const server = Hapi.server({ port: 0 });
    server.route({ method: "GET", path: "/", handler: (request, h) => h.response(request.path) });
    const response = await server.inject("/");
    console.log(response.statusCode);
    console.log(response.payload);
}"#,
    )
    .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["@hapi/hapi".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "200\n/\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_infers_a_real_commander_action_callback_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-commander-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "commander@14.0.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { Command } from "commander";
function main(): void {
    const program = new Command();
    program.argument("<name>");
    program.action((name) => console.log(name));
    program.parse(["node", "app", "Thaw"]);
}"#,
    )
    .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["commander".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "\"Thaw\"\n");
    let _ = std::fs::remove_dir_all(dir);
}
