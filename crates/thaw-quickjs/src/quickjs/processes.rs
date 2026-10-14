fn host_worker_bootstrap(worker_data_json: &str, config_json: &str, thread_id: u32) -> String {
    let worker_data = serde_json::to_string(worker_data_json).unwrap_or_else(|_| "\"null\"".into());
    let config = serde_json::to_string(config_json).unwrap_or_else(|_| "\"{}\"".into());
    format!(
        r#"
        globalThis.__thaw_host_worker_events = [];
        globalThis.__thaw_host_worker_closed = false;
        const __thaw_host_event_queue = globalThis.__thaw_host_worker_events;
        const __thaw_array_push = Array.prototype.push;
        const __thaw_apply = Reflect.apply;
        const __thaw_detach_buffer = __thaw_detach_array_buffer;
        const __thaw_enqueue_host_event = event => __thaw_apply(__thaw_array_push, __thaw_host_event_queue, [event]);
        const __thaw_validate_port_transfer = MessagePort.prototype.__thawValidateTransfer;

        const __thaw_get_own_descriptor = Object.getOwnPropertyDescriptor;
        const __thaw_has_own = Object.prototype.hasOwnProperty;
        const __thaw_is_extensible = Object.isExtensible;
        const __thaw_define_property = Object.defineProperty;
        const __thaw_live_children_key = Symbol.for('thaw.worker.liveChildren');
        if (globalThis.process && !__thaw_has_own.call(globalThis.process, __thaw_live_children_key)) __thaw_define_property(globalThis.process, __thaw_live_children_key, {{ value: 0, writable: true, configurable: true }});
        const __thaw_map_get = Map.prototype.get;
        const __thaw_map_set = Map.prototype.set;
        const __thaw_map_delete = Map.prototype.delete;
        const __thaw_clear_timeout = clearTimeout;
        const __thaw_uint8_array = Uint8Array;
        const __thaw_is_message_port = globalThis.__thaw_is_message_port;
        const __thaw_transfer_port = MessagePort.prototype.__thawTransferToValidated;
        const __thaw_host_worker_listeners = new Map();
        const __thaw_host_ports = new Map();
        let __thaw_host_port_sequence = 1;
        function __thaw_stage_worker_ports(transfer) {{
          const entries = [], portIds = new Map(), buffers = [], seen = new Set();
          for (const value of Array.from(transfer || [])) {{
            if (seen.has(value)) throw new DOMException('transfer list contains duplicate values', 'DataCloneError');
            seen.add(value);
            if (globalThis.__thaw_untransferable_objects && globalThis.__thaw_untransferable_objects.has(value)) throw new DOMException('Object is marked as untransferable', 'DataCloneError');
            if (value instanceof ArrayBuffer) {{ new __thaw_uint8_array(value); buffers.push(value); continue; }}
            if (!__thaw_is_message_port || !__thaw_is_message_port(value)) throw new DOMException('value is not transferable', 'DataCloneError');
            const id = value.__thawHostPortId;
            if (id) {{
              if (__thaw_apply(__thaw_map_get, __thaw_host_ports, [String(id)]) !== value || value.__thawClosed) throw new DOMException('MessagePort is already detached', 'DataCloneError');
              entries.push({{ source: value, id: String(id), host: true }});
            }} else {{
              __thaw_apply(__thaw_validate_port_transfer, value, []);
              entries.push({{ source: value, id: 'w:{thread_id}:' + (__thaw_host_port_sequence++), shell: new MessagePort(), host: false }});
            }}
            portIds.set(value, entries[entries.length - 1].id);
          }}
          return {{ entries, portIds, buffers }};
        }}
        function __thaw_validate_worker_ports(stage) {{
          for (const entry of stage.entries) {{
            if (entry.host) {{
              const closed = __thaw_get_own_descriptor(entry.source, '__thawClosed'), queue = __thaw_get_own_descriptor(entry.source, '__thawQueue'), peer = __thaw_get_own_descriptor(entry.source, '__thawPeer');
              if (__thaw_apply(__thaw_map_get, __thaw_host_ports, [entry.id]) !== entry.source || !closed || !__thaw_apply(__thaw_has_own, closed, ['value']) || closed.value || !closed.writable || !queue || !__thaw_apply(__thaw_has_own, queue, ['value']) || !queue.writable || !peer || !__thaw_apply(__thaw_has_own, peer, ['value']) || !peer.writable) throw new DOMException('MessagePort is already detached', 'DataCloneError');
              const length = __thaw_get_own_descriptor(queue.value, 'length'); if (!length || !length.writable) throw new DOMException('MessagePort is already detached', 'DataCloneError');
            }} else {{
              const closed = __thaw_get_own_descriptor(entry.source, '__thawClosed'), queue = __thaw_get_own_descriptor(entry.source, '__thawQueue'), peer = __thaw_get_own_descriptor(entry.source, '__thawPeer');
              if (!closed || !closed.writable || !queue || !queue.writable || !peer || !peer.writable || !__thaw_is_message_port(peer.value)) throw new DOMException('MessagePort is already detached', 'DataCloneError');
              __thaw_apply(__thaw_validate_port_transfer, entry.source, []);
              const transferred = __thaw_get_own_descriptor(entry.source, '__thawTransferredPort'); entry.hasTransferReference = !!transferred; if (transferred ? !transferred.writable : !__thaw_is_extensible(entry.source)) throw new DOMException('MessagePort is already detached', 'DataCloneError'); const hostId = __thaw_get_own_descriptor(entry.source, '__thawHostPortId'); entry.hadHostId = !!hostId; if (hostId ? !hostId.writable : !__thaw_is_extensible(entry.source)) throw new DOMException('MessagePort is already detached', 'DataCloneError');
              const peerField = __thaw_get_own_descriptor(peer.value, '__thawPeer'); if (!peerField || !peerField.writable) throw new DOMException('MessagePort is already detached', 'DataCloneError');
            }}
          }}
          for (const buffer of stage.buffers) new __thaw_uint8_array(buffer);
        }}
        function __thaw_commit_worker_ports(stage) {{
          for (const entry of stage.entries) {{
            if (entry.host) {{ entry.source.__thawClosed = true; entry.source.__thawQueue = []; entry.source.__thawPeer = null; __thaw_apply(__thaw_map_delete, __thaw_host_ports, [entry.id]); }}
            else {{
              const moved = __thaw_apply(__thaw_transfer_port, entry.source, [entry.shell, entry.hasTransferReference]);
              if (entry.hadHostId) entry.source.__thawHostPortId = entry.id; else __thaw_define_property(entry.source, '__thawHostPortId', {{ value: entry.id, writable: true, configurable: true }}); __thaw_configure_worker_port_bridge(moved, entry.id);
              __thaw_apply(__thaw_map_set, __thaw_host_ports, [entry.id, moved.__thawPeer]);
            }}
          }}
          for (const buffer of stage.buffers) __thaw_detach_buffer(buffer);
        }}
        function __thaw_configure_worker_port_bridge(port, id) {{
          __thaw_define_property(port, '__thawHostPortId', {{ value: id, writable: true, configurable: true }});
          __thaw_define_property(port, '__thawSchedule', {{ value: function() {{
            while (port.__thawQueue.length) {{ const record = port.__thawQueue.shift(), stage = __thaw_stage_worker_ports(record.ports), payload = __thaw_worker_encode(record.data, stage.portIds), ports = stage.entries.map(entry => entry.id); __thaw_validate_worker_ports(stage); __thaw_enqueue_host_event({{ type: 'port', port: id, payload, ports }}); __thaw_commit_worker_ports(stage); }}
          }}, writable: true, configurable: true }});
        }}
        globalThis.__thaw_create_worker_port = function(id) {{
          id = String(id); if (__thaw_host_ports.has(id)) return __thaw_host_ports.get(id);
          const port = new MessagePort(); __thaw_define_property(port, '__thawHostPortId', {{ value: id, writable: true, configurable: true }});
          port.postMessage = function(value, transfer) {{ const stage = __thaw_stage_worker_ports(transfer), payload = __thaw_worker_encode(value, stage.portIds), ports = stage.entries.map(entry => entry.id); __thaw_validate_worker_ports(stage); __thaw_enqueue_host_event({{ type: 'port', port: id, payload, ports }}); __thaw_commit_worker_ports(stage); }};
          const close = port.close.bind(port); port.close = function() {{ __thaw_host_ports.delete(id); close(); }};
          __thaw_apply(__thaw_map_set, __thaw_host_ports, [id, port]); return port;
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
            const stage = __thaw_stage_worker_ports(transfer), payload = __thaw_worker_encode(value, stage.portIds), ports = stage.entries.map(entry => entry.id);
            __thaw_validate_worker_ports(stage);
            __thaw_enqueue_host_event({{ type: 'message', payload, ports }});
            __thaw_commit_worker_ports(stage);
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
        let __thaw_stdin_decoder = null, __thaw_stdin_decoder_type = null, __thaw_stdin_ended = false;
        function __thaw_emit_stdin(name, value) {{ for (const listener of (__thaw_stdin_listeners.get(name) || []).slice()) listener(value); }}
        function __thaw_get_stdin_decoder_type() {{
          if (__thaw_stdin_decoder_type) return __thaw_stdin_decoder_type;
          if (typeof globalThis.require !== 'function') throw new Error('StringDecoder is not available in this Worker');
          const loaded = globalThis.require('node:string_decoder');
          if (!loaded || typeof loaded.StringDecoder !== 'function') throw new Error('StringDecoder is not available in this Worker');
          __thaw_stdin_decoder_type = loaded.StringDecoder;
          return __thaw_stdin_decoder_type;
        }}
        process.stdin = {{
          setEncoding(encoding) {{ const next = new (__thaw_get_stdin_decoder_type())(encoding); if (__thaw_stdin_decoder) {{ const tail = __thaw_stdin_decoder.end(); if (tail) __thaw_emit_stdin('data', tail); }} __thaw_stdin_decoder = next; this.encoding = next.encoding; return this; }},
          on(name, listener) {{ const key = String(name), list = __thaw_stdin_listeners.get(key) || []; list.push(listener); __thaw_stdin_listeners.set(key, list); return this; }},
          once(name, listener) {{ const wrapped = value => {{ this.off(name, wrapped); listener(value); }}; wrapped.listener = listener; return this.on(name, wrapped); }},
          off(name, listener) {{ const key = String(name), list = __thaw_stdin_listeners.get(key) || []; __thaw_stdin_listeners.set(key, list.filter(item => item !== listener && item.listener !== listener)); return this; }},
          resume() {{ return this; }}, pause() {{ return this; }}
        }};
        function __thaw_worker_output_bytes(value, encoding) {{
          if (typeof value === 'string') return Buffer.from(value, encoding || 'utf8');
          if (ArrayBuffer.isView(value)) return Buffer.from(value.buffer, value.byteOffset, value.byteLength);
          const error = new TypeError('The chunk argument must be a string, Buffer, TypedArray, or DataView'); error.code = 'ERR_INVALID_ARG_TYPE'; throw error;
        }}
        function __thaw_worker_write_output(type, value, encoding, callback) {{
          if (typeof encoding === 'function') {{ callback = encoding; encoding = undefined; }}
          const bytes = __thaw_worker_output_bytes(value, encoding);
          __thaw_enqueue_host_event({{ type, payload: bytes.toString('hex') }});
          if (typeof callback === 'function') queueMicrotask(callback);
          return true;
        }}
        process.stdout = {{ write(value, encoding, callback) {{ return __thaw_worker_write_output('stdout', value, encoding, callback); }} }};
        process.stderr = {{ write(value, encoding, callback) {{ return __thaw_worker_write_output('stderr', value, encoding, callback); }} }};
        if (__thaw_host_worker_config.stdout) console.log = console.info = (...values) => process.stdout.write(values.map(String).join(' ') + '\n');
        if (__thaw_host_worker_config.stderr) console.warn = console.error = (...values) => process.stderr.write(values.map(String).join(' ') + '\n');
        function __thaw_host_post_message_to_thread(target, value, transferList, timeout) {{
          target = Number(target);
          if (target === {thread_id}) {{ const error = new Error('Cannot send a message to the same thread'); error.code = 'ERR_WORKER_MESSAGING_SAME_THREAD'; return Promise.reject(error); }}
          const timeoutMs = timeout === undefined ? undefined : Number(timeout), stage = __thaw_stage_worker_ports(transferList), payload = __thaw_worker_encode(value, stage.portIds), ports = stage.entries.map(entry => entry.id), request = __thaw_direct_request++;
          return new Promise((resolve, reject) => {{
            let timer;
            if (timeoutMs !== undefined && timeoutMs >= 0) timer = setTimeout(() => {{ if (__thaw_apply(__thaw_map_delete, __thaw_direct_pending, [request])) {{ const error = new Error('The destination thread did not process the message'); error.code = 'ERR_WORKER_MESSAGING_TIMEOUT'; reject(error); }} }}, timeoutMs);
            __thaw_apply(__thaw_map_set, __thaw_direct_pending, [request, {{ resolve, reject, timer }}]);
            try {{ __thaw_validate_worker_ports(stage); }} catch (error) {{ __thaw_apply(__thaw_map_delete, __thaw_direct_pending, [request]); if (timer) __thaw_clear_timeout(timer); reject(error); return; }}
            __thaw_enqueue_host_event({{ type: 'direct', target, source: {thread_id}, request, payload, ports }});
            __thaw_commit_worker_ports(stage);
          }});
        }}
        globalThis.__thaw_worker_module = {{ isMainThread: false, threadId: {thread_id}, threadName: String(__thaw_host_worker_config.threadName || ''), workerData, parentPort, resourceLimits: Object.assign({{}}, __thaw_host_worker_config.resourceLimits || {{}}), MessageChannel, MessagePort, BroadcastChannel, receiveMessageOnPort(port) {{ const record = port && port.__thawQueue && port.__thawQueue.shift(); return record ? {{ message: record.data }} : undefined; }}, postMessageToThread: __thaw_host_post_message_to_thread }};
        if (typeof globalThis.__thaw_bundle_create_require === 'function') {{ try {{ const nestedWorkerThreads = globalThis.__thaw_bundle_create_require('')('node:worker_threads'); if (nestedWorkerThreads && typeof nestedWorkerThreads.Worker === 'function') globalThis.__thaw_worker_module.Worker = nestedWorkerThreads.Worker; }} catch (_) {{}} }}
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
          if (__thaw_stdin_ended) return;
          if (ended) {{
            __thaw_stdin_ended = true;
            if (__thaw_stdin_decoder) {{ const tail = __thaw_stdin_decoder.end(); if (tail) __thaw_emit_stdin('data', tail); }}
            __thaw_emit_stdin('end', undefined);
            return;
          }}
          const bytes = Buffer.from(payload, 'hex');
          const value = __thaw_stdin_decoder ? __thaw_stdin_decoder.write(bytes) : bytes;
          if (!__thaw_stdin_decoder || value.length) __thaw_emit_stdin('data', value);
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
          __thaw_apply(__thaw_map_delete, __thaw_direct_pending, [Number(request)]); if (pending.timer) __thaw_clear_timeout(pending.timer);
          if (!errorText) pending.resolve(); else {{ const separator = errorText.indexOf(':'), code = separator < 0 ? errorText : errorText.slice(0, separator), message = separator < 0 ? errorText : errorText.slice(separator + 1), error = new Error(message); error.code = code; if (code === 'ERR_WORKER_MESSAGING_ERRORED') error.cause = new Error(message); pending.reject(error); }}
        }};
        globalThis.__thaw_host_worker_drain = function() {{ return JSON.stringify(__thaw_host_worker_events.splice(0)); }};
        globalThis.__thaw_host_worker_should_exit = function() {{
          return globalThis.__thaw_host_worker_closed || (((__thaw_host_worker_listeners.get('message') || []).length === 0) && ((!process.listenerCount || process.listenerCount('workerMessage') === 0)) && __thaw_direct_pending.size === 0 && __thaw_host_ports.size === 0 && ((__thaw_stdin_listeners.get('data') || []).length === 0) && ((__thaw_stdin_listeners.get('end') || []).length === 0) && ((process[Symbol.for('thaw.worker.liveChildren')] || 0) === 0) && __thaw_next_timer_delay() < 0);
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
        let ports = event.get("ports").and_then(|value| value.as_array()).map(|values| values.iter().filter_map(|value| value.as_str().map(str::to_string)).collect()).unwrap_or_default();
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
                ports,
            },
            Some("port") => HostWorkerEvent::PortMessage {
                port: event
                    .get("port")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
                payload,
                ports,
            },
            _ => HostWorkerEvent::Message { payload, ports },
        };
        let _ = events.send(event);
    }
    Ok(())
}

