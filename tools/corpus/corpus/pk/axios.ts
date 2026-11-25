import { create, AxiosError } from "axios";
// No network: a canned adapter answers every request. Status >= 400 is thrown as AxiosError, like the real http adapter does.
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
const client = create({
  baseURL: "https://api.example.test",
  adapter: async (config: any) => {
    const isItems = String(config.url).startsWith("/items");
    const status = isItems ? 200 : 404;
    const data = config.method === "post" ? JSON.parse(config.data) : isItems ? [{ id: 1, name: "a" }] : { error: "nope" };
    const response = { data, status, statusText: status === 200 ? "OK" : "Not Found", headers: {}, config, request: {} };
    if (status >= 400) throw new AxiosError("Request failed with status code " + status, "ERR_BAD_REQUEST", config, null, response as any);
    return response;
  },
});
async function main(): Promise<void> {
  const r = await client.get("/items");
  line("status", r.status);
  line("data", r.data);
  try {
    await client.get("/missing");
  } catch (e: any) {
    line("err", [e.code, e.message, e.response?.status, e.response?.data]);
  }
  line("post", (await client.post("/items", { name: "b" })).data);
  // WORKLOAD: N=300 canned requests (estimate).
  const N = 300;
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const res = await client.get("/items?i=" + i);
    acc = (acc * 31 + JSON.stringify(res.data).length + (i % 7)) | 0;
  }
  line("checksum", acc);
}
