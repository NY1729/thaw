fn host_worker_bootstrap(worker_data_json: &str, config_json: &str, thread_id: u32) -> String {
    let worker_data = serde_json::to_string(worker_data_json).unwrap_or_else(|_| "\"null\"".into());
    let config = serde_json::to_string(config_json).unwrap_or_else(|_| "\"{}\"".into());
    format!(
        r#"
        globalThis.__thaw_host_worker_events = [];
        globalThis.__thaw_host_worker_closed = false;
        const __thaw_host_worker_listeners = new Map();
        const __thaw_host_ports = new Map();
        let __thaw_host_port_sequence = 1;
        function __thaw_configure_worker_port_bridge(port, id) {{
          port.__thawHostPortId = id;
          port.__thawSchedule = function() {{
            while (port.__thawQueue.length) {{ const record = port.__thawQueue.shift(); __thaw_host_worker_events.push({{ type: 'port', port: id, payload: __thaw_worker_encode(record.data) }}); }}
          }};
        }}
        function __thaw_prepare_worker_ports(transfer) {{
          for (const port of transfer || []) {{
            if (!(port instanceof MessagePort)) continue;
            if (port.__thawHostPortId) continue;
            const id = 'w:{thread_id}:' + (__thaw_host_port_sequence++), moved = port.__thawTransfer();
            port.__thawHostPortId = id; __thaw_configure_worker_port_bridge(moved, id);
            const receiver = moved.__thawPeer, close = receiver.close.bind(receiver); receiver.close = function() {{ __thaw_host_ports.delete(id); close(); }};
            __thaw_host_ports.set(id, receiver);
          }}
        }}
        globalThis.__thaw_create_worker_port = function(id) {{
          id = String(id); if (__thaw_host_ports.has(id)) return __thaw_host_ports.get(id);
          const port = new MessagePort(); port.__thawHostPortId = id;
          port.postMessage = function(value, transfer) {{ __thaw_prepare_worker_ports(transfer); __thaw_host_worker_events.push({{ type: 'port', port: id, payload: __thaw_worker_encode(value) }}); }};
          const close = port.close.bind(port); port.close = function() {{ __thaw_host_ports.delete(id); close(); }};
          __thaw_host_ports.set(id, port); return port;
        }};
        function __thaw_host_worker_on(name, listener) {{
          const key = String(name), list = __thaw_host_worker_listeners.get(key) || [];
          list.push(listener); __thaw_host_worker_listeners.set(key, list); return parentPort;
        }}
        function __thaw_host_worker_off(name, listener) {{
          const key = String(name), list = __thaw_host_worker_listeners.get(key) || [];
          __thaw_host_worker_listeners.set(key, list.filter(item => item !== listener && item.listener !== listener)); return parentPort;
        }}
        const parentPort = {{
          postMessage(value, transfer) {{
            __thaw_prepare_worker_ports(transfer);
            const payload = __thaw_worker_encode(value);
            __thaw_host_worker_events.push({{ type: 'message', payload }});
          }},
          on: __thaw_host_worker_on,
          addListener: __thaw_host_worker_on,
          once(name, listener) {{ const wrapped = (...args) => {{ __thaw_host_worker_off(name, wrapped); listener(...args); }}; wrapped.listener = listener; return __thaw_host_worker_on(name, wrapped); }},
          off: __thaw_host_worker_off,
          removeListener: __thaw_host_worker_off,
          close() {{ globalThis.__thaw_host_worker_closed = true; }},
          start() {{}}, ref() {{ return this; }}, unref() {{ return this; }}, hasRef() {{ return true; }}
        }};
        const workerData = __thaw_worker_decode({worker_data});
        const __thaw_host_worker_config = JSON.parse({config});
        let __thaw_direct_request = 1;
        const __thaw_direct_pending = new Map();
        globalThis.module = {{ exports: {{}} }};
        globalThis.exports = globalThis.module.exports;
        process.env = Object.assign({{}}, __thaw_host_worker_config.env || {{}});
        if (__thaw_host_worker_config.shareEnv) process.env = new Proxy({{}}, {{
          get(target, key) {{ return typeof key === 'symbol' ? target[key] : __thaw_shared_env_get(String(key)); }},
          set(target, key, value) {{ if (typeof key !== 'symbol') __thaw_shared_env_set(String(key), String(value)); else target[key] = value; return true; }},
          deleteProperty(target, key) {{ return typeof key === 'symbol' ? delete target[key] : __thaw_shared_env_delete(String(key)); }},
          ownKeys() {{ return JSON.parse(__thaw_shared_env_keys()); }},
          getOwnPropertyDescriptor(target, key) {{ if (typeof key === 'symbol') return Object.getOwnPropertyDescriptor(target, key); const value = __thaw_shared_env_get(String(key)); return value === undefined ? undefined : {{ value, writable: true, enumerable: true, configurable: true }}; }}
        }});
        process.argv = Array.from(__thaw_host_worker_config.argv || []);
        process.execArgv = Array.from(__thaw_host_worker_config.execArgv || []);
        const __thaw_stdin_listeners = new Map();
        process.stdin = {{
          setEncoding(encoding) {{ this.encoding = String(encoding); return this; }},
          on(name, listener) {{ const key = String(name), list = __thaw_stdin_listeners.get(key) || []; list.push(listener); __thaw_stdin_listeners.set(key, list); return this; }},
          once(name, listener) {{ const wrapped = value => {{ this.off(name, wrapped); listener(value); }}; wrapped.listener = listener; return this.on(name, wrapped); }},
          off(name, listener) {{ const key = String(name), list = __thaw_stdin_listeners.get(key) || []; __thaw_stdin_listeners.set(key, list.filter(item => item !== listener && item.listener !== listener)); return this; }},
          resume() {{ return this; }}, pause() {{ return this; }}
        }};
        process.stdout = {{ write(value) {{ __thaw_host_worker_events.push({{ type: 'stdout', payload: String(value) }}); return true; }} }};
        process.stderr = {{ write(value) {{ __thaw_host_worker_events.push({{ type: 'stderr', payload: String(value) }}); return true; }} }};
        if (__thaw_host_worker_config.stdout) console.log = console.info = (...values) => process.stdout.write(values.map(String).join(' ') + '\n');
        if (__thaw_host_worker_config.stderr) console.warn = console.error = (...values) => process.stderr.write(values.map(String).join(' ') + '\n');
        function __thaw_host_post_message_to_thread(target, value, transferList, timeout) {{
          target = Number(target);
          if (target === {thread_id}) {{ const error = new Error('Cannot send a message to the same thread'); error.code = 'ERR_WORKER_MESSAGING_SAME_THREAD'; return Promise.reject(error); }}
          const cloned = structuredClone(value, {{ transfer: transferList || [] }}), payload = __thaw_worker_encode(cloned), request = __thaw_direct_request++;
          return new Promise((resolve, reject) => {{
            let timer;
            if (timeout !== undefined && Number(timeout) >= 0) timer = setTimeout(() => {{ if (__thaw_direct_pending.delete(request)) {{ const error = new Error('The destination thread did not process the message'); error.code = 'ERR_WORKER_MESSAGING_TIMEOUT'; reject(error); }} }}, Number(timeout));
            __thaw_direct_pending.set(request, {{ resolve, reject, timer }});
            __thaw_host_worker_events.push({{ type: 'direct', target, source: {thread_id}, request, payload }});
          }});
        }}
        globalThis.__thaw_worker_module = {{ isMainThread: false, threadId: {thread_id}, threadName: String(__thaw_host_worker_config.threadName || ''), workerData, parentPort, resourceLimits: Object.assign({{}}, __thaw_host_worker_config.resourceLimits || {{}}), MessageChannel, MessagePort, BroadcastChannel, receiveMessageOnPort(port) {{ const record = port && port.__thawQueue && port.__thawQueue.shift(); return record ? {{ message: record.data }} : undefined; }}, postMessageToThread: __thaw_host_post_message_to_thread }};
        globalThis.require = function(name) {{
          if (name === 'worker_threads' || name === 'node:worker_threads') return globalThis.__thaw_worker_module;
          if (typeof globalThis.__thaw_bundle_create_require === 'function') return globalThis.__thaw_bundle_create_require('')(name);
          throw new Error("require('" + name + "') is not available in this Worker");
        }};
        globalThis.__thaw_host_worker_deliver = function(payload) {{
          const value = __thaw_worker_decode(payload);
          for (const listener of (__thaw_host_worker_listeners.get('message') || []).slice()) listener(value);
        }};
        globalThis.__thaw_host_worker_stdin = function(payload, ended) {{
          const name = ended ? 'end' : 'data';
          for (const listener of (__thaw_stdin_listeners.get(name) || []).slice()) listener(ended ? undefined : payload);
        }};
        globalThis.__thaw_host_worker_port = function(id, payload) {{
          const port = __thaw_host_ports.get(String(id)); if (!port) return;
          port.__thawQueue.push({{ data: __thaw_worker_decode(payload), ports: [] }}); port.__thawSchedule();
        }};
        globalThis.__thaw_host_worker_direct_deliver = function(payload, source) {{
          if (!process.listenerCount || process.listenerCount('workerMessage') === 0) return 'ERR_WORKER_MESSAGING_FAILED:The destination thread has no workerMessage listener';
          try {{ process.emit('workerMessage', __thaw_worker_decode(payload), Number(source)); return ''; }}
          catch (error) {{ return 'ERR_WORKER_MESSAGING_ERRORED:' + String(error && error.message || error); }}
        }};
        globalThis.__thaw_host_worker_direct_result = function(request, errorText) {{
          const pending = __thaw_direct_pending.get(Number(request)); if (!pending) return;
          __thaw_direct_pending.delete(Number(request)); if (pending.timer) clearTimeout(pending.timer);
          if (!errorText) pending.resolve(); else {{ const separator = errorText.indexOf(':'), code = separator < 0 ? errorText : errorText.slice(0, separator), message = separator < 0 ? errorText : errorText.slice(separator + 1), error = new Error(message); error.code = code; if (code === 'ERR_WORKER_MESSAGING_ERRORED') error.cause = new Error(message); pending.reject(error); }}
        }};
        globalThis.__thaw_host_worker_drain = function() {{ return JSON.stringify(__thaw_host_worker_events.splice(0)); }};
        globalThis.__thaw_host_worker_should_exit = function() {{
          return globalThis.__thaw_host_worker_closed || (((__thaw_host_worker_listeners.get('message') || []).length === 0) && ((!process.listenerCount || process.listenerCount('workerMessage') === 0)) && __thaw_direct_pending.size === 0 && __thaw_host_ports.size === 0 && ((__thaw_stdin_listeners.get('data') || []).length === 0) && ((__thaw_stdin_listeners.get('end') || []).length === 0) && __thaw_next_timer_delay() < 0);
        }};
        "#
    )
}