fn execute_worker_pending_job(ctx: &Ctx<'_>) -> Result<bool, String> {
    let ran = ctx.execute_pending_job();
    // rquickjs::Ctx::execute_pending_job maps QuickJS -1 to `true`, the
    // same value as a successful job. Inspect and consume the exception
    // before another job or addon teardown hides its origin.
    if ctx.has_exception() { Err(describe_exception(ctx)) } else { Ok(ran) }
}

fn close_worker_napi() -> Result<(), String> {
    let Some((begin, poll, take_error, finish)) = *NAPI_SHUTDOWN_BRIDGE.lock().unwrap() else {
        return Ok(());
    };
    // Each native phase may call back into JS, but must return through the
    // outer Context::with boundary before the next phase advances shutdown.
    let started = with_context(|ctx| {
        let _active = ActiveNapiContext::enter(&ctx);
        begin()
    });
    if started == 0 { return Err("native addon shutdown could not start".into()); }
    let mut errors = Vec::new();
    loop {
        let (ready, poll_errors) = with_context(|ctx| {
            let _active = ActiveNapiContext::enter(&ctx);
            let ready = poll();
            let mut errors = Vec::new();
            loop {
                let error = take_error();
                if error.is_null() { break; }
                errors.push(unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned());
                unsafe { thaw_arena::destroy_string(error) };
            }
            (ready, errors)
        });
        errors.extend(poll_errors);
        // poll may have invoked a cleanup hook or finalizer which queued a
        // QuickJS reaction. Return through a bounded context entry before the
        // next native poll or final unload decision.
        let (ran_job, job_errors) = with_context(|ctx| {
            let _active = ActiveNapiContext::enter(&ctx);
            let mut ran_job = false;
            let mut errors = Vec::new();
            loop {
                let tick = match drain_next_tick_queue(&ctx) {
                    Ok(tick) => tick,
                    Err(error) => {
                        errors.push(match error {
                            rquickjs::Error::Exception => describe_exception(&ctx),
                            error => error.to_string(),
                        });
                        true
                    }
                };
                let job = match execute_worker_pending_job(&ctx) {
                    Ok(job) => job,
                    Err(error) => { errors.push(error); true }
                };
                ran_job |= tick || job;
                if !tick && !job { break; }
            }
            (ran_job, errors)
        });
        errors.extend(job_errors);
        if ready != 0 && !ran_job {
            let finished = with_context(|ctx| {
                let _active = ActiveNapiContext::enter(&ctx);
                finish()
            });
            if finished != 0 { break; }
        }
        // The Worker must keep its Ctx and thread-local Host alive until the
        // exact loops it owns have no callback-capable handles or requests.
        // An addon that never closes an unreferenced handle cannot complete
        // this safe shutdown; returning an error would free its callback roots.
        if !ran_job { std::thread::sleep(Duration::from_millis(1)); }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
}

fn run_host_worker(
    start: HostWorkerStart,
    commands: Receiver<HostWorkerCommand>,
    events: Sender<HostWorkerEvent>,
) {
    let initialized = with_context(|ctx| -> Result<(), String> {
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
        Ok(())
    });
    let work = match initialized {
        Err(error) => Err(error),
        Ok(()) => loop {
            let should_exit = with_context(|ctx| -> Result<bool, String> {
                loop {
                    drain_next_tick_queue(&ctx).map_err(|error| error.to_string())?;
                    if !execute_worker_pending_job(&ctx)? { break; }
                }
                let run_due: Function = ctx
                    .globals()
                    .get("__thaw_run_due_timers")
                    .map_err(|error| error.to_string())?;
                run_due.call::<_, usize>(()).map_err(|error| error.to_string())?;
                loop {
                    drain_next_tick_queue(&ctx).map_err(|error| error.to_string())?;
                    if !execute_worker_pending_job(&ctx)? { break; }
                }
                drain_host_worker_events(&ctx, &events)?;
                // Native completions belong to this Worker thread. Keep its Ctx
                // active while dispatching and then run JS jobs they scheduled.
                poll_napi_bridge(&ctx);
                // Nested Workers (and any other host event source registered by the
                // script) deliver through the same platform-event poll as the main thread.
                if let Ok(poll) = ctx.globals().get::<_, Function>("__thaw_poll_platform_events") {
                    poll.call::<_, ()>(()).map_err(|error| match error {
                        rquickjs::Error::Exception => describe_exception(&ctx),
                        error => error.to_string(),
                    })?;
                }
                loop {
                    drain_next_tick_queue(&ctx).map_err(|error| error.to_string())?;
                    if !execute_worker_pending_job(&ctx)? { break; }
                }
                // Callbacks run by the polls above may have posted messages; flush
                // them before the exit check can end this thread.
                drain_host_worker_events(&ctx, &events)?;

                let should_exit: Function = ctx
                    .globals()
                    .get("__thaw_host_worker_should_exit")
                    .map_err(|error| error.to_string())?;
                should_exit.call::<_, bool>(()).map_err(|error| error.to_string())
            });
            let should_exit = match should_exit {
                Ok(should_exit) => should_exit,
                Err(error) => break Err(error),
            };
            if should_exit && !napi_bridge_pending() && !host_workers_active() {
                break Ok(0);
            }

            match commands.recv_timeout(Duration::from_millis(1)) {
                Ok(HostWorkerCommand::Terminate) => break Ok(1),
                Ok(command) => {
                    let delivered = with_context(|ctx| -> Result<(), String> {
                        match command {
                            HostWorkerCommand::Message(payload) => {
                                let deliver: Function = ctx.globals()
                                    .get("__thaw_host_worker_deliver")
                                    .map_err(|error| error.to_string())?;
                                deliver.call::<_, ()>((payload,)).map_err(|error| match error {
                                    rquickjs::Error::Exception => describe_exception(&ctx),
                                    error => error.to_string(),
                                })?;
                            }
                            HostWorkerCommand::Stdin(payload) => {
                                let deliver: Function = ctx.globals()
                                    .get("__thaw_host_worker_stdin")
                                    .map_err(|error| error.to_string())?;
                                deliver.call::<_, ()>((payload, false))
                                    .map_err(|error| error.to_string())?;
                            }
                            HostWorkerCommand::StdinEnd => {
                                let deliver: Function = ctx.globals()
                                    .get("__thaw_host_worker_stdin")
                                    .map_err(|error| error.to_string())?;
                                deliver.call::<_, ()>((String::new(), true))
                                    .map_err(|error| error.to_string())?;
                            }
                            HostWorkerCommand::DirectMessage { payload, source, request, reply } => {
                                let deliver: Function = ctx.globals()
                                    .get("__thaw_host_worker_direct_deliver")
                                    .map_err(|error| error.to_string())?;
                                let error: String = deliver.call((payload, source))
                                    .map_err(|error| error.to_string())?;
                                let error = (!error.is_empty()).then_some(error);
                                if let Some(reply) = reply {
                                    let _ = reply.send(HostWorkerCommand::DirectResult { request, error });
                                } else {
                                    let _ = events.send(HostWorkerEvent::ParentDirectResult { request, error });
                                }
                            }
                            HostWorkerCommand::DirectResult { request, error } => {
                                let resolve: Function = ctx.globals()
                                    .get("__thaw_host_worker_direct_result")
                                    .map_err(|error| error.to_string())?;
                                resolve.call::<_, ()>((request, error.unwrap_or_default()))
                                    .map_err(|error| error.to_string())?;
                            }
                            HostWorkerCommand::PortMessage { port, payload } => {
                                let deliver: Function = ctx.globals()
                                    .get("__thaw_host_worker_port")
                                    .map_err(|error| error.to_string())?;
                                deliver.call::<_, ()>((port, payload))
                                    .map_err(|error| error.to_string())?;
                            }
                            HostWorkerCommand::Terminate => return Ok(()),
                        }
                        Ok(())
                    });
                    if let Err(error) = delivered { break Err(error); }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break Ok(1),
            }
        }
    };
    let shutdown = close_worker_napi();
    let result = match (work, shutdown) {
        (Ok(code), Ok(())) => Ok(code),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(shutdown)) => Err(shutdown),
        (Err(error), Err(shutdown)) => Err(format!("{error}\n{shutdown}")),
    };
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
                refed: true,
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

fn set_host_worker_refed(handle: u32, refed: bool) -> bool {
    HOST_WORKERS.with(|table| {
        let mut table = table.borrow_mut();
        let Some(worker) = table.workers.get_mut(&handle) else {
            return false;
        };
        worker.refed = refed;
        true
    })
}

fn host_workers_active() -> bool {
    HOST_WORKERS.with(|table| table.borrow().workers.values().any(|worker| worker.refed))
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
                    Ok(HostWorkerEvent::Message { payload, ports }) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "message", "payload": payload, "ports": ports }));
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
                        ports,
                    }) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "direct", "target": target, "source": source, "request": request, "payload": payload, "ports": ports }));
                    }
                    Ok(HostWorkerEvent::ParentDirectResult { request, error }) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "directResult", "request": request, "error": error }));
                    }
                    Ok(HostWorkerEvent::PortMessage { port, payload, ports }) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "port", "port": port, "payload": payload, "ports": ports }));
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

