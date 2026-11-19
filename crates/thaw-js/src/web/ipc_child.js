// The cluster worker's IPC channel on this realm's `process` (ipc.rs installs it when NODE_UNIQUE_ID is set).
// Rust owns the stdin reader and the stdout lock; this side only serializes, and turns the events Rust queues
// for this realm into `process` events. The intrinsics are captured so user code cannot replace them.
(function () {
  const stringify = JSON.stringify, parse = JSON.parse;
  const proc = process;
  proc.connected = true;
  // Node's Channel reference counter (same rule as the registry's makeChannel in child_process). Nothing reads it:
  // the child's loop is held by ipc.rs, not by this object.
  // ponytail: ref state is not wired to the loop; upgrade = ipc.rs reads a ref flag, as it does for a message listener.
  function makeChannel() {
    let refs = 0, explicit = false, ref = true;
    const channel = {};
    channel.ref = function () { explicit = true; ref = true; };
    channel.unref = function () { explicit = true; ref = false; };
    channel.refCounted = function () { if (++refs === 1 && !explicit) ref = true; };
    channel.unrefCounted = function () { if (--refs === 0 && !explicit) ref = false; };
    return channel;
  }
  proc.channel = makeChannel();
  proc.send = function (value, handle, options, callback) {
    if (typeof handle === 'function') callback = handle;
    else if (typeof options === 'function') callback = options;
    let body;
    try { body = stringify(value); } catch (error) {
      if (typeof callback !== 'function') throw error;
      process.nextTick(callback, error);
      return false;
    }
    if (!proc.connected) {
      const closed = new Error('Channel closed');
      closed.code = 'ERR_IPC_CHANNEL_CLOSED';
      process.nextTick(function () {
        if (typeof callback === 'function') callback(closed);
        else proc.emit('error', closed);
      });
      return false;
    }
    __thaw_ipc_send(body);
    if (typeof callback === 'function') process.nextTick(callback, null);
    return true;
  };
  proc.disconnect = function () {
    if (!proc.connected) return;
    proc.connected = false;
    __thaw_ipc_disconnect();
  };
  globalThis.__thaw_ipc_message = function (line) {
    const message = parse(line);
    if (message !== null && typeof message === 'object' && typeof message.cmd === 'string' && message.cmd.indexOf('NODE_') === 0) proc.emit('internalMessage', message);
    else proc.emit('message', message);
  };
  globalThis.__thaw_ipc_disconnected = function () {
    proc.connected = false;
    proc.channel = null;
    proc.emit('disconnect');
  };
})();
