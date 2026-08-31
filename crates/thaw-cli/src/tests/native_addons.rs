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
fn registry_native_addon_returned_function_runs_end_to_end() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-native-factory-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("native-factory");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function multiplier(factor: number): (value: number) => number;\n",
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
            __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
                (void)exports; napi_value fn;
                napi_create_function(env, "multiplier", 10, multiplier, 0, &fn); return fn;
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
        "import { multiplier } from \"native-factory\"; function main(): void { const triple = multiplier(3); console.log(triple(14)); }\n",
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