fn parse_child_options(options_json: &str) -> io::Result<serde_json::Value> {
    let options: serde_json::Value = serde_json::from_str(options_json)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid child options JSON"))?;
    if !options.is_object() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "child options must be an object",
        ));
    }
    Ok(options)
}

fn requested_child_id(options: &serde_json::Value, key: &str) -> io::Result<Option<i32>> {
    match options.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(value)) => value
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "credential must be int32")),
        Some(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "credential must be int32",
        )),
    }
}

fn child_spawn_error_code(error: &io::Error) -> &'static str {
    #[cfg(unix)]
    if error.raw_os_error() == Some(libc::EPERM) {
        return "EPERM";
    }
    #[cfg(unix)]
    if error.raw_os_error() == Some(libc::EACCES) {
        return "EACCES";
    }
    if error.kind() == io::ErrorKind::NotFound {
        "ENOENT"
    } else if error.kind() == io::ErrorKind::PermissionDenied {
        "EACCES"
    } else if error.kind() == io::ErrorKind::InvalidInput {
        "EINVAL"
    } else if error.kind() == io::ErrorKind::Unsupported {
        "ENOTSUP"
    } else {
        "UNKNOWN"
    }
}

#[cfg(all(unix, any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita")))]
type ChildPlatformId = u16;
#[cfg(all(unix, target_os = "nto"))]
type ChildPlatformId = i32;
#[cfg(all(unix, not(any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita", target_os = "nto"))))]
type ChildPlatformId = u32;

