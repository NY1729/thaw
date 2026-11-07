import { isEmail, isURL, isIP, isUUID, isISO8601 } from "validator";
// fixed input lists. N = 20k rounds of checks (estimate, see ms.ts).
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
const EMAILS = ["a@b.co", "bad@", "x.y+z@sub.example.org", "no at sign"];
const URLS = ["https://example.com/path?q=1", "ftp://x", "not a url", "http://localhost:8080"];
const IPS = ["127.0.0.1", "::1", "256.1.1.1", "2001:db8::1"];
async function main(): Promise<void> {
  const N = 1500;
  line("emails", EMAILS.map((e) => isEmail(e)));
  line("urls", URLS.map((u) => isURL(u)));
  line("ips", IPS.map((ip) => isIP(ip)));
  line("uuid", isUUID("6ba7b810-9dad-11d1-80b4-00c04fd430c8"));
  line("iso", isISO8601("2024-01-02T03:04:05Z"));
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const e = isEmail("user" + i + "@example.com") ? 1 : 0;
    const u = isURL("https://h" + (i % 50) + ".example.com/p") ? 2 : 0;
    acc = (acc * 31 + e + u + (isIP("10.0." + (i % 256) + ".1") ? 4 : 0)) | 0;
  }
  line("checksum", acc);
}