fn drain_host_worker_events(ctx: &Ctx<'_>, events: &Sender<HostWorkerEvent>) -> Result<(), String> {
    let drain: Function = ctx
        .globals()
        .get("__thaw_host_worker_drain")
        .map_err(|error| error.to_string())?;
    let payloads: String = drain.call(()).map_err(|error| error.to_string())?;
    let payloads: Vec<serde_json::Value> =
        serde_json::from_str(&payloads).map_err(|error| error.to_string())?;
    for event in payloads {
        let kind = event.get("type").and_then(|value| value.as_str());
        let payload = event
            .get("payload")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let event = match kind {
            Some("stdout") => HostWorkerEvent::Stdout(payload),
            Some("stderr") => HostWorkerEvent::Stderr(payload),
            Some("direct") => HostWorkerEvent::DirectRequest {
                target: event
                    .get("target")
                    .and_then(|value| value.as_u64())
                    .unwrap_or_default() as u32,
                source: event
                    .get("source")
                    .and_then(|value| value.as_u64())
                    .unwrap_or_default() as u32,
                request: event
                    .get("request")
                    .and_then(|value| value.as_u64())
                    .unwrap_or_default(),
                payload,
            },
            Some("port") => HostWorkerEvent::PortMessage {
                port: event
                    .get("port")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
                payload,
            },
            _ => HostWorkerEvent::Message(payload),
        };
        let _ = events.send(event);
    }
    Ok(())
}

