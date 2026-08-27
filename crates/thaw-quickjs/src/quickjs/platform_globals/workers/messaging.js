  if (typeof globalThis.MessageChannel !== 'function') {
    class MessagePort extends EventTarget {
      constructor() {
        super();
        this.__thawPeer = null;
        this.__thawQueue = [];
        this.__thawScheduled = false;
        this.__thawClosed = false;
        this.__thawRefed = true;
        this.__thawNodeListeners = new Map();
        this.__thawOnMessage = null;
        this.__thawOnMessageError = null;
      }
      postMessage(value, transfer = []) {
        if (this.__thawClosed || !this.__thawPeer || this.__thawPeer.__thawClosed) return;
        let copy;
        try { copy = structuredClone(value, { transfer }); }
        catch (error) {
          const peer = this.__thawPeer;
          queueMicrotask(() => peer.__thawDispatchError(error));
          return;
        }
        const ports = transfer.filter(item => item instanceof MessagePort)
          .map(item => item.__thawTransferredPort || item);
        this.__thawPeer.__thawQueue.push({ data: copy, ports });
        this.__thawPeer.__thawSchedule();
      }
      __thawTransfer() {
        if (this.__thawClosed || !this.__thawPeer) {
          throw new DOMException('MessagePort is already detached', 'DataCloneError');
        }
        const transferred = new MessagePort();
        transferred.__thawPeer = this.__thawPeer;
        transferred.__thawQueue = this.__thawQueue;
        this.__thawPeer.__thawPeer = transferred;
        this.__thawQueue = [];
        this.__thawPeer = null;
        this.__thawClosed = true;
        this.__thawTransferredPort = transferred;
        return transferred;
      }
      __thawSchedule() {
        if (this.__thawScheduled || this.__thawClosed) return;
        this.__thawScheduled = true;
        queueMicrotask(() => {
          this.__thawScheduled = false;
          while (!this.__thawClosed && this.__thawQueue.length) {
            const record = this.__thawQueue.shift();
            const event = new MessageEvent('message', { data: record.data, ports: record.ports });
            this.dispatchEvent(event);
            if (typeof this.__thawOnMessage === 'function') this.__thawOnMessage.call(this, event);
            for (const listener of (this.__thawNodeListeners.get('message') || []).slice()) listener.call(this, record.data);
          }
        });
      }
      __thawDispatchError(error) {
        const event = new MessageEvent('messageerror', { data: error });
        this.dispatchEvent(event);
        if (typeof this.__thawOnMessageError === 'function') this.__thawOnMessageError.call(this, event);
        for (const listener of (this.__thawNodeListeners.get('messageerror') || []).slice()) listener.call(this, error);
      }
      start() { this.__thawSchedule(); }
      close() {
        if (this.__thawClosed) return;
        this.__thawClosed = true;
        this.__thawQueue.length = 0;
        this.dispatchEvent(new Event('close'));
        for (const listener of (this.__thawNodeListeners.get('close') || []).slice()) listener.call(this);
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
      set onmessage(listener) { this.__thawOnMessage = listener; if (listener) this.start(); }
      get onmessage() { return this.__thawOnMessage; }
      set onmessageerror(listener) { this.__thawOnMessageError = listener; }
      get onmessageerror() { return this.__thawOnMessageError; }
    }
    globalThis.MessagePort = MessagePort;
    globalThis.MessageChannel = class MessageChannel {
      constructor() {
        this.port1 = new MessagePort();
        this.port2 = new MessagePort();
        this.port1.__thawPeer = this.port2;
        this.port2.__thawPeer = this.port1;
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
        this.__thawOnMessage = null;
        this.__thawOnMessageError = null;
        this.__thawQueue = [];
        const channels = broadcastChannels.get(this.name) || new Set();
        channels.add(this);
        broadcastChannels.set(this.name, channels);
      }
      postMessage(value) {
        if (this.__thawClosed) throw new DOMException('BroadcastChannel is closed', 'InvalidStateError');
        const channels = broadcastChannels.get(this.name) || [];
        for (const channel of channels) {
          if (channel === this || channel.__thawClosed) continue;
          let copy;
          try { copy = structuredClone(value); }
          catch (error) {
            queueMicrotask(() => channel.__thawDispatchError(error));
            continue;
          }
          channel.__thawQueue.push({ data: copy });
          queueMicrotask(() => {
            if (channel.__thawClosed) return;
            const record = channel.__thawQueue.shift();
            if (!record) return;
            const event = new MessageEvent('message', { data: record.data });
            channel.dispatchEvent(event);
            if (typeof channel.__thawOnMessage === 'function') channel.__thawOnMessage.call(channel, event);
          });
        }
      }
      __thawDispatchError(error) {
        const event = new MessageEvent('messageerror', { data: error });
        this.dispatchEvent(event);
        if (typeof this.__thawOnMessageError === 'function') this.__thawOnMessageError.call(this, event);
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
      set onmessage(listener) { this.__thawOnMessage = listener; }
      get onmessage() { return this.__thawOnMessage; }
      set onmessageerror(listener) { this.__thawOnMessageError = listener; }
      get onmessageerror() { return this.__thawOnMessageError; }
    };
  }