#[cfg(all(unix, not(target_os = "l4re")))]
fn platform_child_id(value: i32) -> ChildPlatformId {
    value as ChildPlatformId
}

fn configure_child_command(
    command: &str,
    arguments: &[String],
    options: &serde_json::Value,
) -> io::Result<Command> {
    if !options.is_object() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "child options must be an object",
        ));
    }
    let uid = requested_child_id(options, "uid")?;
    let gid = requested_child_id(options, "gid")?;
    #[cfg(not(unix))]
    if uid.is_some() || gid.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "child credentials are unsupported on this platform",
        ));
    }
    #[cfg(all(unix, target_os = "l4re"))]
    if uid.is_some() || gid.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "child credentials are unsupported on this platform",
        ));
    }

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
    #[cfg(all(unix, not(target_os = "l4re")))]
    {
        if let Some(gid) = gid {
            child.gid(platform_child_id(gid));
        }
        if let Some(uid) = uid {
            child.uid(platform_child_id(uid));
        }
        #[cfg(not(any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita")))]
        if gid.is_some() && uid.is_none() {
            // Match libuv's gid-only supplementary-group clearing; permission failure is ignored.
            unsafe {
                child.pre_exec(|| {
                    let _ = libc::setgroups(0, std::ptr::null());
                    Ok(())
                });
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
    Ok(child)
}

fn run_host_child(
    command: String,
    arguments: Vec<String>,
    options_json: String,
    commands: Receiver<HostChildCommand>,
    events: Sender<HostChildEvent>,
) {
    let options = match parse_child_options(&options_json) {
        Ok(options) => options,
        Err(error) => {
            let _ = events.send(HostChildEvent::Error {
                message: error.to_string(),
                code: child_spawn_error_code(&error).to_string(),
            });
            let _ = events.send(HostChildEvent::Exit { code: None, signal: None });
            return;
        }
    };
    let mut process = match configure_child_command(&command, &arguments, &options) {
        Ok(process) => process,
        Err(error) => {
            let _ = events.send(HostChildEvent::Error {
                message: error.to_string(),
                code: child_spawn_error_code(&error).to_string(),
            });
            let _ = events.send(HostChildEvent::Exit { code: None, signal: None });
            return;
        }
    };
    process
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut process = match process.spawn() {
        Ok(process) => process,
        Err(error) => {
            let _ = events.send(HostChildEvent::Error {
                message: error.to_string(),
                code: child_spawn_error_code(&error).to_string(),
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
    let (command_sender, command_receiver) = mpsc::channel();
    let (event_sender, event_receiver) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        run_host_child(command, arguments, options_json, command_receiver, event_sender);
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

fn start_host_stdin() {
    HOST_STDIN.with(|slot| {
        if slot.borrow().is_some() {
            return;
        }
        let (sender, events) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut input = io::stdin();
            let mut buffer = [0u8; 4096];
            loop {
                match input.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(length) => {
                        if sender
                            .send(HostStdinEvent::Data(buffer[..length].to_vec()))
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            let _ = sender.send(HostStdinEvent::End);
        });
        *slot.borrow_mut() = Some(HostStdin {
            events,
            thread: Some(thread),
        });
    });
}

fn host_stdin_active() -> bool {
    HOST_STDIN.with(|slot| slot.borrow().is_some())
}

fn poll_host_stdin() -> String {
    HOST_STDIN.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(stdin) = slot.as_mut() else {
            return "[]".into();
        };
        let mut output = Vec::new();
        let mut finished = false;
        loop {
            match stdin.events.try_recv() {
                Ok(HostStdinEvent::Data(value)) => {
                    output.push(serde_json::json!({ "type": "data", "value": hex_encode(&value) }));
                }
                Ok(HostStdinEvent::End) | Err(TryRecvError::Disconnected) => {
                    output.push(serde_json::json!({ "type": "end" }));
                    finished = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        if finished {
            if let Some(thread) = stdin.thread.take() {
                let _ = thread.join();
            }
            *slot = None;
        }
        serde_json::to_string(&output).unwrap_or_else(|_| "[]".into())
    })
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
    let options = match parse_child_options(&options_json) {
        Ok(options) => options,
        Err(error) => return serde_json::json!({
            "error": error.to_string(),
            "code": child_spawn_error_code(&error),
            "errno": error.raw_os_error(),
            "path": command,
        }).to_string(),
    };
    let mut child = match configure_child_command(&command, &arguments, &options) {
        Ok(child) => child,
        Err(error) => return serde_json::json!({
            "error": error.to_string(),
            "code": child_spawn_error_code(&error),
            "errno": error.raw_os_error(),
            "path": command,
        }).to_string(),
    };
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
            return serde_json::json!({
                "error": error.to_string(),
                "code": child_spawn_error_code(&error),
                "errno": error.raw_os_error(),
                "path": command,
            })
            .to_string();
        }
    };
    let pid = child.id();
    if let Some(input) = input {
        let failure = |error: &io::Error, code: &str| {
            serde_json::json!({
                "error": error.to_string(),
                "code": code,
                "pid": pid,
            })
            .to_string()
        };
        let stop_child = |child: &mut std::process::Child| {
            let _ = child.kill();
            let _ = child.wait();
        };
        let capture_cleanup_failed =
            |thread: std::thread::JoinHandle<io::Result<Vec<u8>>>| {
                thread.join().map_or(true, |result| result.is_err())
            };
        let include_cleanup_failure = |error: io::Error, failed: bool| {
            if failed {
                io::Error::other(format!("{error}; output capture cleanup failed"))
            } else {
                error
            }
        };
        let Some(stdout) = child.stdout.take() else {
            let error = io::Error::other("missing child stdout pipe");
            stop_child(&mut child);
            return failure(&error, "UNKNOWN");
        };
        let Some(stderr) = child.stderr.take() else {
            let error = io::Error::other("missing child stderr pipe");
            stop_child(&mut child);
            return failure(&error, "UNKNOWN");
        };
        let mut stdout_thread = Some(match std::thread::Builder::new().spawn(move || {
            let mut output = Vec::new();
            let mut stdout = stdout;
            stdout.read_to_end(&mut output)?;
            Ok::<_, io::Error>(output)
        }) {
            Ok(thread) => thread,
            Err(error) => {
                stop_child(&mut child);
                return failure(&error, "UNKNOWN");
            }
        });
        let mut stderr_thread = Some(match std::thread::Builder::new().spawn(move || {
            let mut output = Vec::new();
            let mut stderr = stderr;
            stderr.read_to_end(&mut output)?;
            Ok::<_, io::Error>(output)
        }) {
            Ok(thread) => thread,
            Err(error) => {
                stop_child(&mut child);
                let error = include_cleanup_failure(
                    error,
                    capture_cleanup_failed(stdout_thread.take().unwrap()),
                );
                return failure(&error, "UNKNOWN");
            }
        });
        let Some(stdin) = child.stdin.take() else {
            let error = io::Error::other("missing child stdin pipe");
            stop_child(&mut child);
            let stdout_failed = capture_cleanup_failed(stdout_thread.take().unwrap());
            let stderr_failed = capture_cleanup_failed(stderr_thread.take().unwrap());
            let error = include_cleanup_failure(error, stdout_failed || stderr_failed);
            return failure(&error, "UNKNOWN");
        };
        let (writer_result_sender, writer_result_receiver) = mpsc::sync_channel(1);
        let writer_thread = match std::thread::Builder::new().spawn(move || {
            let mut stdin = stdin;
            let result = stdin.write_all(&input);
            drop(stdin);
            let _ = writer_result_sender.send(result);
        }) {
            Ok(thread) => thread,
            Err(error) => {
                stop_child(&mut child);
                let stdout_failed = capture_cleanup_failed(stdout_thread.take().unwrap());
                let stderr_failed = capture_cleanup_failed(stderr_thread.take().unwrap());
                let error = include_cleanup_failure(error, stdout_failed || stderr_failed);
                return failure(&error, "UNKNOWN");
            }
        };

        let mut writer_result = None;
        let mut status = None;
        let mut supervisor_error = None;
        let mut stdout = None;
        let mut stderr = None;
        loop {
            if writer_result.is_none() {
                match writer_result_receiver.try_recv() {
                    Ok(result) => {
                        writer_result = Some(result);
                        if writer_result.as_ref().is_some_and(Result::is_err) {
                            break;
                        }
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        supervisor_error = Some(io::Error::other(
                            "stdin writer result channel disconnected",
                        ));
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            if stdout.is_none()
                && stdout_thread
                    .as_ref()
                    .is_some_and(std::thread::JoinHandle::is_finished)
            {
                let result = stdout_thread.take().unwrap().join()
                    .map_err(|_| io::Error::other("stdout capture thread panicked"))
                    .and_then(|result| result);
                match result {
                    Ok(output) => stdout = Some(output),
                    Err(error) => {
                        supervisor_error = Some(error);
                        break;
                    }
                }
            }
            if stderr.is_none()
                && stderr_thread
                    .as_ref()
                    .is_some_and(std::thread::JoinHandle::is_finished)
            {
                let result = stderr_thread.take().unwrap().join()
                    .map_err(|_| io::Error::other("stderr capture thread panicked"))
                    .and_then(|result| result);
                match result {
                    Ok(output) => stderr = Some(output),
                    Err(error) => {
                        supervisor_error = Some(error);
                        break;
                    }
                }
            }
            if status.is_none() {
                match child.try_wait() {
                    Ok(Some(exit_status)) => status = Some(exit_status),
                    Ok(None) => {}
                    Err(error) => {
                        supervisor_error = Some(error);
                        break;
                    }
                }
            }
            if status.is_some() && writer_result.is_some() && stdout.is_some() && stderr.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }

        let write_error = writer_result.and_then(Result::err);
        if write_error.is_some() || supervisor_error.is_some() {
            stop_child(&mut child);
        }
        let writer_join = writer_thread.join();
        let stdout_result = stdout_thread
            .take()
            .map(|thread| {
                thread
                    .join()
                    .map_err(|_| io::Error::other("stdout capture thread panicked"))
                    .and_then(|result| result)
            })
            .unwrap_or_else(|| stdout.ok_or_else(|| io::Error::other("stdout capture result unavailable")));
        let stderr_result = stderr_thread
            .take()
            .map(|thread| {
                thread
                    .join()
                    .map_err(|_| io::Error::other("stderr capture thread panicked"))
                    .and_then(|result| result)
            })
            .unwrap_or_else(|| stderr.ok_or_else(|| io::Error::other("stderr capture result unavailable")));

        if let Some(error) = write_error {
            return failure(&error, "EPIPE");
        }
        if let Some(error) = supervisor_error {
            return failure(&error, "UNKNOWN");
        }
        if writer_join.is_err() {
            return failure(&io::Error::other("stdin writer thread panicked"), "UNKNOWN");
        }
        let stdout = match stdout_result {
            Ok(output) => output,
            Err(error) => return failure(&error, "UNKNOWN"),
        };
        let stderr = match stderr_result {
            Ok(output) => output,
            Err(error) => return failure(&error, "UNKNOWN"),
        };
        let Some(status) = status else {
            return failure(&io::Error::other("child status unavailable"), "UNKNOWN");
        };
        Ok(std::process::Output { status, stdout, stderr })
    } else {
        child.wait_with_output()
    }
    .map(|output| {
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
    })
    .unwrap_or_else(|error| {
        serde_json::json!({
            "error": error.to_string(),
            "code": "UNKNOWN",
            "pid": pid,
        })
        .to_string()
    })
}


#[cfg(test)]
#[test]
fn host_worker_late_ref_keeps_queued_terminal_events_alive_until_poll() {
    let (commands, _command_receiver) = std::sync::mpsc::channel();
    let (event_sender, events) = std::sync::mpsc::channel();
    let (release_sender, release_receiver) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        release_receiver.recv().unwrap();
        event_sender.send(HostWorkerEvent::Message { payload: "queued".into(), ports: Vec::new() }).unwrap();
        event_sender.send(HostWorkerEvent::Exit(0)).unwrap();
    });
    let handle = u32::MAX;
    HOST_WORKERS.with(|table| {
        table.borrow_mut().workers.insert(
            handle,
            HostWorker { commands, events, thread: Some(thread), refed: false },
        );
    });

    assert!(!host_workers_active());
    assert_eq!(poll_host_workers(), "[]");
    release_sender.send(()).unwrap();
    HOST_WORKERS.with(|table| {
        while !table.borrow().workers[&handle].thread.as_ref().unwrap().is_finished() {
            std::thread::yield_now();
        }
    });

    // A sibling callback can re-ref after the previous poll, once this thread
    // has queued terminal events but before the public Exit is dispatched.
    assert!(set_host_worker_refed(handle, true));
    assert!(host_workers_active());
    let events = poll_host_workers();
    assert!(events.contains("queued"));
    assert!(events.contains("\"type\":\"exit\""));
    assert!(!host_workers_active());
}

#[cfg(all(test, unix))]
#[test]
fn sync_input_and_both_output_pipes_make_progress() {
    let input = vec![b'x'; 256 * 1024];
    let record: serde_json::Value = serde_json::from_str(&run_child_process(
        "/bin/sh".into(),
        serde_json::json!(["-c", "exec 3>&2; dd if=/dev/zero bs=4096 count=64 2>/dev/null; dd if=/dev/zero bs=4096 count=64 1>&3 2>/dev/null; exec 3>&-; cat"]).to_string(),
        serde_json::json!({"input": hex_encode(&input)}).to_string(),
    )).unwrap();
    assert_eq!(record["status"], 0);
    assert!(record.get("error").is_none());
    let stdout = hex_decode(record["stdout"].as_str().unwrap());
    let stderr = hex_decode(record["stderr"].as_str().unwrap());
    assert_eq!(stdout.len(), 512 * 1024);
    assert!(stdout[..256 * 1024].iter().all(|byte| *byte == 0));
    assert_eq!(&stdout[256 * 1024..], input.as_slice());
    assert_eq!(stderr, vec![0; 256 * 1024]);
}

#[cfg(all(test, unix))]
#[test]
fn sync_input_epipe_kills_direct_child() {
    let record: serde_json::Value = serde_json::from_str(&run_child_process(
        "/bin/sh".into(),
        serde_json::json!(["-c", "exec 0<&-; exec sleep 30"]).to_string(),
        serde_json::json!({"input": hex_encode(&vec![b'x'; 4 * 1024 * 1024])}).to_string(),
    )).unwrap();
    assert_eq!(record["code"], "EPIPE");
    assert!(record["pid"].as_u64().is_some());
}

#[cfg(test)]
mod process_credential_tests {
    use super::*;

    #[test]
    fn native_credential_options_accept_int32_and_reject_malformed_values() {
        for source in [
            r#"{}"#,
            r#"{"uid":null}"#,
            r#"{"gid":null}"#,
            r#"{"uid":0,"gid":0}"#,
            r#"{"uid":-2147483648,"gid":2147483647}"#,
            r#"{"uid":-1}"#,
        ] {
            parse_child_options(source).unwrap();
        }
        #[cfg(all(unix, not(target_os = "l4re")))]
        for source in [
            r#"{"uid":0,"gid":0}"#,
            r#"{"uid":-2147483648,"gid":2147483647}"#,
            r#"{"uid":-1}"#,
        ] {
            let options = parse_child_options(source).unwrap();
            assert!(configure_child_command("credential-helper", &[], &options).is_ok());
        }
        for source in [
            "not json",
            "null",
            "[]",
            r#"{"uid":"1"}"#,
            r#"{"gid":true}"#,
            r#"{"uid":1.5}"#,
            r#"{"uid":2147483648}"#,
            r#"{"gid":-2147483649}"#,
            r#"{"uid":9223372036854775808}"#,
        ] {
            let error = parse_child_options(source)
                .and_then(|options| configure_child_command("credential-helper", &[], &options).map(|_| options))
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{source}");
            assert_eq!(child_spawn_error_code(&error), "EINVAL");
        }
    }

    #[cfg(all(unix, not(target_os = "l4re")))]
    #[test]
    fn signed_int32_ids_keep_the_standard_platform_cast_and_request_presence() {
        assert_eq!(platform_child_id(-1), (-1_i32 as ChildPlatformId));
        assert_eq!(platform_child_id(i32::MIN), (i32::MIN as ChildPlatformId));
        for source in [r#"{"uid":-1}"#, r#"{"gid":-2147483648}"#] {
            let options = parse_child_options(source).unwrap();
            assert!(configure_child_command("credential-helper", &[], &options).is_ok());
        }
    }

    #[cfg(all(unix, not(any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita", target_os = "nto", target_os = "l4re"))))]
    #[test]
    fn unsigned_32_bit_ids_match_native_cast_boundaries() {
        assert_eq!(platform_child_id(-1), u32::MAX);
        assert_eq!(platform_child_id(i32::MIN), 2_147_483_648);
    }

    #[test]
    fn async_invalid_native_options_queue_error_then_terminal_exit_only() {
        let (_commands, command_receiver) = mpsc::channel();
        let (events_sender, events_receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            run_host_child("/path/not/started".into(), Vec::new(), "[]".into(), command_receiver, events_sender);
        });
        match events_receiver.recv_timeout(Duration::from_secs(2)).unwrap() {
            HostChildEvent::Error { code, .. } => assert_eq!(code, "EINVAL"),
            _ => panic!("expected startup Error event"),
        }
        assert!(matches!(events_receiver.recv_timeout(Duration::from_secs(2)).unwrap(), HostChildEvent::Exit { code: None, signal: None }));
        assert!(events_receiver.try_recv().is_err());
        worker.join().unwrap();
    }

    #[cfg(all(unix, not(any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita", target_os = "l4re"))))]
    struct IdentityHelperDirectory(std::path::PathBuf);

    #[cfg(all(unix, not(any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita", target_os = "l4re"))))]
    impl Drop for IdentityHelperDirectory {
        fn drop(&mut self) {
            if let Err(error) = std::fs::remove_dir_all(&self.0) {
                use std::io::Write;
                let _ = writeln!(std::io::stderr().lock(), "failed to remove credential helper directory {:?}: {error}", self.0);
            }
        }
    }

    #[cfg(all(unix, not(any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita", target_os = "l4re"))))]
    fn traversable_directory(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
        use std::os::unix::fs::PermissionsExt;
        let canonical = std::fs::canonicalize(path).map_err(|error| format!("canonicalize {path:?}: {error}"))?;
        let mut current = canonical.as_path();
        loop {
            let metadata = std::fs::metadata(current).map_err(|error| format!("metadata {current:?}: {error}"))?;
            if !metadata.is_dir() {
                return Err(format!("{current:?} is not a directory"));
            }
            if metadata.permissions().mode() & 0o001 == 0 {
                return Err(format!("{current:?} is not traversable by a dropped identity"));
            }
            match current.parent() {
                Some(parent) => current = parent,
                None => return Ok(canonical),
            }
        }
    }

    #[cfg(all(unix, not(any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita", target_os = "l4re"))))]
    fn assert_traversable_helper(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        let canonical = std::fs::canonicalize(path).expect("canonical credential helper path");
        let metadata = std::fs::metadata(&canonical).expect("credential helper metadata");
        assert!(metadata.is_file(), "credential helper is not a file: {canonical:?}");
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o755, "credential helper must be exactly mode 0755");
        assert_ne!(metadata.permissions().mode() & 0o001, 0, "credential helper must be world executable");
        let mut current = canonical.parent();
        while let Some(path) = current {
            let metadata = std::fs::metadata(path).expect("credential helper ancestor metadata");
            assert!(metadata.is_dir(), "credential helper ancestor is not a directory: {path:?}");
            assert_ne!(metadata.permissions().mode() & 0o001, 0, "helper path is not traversable by dropped identity: {path:?}");
            current = path.parent();
        }
    }

    #[cfg(all(unix, not(any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita", target_os = "l4re"))))]
    fn identity_helper_command() -> (IdentityHelperDirectory, String, Vec<String>, serde_json::Map<String, serde_json::Value>) {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let temporary_root = std::env::temp_dir();
        let root = traversable_directory(&temporary_root)
            .or_else(|_| traversable_directory(std::path::Path::new("/tmp")))
            .unwrap_or_else(|error| panic!("no traversable temporary root ({} or /tmp): {error}", temporary_root.display()));
        let mut random = [0u8; 8];
        getrandom::getrandom(&mut random).expect("credential helper directory randomness");
        let directory = root.join(format!("thaw-credential-helper-{}", hex_encode(&random)));
        std::fs::DirBuilder::new().mode(0o700).create(&directory)
            .unwrap_or_else(|error| panic!("create owned credential helper directory {directory:?}: {error}"));
        let guard = IdentityHelperDirectory(directory.clone());
        let executable = std::env::current_exe().expect("test executable path");
        let helper = directory.join("credential-helper");
        std::fs::copy(&executable, &helper).expect("copy test executable to owned credential helper directory");
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).expect("set credential helper mode");
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).expect("set credential helper directory mode");
        assert_eq!(std::fs::metadata(&directory).unwrap().permissions().mode() & 0o7777, 0o755, "owned credential helper directory must be exactly mode 0755");
        assert_traversable_helper(&helper);
        let args = vec!["--exact".into(), "process_credential_tests::identity_helper_test_mode".into(), "--nocapture".into()];
        let mut env = serde_json::Map::new();
        env.insert("THAW_PROCESS_CREDENTIAL_HELPER".into(), serde_json::Value::String("1".into()));
        (guard, helper.to_string_lossy().into_owned(), args, env)
    }

    #[cfg(all(unix, not(any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita", target_os = "l4re"))))]
    #[test]
    fn identity_helper_test_mode() {
        if std::env::var_os("THAW_PROCESS_CREDENTIAL_HELPER").is_none() {
            return;
        }
        let group_count = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
        println!("THAW_CREDENTIAL_IDENTITY={},{},{},{},{}", unsafe { libc::getuid() }, unsafe { libc::geteuid() }, unsafe { libc::getgid() }, unsafe { libc::getegid() }, group_count.max(0));
    }

    #[cfg(all(unix, not(any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita", target_os = "l4re"))))]
    fn run_identity(options: serde_json::Value) -> serde_json::Value {
        let (_helper_directory, command, arguments, mut env) = identity_helper_command();
        if let Some(extra) = options.get("env").and_then(serde_json::Value::as_object) {
            env.extend(extra.clone());
        }
        let mut options = options;
        options["env"] = serde_json::Value::Object(env);
        let record = run_child_process(command, serde_json::to_string(&arguments).unwrap(), options.to_string());
        serde_json::from_str(&record).unwrap()
    }

    #[cfg(all(unix, not(any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita", target_os = "l4re"))))]
    fn identity_from_record(record: &serde_json::Value) -> [u32; 5] {
        assert_eq!(record["status"], 0, "{record}");
        let stdout = hex_decode(record["stdout"].as_str().unwrap());
        let output = String::from_utf8(stdout).unwrap();
        let identity = output.lines().find_map(|line| line.strip_prefix("THAW_CREDENTIAL_IDENTITY=")).expect("identity helper marker");
        let values: Vec<u32> = identity.split(',').map(|part| part.parse().unwrap()).collect();
        values.try_into().unwrap()
    }

    #[cfg(all(unix, not(any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita", target_os = "nto", target_os = "l4re"))))]
    #[test]
    fn root_credentials_change_child_only_and_gid_only_clears_groups() {
        let euid = unsafe { libc::geteuid() };
        if euid != 0 {
            eprintln!("SKIP: changed-credential fixture requires root");
            return;
        }
        let before = (unsafe { libc::getuid() }, unsafe { libc::geteuid() }, unsafe { libc::getgid() }, unsafe { libc::getegid() });
        let mut nonempty_groups = Vec::<libc::gid_t>::new();
        let count = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
        if count > 0 {
            nonempty_groups.resize(count as usize, 0);
            unsafe { libc::getgroups(count, nonempty_groups.as_mut_ptr()); }
        }
        let uid_only = identity_from_record(&run_identity(serde_json::json!({"uid": 65534})));
        assert_eq!(&uid_only[..4], &[65534, 65534, before.2, before.3]);
        let gid_only = identity_from_record(&run_identity(serde_json::json!({"gid": 65534})));
        assert_eq!(&gid_only[..4], &[before.0, before.1, 65534, 65534]);
        if !nonempty_groups.is_empty() {
            assert_eq!(gid_only[4], 0, "gid-only credential request must clear inherited supplementary groups");
        } else {
            eprintln!("SKIP: gid-only group-clear assertion requires nonempty inherited groups");
        }
        let both = identity_from_record(&run_identity(serde_json::json!({"uid": 65534, "gid": 65534})));
        assert_eq!(&both[..4], &[65534, 65534, 65534, 65534]);
        let omitted = identity_from_record(&run_identity(serde_json::json!({})));
        assert_eq!(&omitted[..4], &[before.0, before.1, before.2, before.3]);
        assert_eq!((unsafe { libc::getuid() }, unsafe { libc::geteuid() }, unsafe { libc::getgid() }, unsafe { libc::getegid() }), before);
    }

    #[cfg(all(unix, not(any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita", target_os = "nto", target_os = "l4re"))))]
    #[test]
    fn unprivileged_uid_zero_fails_before_helper_execution() {
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("SKIP: EPERM fixture requires an unprivileged test process");
            return;
        }
        let record = run_identity(serde_json::json!({"uid": 0}));
        assert_eq!(record["code"], "EPERM");
        assert!(record["stdout"].as_str().unwrap_or_default().is_empty());
    }

    #[cfg(all(unix, target_os = "linux"))]
    #[test]
    fn linux_negative_uid_is_a_real_request_and_fails_before_helper_execution() {
        let record = run_identity(serde_json::json!({"uid": -1}));
        assert!(record.get("error").is_some(), "uid=-1 must reach the OS credential operation: {record}");
        assert!(record["stdout"].as_str().unwrap_or_default().is_empty(), "credential failure must precede helper execution");
    }

    #[cfg(all(test, unix, not(any(target_os = "vxworks", target_os = "espidf", target_os = "horizon", target_os = "vita", target_os = "l4re"))))]
    #[test]
    fn successful_async_identity_helper_delivers_spawn_output_exit_and_releases_handle() {
        let parent = (unsafe { libc::getuid() }, unsafe { libc::geteuid() }, unsafe { libc::getgid() }, unsafe { libc::getegid() });
        let (uid, gid) = if parent.1 == 0 {
            (65_534, 65_534)
        } else if parent.0 == parent.1 && parent.2 == parent.3 {
            (parent.0, parent.2)
        } else {
            eprintln!("SKIP: async credential identity fixture requires matching real/effective IDs or root");
            return;
        };
        let (_helper_directory, command, arguments, env) = identity_helper_command();
        let options = serde_json::json!({
            "uid": uid,
            "gid": gid,
            "env": env
        }).to_string();
        let handle = spawn_host_child(command, serde_json::to_string(&arguments).unwrap(), options);
        assert!(send_host_child_stdin(handle, String::new(), true));
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut events = Vec::new();
        let mut stdout = Vec::new();
        loop {
            for event in serde_json::from_str::<Vec<serde_json::Value>>(&poll_host_children()).unwrap() {
                if event["type"] == "stdout" {
                    stdout.extend(hex_decode(event["value"].as_str().unwrap()));
                }
                events.push(event);
            }
            if !host_children_active() {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "credential child did not finish within two seconds");
            std::thread::yield_now();
        }
        assert_eq!(events.first().unwrap()["type"], "spawn");
        assert_eq!(events.last().unwrap()["type"], "exit");
        assert_eq!(events.last().unwrap()["code"], 0);
        let output = String::from_utf8(stdout).unwrap();
        let identity = output.lines().find_map(|line| line.strip_prefix("THAW_CREDENTIAL_IDENTITY=")).expect("async identity helper output");
        let observed: Vec<u32> = identity.split(',').map(|part| part.parse().unwrap()).collect();
        assert_eq!(&observed[..4], &[uid, uid, gid, gid], "async command must apply both requested credentials");
        assert_eq!((unsafe { libc::getuid() }, unsafe { libc::geteuid() }, unsafe { libc::getgid() }, unsafe { libc::getegid() }), parent, "credential application must not mutate the parent");
        assert!(!events.iter().any(|event| event["type"] == "error"));
        assert!(!host_children_active(), "terminal poll must release the child handle");
    }

    #[cfg(not(unix))]
    #[test]
    fn non_unix_credentials_fail_before_command_lookup() {
        let options = parse_child_options(r#"{"uid":0}"#).unwrap();
        let error = configure_child_command("/missing/credential-command", &[], &options).unwrap_err();
        assert_eq!(child_spawn_error_code(&error), "ENOTSUP");
        let record: serde_json::Value = serde_json::from_str(&run_child_process(
            "/missing/credential-command".into(), "[]".into(), r#"{"gid":0}"#.into(),
        )).unwrap();
        assert_eq!(record["code"], "ENOTSUP");
        let omitted: serde_json::Value = serde_json::from_str(&run_child_process(
            "/missing/credential-command".into(), "[]".into(), "{}".into(),
        )).unwrap();
        assert_eq!(omitted["code"], "ENOENT");
        let (commands, command_receiver) = mpsc::channel();
        let (events_sender, events_receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || run_host_child(
            "/missing/credential-command".into(), Vec::new(), r#"{"gid":0}"#.into(), command_receiver, events_sender,
        ));
        match events_receiver.recv_timeout(Duration::from_secs(2)).unwrap() {
            HostChildEvent::Error { code, .. } => assert_eq!(code, "ENOTSUP"),
            _ => panic!("expected unsupported-platform startup Error"),
        }
        assert!(matches!(events_receiver.recv_timeout(Duration::from_secs(2)).unwrap(), HostChildEvent::Exit { code: None, signal: None }));
        assert!(events_receiver.try_recv().is_err());
        worker.join().unwrap();
        drop(commands);
    }

    #[cfg(all(unix, target_os = "l4re"))]
    #[test]
    fn l4re_credentials_fail_closed_when_std_cannot_apply_them() {
        let options = parse_child_options(r#"{"uid":0}"#).unwrap();
        let error = configure_child_command("/missing/credential-command", &[], &options).unwrap_err();
        assert_eq!(child_spawn_error_code(&error), "ENOTSUP");
    }
}
