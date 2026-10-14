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
        if (onSet && this[slot]) onSet.call(this);
      },
      enumerable: true, configurable: true
    });
  };
  if (typeof globalThis.MessageChannel !== 'function') {
    const __thawDefineProperty = Object.defineProperty;
    const __thawMessagePortBrand = new WeakSet();
    const __thawWeakSetHas = WeakSet.prototype.has;
    const __thawApply = Reflect.apply;
    const __thawIsMessagePort = value => __thawApply(__thawWeakSetHas, __thawMessagePortBrand, [value]);
    class MessagePort extends EventTarget {
      constructor() {
        super();
        __thawMessagePortBrand.add(this);
        this.__thawPeer = null;
        this.__thawQueue = [];
        this.__thawScheduled = false;
        this.__thawStarted = false;
        this.__thawClosed = false;
        this.__thawRefed = true;
        this.__thawNodeListeners = new Map();
      }
      postMessage(value, transferOrOptions = []) {
        const transfer = transferOrOptions !== null && typeof transferOrOptions === 'object' && !(Symbol.iterator in transferOrOptions)
          ? (transferOrOptions.transfer === undefined ? [] : transferOrOptions.transfer)
          : transferOrOptions;
        if (Array.prototype.includes.call(Array.from(transfer), this)) throw new DOMException('Transfer list contains source port', 'DataCloneError');
        if (this.__thawClosed || !this.__thawPeer || this.__thawPeer.__thawClosed) return;
        const copy = structuredClone(value, { transfer });
        const ports = transfer.filter(item => item instanceof MessagePort)
          .map(item => item.__thawTransferredPort || item);
        this.__thawPeer.__thawQueue.push({ data: copy, ports });
        this.__thawPeer.__thawSchedule();
      }
      __thawValidateTransfer() {
        if (!__thawIsMessagePort(this)
            || this.__thawClosed || !this.__thawPeer) {
          throw new DOMException('MessagePort is already detached', 'DataCloneError');
        }
      }
      __thawTransferToValidated(transferred, hasTransferReference) {
        transferred.__thawPeer = this.__thawPeer;
        transferred.__thawChannelPair = this.__thawChannelPair;
        transferred.__thawQueue = this.__thawQueue;
        this.__thawPeer.__thawPeer = transferred;
        this.__thawQueue = [];
        this.__thawPeer = null;
        this.__thawClosed = true;
        if (hasTransferReference) this.__thawTransferredPort = transferred;
        else __thawDefineProperty(this, '__thawTransferredPort', {
          value: transferred, writable: true, configurable: true
        });
        return transferred;
      }
      __thawTransferTo(transferred) {
        this.__thawValidateTransfer();
        return this.__thawTransferToValidated(transferred, Object.prototype.hasOwnProperty.call(this, '__thawTransferredPort'));
      }
      __thawTransfer() {
        this.__thawValidateTransfer();
        return this.__thawTransferTo(new MessagePort());
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
      start() { this.__thawStarted = true; this.__thawSchedule(); }
      close() {
        if (this.__thawClosed) return;
        const peer = this.__thawPeer;
        this.__thawClosed = true;
        this.__thawQueue.length = 0;
        this.dispatchEvent(new Event('close'));
        for (const listener of (this.__thawNodeListeners.get('close') || []).slice()) listener.call(this);
        // Closing one end of a MessageChannel closes the entangled end too; host-owned worker ports manage their own peer.
        if (peer && this.__thawChannelPair && peer.__thawChannelPair && !peer.__thawClosed) peer.__thawCloseAfterDrain();
      }
      ref() { this.__thawRefed = true; return this; }
      unref() { this.__thawRefed = false; return this; }
      hasRef() { return this.__thawRefed; }
      on(name, listener) { const key = String(name); const list = this.__thawNodeListeners.get(key) || []; list.push(listener); this.__thawNodeListeners.set(key, list); if (key === 'message') this.start(); return this; }
      once(name, listener) { const wrapped = (...args) => { this.off(name, wrapped); listener.apply(this, args); }; wrapped.listener = listener; return this.on(name, wrapped); }
      off(name, listener) { const key = String(name); const list = this.__thawNodeListeners.get(key) || []; this.__thawNodeListeners.set(key, list.filter(entry => entry !== listener && entry.listener !== listener)); return this; }
      addListener(name, listener) { return this.on(name, listener); }
      removeListener(name, listener) { return this.off(name, listener); }
      removeAllListeners(name) { if (name === undefined) this.__thawNodeListeners.clear(); else this.__thawNodeListeners.delete(String(name)); return this; }
    }
    __thawDefineEventHandler(MessagePort.prototype, 'message', function() { this.start(); });
    __thawDefineEventHandler(MessagePort.prototype, 'messageerror');
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
