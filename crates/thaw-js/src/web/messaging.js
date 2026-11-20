// MessageEvent (ported from thaw-quickjs `platform_globals/workers/blob_events.js`); messaging.js below needs it.
if (typeof globalThis.MessageEvent !== 'function') {
  const messageFields = new WeakMap();
  const messageOf = self => {
    const record = messageFields.get(self);
    if (!record) throw new TypeError('Illegal invocation');
    return record;
  };
  globalThis.MessageEvent = class MessageEvent extends Event {
    constructor(type, options = {}) {
      super(type, options);
      messageFields.set(this, {
        data: options.data === undefined ? null : options.data,
        origin: options.origin === undefined ? '' : String(options.origin),
        lastEventId: options.lastEventId === undefined ? '' : String(options.lastEventId),
        source: options.source === undefined ? null : options.source,
        ports: options.ports === undefined ? [] : Array.from(options.ports),
      });
    }
    // Node (legacy): returns a new MessageEvent built from the arguments; this one is unchanged.
    initMessageEvent(type, bubbles = false, cancelable = false, data = null, origin = '', lastEventId = '', source = null, ports = []) {
      messageOf(this);
      if (arguments.length === 0) throw new TypeError('MessageEvent.initMessageEvent: 1 argument required, but 0 found.');
      return new MessageEvent(type, { bubbles, cancelable, data, origin, lastEventId, source, ports });
    }
  };
  for (const key of ['data', 'origin', 'lastEventId', 'source', 'ports']) {
    Object.defineProperty(globalThis.MessageEvent.prototype, key, {
      get() { return messageOf(this)[key]; },
      enumerable: true, configurable: true,
    });
  }
}
// MessagePort / MessageChannel / BroadcastChannel (verbatim from thaw-quickjs `platform_globals/workers/messaging.js`).
  // `on<type>` event-handler attributes are one listener, registered when the
  // handler is first set, so it runs in listener order and honours
  // stopImmediatePropagation like any other listener.
  const __thawDefineEventHandler = (prototype, type, onSet) => {
    const slot = Symbol(`on${type}`), registered = Symbol(`on${type} registered`);
    Object.defineProperty(prototype, `on${type}`, {
      get() { return this[slot] === undefined ? null : this[slot]; },
      set(handler) {
        this[slot] = typeof handler === 'function' ? handler : null;
        if (this[slot] && !this[registered]) {
          this[registered] = true;
          this.addEventListener(type, event => { const current = this[slot]; if (current) Reflect.apply(current, this, [event]); });
        }
        if (onSet) onSet.call(this);
      },
      enumerable: true, configurable: true
    });
  };
  if (typeof globalThis.MessageChannel !== 'function') {
    const __thawMessagePortBrand = new WeakSet();
    const __thawWeakSetHas = WeakSet.prototype.has;
    const __thawApply = Reflect.apply;
    const __thawIsMessagePort = value => __thawApply(__thawWeakSetHas, __thawMessagePortBrand, [value]);
    // Cross-thread ends (worker_threads PR3): a port whose peer lives on another thread has `__thawPeerAddr` (the
    // peer's address) and `__thawAddr` (its own). `portsByAddr` finds the end a routed event is for; `remotePorts`
    // are the ends whose listeners keep the loop alive (`__thaw_port_live`). The routes are in worker.rs.
    const portsByAddr = new Map(), remotePorts = new Set();
    const refreshPortLive = () => {
      const live = [...remotePorts].some(port => port.__thawRefed && !port.__thawClosed
        && (port.onmessage !== null || (port.__thawNodeListeners.get('message') || []).length > 0));
      __thaw_port_live(live);
    };
    class MessagePort extends EventTarget {
      constructor() {
        super();
        __thawMessagePortBrand.add(this);
        this.__thawPeer = null;
        this.__thawAddr = null;
        this.__thawPeerAddr = null;
        this.__thawPeerGone = false;
        this.__thawQueue = [];
        this.__thawScheduled = false;
        this.__thawStarted = false;
        this.__thawClosed = false;
        this.__thawRefed = true;
        this.__thawNodeListeners = new Map();
      }
      // Returns true when the message is posted, undefined on a closed or transferred port (Node). The list is
      // checked (and the source-port rule applied) before anything is cloned or moved.
      postMessage(value, transferOrOptions = []) {
        if (this.__thawClosed) return undefined;
        if (this.__thawPeerAddr !== null) {
          const list = __thaw_transfer_of(transferOrOptions);
          if (list.includes(this)) throw new DOMException('Transfer list contains source port', 'DataCloneError');
          const peer = this.__thawPeerAddr;
          __thaw_port_send(peer, __thaw_worker_encode(value, list, __thaw_port_owner(peer)));
          return true;
        }
        if (!this.__thawPeer) return undefined;
        const list = __thaw_transfer_of(transferOrOptions);
        if (list.includes(this)) throw new DOMException('Transfer list contains source port', 'DataCloneError');
        const target = this.__thawPeer;
        const { value: copy, ports } = __thaw_transfer_clone(value, list);
        // a target moved by this same message is gone (Node: "target port was posted to itself"): the message is lost
        if (!target.__thawClosed) {
          target.__thawQueue.push({ data: copy, ports });
          target.__thawSchedule();
        }
        return true;
      }
      // A port in a transfer list: `transferred` (a new port, from transferClone) takes over this port's queue and
      // its entangled peer, and this end is neutralised. Node emits 'close' on this end, asynchronously. A cross-thread
      // end moves its address to `transferred` (its peer keeps sending to that address).
      __thawTransferTo(transferred) {
        if (this.__thawPeerAddr !== null) {
          transferred.__thawAddr = this.__thawAddr;
          transferred.__thawPeerAddr = this.__thawPeerAddr;
          portsByAddr.set(this.__thawAddr, transferred);
          remotePorts.delete(this);
          remotePorts.add(transferred);
          this.__thawAddr = null;
          this.__thawPeerAddr = null;
        } else {
          transferred.__thawPeer = this.__thawPeer;
          transferred.__thawChannelPair = this.__thawChannelPair;
          this.__thawPeer.__thawPeer = transferred;
          this.__thawPeer = null;
        }
        transferred.__thawQueue = this.__thawQueue;
        this.__thawQueue = [];
        this.__thawClosed = true;
        queueMicrotask(() => this.__thawEmitClose());
        refreshPortLive();
      }
      __thawEmitClose() {
        this.dispatchEvent(new Event('close'));
        for (const listener of (this.__thawNodeListeners.get('close') || []).slice()) listener.call(this);
      }
      __thawSchedule() {
        if (!this.__thawStarted || this.__thawScheduled || this.__thawClosed) return;
        this.__thawScheduled = true;
        queueMicrotask(() => {
          this.__thawScheduled = false;
          while (!this.__thawClosed && this.__thawQueue.length) {
            const record = this.__thawQueue.shift();
            const event = new MessageEvent('message', { data: record.data, ports: record.ports });
            this.dispatchEvent(event);
            for (const listener of (this.__thawNodeListeners.get('message') || []).slice()) listener.call(this, record.data);
          }
          if (this.__thawPendingClose && !this.__thawClosed && !this.__thawQueue.length) this.close();
        });
      }
      __thawDispatchError(error) {
        const event = new MessageEvent('messageerror', { data: error });
        this.dispatchEvent(event);
        for (const listener of (this.__thawNodeListeners.get('messageerror') || []).slice()) listener.call(this, error);
      }
      // The entangled end closes once the messages already posted to it have been delivered.
      __thawCloseAfterDrain() {
        if (this.__thawQueue.length && this.__thawStarted) this.__thawPendingClose = true;
        else this.close();
      }
      start() { this.__thawStarted = true; this.__thawSchedule(); refreshPortLive(); }
      close() {
        if (this.__thawClosed) return;
        const peer = this.__thawPeer;
        const remote = this.__thawPeerAddr;
        this.__thawClosed = true;
        this.__thawQueue.length = 0;
        this.__thawEmitClose();
        if (remote !== null) {
          // a cross-thread end: its address goes, and the peer hears of it unless it already hung up
          portsByAddr.delete(this.__thawAddr);
          remotePorts.delete(this);
          __thaw_port_closed(this.__thawAddr, remote, !this.__thawPeerGone);
          refreshPortLive();
        }
        // Closing one end of a MessageChannel closes the entangled end too; host-owned worker ports manage their own peer.
        if (peer && this.__thawChannelPair && peer.__thawChannelPair && !peer.__thawClosed) peer.__thawCloseAfterDrain();
      }
      ref() { this.__thawRefed = true; refreshPortLive(); }
      unref() { this.__thawRefed = false; refreshPortLive(); }
      hasRef() { return this.__thawRefed; }
      on(name, listener) { const key = String(name); const list = this.__thawNodeListeners.get(key) || []; list.push(listener); this.__thawNodeListeners.set(key, list); if (key === 'message') this.start(); return this; }
      once(name, listener) { const wrapped = (...args) => { this.off(name, wrapped); listener.apply(this, args); }; wrapped.listener = listener; return this.on(name, wrapped); }
      off(name, listener) { const key = String(name); const list = this.__thawNodeListeners.get(key) || []; this.__thawNodeListeners.set(key, list.filter(entry => entry !== listener && entry.listener !== listener)); refreshPortLive(); return this; }
      addListener(name, listener) { return this.on(name, listener); }
      removeListener(name, listener) { return this.off(name, listener); }
      removeAllListeners(name) { if (name === undefined) this.__thawNodeListeners.clear(); else this.__thawNodeListeners.delete(String(name)); refreshPortLive(); return this; }
    }
    __thawDefineEventHandler(MessagePort.prototype, 'message', function() { this.start(); });
    __thawDefineEventHandler(MessagePort.prototype, 'messageerror');
    // Hooks the realm's dispatch (worker_threads.js, worker_bootstrap.js) and worker_wire.js call.
    // A message to an end of this realm: decoded (its ports land here) and queued on that end.
    globalThis.__thaw_port_deliver = (addr, wire) => {
      const port = portsByAddr.get(addr);
      if (port === undefined || port.__thawClosed) { __thaw_wire_release(wire); return; }
      const { value, ports } = __thaw_worker_decode_full(wire);
      port.__thawQueue.push({ data: value, ports });
      port.__thawSchedule();
    };
    // The peer of an end of this realm closed: it closes once the messages already queued have been delivered.
    globalThis.__thaw_port_hangup = addr => {
      const port = portsByAddr.get(addr);
      if (port === undefined) return;
      port.__thawPeerGone = true;
      port.__thawCloseAfterDrain();
    };
    // A port end arriving in this realm (worker_wire.js decodes a `Port` node into this).
    globalThis.__thaw_port_adopt = (addr, peerAddr, records) => {
      const port = new MessagePort();
      port.__thawAddr = addr;
      port.__thawPeerAddr = peerAddr;
      port.__thawQueue = records;
      __thaw_port_landed(addr);
      portsByAddr.set(addr, port);
      remotePorts.add(port);
      refreshPortLive();
      return port;
    };
    // Encoding a listed port (worker_wire.js): `prepare` reads what moves (its address pair and queued messages, no
    // side effect yet), `commit` moves it once the whole value has encoded. A port whose peer is listed too is refused.
    globalThis.__thaw_port_prepare = (port, list, dest) => {
      const remote = port.__thawPeerAddr !== null;
      const peer = remote ? null : port.__thawPeer;
      if (peer !== null && list.includes(peer)) throw new DOMException('both ends of a MessagePort channel in one transfer list are not supported yet', 'DataCloneError');
      const qs = port.__thawQueue.map(record => __thaw_worker_encode(record.data, [], dest));
      return { remote, peer, p: remote ? port.__thawAddr : __thaw_port_address(), q: remote ? port.__thawPeerAddr : __thaw_port_address(), qs };
    };
    globalThis.__thaw_port_commit = (port, plan, dest) => {
      if (!plan.remote) {
        // the in-realm peer becomes this port's cross-thread peer (it keeps its queue, which is its own)
        const peer = plan.peer;
        peer.__thawPeer = null;
        peer.__thawAddr = plan.q;
        peer.__thawPeerAddr = plan.p;
        portsByAddr.set(plan.q, peer);
        remotePorts.add(peer);
      }
      __thaw_port_move(plan.p, plan.q, dest, !plan.remote);
      // this end is neutralised: a silent no-op, and 'close' on this end (asynchronously)
      if (port.__thawAddr !== null && portsByAddr.get(port.__thawAddr) === port) portsByAddr.delete(port.__thawAddr);
      remotePorts.delete(port);
      port.__thawAddr = null;
      port.__thawPeerAddr = null;
      port.__thawPeer = null;
      port.__thawQueue = [];
      port.__thawClosed = true;
      queueMicrotask(() => port.__thawEmitClose());
      refreshPortLive();
    };
    globalThis.MessagePort = MessagePort;
    globalThis.__thaw_is_message_port = __thawIsMessagePort;
    globalThis.__thaw_message_port_postmessage_with_uncloneable_checks = MessagePort.prototype.postMessage;
    globalThis.MessageChannel = class MessageChannel {
      constructor() {
        this.port1 = new MessagePort();
        this.port2 = new MessagePort();
        this.port1.__thawPeer = this.port2;
        this.port2.__thawPeer = this.port1;
        this.port1.__thawChannelPair = this.port2.__thawChannelPair = true;
      }
    };
  }
  if (typeof globalThis.BroadcastChannel !== 'function') {
    const broadcastChannels = new Map();
    globalThis.BroadcastChannel = class BroadcastChannel extends EventTarget {
      constructor(name) {
        super();
        this.name = String(name);
        this.__thawClosed = false;
        this.__thawQueue = [];
        const channels = broadcastChannels.get(this.name) || new Set();
        channels.add(this);
        broadcastChannels.set(this.name, channels);
      }
      postMessage(value) {
        if (this.__thawClosed) throw new DOMException('BroadcastChannel is closed', 'InvalidStateError');
        const channels = broadcastChannels.get(this.name) || [];
        const snapshot = structuredClone(value);
        for (const channel of channels) {
          if (channel === this || channel.__thawClosed) continue;
          const copy = structuredClone(snapshot);
          channel.__thawQueue.push({ data: copy });
          queueMicrotask(() => {
            if (channel.__thawClosed) return;
            const record = channel.__thawQueue.shift();
            if (!record) return;
            const event = new MessageEvent('message', { data: record.data });
            channel.dispatchEvent(event);
          });
        }
      }
      __thawDispatchError(error) {
        const event = new MessageEvent('messageerror', { data: error });
        this.dispatchEvent(event);
      }
      close() {
        if (this.__thawClosed) return;
        this.__thawClosed = true;
        const channels = broadcastChannels.get(this.name);
        if (channels) {
          channels.delete(this);
          if (channels.size === 0) broadcastChannels.delete(this.name);
        }
      }
      ref() { return this; }
      unref() { return this; }
    };
    __thawDefineEventHandler(globalThis.BroadcastChannel.prototype, 'message');
    __thawDefineEventHandler(globalThis.BroadcastChannel.prototype, 'messageerror');
  }
