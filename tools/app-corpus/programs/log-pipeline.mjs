// Log processor: streams pipeline with a line-splitting Transform and an aggregating Writable.
import { Readable, Transform, Writable } from 'node:stream';
import { pipeline } from 'node:stream/promises';

const raw = Array.from({ length: 50 }, (_, i) => {
  const level = ['info', 'warn', 'error'][i % 3];
  return `2024-01-${String((i % 28) + 1).padStart(2, '0')} ${level} svc-${i % 4} took=${(i * 37) % 200}ms`;
}).join('\n') + '\n';

class LineSplit extends Transform {
  constructor() {
    super({ readableObjectMode: true });
    this.rest = '';
  }
  _transform(chunk, _enc, cb) {
    const parts = (this.rest + chunk.toString()).split('\n');
    this.rest = parts.pop();
    for (const p of parts) this.push(p);
    cb();
  }
  _flush(cb) {
    if (this.rest) this.push(this.rest);
    cb();
  }
}

const parse = new Transform({
  objectMode: true,
  transform(line, _e, cb) {
    const m = /^(\S+) (\w+) (\S+) took=(\d+)ms$/.exec(line);
    cb(null, m ? { date: m[1], level: m[2], svc: m[3], ms: Number(m[4]) } : undefined);
  },
});

const stats = {};
const sink = new Writable({
  objectMode: true,
  write(rec, _e, cb) {
    const s = (stats[rec.svc] ??= { n: 0, ms: 0, errors: 0 });
    s.n++;
    s.ms += rec.ms;
    if (rec.level === 'error') s.errors++;
    cb();
  },
});

// feed in odd-sized chunks so lines straddle chunk boundaries
const chunks = [];
for (let i = 0; i < raw.length; i += 13) chunks.push(raw.slice(i, i + 13));
await pipeline(Readable.from(chunks), new LineSplit(), parse, sink);
console.log(stats);