fn run_host_worker(
    start: HostWorkerStart,
    commands: Receiver<HostWorkerCommand>,
    events: Sender<HostWorkerEvent>,
) {
    let result = with_context(|ctx| -> Result<i32, String> {
        install_shared_environment_functions(&ctx, start.shared_env.clone())?;
        if !start.bundle_source.is_empty() {
            load_impl(ctx.clone(), &start.bundle_source)?;
        }
        load_impl(
            ctx.clone(),
            &host_worker_bootstrap(&start.worker_data_json, &start.config_json, start.thread_id),
        )?;
        let _ = events.send(HostWorkerEvent::Online);
        load_impl(
            ctx.clone(),
            &format!("(function() {{\n{}\n}}).call(globalThis);", start.source),
        )?;
        loop {
            while ctx.execute_pending_job() {}
            let run_due: Function = ctx
                .globals()
                .get("__thaw_run_due_timers")
                .map_err(|error| error.to_string())?;
            run_due
                .call::<_, usize>(())
                .map_err(|error| error.to_string())?;
            while ctx.execute_pending_job() {}
            drain_host_worker_events(&ctx, &events)?;

            let should_exit: Function = ctx
                .globals()
                .get("__thaw_host_worker_should_exit")
                .map_err(|error| error.to_string())?;
            if should_exit
                .call::<_, bool>(())
                .map_err(|error| error.to_string())?
            {
                return Ok(0);
            }

            match commands.recv_timeout(Duration::from_millis(1)) {
                Ok(HostWorkerCommand::Message(payload)) => {
                    let deliver: Function = ctx
                        .globals()
                        .get("__thaw_host_worker_deliver")
                        .map_err(|error| error.to_string())?;
                    deliver
                        .call::<_, ()>((payload,))
                        .map_err(|error| match error {
                            rquickjs::Error::Exception => describe_exception(&ctx),
                            error => error.to_string(),
                        })?;
                }
                Ok(HostWorkerCommand::Stdin(payload)) => {
                    let deliver: Function = ctx
                        .globals()
                        .get("__thaw_host_worker_stdin")
                        .map_err(|error| error.to_string())?;
                    deliver
                        .call::<_, ()>((payload, false))
                        .map_err(|error| error.to_string())?;
                }
                Ok(HostWorkerCommand::StdinEnd) => {
                    let deliver: Function = ctx
                        .globals()
                        .get("__thaw_host_worker_stdin")
                        .map_err(|error| error.to_string())?;
                    deliver
                        .call::<_, ()>((String::new(), true))
                        .map_err(|error| error.to_string())?;
                }
                Ok(HostWorkerCommand::DirectMessage {
                    payload,
                    source,
                    request,
                    reply,
                }) => {
                    let deliver: Function = ctx
                        .globals()
                        .get("__thaw_host_worker_direct_deliver")
                        .map_err(|error| error.to_string())?;
                    let error: String = deliver
                        .call((payload, source))
                        .map_err(|error| error.to_string())?;
                    let error = (!error.is_empty()).then_some(error);
                    if let Some(reply) = reply {
                        let _ = reply.send(HostWorkerCommand::DirectResult { request, error });
                    } else {
                        let _ = events.send(HostWorkerEvent::ParentDirectResult { request, error });
                    }
                }
                Ok(HostWorkerCommand::DirectResult { request, error }) => {
                    let resolve: Function = ctx
                        .globals()
                        .get("__thaw_host_worker_direct_result")
                        .map_err(|error| error.to_string())?;
                    resolve
                        .call::<_, ()>((request, error.unwrap_or_default()))
                        .map_err(|error| error.to_string())?;
                }
                Ok(HostWorkerCommand::PortMessage { port, payload }) => {
                    let deliver: Function = ctx
                        .globals()
                        .get("__thaw_host_worker_port")
                        .map_err(|error| error.to_string())?;
                    deliver
                        .call::<_, ()>((port, payload))
                        .map_err(|error| error.to_string())?;
                }
                Ok(HostWorkerCommand::Terminate) => return Ok(1),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(1),
            }
        }
    });
    match result {
        Ok(code) => {
            let _ = events.send(HostWorkerEvent::Exit(code));
        }
        Err(error) => {
            let _ = events.send(HostWorkerEvent::Error(error));
            let _ = events.send(HostWorkerEvent::Exit(1));
        }
    }
}

