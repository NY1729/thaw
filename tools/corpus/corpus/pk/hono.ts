import { Hono } from "hono";
// app.request() dispatches in-memory: no sockets, no network.
const app = new Hono();
app.get("/hello/:name", (c) => c.text("hi " + c.req.param("name")));
app.get("/users/:id", (c) => c.json({ id: c.req.param("id"), role: "user" }));
app.post("/sum", async (c) => {
  const body = (await c.req.json()) as { a: number; b: number };
  return c.json({ total: body.a + body.b });
});
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
async function main(): Promise<void> {
  const r1 = await app.request("/hello/world");
  line("text", [r1.status, await r1.text()]);
  const r2 = await app.request("/users/42");
  line("json", [r2.status, await r2.json()]);
  const r3 = await app.request("/sum", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ a: 2, b: 40 }) });
  line("post", [r3.status, await r3.json()]);
  const r4 = await app.request("/missing");
  line("notFound", r4.status);
  // WORKLOAD: N=500 in-memory requests (estimate).
  const N = 500;
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const res = await app.request("/users/" + i);
    acc = (acc * 31 + (await res.text()).length) | 0;
  }
  line("checksum", acc);
}
