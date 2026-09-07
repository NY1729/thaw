import { Hono } from "hono";
import { serve } from "@hono/node-server";
import sharp from "sharp";
import { Buffer } from "node:buffer";

const app = new Hono();

app.get("/health", (c) => c.json({ status: "ok" }));

app.post("/images", async (c) => {
  const input = Buffer.from(await c.req.arrayBuffer());
  const output = await sharp(input).resize(512, 512).webp().toBuffer();
  return c.body(output, 200, { "Content-Type": "image/webp" });
});

function main(): void {
  const port: number = Number(process.env.PORT) || 3030;
  serve({ fetch: app.fetch, port });
  console.log(`http://127.0.0.1:${port}`);
}
