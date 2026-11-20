// ToString(object) propagates a throw from a user toString / valueOf / @@toPrimitive.
function probe(label, f) {
  try { console.log(label, 'ok', JSON.stringify(f())); }
  catch (e) { console.log(label, 'caught', e instanceof Error ? e.name + ':' + e.message : String(e)); }
}
probe('String', () => String({ toString() { throw new Error('a'); } }));
probe('template', () => `x${{ toString() { throw new Error('b'); } }}`);
probe('toPrimitive', () => `${{ [Symbol.toPrimitive]() { throw new Error('c'); } }}`);
probe('valueOf-fallback', () => String({ toString() { return {}; }, valueOf() { throw new Error('d'); } }));
probe('concat', () => '' + { toString() { throw new Error('e'); } });
probe('join', () => String([{ toString() { throw new Error('f'); } }]));
probe('nullproto', () => String(Object.create(null)));
probe('plain', () => String({}));
probe('template-plain', () => `${{}}`);
probe('custom', () => String({ toString() { return 'ok'; } }));
probe('nested-array', () => `a${[1, { toString() { return 'b'; } }]}`);
probe('array', () => String([1, [2, 3]]));