fn install_shared_environment_functions(
    ctx: &Ctx<'_>,
    shared_env: Arc<Mutex<HashMap<String, String>>>,
) -> Result<(), String> {
    let getter_env = shared_env.clone();
    let getter = Function::new(ctx.clone(), move |key: String| {
        getter_env
            .lock()
            .expect("shared Worker environment poisoned")
            .get(&key)
            .cloned()
    })
    .map_err(|error| error.to_string())?;
    let setter_env = shared_env.clone();
    let setter = Function::new(ctx.clone(), move |key: String, value: String| {
        setter_env
            .lock()
            .expect("shared Worker environment poisoned")
            .insert(key, value);
    })
    .map_err(|error| error.to_string())?;
    let delete_env = shared_env.clone();
    let deleter = Function::new(ctx.clone(), move |key: String| {
        delete_env
            .lock()
            .expect("shared Worker environment poisoned")
            .remove(&key)
            .is_some()
    })
    .map_err(|error| error.to_string())?;
    let keys_env = shared_env;
    let keys = Function::new(ctx.clone(), move || {
        serde_json::to_string(
            &keys_env
                .lock()
                .expect("shared Worker environment poisoned")
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".into())
    })
    .map_err(|error| error.to_string())?;
    ctx.globals()
        .set("__thaw_shared_env_get", getter)
        .map_err(|error| error.to_string())?;
    ctx.globals()
        .set("__thaw_shared_env_set", setter)
        .map_err(|error| error.to_string())?;
    ctx.globals()
        .set("__thaw_shared_env_delete", deleter)
        .map_err(|error| error.to_string())?;
    ctx.globals()
        .set("__thaw_shared_env_keys", keys)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn initialize_shared_environment(json: String) -> bool {
    let Ok(values) = serde_json::from_str::<HashMap<String, String>>(&json) else {
        return false;
    };
    HOST_WORKERS.with(|table| {
        *table
            .borrow()
            .shared_env
            .lock()
            .expect("shared Worker environment poisoned") = values;
    });
    true
}

fn spawn_host_worker(
    bundle_source: String,
    source: String,
    worker_data_json: String,
    config_json: String,
    thread_id: u32,
) -> u32 {
    let (command_sender, command_receiver) = mpsc::channel();
    let (event_sender, event_receiver) = mpsc::channel();
    let shared_env = HOST_WORKERS.with(|table| table.borrow().shared_env.clone());
    let thread = std::thread::spawn(move || {
        run_host_worker(
            HostWorkerStart {
                bundle_source,
                source,
                worker_data_json,
                config_json,
                thread_id,
                shared_env,
            },
            command_receiver,
            event_sender,
        );
    });
    HOST_WORKERS.with(|table| {
        let mut table = table.borrow_mut();
        let handle = table.next_handle;
        table.next_handle = table.next_handle.wrapping_add(1).max(1);
        table.workers.insert(
            handle,
            HostWorker {
                commands: command_sender,
                events: event_receiver,
                thread: Some(thread),
            },
        );
        handle
    })
}

fn send_host_worker(handle: u32, payload: String) -> bool {
    HOST_WORKERS.with(|table| {
        table.borrow().workers.get(&handle).is_some_and(|worker| {
            worker
                .commands
                .send(HostWorkerCommand::Message(payload))
                .is_ok()
        })
    })
}

fn send_host_worker_stdin(handle: u32, payload: String, ended: bool) -> bool {
    HOST_WORKERS.with(|table| {
        table.borrow().workers.get(&handle).is_some_and(|worker| {
            let command = if ended {
                HostWorkerCommand::StdinEnd
            } else {
                HostWorkerCommand::Stdin(payload)
            };
            worker.commands.send(command).is_ok()
        })
    })
}

fn route_host_worker_direct(
    origin: u32,
    target: u32,
    payload: String,
    source: u32,
    request: u64,
) -> bool {
    HOST_WORKERS.with(|table| {
        let table = table.borrow();
        let Some(reply) = table
            .workers
            .get(&origin)
            .map(|worker| worker.commands.clone())
        else {
            return false;
        };
        table.workers.get(&target).is_some_and(|worker| {
            worker
                .commands
                .send(HostWorkerCommand::DirectMessage {
                    payload,
                    source,
                    request,
                    reply: Some(reply),
                })
                .is_ok()
        })
    })
}

fn route_parent_direct(target: u32, payload: String, request: u64) -> bool {
    HOST_WORKERS.with(|table| {
        table.borrow().workers.get(&target).is_some_and(|worker| {
            worker
                .commands
                .send(HostWorkerCommand::DirectMessage {
                    payload,
                    source: 0,
                    request,
                    reply: None,
                })
                .is_ok()
        })
    })
}

fn resolve_host_worker_direct(handle: u32, request: u64, error: String) -> bool {
    HOST_WORKERS.with(|table| {
        table.borrow().workers.get(&handle).is_some_and(|worker| {
            worker
                .commands
                .send(HostWorkerCommand::DirectResult {
                    request,
                    error: (!error.is_empty()).then_some(error),
                })
                .is_ok()
        })
    })
}

fn send_host_worker_port(handle: u32, port: String, payload: String) -> bool {
    HOST_WORKERS.with(|table| {
        table.borrow().workers.get(&handle).is_some_and(|worker| {
            worker
                .commands
                .send(HostWorkerCommand::PortMessage { port, payload })
                .is_ok()
        })
    })
}

fn terminate_host_worker(handle: u32) -> bool {
    HOST_WORKERS.with(|table| {
        table
            .borrow()
            .workers
            .get(&handle)
            .is_some_and(|worker| worker.commands.send(HostWorkerCommand::Terminate).is_ok())
    })
}

fn host_workers_active() -> bool {
    HOST_WORKERS.with(|table| !table.borrow().workers.is_empty())
}

fn poll_host_workers() -> String {
    HOST_WORKERS.with(|table| {
        let mut table = table.borrow_mut();
        let handles = table.workers.keys().copied().collect::<Vec<_>>();
        let mut output = Vec::new();
        let mut finished = Vec::new();
        for handle in handles {
            let Some(worker) = table.workers.get(&handle) else {
                continue;
            };
            loop {
                match worker.events.try_recv() {
                    Ok(HostWorkerEvent::Online) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "online" }));
                    }
                    Ok(HostWorkerEvent::Message(payload)) => {
                        output.push(
                            serde_json::json!({ "handle": handle, "type": "message", "payload": payload }),
                        );
                    }
                    Ok(HostWorkerEvent::Stdout(payload)) => {
                        output.push(
                            serde_json::json!({ "handle": handle, "type": "stdout", "payload": payload }),
                        );
                    }
                    Ok(HostWorkerEvent::Stderr(payload)) => {
                        output.push(
                            serde_json::json!({ "handle": handle, "type": "stderr", "payload": payload }),
                        );
                    }
                    Ok(HostWorkerEvent::DirectRequest {
                        target,
                        source,
                        request,
                        payload,
                    }) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "direct", "target": target, "source": source, "request": request, "payload": payload }));
                    }
                    Ok(HostWorkerEvent::ParentDirectResult { request, error }) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "directResult", "request": request, "error": error }));
                    }
                    Ok(HostWorkerEvent::PortMessage { port, payload }) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "port", "port": port, "payload": payload }));
                    }
                    Ok(HostWorkerEvent::Error(error)) => {
                        output.push(
                        serde_json::json!({ "handle": handle, "type": "error", "error": error }),
                    );
                    }
                    Ok(HostWorkerEvent::Exit(code)) => {
                        output.push(
                            serde_json::json!({ "handle": handle, "type": "exit", "code": code }),
                        );
                        finished.push(handle);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished.push(handle);
                        break;
                    }
                }
            }
        }
        finished.sort_unstable();
        finished.dedup();
        for handle in finished {
            if let Some(mut worker) = table.workers.remove(&handle) {
                if let Some(thread) = worker.thread.take() {
                    let _ = thread.join();
                }
            }
        }
        serde_json::to_string(&output).unwrap_or_else(|_| "[]".into())
    })
}

