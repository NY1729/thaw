import { Hono } from "hono";
import { serve } from "@hono/node-server";
import { PrismaClient } from "@prisma/client";

const app = new Hono();
const prisma = new PrismaClient();

app.get("/health", (c) => c.json({ status: "ok" }));

app.get("/orders", async (c) => {
  const orders = await prisma.order.findMany({
    orderBy: { createdAt: "desc" },
  });
  return c.json({ orders });
});

app.post("/orders", async (c) => {
  const input = await c.req.json();
  const order = await prisma.order.create({
    data: {
      id: input.id,
      customer: input.customer,
      total: input.total,
    },
  });
  c.status(201);
  return c.json(order);
});

function main(): void {
  const port: number = Number(process.env.PORT) || 3030;
  serve({ fetch: app.fetch, port });
  console.log(`http://127.0.0.1:${port}`);
}
