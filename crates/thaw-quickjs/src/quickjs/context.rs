fn ensure_context() {
    JS.with(|cell| {
        let mut slot = cell.borrow_mut();
        slot.get_or_insert_with(|| {
            let runtime = Runtime::new().expect("failed to create a QuickJS runtime");
            let context = Context::full(&runtime).expect("failed to create a QuickJS context");
            context.with(|ctx| {
                ctx.globals()
                    .set(
                        "__thaw_os_thread_token",
                        format!("{:?}", std::thread::current().id()),
                    )
                    .expect("failed to install OS thread identity");
                ctx.globals()
                    .set(
                        "__thaw_host_env_json",
                        serde_json::to_string(
                            &std::env::vars().collect::<std::collections::BTreeMap<_, _>>(),
                        )
                        .expect("failed to serialize host environment"),
                    )
                    .expect("failed to install host environment");
                let arguments = std::env::args().collect::<Vec<_>>();
                let mut node_arguments = Vec::with_capacity(arguments.len() + 1);
                node_arguments.push(arguments.first().cloned().unwrap_or_default());
                node_arguments.extend(arguments);
                ctx.globals()
                    .set(
                        "__thaw_host_argv_json",
                        serde_json::to_string(&node_arguments)
                            .expect("failed to serialize host arguments"),
                    )
                    .expect("failed to install host arguments");
                install_napi_bridge(&ctx).expect("failed to install N-API bridge");
                let shared_env = HOST_WORKERS.with(|table| table.borrow().shared_env.clone());
                install_shared_environment_functions(&ctx, shared_env)
                    .expect("failed to install shared Worker environment accessors");
                let shared_env_init = Function::new(ctx.clone(), initialize_shared_environment)
                    .expect("failed to create shared Worker environment initializer");
                ctx.globals()
                    .set("__thaw_shared_env_init", shared_env_init)
                    .expect("failed to install shared Worker environment initializer");
                let stdout = Function::new(ctx.clone(), |text: String| {
                    print!("{text}");
                    let _ = io::stdout().flush();
                })
                .expect("failed to create JavaScript stdout writer");
                let stderr = Function::new(ctx.clone(), |text: String| {
                    eprint!("{text}");
                    let _ = io::stderr().flush();
                })
                .expect("failed to create JavaScript stderr writer");
                ctx.globals()
                    .set("__thaw_console_stdout", stdout)
                    .expect("failed to install JavaScript stdout writer");
                ctx.globals()
                    .set("__thaw_console_stderr", stderr)
                    .expect("failed to install JavaScript stderr writer");
                let cwd = Function::new(ctx.clone(), || -> rquickjs::Result<String> {
                    std::env::current_dir()
                        .map(|path| path.to_string_lossy().into_owned())
                        .map_err(|error| {
                            rquickjs::Error::new_from_js_message("process", "cwd", error.to_string())
                        })
                })
                .expect("failed to create process cwd bridge");
                ctx.globals()
                    .set("__thaw_process_cwd", cwd)
                    .expect("failed to install process cwd bridge");
                let chdir = Function::new(ctx.clone(), |path: String| -> rquickjs::Result<()> {
                    std::env::set_current_dir(path).map_err(|error| {
                        rquickjs::Error::new_from_js_message("process", "chdir", error.to_string())
                    })
                })
                .expect("failed to create process chdir bridge");
                ctx.globals()
                    .set("__thaw_process_chdir", chdir)
                    .expect("failed to install process chdir bridge");
                let configure_signal = Function::new(ctx.clone(), configure_process_signal)
                    .expect("failed to create process signal bridge");
                ctx.globals()
                    .set("__thaw_process_configure_signal", configure_signal)
                    .expect("failed to install process signal bridge");
                let detach_array_buffer =
                    Function::new(ctx.clone(), |mut value: ArrayBuffer<'_>| {
                        value.detach();
                    })
                    .expect("failed to create ArrayBuffer detacher");
                ctx.globals()
                    .set("__thaw_detach_array_buffer", detach_array_buffer)
                    .expect("failed to install ArrayBuffer detacher");
                let quickjs_gc_function = Function::new(ctx.clone(), run_quickjs_gc)
                    .expect("failed to create QuickJS garbage collector trigger");
                ctx.globals()
                    .set("__thaw_gc", quickjs_gc_function)
                    .expect("failed to install QuickJS garbage collector trigger");
                #[cfg(feature = "wasm")]
                {
                let wasm_compile_function = Function::new(ctx.clone(), wasm_compile)
                    .expect("failed to create WebAssembly compiler");
                let wasm_instantiate_function = Function::new(ctx.clone(), wasm_instantiate)
                    .expect("failed to create WebAssembly instantiator");
                let wasm_custom_sections_function =
                    Function::new(ctx.clone(), wasm_custom_sections)
                        .expect("failed to create WebAssembly custom-section reader");
                let wasm_retain_import_function = Function::new(ctx.clone(), wasm_retain_import)
                    .expect("failed to create WebAssembly import retainer");
                let wasm_retain_funcref_function = Function::new(ctx.clone(), wasm_retain_funcref)
                    .expect("failed to create WebAssembly function-reference retainer");
                let wasm_restore_import_function = Function::new(ctx.clone(), wasm_restore_import)
                    .expect("failed to create WebAssembly import restorer");
                let wasm_retain_value_function = Function::new(ctx.clone(), wasm_retain_value)
                    .expect("failed to create WebAssembly externref retainer");
                let wasm_retain_value_at_function =
                    Function::new(ctx.clone(), wasm_retain_value_at)
                        .expect("failed to create WebAssembly externref reactivator");
                let wasm_restore_value_function = Function::new(ctx.clone(), wasm_restore_value)
                    .expect("failed to create WebAssembly externref restorer");
                let wasm_reference_stats_function =
                    Function::new(ctx.clone(), wasm_reference_stats)
                        .expect("failed to create WebAssembly reference statistics reader");
                let wasm_release_function = Function::new(ctx.clone(), wasm_release)
                    .expect("failed to create WebAssembly resource releaser");
                let wasm_release_pending_function =
                    Function::new(ctx.clone(), wasm_release_pending)
                        .expect("failed to create WebAssembly pending-resource releaser");
                let wasm_call_function = Function::new(ctx.clone(), wasm_call)
                    .expect("failed to create WebAssembly function caller");
                let wasm_call_funcref_function = Function::new(ctx.clone(), wasm_call_funcref)
                    .expect("failed to create WebAssembly function-reference caller");
                let wasm_export_funcref_function = Function::new(ctx.clone(), wasm_export_funcref)
                    .expect("failed to create WebAssembly exported function-reference reader");
                let wasm_global_function = Function::new(ctx.clone(), wasm_global)
                    .expect("failed to create WebAssembly global accessor");
                let wasm_memory_create_function = Function::new(ctx.clone(), wasm_memory_create)
                    .expect("failed to create WebAssembly memory allocator");
                let wasm_memory_function = Function::new(ctx.clone(), wasm_memory)
                    .expect("failed to create WebAssembly memory accessor");
                let wasm_table_function = Function::new(ctx.clone(), wasm_table)
                    .expect("failed to create WebAssembly table accessor");
                ctx.globals()
                    .set("__thaw_wasm_compile", wasm_compile_function)
                    .expect("failed to install WebAssembly compiler");
                ctx.globals()
                    .set("__thaw_wasm_instantiate", wasm_instantiate_function)
                    .expect("failed to install WebAssembly instantiator");
                ctx.globals()
                    .set("__thaw_wasm_custom_sections", wasm_custom_sections_function)
                    .expect("failed to install WebAssembly custom-section reader");
                ctx.globals()
                    .set("__thaw_wasm_retain_import", wasm_retain_import_function)
                    .expect("failed to install WebAssembly import retainer");
                ctx.globals()
                    .set("__thaw_wasm_retain_funcref", wasm_retain_funcref_function)
                    .expect("failed to install WebAssembly function-reference retainer");
                ctx.globals()
                    .set("__thaw_wasm_restore_import", wasm_restore_import_function)
                    .expect("failed to install WebAssembly import restorer");
                ctx.globals()
                    .set("__thaw_wasm_retain_value", wasm_retain_value_function)
                    .expect("failed to install WebAssembly externref retainer");
                ctx.globals()
                    .set("__thaw_wasm_retain_value_at", wasm_retain_value_at_function)
                    .expect("failed to install WebAssembly externref reactivator");
                ctx.globals()
                    .set("__thaw_wasm_restore_value", wasm_restore_value_function)
                    .expect("failed to install WebAssembly externref restorer");
                ctx.globals()
                    .set("__thaw_wasm_reference_stats", wasm_reference_stats_function)
                    .expect("failed to install WebAssembly reference statistics reader");
                ctx.globals()
                    .set("__thaw_wasm_release", wasm_release_function)
                    .expect("failed to install WebAssembly resource releaser");
                ctx.globals()
                    .set("__thaw_wasm_release_pending", wasm_release_pending_function)
                    .expect("failed to install WebAssembly pending-resource releaser");
                ctx.globals()
                    .set("__thaw_wasm_call", wasm_call_function)
                    .expect("failed to install WebAssembly function caller");
                ctx.globals()
                    .set("__thaw_wasm_call_funcref", wasm_call_funcref_function)
                    .expect("failed to install WebAssembly function-reference caller");
                ctx.globals()
                    .set("__thaw_wasm_export_funcref", wasm_export_funcref_function)
                    .expect("failed to install WebAssembly exported function-reference reader");
                ctx.globals()
                    .set("__thaw_wasm_global", wasm_global_function)
                    .expect("failed to install WebAssembly global accessor");
                ctx.globals()
                    .set("__thaw_wasm_memory_create", wasm_memory_create_function)
                    .expect("failed to install WebAssembly memory allocator");
                ctx.globals()
                    .set("__thaw_wasm_memory", wasm_memory_function)
                    .expect("failed to install WebAssembly memory accessor");
                ctx.globals()
                    .set("__thaw_wasm_table", wasm_table_function)
                    .expect("failed to install WebAssembly table accessor");
                }
                let worker_spawn = Function::new(
                    ctx.clone(),
                    |bundle_source: String,
                     source: String,
                     worker_data_json: String,
                     config_json: String,
                     thread_id: u32| {
                        spawn_host_worker(
                            bundle_source,
                            source,
                            worker_data_json,
                            config_json,
                            thread_id,
                        )
                    },
                )
                .expect("failed to create Worker spawner");
                let worker_send = Function::new(ctx.clone(), |handle: u32, payload: String| {
                    send_host_worker(handle, payload)
                })
                .expect("failed to create Worker sender");
                let worker_terminate =
                    Function::new(ctx.clone(), |handle: u32| terminate_host_worker(handle))
                        .expect("failed to create Worker terminator");
                let worker_stdin =
                    Function::new(ctx.clone(), |handle: u32, payload: String, ended: bool| {
                        send_host_worker_stdin(handle, payload, ended)
                    })
                    .expect("failed to create Worker stdin sender");
                let worker_route_direct = Function::new(
                    ctx.clone(),
                    |origin: u32, target: u32, payload: String, source: u32, request: u64| {
                        route_host_worker_direct(origin, target, payload, source, request)
                    },
                )
                .expect("failed to create Worker direct router");
                let worker_parent_direct =
                    Function::new(ctx.clone(), |target: u32, payload: String, request: u64| {
                        route_parent_direct(target, payload, request)
                    })
                    .expect("failed to create parent direct router");
                let worker_direct_result =
                    Function::new(ctx.clone(), |handle: u32, request: u64, error: String| {
                        resolve_host_worker_direct(handle, request, error)
                    })
                    .expect("failed to create Worker direct resolver");
                let worker_port =
                    Function::new(ctx.clone(), |handle: u32, port: String, payload: String| {
                        send_host_worker_port(handle, port, payload)
                    })
                    .expect("failed to create Worker port sender");
                let worker_read_source =
                    Function::new(ctx.clone(), |path: String| -> rquickjs::Result<String> {
                        std::fs::read_to_string(&path).map_err(|error| {
                            rquickjs::Error::new_from_js_message(
                                "Worker path",
                                "JavaScript source",
                                format!("failed to read `{path}`: {error}"),
                            )
                        })
                    })
                    .expect("failed to create Worker source reader");
                let worker_poll = Function::new(ctx.clone(), poll_host_workers)
                    .expect("failed to create Worker event poller");
                let worker_active = Function::new(ctx.clone(), host_workers_active)
                    .expect("failed to create Worker activity probe");
                let child_process = Function::new(
                    ctx.clone(),
                    |command: String, arguments: String, options: String| {
                        run_child_process(command, arguments, options)
                    },
                )
                .expect("failed to create child process runner");
                let child_spawn = Function::new(
                    ctx.clone(),
                    |command: String, arguments: String, options: String| {
                        spawn_host_child(command, arguments, options)
                    },
                )
                .expect("failed to create child process spawner");
                let child_stdin =
                    Function::new(ctx.clone(), |handle: u32, value: String, end: bool| {
                        send_host_child_stdin(handle, value, end)
                    })
                    .expect("failed to create child process stdin sender");
                let child_kill = Function::new(ctx.clone(), |handle: u32, signal: i32| {
                    kill_host_child(handle, signal)
                })
                .expect("failed to create child process killer");
                let child_poll = Function::new(ctx.clone(), poll_host_children)
                    .expect("failed to create child process poller");
                let child_active = Function::new(ctx.clone(), host_children_active)
                    .expect("failed to create child process activity probe");
                ctx.globals()
                    .set("__thaw_worker_spawn", worker_spawn)
                    .expect("failed to install Worker spawner");
                ctx.globals()
                    .set("__thaw_worker_send", worker_send)
                    .expect("failed to install Worker sender");
                ctx.globals()
                    .set("__thaw_worker_terminate", worker_terminate)
                    .expect("failed to install Worker terminator");
                ctx.globals()
                    .set("__thaw_worker_stdin", worker_stdin)
                    .expect("failed to install Worker stdin sender");
                ctx.globals()
                    .set("__thaw_worker_route_direct", worker_route_direct)
                    .expect("failed to install Worker direct router");
                ctx.globals()
                    .set("__thaw_worker_parent_direct", worker_parent_direct)
                    .expect("failed to install parent direct router");
                ctx.globals()
                    .set("__thaw_worker_direct_result", worker_direct_result)
                    .expect("failed to install Worker direct resolver");
                ctx.globals()
                    .set("__thaw_worker_port", worker_port)
                    .expect("failed to install Worker port sender");
                ctx.globals()
                    .set("__thaw_worker_read_source", worker_read_source)
                    .expect("failed to install Worker source reader");
                ctx.globals()
                    .set("__thaw_worker_poll", worker_poll)
                    .expect("failed to install Worker event poller");
                ctx.globals()
                    .set("__thaw_worker_active", worker_active)
                    .expect("failed to install Worker activity probe");
                ctx.globals()
                    .set("__thaw_child_process_sync", child_process)
                    .expect("failed to install child process runner");
                ctx.globals()
                    .set("__thaw_child_process_spawn", child_spawn)
                    .expect("failed to install child process spawner");
                ctx.globals()
                    .set("__thaw_child_process_stdin", child_stdin)
                    .expect("failed to install child process stdin sender");
                ctx.globals()
                    .set("__thaw_child_process_kill", child_kill)
                    .expect("failed to install child process killer");
                ctx.globals()
                    .set("__thaw_child_process_poll", child_poll)
                    .expect("failed to install child process poller");
                ctx.globals()
                    .set("__thaw_child_process_active", child_active)
                    .expect("failed to install child process activity probe");
                let random_hex = Function::new(ctx.clone(), |size: u32| {
                    let mut bytes = vec![0u8; size as usize];
                    getrandom::getrandom(&mut bytes).expect("OS random source failed");
                    hex_encode(&bytes)
                })
                .expect("failed to create JavaScript random source");
                let hash_hex = Function::new(ctx.clone(), |algorithm: String, value: String| {
                    hex_encode(&digest_bytes(&algorithm, &hex_decode(&value)))
                })
                .expect("failed to create JavaScript hash function");
                let hmac_hex = Function::new(
                    ctx.clone(),
                    |algorithm: String, key: String, value: String| {
                        hex_encode(&hmac_bytes(
                            &algorithm,
                            &hex_decode(&key),
                            &hex_decode(&value),
                        ))
                    },
                )
                .expect("failed to create JavaScript HMAC function");
                let pbkdf2_hex = Function::new(
                    ctx.clone(),
                    |algorithm: String,
                     password: String,
                     salt: String,
                     iterations: u32,
                     length: u32| {
                        hex_encode(&pbkdf2_bytes(
                            &algorithm,
                            &hex_decode(&password),
                            &hex_decode(&salt),
                            iterations,
                            (length as usize).clamp(10, 64),
                        ))
                    },
                )
                .expect("failed to create JavaScript PBKDF2 function");
                let scrypt_hex = Function::new(
                    ctx.clone(),
                    |password: String,
                     salt: String,
                     length: u32,
                     cost: u32,
                     block_size: u32,
                     parallelization: u32| {
                        if cost < 2 || !cost.is_power_of_two() {
                            return Err(rquickjs::Error::new_from_js_message(
                                "scrypt cost",
                                "power of two greater than one",
                                cost.to_string(),
                            ));
                        }
                        let params = scrypt::Params::new(
                            cost.ilog2() as u8,
                            block_size,
                            parallelization,
                            (length as usize).clamp(10, 64),
                        )
                        .map_err(|error| {
                            rquickjs::Error::new_from_js_message(
                                "scrypt options",
                                "valid scrypt options",
                                format!(
                                    "{error}: N={cost}, r={block_size}, p={parallelization}, length={length}"
                                ),
                            )
                        })?;
                        let mut output = vec![0; length as usize];
                        scrypt::scrypt(
                            &hex_decode(&password),
                            &hex_decode(&salt),
                            &params,
                            &mut output,
                        )
                        .map_err(|error| {
                            rquickjs::Error::new_from_js_message(
                                "scrypt input",
                                "derived key",
                                error.to_string(),
                            )
                        })?;
                        Ok(hex_encode(&output))
                    },
                )
                .expect("failed to create JavaScript scrypt function");
                let cipher_hex = Function::new(
                    ctx.clone(),
                    |algorithm: String,
                     key: String,
                     iv: String,
                     value: String,
                     decrypt: bool,
                     finalize: bool|
                     -> rquickjs::Result<String> {
                        if !algorithm.eq_ignore_ascii_case("aes-256-cbc") {
                            return Err(rquickjs::Error::new_from_js_message(
                                "cipher",
                                "aes-256-cbc",
                                algorithm,
                            ));
                        }
                        let key = hex_decode(&key);
                        let iv = hex_decode(&iv);
                        let value = hex_decode(&value);
                        if decrypt {
                            let cipher = cbc::Decryptor::<aes::Aes256>::new_from_slices(&key, &iv)
                                .map_err(|_| rquickjs::Error::new_from_js("key/iv", "AES-256-CBC"))?;
                            let output = if finalize {
                                cipher.decrypt_padded_vec_mut::<Pkcs7>(&value)
                            } else {
                                cipher.decrypt_padded_vec_mut::<NoPadding>(&value)
                            }
                                .map_err(|_| rquickjs::Error::new_from_js("ciphertext", "padded AES-256-CBC data"))?;
                            Ok(hex_encode(&output))
                        } else {
                            let cipher = cbc::Encryptor::<aes::Aes256>::new_from_slices(&key, &iv)
                                .map_err(|_| rquickjs::Error::new_from_js("key/iv", "AES-256-CBC"))?;
                            let output = if finalize {
                                cipher.encrypt_padded_vec_mut::<Pkcs7>(&value)
                            } else {
                                cipher.encrypt_padded_vec_mut::<NoPadding>(&value)
                            };
                            Ok(hex_encode(&output))
                        }
                    },
                )
                .expect("failed to create JavaScript cipher function");
                // Backs `createPrivateKey`/`createPublicKey`/`KeyObject.
                // asymmetricKeyType` and `createSign`/`createVerify`/
                // `crypto.sign`/`crypto.verify` (`asymmetric_crypto.rs`,
                // `platform_globals/buffer_crypto.js`) -- resolves PEM/
                // DER/passphrase-protected key material into a plain,
                // normalized PEM string every other crypto function here
                // consumes.
                let crypto_import_key = Function::new(
                    ctx.clone(),
                    |bytes_hex: String, is_der: bool, passphrase: String| {
                        crypto_import_key_json(&bytes_hex, is_der, &passphrase)
                    },
                )
                .expect("failed to create JavaScript key-import function");
                let crypto_asymmetric_sign = Function::new(
                    ctx.clone(),
                    |digest_algorithm: String, pem: String, data: String| {
                        crypto_asymmetric_sign_hex(&digest_algorithm, &pem, &hex_decode(&data))
                            .map(|signature| hex_encode(&signature))
                            .map_err(|error| {
                                rquickjs::Error::new_from_js_message("sign input", "valid key/data", error)
                            })
                    },
                )
                .expect("failed to create JavaScript asymmetric sign function");
                let crypto_asymmetric_sign_pss = Function::new(
                    ctx.clone(),
                    |digest_algorithm: String, pem: String, data: String| {
                        crypto_asymmetric_sign_pss_hex(&digest_algorithm, &pem, &hex_decode(&data))
                            .map(|signature| hex_encode(&signature))
                            .map_err(|error| {
                                rquickjs::Error::new_from_js_message("sign input", "valid RSA key/data", error)
                            })
                    },
                )
                .expect("failed to create JavaScript asymmetric PSS sign function");
                let crypto_asymmetric_verify = Function::new(
                    ctx.clone(),
                    |digest_algorithm: String, pem: String, data: String, signature: String| {
                        crypto_asymmetric_verify(
                            &digest_algorithm,
                            &pem,
                            &hex_decode(&data),
                            &hex_decode(&signature),
                        )
                        .map_err(|error| {
                            rquickjs::Error::new_from_js_message("verify input", "valid key/data", error)
                        })
                    },
                )
                .expect("failed to create JavaScript asymmetric verify function");
                let crypto_asymmetric_verify_pss = Function::new(
                    ctx.clone(),
                    |digest_algorithm: String, pem: String, data: String, signature: String| {
                        crypto_asymmetric_verify_pss(
                            &digest_algorithm,
                            &pem,
                            &hex_decode(&data),
                            &hex_decode(&signature),
                        )
                        .map_err(|error| {
                            rquickjs::Error::new_from_js_message("verify input", "valid RSA key/data", error)
                        })
                    },
                )
                .expect("failed to create JavaScript asymmetric PSS verify function");
                let crypto_asymmetric_encrypt = Function::new(
                    ctx.clone(),
                    |pem: String, oaep_digest: String, data: String| {
                        crypto_asymmetric_encrypt_hex(&pem, &oaep_digest, &hex_decode(&data))
                            .map(|value| hex_encode(&value))
                            .map_err(|error| {
                                rquickjs::Error::new_from_js_message(
                                    "encrypt input",
                                    "valid RSA key/data",
                                    error,
                                )
                            })
                    },
                )
                .expect("failed to create JavaScript asymmetric encrypt function");
                let crypto_asymmetric_decrypt = Function::new(
                    ctx.clone(),
                    |pem: String, oaep_digest: String, data: String| {
                        crypto_asymmetric_decrypt_hex(&pem, &oaep_digest, &hex_decode(&data))
                            .map(|value| hex_encode(&value))
                            .map_err(|error| {
                                rquickjs::Error::new_from_js_message(
                                    "decrypt input",
                                    "valid RSA key/data",
                                    error,
                                )
                            })
                    },
                )
                .expect("failed to create JavaScript asymmetric decrypt function");
                let crypto_generate_key_pair = Function::new(
                    ctx.clone(),
                    |key_type: String,
                     modulus_bits_or_curve: String,
                     public_exponent: String,
                     private_key_type: String,
                     public_key_type: String| {
                        crypto_generate_key_pair_json(
                            &key_type,
                            &modulus_bits_or_curve,
                            &public_exponent,
                            &private_key_type,
                            &public_key_type,
                        )
                        .map_err(|error| {
                            rquickjs::Error::new_from_js_message(
                                "generateKeyPair input",
                                "valid key type/options",
                                error,
                            )
                        })
                    },
                )
                .expect("failed to create JavaScript key-pair generation function");
                let hpack_huffman_encode = Function::new(ctx.clone(), |value: String| {
                    let mut output = Vec::new();
                    httlib_huffman::encode(&hex_decode(&value), &mut output)
                        .map(|()| hex_encode(&output))
                        .map_err(|error| {
                            rquickjs::Error::new_from_js_message(
                                "HPACK bytes",
                                "Huffman bytes",
                                error.to_string(),
                            )
                        })
                })
                .expect("failed to create HPACK Huffman encoder");
                let hpack_huffman_decode = Function::new(ctx.clone(), |value: String| {
                    let mut output = Vec::new();
                    httlib_huffman::decode(
                        &hex_decode(&value),
                        &mut output,
                        httlib_huffman::DecoderSpeed::FourBits,
                    )
                    .map(|()| hex_encode(&output))
                    .map_err(|error| {
                        rquickjs::Error::new_from_js_message(
                            "HPACK Huffman bytes",
                            "decoded bytes",
                            error.to_string(),
                        )
                    })
                })
                .expect("failed to create HPACK Huffman decoder");
                let zlib_hex = Function::new(
                    ctx.clone(),
                    |operation: String,
                     format: String,
                     value: String|
                     -> rquickjs::Result<String> {
                        let input = hex_decode(&value);
                        let result = if operation == "compress" {
                            compress_bytes(&format, &input)
                        } else {
                            decompress_bytes(&format, &input)
                        };
                        result.map(|bytes| hex_encode(&bytes)).map_err(|error| {
                            rquickjs::Error::new_from_js_message(
                                "zlib",
                                "Buffer",
                                error.to_string(),
                            )
                        })
                    },
                )
                .expect("failed to create JavaScript compression function");
                let web_zlib_streams = Rc::new(RefCell::new(HashMap::<u32, WebZlibStream>::new()));
                let next_web_zlib_stream = Rc::new(Cell::new(1u32));
                let create_web_zlib_stream = {
                    let streams = Rc::clone(&web_zlib_streams);
                    let next = Rc::clone(&next_web_zlib_stream);
                    Function::new(
                        ctx.clone(),
                        move |operation: String, format: String| -> rquickjs::Result<u32> {
                            let stream =
                                WebZlibStream::new(&operation, &format).map_err(|error| {
                                    rquickjs::Error::new_from_js_message(
                                        "zlib stream",
                                        "handle",
                                        error.to_string(),
                                    )
                                })?;
                            let handle = next.get();
                            next.set(handle.wrapping_add(1).max(1));
                            streams.borrow_mut().insert(handle, stream);
                            Ok(handle)
                        },
                    )
                    .expect("failed to create streaming zlib allocator")
                };
                let write_web_zlib_stream = {
                    let streams = Rc::clone(&web_zlib_streams);
                    Function::new(
                        ctx.clone(),
                        move |handle: u32,
                              value: String,
                              finish: bool|
                              -> rquickjs::Result<String> {
                            let input = hex_decode(&value);
                            let result = if finish {
                                let stream =
                                    streams.borrow_mut().remove(&handle).ok_or_else(|| {
                                        rquickjs::Error::new_from_js_message(
                                            "zlib stream",
                                            "handle",
                                            "unknown stream handle",
                                        )
                                    })?;
                                stream.finish(&input)
                            } else {
                                streams
                                    .borrow_mut()
                                    .get_mut(&handle)
                                    .ok_or_else(|| {
                                        rquickjs::Error::new_from_js_message(
                                            "zlib stream",
                                            "handle",
                                            "unknown stream handle",
                                        )
                                    })?
                                    .write_and_flush(&input)
                            };
                            result.map(|bytes| hex_encode(&bytes)).map_err(|error| {
                                rquickjs::Error::new_from_js_message(
                                    "zlib stream",
                                    "Buffer",
                                    error.to_string(),
                                )
                            })
                        },
                    )
                    .expect("failed to create streaming zlib writer")
                };
                let drop_web_zlib_stream = {
                    let streams = Rc::clone(&web_zlib_streams);
                    Function::new(ctx.clone(), move |handle: u32| {
                        streams.borrow_mut().remove(&handle).is_some()
                    })
                    .expect("failed to create streaming zlib closer")
                };
                let fs_function = Function::new(
                    ctx.clone(),
                    |operation: String, path: String, value: String, recursive: bool| {
                        host_fs(operation, path, value, recursive)
                    },
                )
                .expect("failed to create JavaScript filesystem function");
                let tcp_connect = Function::new(ctx.clone(), |host: String, port: u32| {
                    net_connect(&host, port as u16)
                })
                .expect("failed to create JavaScript TCP connector");
                let tcp_write = Function::new(ctx.clone(), |handle: u32, value: String| {
                    net_write(handle, &hex_decode(&value))
                })
                .expect("failed to create JavaScript TCP writer");
                let tcp_finish = Function::new(ctx.clone(), |handle: u32| net_finish(handle))
                    .expect("failed to create JavaScript TCP finisher");
                let tcp_shutdown =
                    Function::new(ctx.clone(), |handle: u32| net_shutdown_write(handle))
                        .expect("failed to create JavaScript TCP shutdown function");
                let tcp_poll_read = Function::new(ctx.clone(), |handle: u32| net_poll_read(handle))
                    .expect("failed to create JavaScript TCP polling reader");
                let tcp_destroy = Function::new(ctx.clone(), |handle: u32| net_destroy(handle))
                    .expect("failed to create JavaScript TCP closer");
                let tcp_listen = Function::new(ctx.clone(), |host: String, port: u32| {
                    net_listen(&host, port as u16)
                })
                .expect("failed to create JavaScript TCP listener");
                let tcp_accept = Function::new(ctx.clone(), |handle: u32| net_accept(handle))
                    .expect("failed to create JavaScript TCP acceptor");
                let tcp_poll_accept =
                    Function::new(ctx.clone(), |handle: u32| net_poll_accept(handle))
                        .expect("failed to create JavaScript TCP polling acceptor");
                let tcp_read = Function::new(ctx.clone(), |handle: u32| net_read_all(handle))
                    .expect("failed to create JavaScript TCP reader");
                let tcp_close_listener =
                    Function::new(ctx.clone(), |handle: u32| net_close_listener(handle))
                        .expect("failed to create JavaScript TCP listener closer");
                let udp_bind_function = Function::new(ctx.clone(), |host: String, port: u32| {
                    udp_bind(&host, port as u16)
                })
                .expect("failed to create JavaScript UDP binder");
                let udp_send_function = Function::new(
                    ctx.clone(),
                    |handle: u32, value: String, host: String, port: u32| {
                        udp_send(handle, &hex_decode(&value), &host, port as u16)
                    },
                )
                .expect("failed to create JavaScript UDP sender");
                let udp_receive_function =
                    Function::new(ctx.clone(), |handle: u32| udp_receive(handle))
                        .expect("failed to create JavaScript UDP receiver");
                let udp_close_function =
                    Function::new(ctx.clone(), |handle: u32| udp_close(handle))
                        .expect("failed to create JavaScript UDP closer");
                let tls_connect_function = Function::new(
                    ctx.clone(),
                    |host: String, port: u32, server_name: String, ca: String| {
                        tls_connect(TlsClientOptions {
                            host: &host,
                            port: port as u16,
                            server_name: &server_name,
                            ca_spec: &ca,
                            cert_spec: "",
                            key_spec: "",
                            alpn_spec: "",
                            report_alpn: false,
                            reject_unauthorized: true,
                        })
                    },
                )
                .expect("failed to create JavaScript TLS connector");
                let tls_connect_with_identity_function = Function::new(
                    ctx.clone(),
                    |host: String,
                     port: u32,
                     server_name: String,
                     ca: String,
                     cert: String,
                     key: String| {
                        tls_connect(TlsClientOptions {
                            host: &host,
                            port: port as u16,
                            server_name: &server_name,
                            ca_spec: &ca,
                            cert_spec: &cert,
                            key_spec: &key,
                            alpn_spec: "",
                            report_alpn: false,
                            reject_unauthorized: true,
                        })
                    },
                )
                .expect("failed to create JavaScript mutual TLS connector");
                let tls_connect_with_options_function = Function::new(
                    ctx.clone(),
                    |host: String,
                     port: u32,
                     server_name: String,
                     ca: String,
                     cert: String,
                     key: String,
                     options: String| {
                        let (verification, alpn) =
                            options.split_once('|').unwrap_or(("1", options.as_str()));
                        tls_connect(TlsClientOptions {
                            host: &host,
                            port: port as u16,
                            server_name: &server_name,
                            ca_spec: &ca,
                            cert_spec: &cert,
                            key_spec: &key,
                            alpn_spec: alpn,
                            report_alpn: true,
                            reject_unauthorized: verification != "0",
                        })
                    },
                )
                .expect("failed to create JavaScript TLS options connector");
                let tls_write_function =
                    Function::new(ctx.clone(), |handle: u32, value: String| {
                        tls_write(handle, &hex_decode(&value))
                    })
                    .expect("failed to create JavaScript TLS writer");
                let tls_finish_function =
                    Function::new(ctx.clone(), |handle: u32| tls_finish(handle))
                        .expect("failed to create JavaScript TLS finisher");
                let tls_shutdown_function =
                    Function::new(ctx.clone(), |handle: u32| tls_shutdown_write(handle))
                        .expect("failed to create JavaScript TLS shutdown function");
                let tls_poll_read_function =
                    Function::new(ctx.clone(), |handle: u32| tls_poll_read(handle))
                        .expect("failed to create JavaScript TLS polling reader");
                let tls_destroy_function =
                    Function::new(ctx.clone(), |handle: u32| tls_destroy(handle))
                        .expect("failed to create JavaScript TLS closer");
                let tls_alpn_function = Function::new(ctx.clone(), |handle: u32| tls_alpn(handle))
                    .expect("failed to create JavaScript TLS ALPN reader");
                let tls_peer_certificate_function =
                    Function::new(ctx.clone(), |handle: u32| tls_certificate(handle, true))
                        .expect("failed to create JavaScript TLS peer certificate reader");
                let tls_local_certificate_function =
                    Function::new(ctx.clone(), |handle: u32| tls_certificate(handle, false))
                        .expect("failed to create JavaScript TLS local certificate reader");
                let tls_peer_certificate_metadata_function =
                    Function::new(ctx.clone(), |handle: u32| {
                        tls_certificate_metadata(handle, true)
                    })
                    .expect("failed to create JavaScript TLS peer certificate metadata reader");
                let tls_local_certificate_metadata_function =
                    Function::new(ctx.clone(), |handle: u32| {
                        tls_certificate_metadata(handle, false)
                    })
                    .expect("failed to create JavaScript TLS local certificate metadata reader");
                let tls_server_listen_function = Function::new(
                    ctx.clone(),
                    |host: String, port: u32, cert: String, key: String| {
                        tls_server_listen(TlsServerOptions {
                            host: &host,
                            port: port as u16,
                            cert_spec: &cert,
                            key_spec: &key,
                            ca_spec: "",
                            request_cert: false,
                            reject_unauthorized: true,
                            alpn_spec: "",
                        })
                    },
                )
                .expect("failed to create JavaScript TLS listener");
                let tls_server_listen_with_ca_function = Function::new(
                    ctx.clone(),
                    |host: String,
                     port: u32,
                     cert: String,
                     key: String,
                     ca: String,
                     reject_unauthorized: bool| {
                        tls_server_listen(TlsServerOptions {
                            host: &host,
                            port: port as u16,
                            cert_spec: &cert,
                            key_spec: &key,
                            ca_spec: &ca,
                            request_cert: true,
                            reject_unauthorized,
                            alpn_spec: "",
                        })
                    },
                )
                .expect("failed to create JavaScript mutual TLS listener");
                let tls_server_listen_with_options_function = Function::new(
                    ctx.clone(),
                    |host: String,
                     port: u32,
                     cert: String,
                     key: String,
                     ca: String,
                     flags: u32,
                     alpn: String| {
                        tls_server_listen(TlsServerOptions {
                            host: &host,
                            port: port as u16,
                            cert_spec: &cert,
                            key_spec: &key,
                            ca_spec: &ca,
                            request_cert: flags & 1 != 0,
                            reject_unauthorized: flags & 2 != 0,
                            alpn_spec: &alpn,
                        })
                    },
                )
                .expect("failed to create JavaScript TLS options listener");
                let tls_server_accept_function =
                    Function::new(ctx.clone(), |handle: u32| tls_server_accept(handle))
                        .expect("failed to create JavaScript TLS acceptor");
                let tls_server_poll_accept_function =
                    Function::new(ctx.clone(), |handle: u32| tls_server_poll_accept(handle))
                        .expect("failed to create JavaScript TLS polling acceptor");
                let tls_server_read_function =
                    Function::new(ctx.clone(), |handle: u32| tls_server_read(handle))
                        .expect("failed to create JavaScript TLS reader");
                let tls_server_close_function =
                    Function::new(ctx.clone(), |handle: u32| tls_server_close_listener(handle))
                        .expect("failed to create JavaScript TLS listener closer");
                let os_info_function = Function::new(ctx.clone(), os_info_json)
                    .expect("failed to create JavaScript OS information source");
                // Backs `Intl.DateTimeFormat` (`platform_globals/intl.js`) --
                // the one native primitive needed to compute a real,
                // DST-aware IANA timezone offset/breakdown for a given
                // instant. See `quickjs/intl.rs`'s own doc comment.
                let intl_zoned_parts_function = Function::new(
                    ctx.clone(),
                    |tz_name: String, timestamp_ms: f64| {
                        intl_zoned_parts_json(&tz_name, timestamp_ms)
                    },
                )
                .expect("failed to create JavaScript Intl timezone source");
                ctx.globals()
                    .set("__thaw_crypto_random_hex", random_hex)
                    .expect("failed to install JavaScript random source");
                ctx.globals()
                    .set("__thaw_crypto_hash_hex", hash_hex)
                    .expect("failed to install JavaScript hash function");
                ctx.globals()
                    .set("__thaw_crypto_hmac_hex", hmac_hex)
                    .expect("failed to install JavaScript HMAC function");
                ctx.globals()
                    .set("__thaw_crypto_pbkdf2_hex", pbkdf2_hex)
                    .expect("failed to install JavaScript PBKDF2 function");
                ctx.globals()
                    .set("__thaw_crypto_scrypt_hex", scrypt_hex)
                    .expect("failed to install JavaScript scrypt function");
                ctx.globals()
                    .set("__thaw_crypto_cipher_hex", cipher_hex)
                    .expect("failed to install JavaScript cipher function");
                ctx.globals()
                    .set("__thaw_crypto_import_key_json", crypto_import_key)
                    .expect("failed to install JavaScript key-import function");
                ctx.globals()
                    .set("__thaw_crypto_asymmetric_sign_hex", crypto_asymmetric_sign)
                    .expect("failed to install JavaScript asymmetric sign function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_asymmetric_sign_pss_hex",
                        crypto_asymmetric_sign_pss,
                    )
                    .expect("failed to install JavaScript asymmetric PSS sign function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_asymmetric_verify",
                        crypto_asymmetric_verify,
                    )
                    .expect("failed to install JavaScript asymmetric verify function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_asymmetric_verify_pss",
                        crypto_asymmetric_verify_pss,
                    )
                    .expect("failed to install JavaScript asymmetric PSS verify function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_asymmetric_encrypt_hex",
                        crypto_asymmetric_encrypt,
                    )
                    .expect("failed to install JavaScript asymmetric encrypt function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_asymmetric_decrypt_hex",
                        crypto_asymmetric_decrypt,
                    )
                    .expect("failed to install JavaScript asymmetric decrypt function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_generate_key_pair_json",
                        crypto_generate_key_pair,
                    )
                    .expect("failed to install JavaScript key-pair generation function");
                ctx.globals()
                    .set("__thaw_intl_zoned_parts", intl_zoned_parts_function)
                    .expect("failed to install JavaScript Intl timezone source");
                ctx.globals()
                    .set("__thaw_hpack_huffman_encode", hpack_huffman_encode)
                    .expect("failed to install HPACK Huffman encoder");
                ctx.globals()
                    .set("__thaw_hpack_huffman_decode", hpack_huffman_decode)
                    .expect("failed to install HPACK Huffman decoder");
                ctx.globals()
                    .set("__thaw_zlib_hex", zlib_hex)
                    .expect("failed to install JavaScript compression function");
                ctx.globals()
                    .set("__thaw_zlib_stream_create", create_web_zlib_stream)
                    .expect("failed to install streaming zlib allocator");
                ctx.globals()
                    .set("__thaw_zlib_stream_write", write_web_zlib_stream)
                    .expect("failed to install streaming zlib writer");
                ctx.globals()
                    .set("__thaw_zlib_stream_drop", drop_web_zlib_stream)
                    .expect("failed to install streaming zlib closer");
                ctx.globals()
                    .set("__thaw_fs", fs_function)
                    .expect("failed to install JavaScript filesystem function");
                ctx.globals()
                    .set("__thaw_net_connect", tcp_connect)
                    .expect("failed to install JavaScript TCP connector");
                ctx.globals()
                    .set("__thaw_net_write", tcp_write)
                    .expect("failed to install JavaScript TCP writer");
                ctx.globals()
                    .set("__thaw_net_finish", tcp_finish)
                    .expect("failed to install JavaScript TCP finisher");
                ctx.globals()
                    .set("__thaw_net_shutdown", tcp_shutdown)
                    .expect("failed to install JavaScript TCP shutdown function");
                ctx.globals()
                    .set("__thaw_net_poll_read", tcp_poll_read)
                    .expect("failed to install JavaScript TCP polling reader");
                ctx.globals()
                    .set("__thaw_net_destroy", tcp_destroy)
                    .expect("failed to install JavaScript TCP closer");
                ctx.globals()
                    .set("__thaw_net_listen", tcp_listen)
                    .expect("failed to install JavaScript TCP listener");
                ctx.globals()
                    .set("__thaw_net_accept", tcp_accept)
                    .expect("failed to install JavaScript TCP acceptor");
                ctx.globals()
                    .set("__thaw_net_poll_accept", tcp_poll_accept)
                    .expect("failed to install JavaScript TCP polling acceptor");
                ctx.globals()
                    .set("__thaw_net_read", tcp_read)
                    .expect("failed to install JavaScript TCP reader");
                ctx.globals()
                    .set("__thaw_net_close_listener", tcp_close_listener)
                    .expect("failed to install JavaScript TCP listener closer");
                ctx.globals()
                    .set("__thaw_udp_bind", udp_bind_function)
                    .expect("failed to install JavaScript UDP binder");
                ctx.globals()
                    .set("__thaw_udp_send", udp_send_function)
                    .expect("failed to install JavaScript UDP sender");
                ctx.globals()
                    .set("__thaw_udp_receive", udp_receive_function)
                    .expect("failed to install JavaScript UDP receiver");
                ctx.globals()
                    .set("__thaw_udp_close", udp_close_function)
                    .expect("failed to install JavaScript UDP closer");
                ctx.globals()
                    .set("__thaw_tls_connect", tls_connect_function)
                    .expect("failed to install JavaScript TLS connector");
                ctx.globals()
                    .set(
                        "__thaw_tls_connect_with_identity",
                        tls_connect_with_identity_function,
                    )
                    .expect("failed to install JavaScript mutual TLS connector");
                ctx.globals()
                    .set(
                        "__thaw_tls_connect_with_options",
                        tls_connect_with_options_function,
                    )
                    .expect("failed to install JavaScript TLS options connector");
                ctx.globals()
                    .set("__thaw_tls_write", tls_write_function)
                    .expect("failed to install JavaScript TLS writer");
                ctx.globals()
                    .set("__thaw_tls_finish", tls_finish_function)
                    .expect("failed to install JavaScript TLS finisher");
                ctx.globals()
                    .set("__thaw_tls_shutdown", tls_shutdown_function)
                    .expect("failed to install JavaScript TLS shutdown function");
                ctx.globals()
                    .set("__thaw_tls_poll_read", tls_poll_read_function)
                    .expect("failed to install JavaScript TLS polling reader");
                ctx.globals()
                    .set("__thaw_tls_destroy", tls_destroy_function)
                    .expect("failed to install JavaScript TLS closer");
                ctx.globals()
                    .set("__thaw_tls_alpn", tls_alpn_function)
                    .expect("failed to install JavaScript TLS ALPN reader");
                ctx.globals()
                    .set("__thaw_tls_peer_certificate", tls_peer_certificate_function)
                    .expect("failed to install JavaScript TLS peer certificate reader");
                ctx.globals()
                    .set(
                        "__thaw_tls_local_certificate",
                        tls_local_certificate_function,
                    )
                    .expect("failed to install JavaScript TLS local certificate reader");
                ctx.globals()
                    .set(
                        "__thaw_tls_peer_certificate_metadata",
                        tls_peer_certificate_metadata_function,
                    )
                    .expect("failed to install JavaScript TLS peer certificate metadata reader");
                ctx.globals()
                    .set(
                        "__thaw_tls_local_certificate_metadata",
                        tls_local_certificate_metadata_function,
                    )
                    .expect("failed to install JavaScript TLS local certificate metadata reader");
                ctx.globals()
                    .set("__thaw_tls_server_listen", tls_server_listen_function)
                    .expect("failed to install JavaScript TLS listener");
                ctx.globals()
                    .set(
                        "__thaw_tls_server_listen_with_ca",
                        tls_server_listen_with_ca_function,
                    )
                    .expect("failed to install JavaScript mutual TLS listener");
                ctx.globals()
                    .set(
                        "__thaw_tls_server_listen_with_options",
                        tls_server_listen_with_options_function,
                    )
                    .expect("failed to install JavaScript TLS options listener");
                ctx.globals()
                    .set("__thaw_tls_server_accept", tls_server_accept_function)
                    .expect("failed to install JavaScript TLS acceptor");
                ctx.globals()
                    .set(
                        "__thaw_tls_server_poll_accept",
                        tls_server_poll_accept_function,
                    )
                    .expect("failed to install JavaScript TLS polling acceptor");
                ctx.globals()
                    .set("__thaw_tls_server_read", tls_server_read_function)
                    .expect("failed to install JavaScript TLS reader");
                ctx.globals()
                    .set("__thaw_tls_server_close", tls_server_close_function)
                    .expect("failed to install JavaScript TLS listener closer");
                ctx.globals()
                    .set("__thaw_os_info", os_info_function)
                    .expect("failed to install JavaScript OS information source");
                ctx.eval::<(), _>(PLATFORM_GLOBALS)
                    .expect("failed to install JavaScript platform globals");
            });
            (runtime, context)
        });
    });
}

