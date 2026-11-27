import * as events from "node:events";
import * as os from "node:os";
import * as crypto from "node:crypto";
import * as url from "node:url";
import * as sd from "node:string_decoder";
import * as util from "node:util";

function t(label: string, f: () => any): void {
  try { console.log(label, "ok", String(f())); } catch (e: any) { console.log(label, e.name, e.code, e.message); }
}
const shapes: any[] = [["a"], new Date(0), { a: 1 }, 5, "s", null, undefined, 5n, () => 1];
for (let i = 0; i < shapes.length; i++) {
  const v: any = shapes[i];
  t("on" + i, () => new events.EventEmitter().on("x", v).listenerCount("x"));
  t("off" + i, () => new events.EventEmitter().off("x", v).listenerCount("x"));
  t("setMax" + i, () => new events.EventEmitter().setMaxListeners(v).getMaxListeners());
  t("getPriority" + i, () => typeof os.getPriority(v));
  t("randomBytes" + i, () => crypto.randomBytes(v).length);
  t("randomUUID" + i, () => crypto.randomUUID(v).length);
  t("update" + i, () => crypto.createHash("md5").update(v).digest("hex"));
  t("updateEnc" + i, () => crypto.createHash("md5").update("x", v).digest("hex"));
  t("hmac" + i, () => crypto.createHmac("sha256", "k").update(v).digest("hex"));
  t("styleText" + i, () => util.styleText("red", v));
  t("httpOpts" + i, () => url.urlToHttpOptions(v).pathname);
  t("format" + i, () => url.format(v));
  t("sdWrite" + i, () => new sd.StringDecoder("utf8").write(v));
}
t("sml-neg", () => new events.EventEmitter().setMaxListeners(-1).getMaxListeners());
t("rb-neg", () => crypto.randomBytes(-1).length);
const n = null;
t("byteLength-null", () => Buffer.byteLength(n));
const u = undefined;
t("byteLength-undefined", () => Buffer.byteLength(u));
