// MessageEvent: data/origin/lastEventId/source/ports are prototype getters; initMessageEvent returns a new event.
const fn = f => (f ? `${f.name}/${f.length}` : '-');
for (const k of ['data', 'origin', 'lastEventId', 'source', 'ports']) {
  const d = Object.getOwnPropertyDescriptor(MessageEvent.prototype, k);
  console.log(k, fn(d.get), d.set, d.enumerable, d.configurable);
}
const init = Object.getOwnPropertyDescriptor(MessageEvent.prototype, 'initMessageEvent');
console.log('initMessageEvent', fn(init.value), init.writable, init.enumerable, init.configurable);
const m = new MessageEvent('message', { data: { v: 1 }, origin: 'https://o', lastEventId: 'id', ports: [] });
console.log(Object.keys(m).length, m.data.v, m.origin, m.lastEventId, m.source, m.ports.length, m.type, m.bubbles);
const n = m.initMessageEvent('mm', true, true, 'd', 'o2', 'l2', null, []);
console.log(m.type, m.data.v, m.bubbles, '|', n.type, n.data, n.origin, n.lastEventId, n.bubbles, n.cancelable, n instanceof MessageEvent);
const bare = new MessageEvent('message', { data: 1 }).initMessageEvent('nn');
console.log(bare.type, bare.bubbles, bare.cancelable, bare.data, JSON.stringify(bare.origin), bare.source, bare.ports.length);
try { m.initMessageEvent(); } catch (err) { console.log('no args', err instanceof TypeError); }
try { Object.getOwnPropertyDescriptor(MessageEvent.prototype, 'data').get.call(new Event('x')); }
catch (err) { console.log('brand', err instanceof TypeError); }
const port = new MessageChannel();
port.port2.addEventListener('message', ev => { console.log('port event', ev.data, ev.origin, ev.ports.length); port.port1.close(); });
port.port2.start();
port.port1.postMessage('hi');
