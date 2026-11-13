const log = [];
// case A: marks in sync code, observer observed in the same tick
const oa = new PerformanceObserver((list, obs) => log.push('A-cb:' + list.getEntries().map(e => e.name).join('|') + ' same=' + (obs === oa)));
oa.observe({ entryTypes: ['mark'] });
performance.mark('a1', { startTime: 1 });
performance.mark('a2', { startTime: 2 });
log.push('A-sync-end');
Promise.resolve().then(() => log.push('A-micro'));
setImmediate(() => log.push('A-immediate'));
// case B: buffered measure observer, takeRecords, disconnect
performance.mark('b1', { startTime: 3 });
performance.measure('bm', { start: 3, end: 4 });
const ob = new PerformanceObserver(list => log.push('B-cb ' + list.getEntries().map(e => e.entryType + ':' + e.name).join(',')));
ob.observe({ type: 'measure', buffered: true });
log.push('B-sync-take=' + ob.takeRecords().length);
ob.disconnect();
log.push('B-after-disconnect=' + ob.takeRecords().length);
// case C (setImmediate only, so no timer races the loop): two observers, dispatch order follows observe() order; later ticks queue again
const o1 = new PerformanceObserver(list => log.push('o1:' + list.getEntries().length));
const o2 = new PerformanceObserver(list => log.push('o2:' + list.getEntriesByType('measure').length + '/' + list.getEntries().length));
o1.observe({ type: 'mark' });
o2.observe({ entryTypes: ['mark', 'measure'] });
performance.mark('x', { startTime: 6 });
performance.mark('y', { startTime: 7 });
performance.measure('z', { start: 6, end: 7 });
setImmediate(() => {
  log.push('t1');
  performance.mark('later', { startTime: 8 });
  setImmediate(() => console.log(log.join(' ; ')));
});
