import axios from "axios";
async function main(): Promise<void> {
  const base = "http://127.0.0.1:" + process.env.AXIOS_PORT;
  const r = await axios.get(base + "/hello");
  console.log(r.status, r.data);
  const p = await axios.post(base + "/echo", { a: 1 });
  console.log(p.status, JSON.stringify(p.data));
  const g = await axios.get(base + "/gzip");
  console.log(g.status, JSON.stringify(g.data));
  try {
    await axios.get(base + "/missing");
  } catch (e: any) {
    console.log("err", axios.isAxiosError(e), e.response.status, e.response.data);
  }
}
