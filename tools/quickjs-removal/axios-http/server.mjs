import http from "node:http";
import zlib from "node:zlib";
const s = http.createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    if (req.url === "/hello") { res.setHeader("content-type", "text/plain"); res.end("hello world"); }
    else if (req.url === "/echo") { res.setHeader("content-type", "application/json"); res.end(JSON.stringify({ method: req.method, got: JSON.parse(body || "null") })); }
    else if (req.url === "/gzip") { res.setHeader("content-type", "application/json"); res.setHeader("content-encoding", "gzip"); res.end(zlib.gzipSync(JSON.stringify({ z: "compressed" }))); }
    else { res.statusCode = 404; res.end("nope"); }
  });
});
s.listen(0, "127.0.0.1", () => { console.log(s.address().port); });
setTimeout(() => process.exit(0), 60000).unref();