fn configure_child_command(
    command: &str,
    arguments: &[String],
    options: &serde_json::Value,
) -> Command {
    let mut child = Command::new(command);
    child.args(arguments);
    if let Some(cwd) = options.get("cwd").and_then(|value| value.as_str()) {
        child.current_dir(cwd);
    }
    if let Some(environment) = options.get("env").and_then(|value| value.as_object()) {
        child.env_clear();
        for (name, value) in environment {
            if let Some(value) = value.as_str() {
                child.env(name, value);
            }
        }
    }
    #[cfg(unix)]
    if options
        .get("detached")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        // SAFETY: setsid has no memory-safety preconditions and this closure
        // performs no allocation or lock acquisition after fork.
        unsafe {
            child.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
    }
    child
}

fn run_host_child(
    command: String,
    arguments: Vec<String>,
    options: serde_json::Value,
    commands: Receiver<HostChildCommand>,
    events: Sender<HostChildEvent>,
) {
    let mut process = configure_child_command(&command, &arguments, &options);
    process
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut process = match process.spawn() {
        Ok(process) => process,
        Err(error) => {
            let code = if error.kind() == io::ErrorKind::NotFound {
                "ENOENT"
            } else if error.kind() == io::ErrorKind::PermissionDenied {
                "EACCES"
            } else {
                "UNKNOWN"
            };
            let _ = events.send(HostChildEvent::Error {
                message: error.to_string(),
                code: code.to_string(),
            });
            let _ = events.send(HostChildEvent::Exit {
                code: None,
                signal: None,
            });
            return;
        }
    };
    let _ = events.send(HostChildEvent::Spawn(process.id()));
    let stdout_events = events.clone();
    let stdout = process.stdout.take();
    let stdout_thread = std::thread::spawn(move || {
        if let Some(mut stdout) = stdout {
            let mut buffer = [0u8; 8192];
            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(length) => {
                        let _ =
                            stdout_events.send(HostChildEvent::Stdout(buffer[..length].to_vec()));
                    }
                    Err(_) => break,
                }
            }
        }
    });
    let stderr_events = events.clone();
    let stderr = process.stderr.take();
    let stderr_thread = std::thread::spawn(move || {
        if let Some(mut stderr) = stderr {
            let mut buffer = [0u8; 8192];
            loop {
                match stderr.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(length) => {
                        let _ =
                            stderr_events.send(HostChildEvent::Stderr(buffer[..length].to_vec()));
                    }
                    Err(_) => break,
                }
            }
        }
    });
    let mut stdin = process.stdin.take();
    let status = loop {
        match commands.recv_timeout(Duration::from_millis(2)) {
            Ok(HostChildCommand::Stdin(value)) => {
                if let Some(input) = &mut stdin {
                    let _ = input.write_all(&value);
                    let _ = input.flush();
                }
            }
            Ok(HostChildCommand::StdinEnd) => stdin = None,
            Ok(HostChildCommand::Kill(signal)) => {
                #[cfg(unix)]
                unsafe {
                    libc::kill(process.id() as libc::pid_t, signal);
                }
                #[cfg(not(unix))]
                let _ = process.kill();
                stdin = None;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => stdin = None,
        }
        match process.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(error) => {
                let _ = events.send(HostChildEvent::Error {
                    message: error.to_string(),
                    code: "UNKNOWN".to_string(),
                });
                break None;
            }
        }
    };
    drop(stdin);
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();
    #[cfg(unix)]
    let signal = status.as_ref().and_then(std::process::ExitStatus::signal);
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    let _ = events.send(HostChildEvent::Exit {
        code: status.and_then(|status| status.code()),
        signal,
    });
}

