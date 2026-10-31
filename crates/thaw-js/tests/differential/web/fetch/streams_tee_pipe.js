const rs = new ReadableStream({ start(c) { c.enqueue('a'); c.enqueue('b'); c.close(); } });
const [x, y] = rs.tee();
(async () => {
  const up = new TransformStream({ transform(ch, c) { c.enqueue(ch.toUpperCase()); } });
  const out = [];
  for await (const v of x.pipeThrough(up)) out.push(v);
  console.log(out.join());
  const r = y.getReader();
  console.log(JSON.stringify(await r.read()), JSON.stringify(await r.read()), JSON.stringify(await r.read()));
  const bytes = [];
  const enc = new ReadableStream({ start(c) { c.enqueue('é'); c.close(); } }).pipeThrough(new TextEncoderStream());
  for await (const v of enc) bytes.push(...v);
  console.log(bytes.join());
})();