fn with_context<R>(f: impl FnOnce(Ctx<'_>) -> R) -> R {
    ensure_context();
    JS.with(|cell| {
        cell.borrow()
            .as_ref()
            .expect("QuickJS context was initialized")
            .1
            .with(f)
    })
}

/// Like [`with_context`], but reuses the currently-active `Ctx` (see
/// `ActiveNapiContext`) instead of calling `with_context` again when one
/// is already active -- needed for a native callback invoked *by* a
/// dynamic call (real example: zod's `.superRefine((val, ctx) => { ctx.
/// addIssue(...); })`) that itself makes a further dynamic call from
/// inside its own body: the *outer* dynamic call is still on the stack
/// at that point (its own `with_context`'s `RefCell` borrow, and
/// `Context::with`'s own internal runtime lock, both still held), so a
/// second top-level `with_context` call would panic ("RefCell already
/// borrowed") rather than deadlock or corrupt anything -- confirmed via
/// a real repro. Bypassing `with_context`/`Context::with` entirely on
/// the reentrant path (reusing the already-active `Ctx` directly, not
/// re-locking anything) avoids both.
fn with_active_or_context<R>(f: impl for<'js> FnOnce(Ctx<'js>) -> R) -> R {
    ACTIVE_NAPI_CONTEXT.with(|active| {
        let active = active.get();
        if active.is_null() {
            with_context(f)
        } else {
            // SAFETY: `active` was set by `ActiveNapiContext::enter`,
            // called with a `Ctx` still alive on the stack of the outer
            // call currently reentering into us -- it hasn't been
            // dropped, only reborrowed here for the duration of `f`.
            let ctx = unsafe { (*(active as *const Ctx<'static>)).clone() };
            f(ctx)
        }
    })
}