fn spawn_host_child(command: String, arguments_json: String, options_json: String) -> u32 {
    let arguments: Vec<String> = serde_json::from_str(&arguments_json).unwrap_or_default();
    let options: serde_json::Value = serde_json::from_str(&options_json).unwrap_or_default();
    let (command_sender, command_receiver) = mpsc::channel();
    let (event_sender, event_receiver) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        run_host_child(command, arguments, options, command_receiver, event_sender);
    });
    HOST_CHILDREN.with(|table| {
        let mut table = table.borrow_mut();
        let handle = table.next_handle;
        table.next_handle = table.next_handle.wrapping_add(1).max(1);
        table.children.insert(
            handle,
            HostChild {
                commands: command_sender,
                events: event_receiver,
                thread: Some(thread),
            },
        );
        handle
    })
}

fn send_host_child_stdin(handle: u32, value: String, end: bool) -> bool {
    HOST_CHILDREN.with(|table| {
        let table = table.borrow();
        let Some(child) = table.children.get(&handle) else {
            return false;
        };
        if !value.is_empty()
            && child
                .commands
                .send(HostChildCommand::Stdin(hex_decode(&value)))
                .is_err()
        {
            return false;
        }
        !end || child.commands.send(HostChildCommand::StdinEnd).is_ok()
    })
}

