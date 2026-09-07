import { Hono } from "hono";
import { serve } from "@hono/node-server";
import { PrismaClient } from "@prisma/client";

const app = new Hono();
const prisma = new PrismaClient();

app.get("/api/posts", async (c) => {
  const posts = await prisma.post.findMany({ orderBy: { createdAt: "desc" } });
  return c.json({ posts });
});

app.post("/api/posts", async (c) => {
  const input = await c.req.json();
  if (!input.author || !input.message) {
    c.status(400);
    return c.json({ error: "author and message are required" });
  }
  const post = await prisma.post.create({
    data: { id: crypto.randomUUID(), author: input.author, message: input.message },
  });
  c.status(201);
  return c.json(post);
});

app.get("*", (c) => {
  const path = thawAssetRoute(c.req.path);
  if (!thawHasAsset(path)) {
    c.status(404);
    return c.text("Not Found");
  }
  c.header("Content-Type", thawAssetContentType(path));
  return c.body(thawAsset(path));
});

function main(): void {
  const port: number = Number(process.env.PORT) || 3030;
  serve({ fetch: app.fetch, port });
  console.log(`http://127.0.0.1:${port}`);
}
