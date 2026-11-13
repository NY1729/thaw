const log = line => console.log(line);
const m = performance.mark('a', { startTime: 5, detail: { k: 1 } });
log([m.name, m.entryType, m.startTime, m.duration, JSON.stringify(m.detail)].join(' '));
log(JSON.stringify(m));
performance.mark('b', { startTime: 20 });
performance.mark('c', { startTime: 10 });
const ab = performance.measure('ab', 'a', 'b');
log([ab.name, ab.entryType, ab.startTime, ab.duration, ab.detail].join(' '));
const shapes = [
  performance.measure('s-start-end', { start: 1, end: 4 }),
  performance.measure('s-end-dur', { end: 20, duration: 5 }),
  performance.measure('s-start-dur', { start: 'a', duration: 2 }),
  performance.measure('s-names', 'c', 'b'),
  performance.measure('s-end-name', { end: 'c' }),
  performance.measure('s-detail', { start: 2, end: 3, detail: 'd' }),
  performance.measure('s-empty-opts', {}, 'b'),
];
for (const s of shapes) log([s.name, s.startTime, s.duration, JSON.stringify(s.detail)].join(' '));
log(performance.getEntriesByType('mark').map(e => e.name).join(','));
log(performance.getEntriesByType('measure').map(e => e.name).join(','));
log(performance.getEntriesByName('ab').length + ' ' + performance.getEntriesByName('a', 'measure').length + ' ' + performance.getEntriesByName('a', 'mark').length);
log(performance.getEntries().map(e => e.name).join(','));
performance.clearMarks('c');
log(performance.getEntriesByType('mark').map(e => e.name).join(','));
performance.clearMeasures('ab');
log(performance.getEntriesByType('measure').map(e => e.name).join(','));
performance.clearMeasures();
log(performance.getEntries().map(e => e.name).join(','));
performance.clearMarks();
log(performance.getEntries().length);
log(String(performance.mark('n', { startTime: 0 }).detail));
const d = { x: 1 };
const dm = performance.mark('dc', { startTime: 1, detail: d });
log((dm.detail !== d) + ' ' + dm.detail.x);
log(JSON.stringify(new PerformanceMark('pm', { startTime: 5, detail: 9 }).toJSON()));
log(String(performance.getEntries().length));
console.log(performance.mark('insp', { startTime: 5 })[Symbol.for('nodejs.util.inspect.custom')](2, {}));
console.log(performance.measure('mi', { start: 1, end: 4, detail: 'd' })[Symbol.for('nodejs.util.inspect.custom')](2, {}));
console.log(new PerformanceMark('nm', { startTime: 2 })[Symbol.for('nodejs.util.inspect.custom')](2, {}));