fn kill_host_child(handle: u32, signal: i32) -> bool {
    HOST_CHILDREN.with(|table| {
        table
            .borrow()
            .children
            .get(&handle)
            .is_some_and(|child| child.commands.send(HostChildCommand::Kill(signal)).is_ok())
    })
}

fn host_children_active() -> bool {
    HOST_CHILDREN.with(|table| !table.borrow().children.is_empty())
}

fn poll_host_children() -> String {
    HOST_CHILDREN.with(|table| {
        let mut table = table.borrow_mut();
        let mut output = Vec::new();
        let mut finished = Vec::new();
        for (&handle, child) in &table.children {
            loop {
                match child.events.try_recv() {
                    Ok(HostChildEvent::Spawn(pid)) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "spawn", "pid": pid }));
                    }
                    Ok(HostChildEvent::Stdout(value)) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "stdout", "value": hex_encode(&value) }));
                    }
                    Ok(HostChildEvent::Stderr(value)) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "stderr", "value": hex_encode(&value) }));
                    }
                    Ok(HostChildEvent::Error { message, code }) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "error", "message": message, "code": code }));
                    }
                    Ok(HostChildEvent::Exit { code, signal }) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "exit", "code": code, "signal": signal }));
                        finished.push(handle);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished.push(handle);
                        break;
                    }
                }
            }
        }
        finished.sort_unstable();
        finished.dedup();
        for handle in finished {
            if let Some(mut child) = table.children.remove(&handle) {
                if let Some(thread) = child.thread.take() {
                    let _ = thread.join();
                }
            }
        }
        serde_json::to_string(&output).unwrap_or_else(|_| "[]".into())
    })
}

