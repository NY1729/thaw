import https from "node:https";
import fs from "node:fs";
const dir = new URL("./tls/", import.meta.url);
const s = https.createServer({ cert: fs.readFileSync(new URL("cert.pem", dir)), key: fs.readFileSync(new URL("key.pem", dir)) }, (req, res) => {
  res.setHeader("content-type", "application/json");
  res.end(JSON.stringify({ secure: req.socket.encrypted === true, url: req.url }));
});
s.listen(0, "127.0.0.1", () => console.log(s.address().port));
setTimeout(() => process.exit(0), 60000).unref();
