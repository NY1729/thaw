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
        // process.exit()/chdir() are Worker-local: exit ends only this Worker
        // (reported through the Exit event); chdir is unsupported, as in Node.
        globalThis.__thaw_host_worker_exit_code = undefined;
        const __thaw_worker_exit_marker = {{ __thawWorkerExit: true }};
        if (globalThis.process) {{
          const __thaw_process = globalThis.process;
          __thaw_define_property(__thaw_process, 'chdir', {{ value: function chdir() {{
            const error = new TypeError('process.chdir() is not supported in workers');
            error.code = 'ERR_WORKER_UNSUPPORTED_OPERATION';
            throw error;
          }}, writable: true, configurable: true }});
          __thaw_define_property(__thaw_process, 'exit', {{ value: function exit(code) {{
            const status = code === undefined ? Number(__thaw_process.exitCode || 0) : Number(code);
            __thaw_process.exitCode = status;
            if (globalThis.__thaw_host_worker_exit_code === undefined) {{
              globalThis.__thaw_host_worker_exit_code = status;
              if (typeof __thaw_process.emit === 'function') __thaw_process.emit('exit', status);
            }}
            globalThis.__thaw_host_worker_closed = true;
            throw __thaw_worker_exit_marker;
          }}, writable: true, configurable: true }});
        }}
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
              const peerPort = moved.__thawPeer, peerClose = peerPort.close; __thaw_apply(__thaw_map_set, __thaw_host_ports, [entry.id, peerPort]);
              // Closing the worker-side end releases the bridge entry (it would otherwise keep the Worker alive forever).
              peerPort.close = function() {{ __thaw_apply(__thaw_map_delete, __thaw_host_ports, [entry.id]); return __thaw_apply(peerClose, this, []); }};
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
        // parentPort: messages that arrive while nothing listens are queued (receiveMessageOnPort
        // reads them); close() only closes the port, the Worker ends when nothing else is pending;
        // unref() stops a message listener from keeping the Worker alive.
        let __thaw_parent_refed = true, __thaw_parent_closed = false, __thaw_parent_onmessage = null;
        const __thaw_parent_queue = [];
        function __thaw_parent_has_receiver() {{ return (__thaw_host_worker_listeners.get('message') || []).length > 0 || typeof __thaw_parent_onmessage === 'function'; }}
        function __thaw_parent_dispatch(value) {{
          for (const listener of (__thaw_host_worker_listeners.get('message') || []).slice()) listener(value);
          if (typeof __thaw_parent_onmessage === 'function') __thaw_parent_onmessage.call(parentPort, {{ data: value, type: 'message', target: parentPort }});
        }}
        function __thaw_parent_flush() {{
          while (__thaw_parent_queue.length && !__thaw_parent_closed && __thaw_parent_has_receiver()) __thaw_parent_dispatch(__thaw_parent_queue.shift().data);
        }}
        function __thaw_host_worker_on(name, listener) {{
          const key = String(name), list = __thaw_host_worker_listeners.get(key) || [];
          list.push(listener); __thaw_host_worker_listeners.set(key, list);
          if (key === 'message') queueMicrotask(__thaw_parent_flush);
          return parentPort;
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
          close() {{
            if (__thaw_parent_closed) return;
            __thaw_parent_closed = true;
            for (const listener of (__thaw_host_worker_listeners.get('close') || []).slice()) listener();
          }},
          start() {{ queueMicrotask(__thaw_parent_flush); }},
          ref() {{ __thaw_parent_refed = true; return this; }},
          unref() {{ __thaw_parent_refed = false; return this; }},
          hasRef() {{ return __thaw_parent_refed && !__thaw_parent_closed; }},
          get onmessage() {{ return __thaw_parent_onmessage; }},
          set onmessage(handler) {{ __thaw_parent_onmessage = typeof handler === 'function' ? handler : null; if (__thaw_parent_onmessage) queueMicrotask(__thaw_parent_flush); }}
        }};
        const workerData = __thaw_worker_decode({worker_data});
        const __thaw_host_worker_config = JSON.parse({config});
        globalThis.__thaw_worker_environment_data = new Map(__thaw_host_worker_config.environmentData ? __thaw_worker_decode(__thaw_host_worker_config.environmentData) : []);
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
        let __thaw_stdin_decoder = null, __thaw_stdin_decoder_type = null, __thaw_stdin_ended = false, __thaw_stdin_paused = false;
        const __thaw_stdin_pending = [];
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
          resume() {{ __thaw_stdin_paused = false; queueMicrotask(__thaw_stdin_flush); return this; }},
          pause() {{ __thaw_stdin_paused = true; return this; }}
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
        globalThis.__thaw_worker_module = {{ isMainThread: false, threadId: {thread_id}, threadName: String(__thaw_host_worker_config.threadName || ''), workerData, parentPort, resourceLimits: Object.assign({{}}, __thaw_host_worker_config.resourceLimits || {{}}), MessageChannel, MessagePort, BroadcastChannel, receiveMessageOnPort(port) {{ if (port === parentPort) {{ const queued = __thaw_parent_queue.shift(); return queued ? {{ message: queued.data }} : undefined; }} const record = port && port.__thawQueue && port.__thawQueue.shift(); return record ? {{ message: record.data }} : undefined; }}, getEnvironmentData(key) {{ const value = globalThis.__thaw_worker_environment_data.get(key); return value === undefined ? undefined : structuredClone(value); }}, setEnvironmentData(key, value) {{ if (value === undefined) globalThis.__thaw_worker_environment_data.delete(key); else globalThis.__thaw_worker_environment_data.set(key, structuredClone(value)); }}, postMessageToThread: __thaw_host_post_message_to_thread }};
        if (typeof globalThis.__thaw_bundle_create_require === 'function') {{ try {{ const nestedWorkerThreads = globalThis.__thaw_bundle_create_require('')('node:worker_threads'); if (nestedWorkerThreads && typeof nestedWorkerThreads.Worker === 'function') globalThis.__thaw_worker_module.Worker = nestedWorkerThreads.Worker; }} catch (_) {{}} }}
        globalThis.require = function(name) {{
          if (name === 'worker_threads' || name === 'node:worker_threads') return globalThis.__thaw_worker_module;
          if (typeof globalThis.__thaw_bundle_create_require === 'function') return globalThis.__thaw_bundle_create_require('')(name);
          throw new Error("require('" + name + "') is not available in this Worker");
        }};
        globalThis.__thaw_host_worker_deliver = function(payload) {{
          const value = __thaw_worker_decode(payload);
          if (__thaw_parent_closed) return;
          if (__thaw_parent_has_receiver()) __thaw_parent_dispatch(value);
          else __thaw_parent_queue.push({{ data: value }});
        }};
        function __thaw_stdin_flush() {{
          while (!__thaw_stdin_paused && __thaw_stdin_pending.length) {{ const item = __thaw_stdin_pending.shift(); __thaw_stdin_apply(item.payload, item.ended); }}
        }}
        globalThis.__thaw_host_worker_stdin = function(payload, ended) {{
          if (__thaw_stdin_paused || __thaw_stdin_pending.length) {{ __thaw_stdin_pending.push({{ payload, ended }}); return; }}
          __thaw_stdin_apply(payload, ended);
        }};
        function __thaw_stdin_apply(payload, ended) {{
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
        }}
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
        // Natural end of the event loop: the exit code is process.exitCode and 'exit' listeners run once.
        globalThis.__thaw_host_worker_finalize = function() {{
          if (globalThis.__thaw_host_worker_exit_code === undefined) {{
            const status = Number(process.exitCode || 0);
            globalThis.__thaw_host_worker_exit_code = status;
            if (typeof process.emit === 'function') process.emit('exit', status);
          }}
          return Number(globalThis.__thaw_host_worker_exit_code);
        }};
        globalThis.__thaw_host_worker_drain = function() {{ return JSON.stringify(__thaw_host_worker_events.splice(0)); }};
        globalThis.__thaw_host_worker_should_exit = function() {{
          return globalThis.__thaw_host_worker_closed || ((__thaw_parent_closed || !__thaw_parent_refed || !__thaw_parent_has_receiver()) && (__thaw_parent_closed || !process.listenerCount || process.listenerCount('workerMessage') === 0) && __thaw_direct_pending.size === 0 && __thaw_host_ports.size === 0 && __thaw_stdin_pending.length === 0 && (__thaw_stdin_ended || (((__thaw_stdin_listeners.get('data') || []).length === 0) && ((__thaw_stdin_listeners.get('end') || []).length === 0))) && ((process[Symbol.for('thaw.worker.liveChildren')] || 0) === 0) && __thaw_next_timer_delay() < 0);
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
                target: worker_thread_id(&event, "target")?,
                source: worker_thread_id(&event, "source")?,
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

fn worker_thread_id(event: &serde_json::Value, key: &str) -> Result<u32, String> {
    let value = event.get(key).and_then(|value| value.as_u64()).unwrap_or_default();
    u32::try_from(value).map_err(|_| format!("ERR_OUT_OF_RANGE:The \"{key}\" thread id {value} is out of range"))
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
    // resourceLimits.maxOldGenerationSizeMb bounds the heap this Worker may add on top of its baseline.
    let memory_limit_mb = serde_json::from_str::<serde_json::Value>(&start.config_json)
        .ok()
        .and_then(|config| config.pointer("/resourceLimits/maxOldGenerationSizeMb").and_then(serde_json::Value::as_f64))
        .filter(|limit| *limit > 0.0);
    let terminate = start.terminate.clone();
    let initialized = with_context(|ctx| -> Result<(), String> {
        install_shared_environment_functions(&ctx, start.shared_env.clone())?;
        WORKER_TERMINATE.with(|flag| *flag.borrow_mut() = Some(start.terminate.clone()));
        // SAFETY: the runtime belongs to this thread; the handler only reads thread-local state and an atomic.
        unsafe {
            rquickjs::qjs::JS_SetInterruptHandler(
                rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()),
                Some(vm_interrupt_handler),
                std::ptr::null_mut(),
            );
        }
        if !start.bundle_source.is_empty() {
            load_impl(ctx.clone(), &start.bundle_source)?;
        }
        load_impl(
            ctx.clone(),
            &host_worker_bootstrap(&start.worker_data_json, &start.config_json, start.thread_id),
        )?;
        if let Some(limit_mb) = memory_limit_mb {
            // SAFETY: as above; usage is read into a zeroed out-parameter.
            unsafe {
                let runtime = rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
                let mut usage: rquickjs::qjs::JSMemoryUsage = std::mem::zeroed();
                rquickjs::qjs::JS_ComputeMemoryUsage(runtime, &mut usage);
                let baseline = usage.malloc_size.max(0) as u64;
                rquickjs::qjs::JS_SetMemoryLimit(runtime, baseline.saturating_add((limit_mb * 1024.0 * 1024.0) as u64) as _);
            }
        }
        let _ = events.send(HostWorkerEvent::Online);
        load_worker_script(
            &ctx,
            &format!("(function() {{\n{}\n}}).call(globalThis);", start.source),
        )?;
        Ok(())
    });
    let worker_error = |error: String| -> Result<i32, String> {
        if terminate.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(1);
        }
        if memory_limit_mb.is_some() && error.contains("out of memory") {
            return Err("ERR_WORKER_OUT_OF_MEMORY:Worker terminated due to reaching memory limit: JS heap out of memory".into());
        }
        Err(error)
    };
    let work = match initialized {
        Err(error) => worker_error(error),
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
                        rquickjs::Error::Exception => describe_worker_exception(&ctx),
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
                Err(error) => break worker_error(error),
            };
            if should_exit && !napi_bridge_pending() && !host_workers_active() {
                // The loop ended naturally: process.exitCode becomes the exit code and 'exit' listeners run.
                with_context(|ctx| {
                    if let Ok(finalize) = ctx.globals().get::<_, Function>("__thaw_host_worker_finalize") {
                        let _ = finalize.call::<_, f64>(());
                    }
                });
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
                                    rquickjs::Error::Exception => describe_worker_exception(&ctx),
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
                    if let Err(error) = delivered { break worker_error(error); }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break Ok(1),
            }
        }
    };
    // process.exit() inside the Worker unwinds as a marker exception; its
    // recorded status replaces whatever the unwinding reported.
    let exit_code = with_context(|ctx| {
        let code = ctx.globals().get::<_, Option<f64>>("__thaw_host_worker_exit_code").ok().flatten();
        if code.is_some() {
            // Messages posted before (and by) process.exit() still reach the parent.
            let _ = drain_host_worker_events(&ctx, &events);
        }
        code
    });
    let work = match exit_code {
        Some(code) => Ok(code as _),
        None => work,
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
    let terminate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_terminate = terminate.clone();
    let thread = std::thread::spawn(move || {
        run_host_worker(
            HostWorkerStart {
                bundle_source,
                source,
                worker_data_json,
                config_json,
                thread_id,
                shared_env,
                terminate: worker_terminate,
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
                terminate,
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
            .is_some_and(|worker| {
                // The flag stops a script that never returns to the command loop.
                worker.terminate.store(true, std::sync::atomic::Ordering::Relaxed);
                worker.commands.send(HostWorkerCommand::Terminate).is_ok()
            })
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
    #[cfg(unix)]
    {
        // Like libuv, hand the child default signal dispositions and an empty signal mask so an
        // ignored/blocked signal in the host (nohup, `trap '' INT`) cannot make a child unkillable.
        // SAFETY: signal and sigprocmask are async-signal-safe and the closure allocates nothing.
        unsafe {
            child.pre_exec(|| {
                for signal in 1..32 {
                    if signal != libc::SIGKILL && signal != libc::SIGSTOP {
                        libc::signal(signal, libc::SIG_DFL);
                    }
                }
                let mut mask: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut mask);
                libc::sigprocmask(libc::SIG_SETMASK, &mask, std::ptr::null_mut());
                Ok(())
            });
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
    let stdio = child_stdio_modes(&options);
    process
        .stdin(stdio_for(stdio[0]))
        .stdout(stdio_for(stdio[1]))
        .stderr(stdio_for(stdio[2]));
    let mut process = match spawn_retrying_text_busy(&mut process) {
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
    let output_thread = pump_child_output(process.stdout.take(), process.stderr.take(), events.clone());
    // Writes go through their own thread: a child that stops reading stdin
    // must never keep this control loop from handling kill/exit.
    let mut stdin = process.stdin.take().map(|mut pipe| {
        let (sender, chunks) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            while let Ok(chunk) = chunks.recv() {
                if pipe.write_all(&chunk).and_then(|()| pipe.flush()).is_err() {
                    break;
                }
            }
        });
        sender
    });
    let status = loop {
        match commands.recv_timeout(Duration::from_millis(2)) {
            Ok(HostChildCommand::Stdin(value)) => {
                if let Some(input) = &stdin {
                    let _ = input.send(value);
                }
            }
            Ok(HostChildCommand::StdinEnd) => stdin = None,
            Ok(HostChildCommand::Kill(signal, delivered)) => {
                #[cfg(unix)]
                let sent = unsafe { libc::kill(process.id() as libc::pid_t, signal) } == 0;
                #[cfg(not(unix))]
                let sent = process.kill().is_ok();
                let _ = delivered.send(sent);
                if signal != 0 {
                    stdin = None;
                }
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
    #[cfg(unix)]
    let signal = status.as_ref().and_then(std::process::ExitStatus::signal);
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    let code = status.and_then(|status| status.code());
    // `exit` is the process ending; `close` (the final Exit event) waits for the output pipes to reach EOF, which a
    // detached grandchild holding them open can delay.
    let _ = events.send(HostChildEvent::Exited { code, signal });
    let _ = output_thread.join();
    let _ = events.send(HostChildEvent::Exit { code, signal });
}

/// Read one chunk from a child pipe and forward it; false once the pipe is finished.
fn forward_child_output(pipe: &mut dyn Read, is_stderr: bool, events: &Sender<HostChildEvent>, buffer: &mut [u8]) -> bool {
    match pipe.read(buffer) {
        Ok(0) => false,
        Ok(length) => {
            let bytes = buffer[..length].to_vec();
            let _ = events.send(if is_stderr { HostChildEvent::Stderr(bytes) } else { HostChildEvent::Stdout(bytes) });
            true
        }
        Err(error) => error.kind() == io::ErrorKind::Interrupted,
    }
}

/// A child output pipe: its descriptor, reader and whether it is stderr.
#[cfg(unix)]
type OutputPipe = (i32, Box<dyn Read + Send>, bool);

/// Forward a child's stdout and stderr as events, in the order the pipes became readable.
///
/// One reader thread per pipe races on wake-up: a child that wrote stdout and then stderr could have the
/// stderr event overtake stdout, and execFile's per-pipe `maxBuffer` then overflows on the wrong pipe.
/// epoll reports descriptors in the order they became ready, which is what libuv's poll phase gives Node.
#[cfg(unix)]
fn pump_child_output(
    stdout: Option<std::process::ChildStdout>,
    stderr: Option<std::process::ChildStderr>,
    events: Sender<HostChildEvent>,
) -> std::thread::JoinHandle<()> {
    use std::os::fd::AsRawFd;
    let mut pipes: Vec<OutputPipe> = Vec::new();
    if let Some(pipe) = stdout {
        pipes.push((pipe.as_raw_fd(), Box::new(pipe), false));
    }
    if let Some(pipe) = stderr {
        pipes.push((pipe.as_raw_fd(), Box::new(pipe), true));
    }
    std::thread::spawn(move || {
        let epoll = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        std::thread::scope(|scope| {
            let mut live: Vec<Option<OutputPipe>> = Vec::new();
            let mut registered = 0;
            for (fd, mut reader, is_stderr) in pipes {
                let mut event = libc::epoll_event { events: libc::EPOLLIN as u32, u64: live.len() as u64 };
                if epoll >= 0 && unsafe { libc::epoll_ctl(epoll, libc::EPOLL_CTL_ADD, fd, &mut event) } == 0 {
                    live.push(Some((fd, reader, is_stderr)));
                    registered += 1;
                } else {
                    // Not pollable: give this pipe a blocking reader of its own rather than losing its output.
                    live.push(None);
                    let events = events.clone();
                    scope.spawn(move || {
                        let mut buffer = [0u8; 8192];
                        while forward_child_output(&mut *reader, is_stderr, &events, &mut buffer) {}
                    });
                }
            }
            let mut buffer = [0u8; 8192];
            let mut ready = [libc::epoll_event { events: 0, u64: 0 }; 2];
            while registered > 0 {
                let count = unsafe { libc::epoll_wait(epoll, ready.as_mut_ptr(), ready.len() as i32, -1) };
                if count < 0 {
                    if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    break;
                }
                for event in &ready[..count as usize] {
                    let index = event.u64 as usize;
                    let Some((fd, reader, is_stderr)) = live[index].as_mut() else { continue };
                    if !forward_child_output(&mut **reader, *is_stderr, &events, &mut buffer) {
                        unsafe { libc::epoll_ctl(epoll, libc::EPOLL_CTL_DEL, *fd, std::ptr::null_mut()) };
                        live[index] = None;
                        registered -= 1;
                    }
                }
            }
        });
        if epoll >= 0 {
            unsafe { libc::close(epoll) };
        }
    })
}

#[cfg(not(unix))]
fn pump_child_output(
    stdout: Option<std::process::ChildStdout>,
    stderr: Option<std::process::ChildStderr>,
    events: Sender<HostChildEvent>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        std::thread::scope(|scope| {
            if let Some(mut pipe) = stdout {
                let events = events.clone();
                scope.spawn(move || {
                    let mut buffer = [0u8; 8192];
                    while forward_child_output(&mut pipe, false, &events, &mut buffer) {}
                });
            }
            if let Some(mut pipe) = stderr {
                let events = events.clone();
                scope.spawn(move || {
                    let mut buffer = [0u8; 8192];
                    while forward_child_output(&mut pipe, true, &events, &mut buffer) {}
                });
            }
        });
    })
}

/// `Command::spawn`, retried while the kernel answers ETXTBSY.
///
/// A fork in another thread can briefly hold a copy of the write descriptor of a file that was just
/// written, so exec'ing that file (a freshly copied or generated executable) fails with "Text file
/// busy" until that other child reaches its own exec. The condition clears within milliseconds.
fn spawn_retrying_text_busy(command: &mut Command) -> io::Result<std::process::Child> {
    const ATTEMPTS: u32 = 20;
    for attempt in 1.. {
        match command.spawn() {
            #[cfg(unix)]
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) && attempt < ATTEMPTS => {
                std::thread::sleep(Duration::from_millis(5 * u64::from(attempt)));
            }
            result => return result,
        }
    }
    unreachable!()
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
                refed: true,
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

/// `ChildProcess.kill()`: true only when the signal was really delivered, as
/// reported by the control loop that owns the child (no pid-reuse race).
fn kill_host_child(handle: u32, signal: i32) -> bool {
    let (delivered, reply) = mpsc::channel();
    let sent = HOST_CHILDREN.with(|table| {
        table
            .borrow()
            .children
            .get(&handle)
            .is_some_and(|child| child.commands.send(HostChildCommand::Kill(signal, delivered)).is_ok())
    });
    sent && reply.recv_timeout(Duration::from_secs(1)).unwrap_or(false)
}

fn host_children_active() -> bool {
    HOST_CHILDREN.with(|table| table.borrow().children.values().any(|child| child.refed))
}

/// QuickJS promise hook: forwards promise creation (`init`, with the parent promise) and resolution to the
/// JavaScript `async_hooks` dispatcher. Installed only while an `async_hooks` hook is enabled.
unsafe extern "C" fn async_hooks_promise_hook(
    ctx: *mut rquickjs::qjs::JSContext,
    kind: rquickjs::qjs::JSPromiseHookType,
    promise: rquickjs::qjs::JSValue,
    parent: rquickjs::qjs::JSValue,
    _opaque: *mut std::ffi::c_void,
) {
    use rquickjs::qjs;
    // before/after only cover thenable jobs in QuickJS; init and resolve carry the information hooks need.
    if kind != qjs::JSPromiseHookType_JS_PROMISE_HOOK_INIT
        && kind != qjs::JSPromiseHookType_JS_PROMISE_HOOK_RESOLVE
    {
        return;
    }
    // SAFETY: called by the engine with a live context and live values; every value created here is freed.
    unsafe {
        let global = qjs::JS_GetGlobalObject(ctx);
        let function = qjs::JS_GetPropertyStr(ctx, global, c"__thaw_async_hooks_promise".as_ptr());
        if qjs::JS_IsFunction(ctx, function) {
            let mut arguments = [qjs::JS_NewFloat64(f64::from(kind)), promise, parent];
            let result = qjs::JS_Call(ctx, function, qjs::JS_UNDEFINED, 3, arguments.as_mut_ptr());
            if qjs::JS_IsException(result) {
                let error = qjs::JS_GetException(ctx);
                qjs::JS_FreeValue(ctx, error);
            }
            qjs::JS_FreeValue(ctx, result);
        }
        qjs::JS_FreeValue(ctx, function);
        qjs::JS_FreeValue(ctx, global);
    }
}

fn set_async_hooks_promise_hook(ctx: &Ctx<'_>, enabled: bool) {
    // SAFETY: the runtime pointer comes from the live context that is installing the hook.
    unsafe {
        let runtime = rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
        rquickjs::qjs::JS_SetPromiseHook(
            runtime,
            if enabled { Some(async_hooks_promise_hook) } else { None },
            std::ptr::null_mut(),
        );
    }
}

thread_local! {
    static LOOP_START: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
    static LOOP_IDLE: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

/// The event loop starts when the main script has finished and the host first turns it (or sleeps in it).
fn mark_loop_started() {
    LOOP_START.with(|start| {
        if start.get().is_none() {
            start.set(Some(std::time::Instant::now()));
        }
    });
}

fn record_loop_idle(slept: Duration) {
    LOOP_IDLE.with(|idle| idle.set(idle.get() + slept));
}

/// `[active_ms, idle_ms]` since the event loop started (`[0, 0]` before it did): the time the loop spent
/// sleeping for the next timer/event is idle, everything else is active.
fn host_loop_stats() -> Vec<f64> {
    let Some(start) = LOOP_START.with(Cell::get) else {
        return vec![0.0, 0.0];
    };
    let idle = LOOP_IDLE.with(Cell::get).min(start.elapsed());
    vec![(start.elapsed() - idle).as_secs_f64() * 1000.0, idle.as_secs_f64() * 1000.0]
}

/// `ChildProcess.ref()` / `unref()`: only referenced children keep the event loop alive.
fn set_host_child_ref(handle: u32, refed: bool) {
    HOST_CHILDREN.with(|table| {
        if let Some(child) = table.borrow_mut().children.get_mut(&handle) {
            child.refed = refed;
        }
    });
}

/// The `stdio` modes the JS layer hands to the host: "pipe" (the default), "inherit" or "ignore".
fn child_stdio_modes(options: &serde_json::Value) -> [&str; 3] {
    let mode = |index: usize| {
        match options.get("stdio").and_then(|stdio| stdio.get(index)).and_then(|mode| mode.as_str()) {
            Some("inherit") => "inherit",
            Some("ignore") => "ignore",
            _ => "pipe",
        }
    };
    [mode(0), mode(1), mode(2)]
}

fn stdio_for(mode: &str) -> Stdio {
    match mode {
        "inherit" => Stdio::inherit(),
        "ignore" => Stdio::null(),
        _ => Stdio::piped(),
    }
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

/// Like Node (libuv), never die of SIGPIPE: a write to a pipe whose reader is
/// gone (a killed child's stdin, a closed stdout) must surface as EPIPE.
fn ignore_sigpipe() {
    #[cfg(unix)]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        });
    }
}

fn host_stdin_active() -> bool {
    !HOST_STDIN_UNREFED.with(std::cell::Cell::get) && HOST_STDIN.with(|slot| slot.borrow().is_some())
}

/// `process.stdin.ref()` / `unref()`: an unref'd stdin keeps delivering data
/// but no longer keeps the event loop alive.
fn set_host_stdin_ref(refed: bool) {
    HOST_STDIN_UNREFED.with(|flag| flag.set(!refed));
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
                    Ok(HostChildEvent::Exited { code, signal }) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "exited", "code": code, "signal": signal }));
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
    // Like Node's default `stdio: 'pipe'`, stdin is always a pipe that is closed after
    // the (possibly empty) input, so the child never inherits the parent's stdin.
    let input = Some(
        options
            .get("input")
            .and_then(|value| value.as_str())
            .map(hex_decode)
            .unwrap_or_default(),
    );
    child.stdin(Stdio::piped());
    let timeout = options
        .get("timeout")
        .and_then(serde_json::Value::as_u64)
        .filter(|milliseconds| *milliseconds > 0)
        .map(Duration::from_millis);
    let max_buffer = options
        .get("maxBuffer")
        .and_then(serde_json::Value::as_u64)
        .map(|bytes| bytes as usize);
    let kill_signal = match options.get("killSignal").and_then(serde_json::Value::as_i64) {
        None => 15,
        Some(signal @ 0..=64) => signal as i32,
        Some(signal) => return serde_json::json!({
            "error": format!("The \"killSignal\" argument must be a signal number between 0 and 64. Received {signal}"),
            "code": "EINVAL",
            "errno": null,
            "path": command,
        }).to_string(),
    };
    let mut child = match spawn_retrying_text_busy(&mut child) {
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
    let failure = |error: &io::Error, code: &str| {
        serde_json::json!({ "error": error.to_string(), "code": code, "pid": pid }).to_string()
    };
    let stop_child = |child: &mut std::process::Child| {
        let _ = child.kill();
        let _ = child.wait();
    };
    // Each pipe is drained by its own thread into a shared buffer so that the
    // supervisor below can enforce `timeout`/`maxBuffer` while the child is
    // still running and can still return what was captured.
    // Like libuv's spawnSync, `maxBuffer` bounds stdout and stderr together.
    let total_output = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let capture = |pipe: Option<Box<dyn Read + Send>>| -> io::Result<std::sync::Arc<SyncPipeCapture>> {
        let shared = std::sync::Arc::new(SyncPipeCapture::default());
        let mut pipe = pipe.ok_or_else(|| io::Error::other("missing child output pipe"))?;
        let reader = shared.clone();
        let total_output = total_output.clone();
        std::thread::Builder::new().spawn(move || {
            let mut chunk = [0u8; 8192];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) => break,
                    // Past the limit the child is being stopped: keep draining (and
                    // discarding) so it dies of our signal, not of a closed pipe.
                    Ok(length) => {
                        if reader.exceeded.load(std::sync::atomic::Ordering::SeqCst) {
                            continue;
                        }
                        let mut data = reader.data.lock().unwrap_or_else(|poison| poison.into_inner());
                        data.extend_from_slice(&chunk[..length]);
                        let total = total_output.fetch_add(length, std::sync::atomic::Ordering::SeqCst) + length;
                        if max_buffer.is_some_and(|limit| total > limit) {
                            reader.exceeded.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        *reader.error.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(error);
                        break;
                    }
                }
            }
            reader.finished.store(true, std::sync::atomic::Ordering::SeqCst);
        })?;
        Ok(shared)
    };
    let stdout_pipe: Option<Box<dyn Read + Send>> = child.stdout.take().map(|pipe| Box::new(pipe) as _);
    let stderr_pipe: Option<Box<dyn Read + Send>> = child.stderr.take().map(|pipe| Box::new(pipe) as _);
    let stdout = match capture(stdout_pipe) {
        Ok(shared) => shared,
        Err(error) => {
            stop_child(&mut child);
            return failure(&error, "UNKNOWN");
        }
    };
    let stderr = match capture(stderr_pipe) {
        Ok(shared) => shared,
        Err(error) => {
            stop_child(&mut child);
            return failure(&error, "UNKNOWN");
        }
    };
    let writer_receiver = match input {
        None => None,
        Some(input) => {
            let Some(stdin) = child.stdin.take() else {
                stop_child(&mut child);
                return failure(&io::Error::other("missing child stdin pipe"), "UNKNOWN");
            };
            let (sender, receiver) = mpsc::sync_channel(1);
            let spawned = std::thread::Builder::new().spawn(move || {
                let mut stdin = stdin;
                let result = stdin.write_all(&input);
                drop(stdin);
                let _ = sender.send(result);
            });
            if let Err(error) = spawned {
                stop_child(&mut child);
                return failure(&error, "UNKNOWN");
            }
            Some(receiver)
        }
    };

    let started = std::time::Instant::now();
    let mut signalled_at: Option<std::time::Instant> = None;
    let mut timed_out = false;
    let mut writer_done = writer_receiver.is_none();
    let mut write_error = None;
    let mut status = None;
    let mut supervisor_error = None;
    let signal_child = |child: &mut std::process::Child| {
        #[cfg(unix)]
        unsafe {
            libc::kill(child.id() as libc::pid_t, kill_signal);
        }
        #[cfg(not(unix))]
        let _ = child.kill();
    };
    loop {
        if let Some(receiver) = &writer_receiver {
            if !writer_done {
                match receiver.try_recv() {
                    Ok(result) => {
                        writer_done = true;
                        // A write failure caused by our own kill is not an input error.
                        if signalled_at.is_none() {
                            write_error = result.err();
                            if write_error.is_some() {
                                break;
                            }
                        }
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        supervisor_error =
                            Some(io::Error::other("stdin writer result channel disconnected"));
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(exit_status) => status = exit_status,
                Err(error) => {
                    supervisor_error = Some(error);
                    break;
                }
            }
        }
        let exceeded = stdout.exceeded.load(std::sync::atomic::Ordering::SeqCst)
            || stderr.exceeded.load(std::sync::atomic::Ordering::SeqCst);
        if status.is_none() && signalled_at.is_none() {
            if timeout.is_some_and(|limit| started.elapsed() >= limit) {
                timed_out = true;
                signal_child(&mut child);
                signalled_at = Some(std::time::Instant::now());
            } else if exceeded {
                signal_child(&mut child);
                signalled_at = Some(std::time::Instant::now());
            }
        }
        let readers_finished = stdout.finished.load(std::sync::atomic::Ordering::SeqCst)
            && stderr.finished.load(std::sync::atomic::Ordering::SeqCst);
        // After we killed the child, a grandchild may keep the pipes open
        // forever: stop waiting for them shortly after the child is gone.
        let abandon_readers = signalled_at.is_some_and(|at| at.elapsed() > Duration::from_millis(100));
        if status.is_some() && (writer_done || signalled_at.is_some()) && (readers_finished || abandon_readers) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    if write_error.is_some() || supervisor_error.is_some() {
        stop_child(&mut child);
    }
    if let Some(error) = write_error {
        return failure(&error, "EPIPE");
    }
    if let Some(error) = supervisor_error {
        return failure(&error, "UNKNOWN");
    }
    for pipe in [&stdout, &stderr] {
        if let Some(error) = pipe.error.lock().unwrap_or_else(|poison| poison.into_inner()).take() {
            return failure(&error, "UNKNOWN");
        }
    }
    let Some(status) = status else {
        return failure(&io::Error::other("child status unavailable"), "UNKNOWN");
    };
    let take = |pipe: &SyncPipeCapture| {
        std::mem::take(&mut *pipe.data.lock().unwrap_or_else(|poison| poison.into_inner()))
    };
    let max_buffer_exceeded = stdout.exceeded.load(std::sync::atomic::Ordering::SeqCst)
        || stderr.exceeded.load(std::sync::atomic::Ordering::SeqCst);
    #[cfg(unix)]
    let signal = status.signal();
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    serde_json::json!({
        "pid": pid,
        "status": status.code(),
        "signal": signal,
        "stdout": hex_encode(&take(&stdout)),
        "stderr": hex_encode(&take(&stderr)),
        "timedOut": timed_out,
        "maxBufferExceeded": max_buffer_exceeded,
    })
    .to_string()
}

#[derive(Default)]
struct SyncPipeCapture {
    data: std::sync::Mutex<Vec<u8>>,
    error: std::sync::Mutex<Option<io::Error>>,
    exceeded: std::sync::atomic::AtomicBool,
    finished: std::sync::atomic::AtomicBool,
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
            HostWorker { commands, events, thread: Some(thread), refed: false, terminate: Arc::default() },
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

#[cfg(all(test, unix))]
#[test]
fn sync_timeout_and_kill_signal_stop_a_child_that_never_exits() {
    let started = std::time::Instant::now();
    let record: serde_json::Value = serde_json::from_str(&run_child_process(
        "/bin/sleep".into(),
        serde_json::json!(["30"]).to_string(),
        serde_json::json!({"timeout": 200, "killSignal": 9}).to_string(),
    )).unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(record["timedOut"], true);
    assert_eq!(record["signal"], 9);
    assert!(record["status"].is_null());
}

#[cfg(all(test, unix))]
#[test]
fn sync_max_buffer_stops_a_child_that_keeps_writing() {
    let started = std::time::Instant::now();
    let record: serde_json::Value = serde_json::from_str(&run_child_process(
        "/bin/sh".into(),
        serde_json::json!(["-c", "while :; do echo xxxxxxxxxxxxxxxx; done"]).to_string(),
        serde_json::json!({"maxBuffer": 1000, "killSignal": 15}).to_string(),
    )).unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(record["maxBufferExceeded"], true);
    assert_eq!(record["timedOut"], false);
    assert_eq!(record["signal"], 15);
    assert!(hex_decode(record["stdout"].as_str().unwrap()).len() > 1000);
}

#[cfg(all(test, unix))]
#[test]
fn sync_child_within_limits_reports_neither_flag() {
    let record: serde_json::Value = serde_json::from_str(&run_child_process(
        "/bin/sh".into(),
        serde_json::json!(["-c", "printf abc; printf def >&2"]).to_string(),
        serde_json::json!({"timeout": 5000, "maxBuffer": 1000}).to_string(),
    )).unwrap();
    assert_eq!(record["status"], 0);
    assert_eq!(record["timedOut"], false);
    assert_eq!(record["maxBufferExceeded"], false);
    assert_eq!(hex_decode(record["stdout"].as_str().unwrap()), b"abc");
    assert_eq!(hex_decode(record["stderr"].as_str().unwrap()), b"def");
}

#[test]
fn sync_kill_signal_outside_0_to_64_is_rejected_before_spawn() {
    let record: serde_json::Value = serde_json::from_str(&run_child_process(
        "/bin/sh".into(),
        serde_json::json!(["-c", "exit 0"]).to_string(),
        serde_json::json!({"killSignal": 65}).to_string(),
    )).unwrap();
    assert_eq!(record["code"], "EINVAL");
    assert!(record.get("pid").is_none());
}

#[cfg(all(test, unix))]
#[test]
fn spawned_children_get_default_signal_dispositions() {
    // A host started under nohup (SIGINT ignored) must not hand that to children, as in libuv.
    // SAFETY: only this test touches SIGINT's disposition, and it restores the previous one.
    let previous = unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
    let record: serde_json::Value = serde_json::from_str(&run_child_process(
        "/bin/sh".into(),
        serde_json::json!(["-c", "kill -INT $$; echo survived"]).to_string(),
        "{}".into(),
    )).unwrap();
    unsafe { libc::signal(libc::SIGINT, previous) };
    assert_eq!(record["signal"], 2);
    assert!(hex_decode(record["stdout"].as_str().unwrap()).is_empty());
}

#[cfg(all(test, unix))]
#[test]
fn async_kill_is_handled_while_a_stdin_backlog_cannot_drain() {
    ignore_sigpipe();
    let handle = spawn_host_child("/bin/sleep".into(), serde_json::json!(["30"]).to_string(), "{}".into());
    // `sleep` never reads stdin: far more than a pipe buffer stays queued.
    for _ in 0..16 {
        assert!(send_host_child_stdin(handle, hex_encode(&vec![b'z'; 256 * 1024]), false));
    }
    let started = std::time::Instant::now();
    assert!(kill_host_child(handle, 0), "signal 0 probes a live child without stopping it");
    assert!(kill_host_child(handle, 15));
    let mut exit = None;
    while exit.is_none() {
        for event in serde_json::from_str::<Vec<serde_json::Value>>(&poll_host_children()).unwrap() {
            if event["type"] == "exit" {
                exit = Some(event);
            }
        }
        assert!(started.elapsed() < Duration::from_secs(5), "kill was blocked by the stdin backlog");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(exit.unwrap()["signal"], 15);
    assert!(!kill_host_child(handle, 15), "a released handle reports no delivery");
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

    #[cfg(unix)]
    #[test]
    fn spawning_a_file_that_is_still_open_for_writing_waits_for_it_instead_of_failing() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!("thaw-text-busy-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let script = directory.join("busy.sh");
        std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        // While any descriptor open for writing exists, exec fails with ETXTBSY.
        let writer = std::fs::OpenOptions::new().append(true).open(&script).unwrap();
        assert_eq!(
            Command::new(&script).spawn().err().and_then(|error| error.raw_os_error()),
            Some(libc::ETXTBSY),
            "the fixture must reproduce the busy file"
        );
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            drop(writer);
        });
        let mut command = Command::new(&script);
        let status = spawn_retrying_text_busy(&mut command).expect("spawn must outlast the busy window").wait().unwrap();
        release.join().unwrap();
        let _ = std::fs::remove_dir_all(&directory);
        assert!(status.success());
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
        assert_eq!(events.first().unwrap()["type"], "spawn", "events: {events:?}");
        assert_eq!(events.last().unwrap()["type"], "exit", "events: {events:?}");
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

#[cfg(all(test, unix))]
#[test]
fn sync_child_without_input_gets_a_closed_stdin_pipe_not_the_parents() {
    // The child reports whether its stdin is a pipe it can read EOF from.
    let record: serde_json::Value = serde_json::from_str(&run_child_process(
        "/bin/sh".into(),
        serde_json::json!(["-c", "cat >/dev/null; printf eof"]).to_string(),
        serde_json::json!({"timeout": 5000}).to_string(),
    )).unwrap();
    assert_eq!(record["timedOut"], false);
    assert_eq!(hex_decode(record["stdout"].as_str().unwrap()), b"eof");
}

#[cfg(all(test, unix))]
mod m_wave_tests {
    use super::*;

    fn drain(handle_active: impl Fn() -> bool, deadline: std::time::Instant) -> Vec<serde_json::Value> {
        let mut events = Vec::new();
        loop {
            events.extend(serde_json::from_str::<Vec<serde_json::Value>>(&poll_host_children()).unwrap());
            if !handle_active() {
                return events;
            }
            assert!(std::time::Instant::now() < deadline, "child did not finish in time");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn an_unreferenced_child_does_not_keep_the_event_loop_alive_until_it_is_referenced_again() {
        let handle = spawn_host_child(
            "sleep".to_string(),
            serde_json::json!(["30"]).to_string(),
            serde_json::json!({ "stdio": ["ignore", "ignore", "ignore"] }).to_string(),
        );
        assert!(host_children_active(), "a fresh child is referenced");
        set_host_child_ref(handle, false);
        assert!(!host_children_active(), "an unreferenced child must not count as activity");
        set_host_child_ref(handle, true);
        assert!(host_children_active());
        assert!(kill_host_child(handle, libc::SIGKILL));
        drain(host_children_active, std::time::Instant::now() + Duration::from_secs(5));
        assert!(!host_children_active());
    }

    /// A child that writes stdout and then stderr must have the two reach the event loop in that order,
    /// however the reader threads happen to be scheduled (execFile's per-pipe `maxBuffer` depends on it).
    #[cfg(unix)]
    #[test]
    fn stdout_then_stderr_output_keeps_write_order_across_pipes() {
        for round in 0..300 {
            spawn_host_child(
                "sh".to_string(),
                serde_json::json!(["-c", "printf abcd; printf x >&2"]).to_string(),
                serde_json::json!({}).to_string(),
            );
            let events = drain(host_children_active, std::time::Instant::now() + Duration::from_secs(10));
            let first = |kind: &str| events.iter().position(|event| event["type"] == kind);
            let (out, err) = (first("stdout").expect("stdout event"), first("stderr").expect("stderr event"));
            assert!(out < err, "round {round}: stderr overtook stdout: {events:?}");
        }
    }

    #[test]
    fn inherited_and_ignored_stdio_attach_no_pipes_to_the_host() {
        let handle = spawn_host_child(
            "sh".to_string(),
            serde_json::json!(["-c", "echo to-stdout; echo to-stderr >&2"]).to_string(),
            serde_json::json!({ "stdio": ["ignore", "ignore", "ignore"] }).to_string(),
        );
        let events = drain(host_children_active, std::time::Instant::now() + Duration::from_secs(5));
        let _ = handle;
        assert!(
            !events.iter().any(|event| event["type"] == "stdout" || event["type"] == "stderr"),
            "ignored stdio must not be piped: {events:?}"
        );
        let handle = spawn_host_child(
            "sh".to_string(),
            serde_json::json!(["-c", "echo piped-out"]).to_string(),
            serde_json::json!({}).to_string(),
        );
        let events = drain(host_children_active, std::time::Instant::now() + Duration::from_secs(5));
        let _ = handle;
        let output: Vec<u8> = events
            .iter()
            .filter(|event| event["type"] == "stdout")
            .flat_map(|event| hex_decode(event["value"].as_str().unwrap()))
            .collect();
        assert_eq!(output, b"piped-out\n", "the default stays piped");
    }

    #[test]
    fn event_loop_statistics_split_sleeping_from_working() {
        mark_loop_started();
        let before = host_loop_stats();
        sleep_until_next_poll(5);
        std::thread::sleep(Duration::from_millis(0));
        let after = host_loop_stats();
        assert!(after[1] > before[1], "sleeping in the loop is idle time: {before:?} -> {after:?}");
        assert!(after[0] >= 0.0 && after[1] >= 0.0);
    }
}
