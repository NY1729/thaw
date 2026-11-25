// Event-emitter based state machine (traffic light + order workflow).
import { EventEmitter, once } from 'node:events';

class Machine extends EventEmitter {
  #state;
  #transitions;
  constructor(initial, transitions) {
    super();
    this.#state = initial;
    this.#transitions = transitions;
  }
  get state() {
    return this.#state;
  }
  send(event) {
    const next = this.#transitions[this.#state]?.[event];
    if (!next) {
      this.emit('invalid', this.#state, event);
      return false;
    }
    const prev = this.#state;
    this.#state = next;
    this.emit('transition', prev, next, event);
    this.emit(`enter:${next}`);
    return true;
  }
}

const order = new Machine('new', {
  new: { pay: 'paid', cancel: 'cancelled' },
  paid: { ship: 'shipped', refund: 'cancelled' },
  shipped: { deliver: 'done' },
});
order.on('transition', (a, b, e) => console.log(`${a} --${e}--> ${b}`));
order.on('invalid', (s, e) => console.log(`invalid ${e} in ${s}`));
order.once('enter:done', () => console.log('finished!'));

const waiter = once(order, 'enter:shipped').then(([]) => console.log('shipped seen by once()'));
for (const e of ['ship', 'pay', 'ship', 'deliver', 'deliver', 'refund']) order.send(e);
await waiter;
console.log(order.state, order.listenerCount('transition'), order.eventNames());
try {
  order.emit('error', new Error('boom'));
} catch (e) {
  console.log('caught', e.message);
}