fn run_child_process(command: String, arguments_json: String, options_json: String) -> String {
    let arguments: Vec<String> = serde_json::from_str(&arguments_json).unwrap_or_default();
    let options: serde_json::Value = serde_json::from_str(&options_json).unwrap_or_default();
    let mut child = configure_child_command(&command, &arguments, &options);
    child.stdout(Stdio::piped()).stderr(Stdio::piped());
    let input = options
        .get("input")
        .and_then(|value| value.as_str())
        .map(hex_decode);
    if input.is_some() {
        child.stdin(Stdio::piped());
    }
    let spawned = child.spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            let code = if error.kind() == io::ErrorKind::NotFound {
                "ENOENT"
            } else if error.kind() == io::ErrorKind::PermissionDenied {
                "EACCES"
            } else {
                "UNKNOWN"
            };
            return serde_json::json!({
                "error": error.to_string(),
                "code": code,
                "errno": error.raw_os_error(),
                "path": command,
            })
            .to_string();
        }
    };
    let pid = child.id();
    if let Some(input) = input {
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(error) = stdin.write_all(&input) {
                return serde_json::json!({
                    "error": error.to_string(),
                    "code": "EPIPE",
                    "pid": pid,
                })
                .to_string();
            }
        }
    }
    match child.wait_with_output() {
        Ok(output) => {
            #[cfg(unix)]
            let signal = output.status.signal();
            #[cfg(not(unix))]
            let signal: Option<i32> = None;
            serde_json::json!({
                "pid": pid,
                "status": output.status.code(),
                "signal": signal,
                "stdout": hex_encode(&output.stdout),
                "stderr": hex_encode(&output.stderr),
            })
            .to_string()
        }
        Err(error) => serde_json::json!({
            "error": error.to_string(),
            "code": "UNKNOWN",
            "pid": pid,
        })
        .to_string(),
    }
}
