import { createServer } from "node:http";

function main(): void {
  const port: number = Number(process.env.PORT) || 3000;
  const server = createServer((
    request: { method: string; url: string },
    response: {
      statusCode: number;
      setHeader: (name: string, value: string) => boolean;
      end: (body: string) => boolean;
    },
  ): boolean => {
    if (request.method === "GET" && request.url === "/api/message") {
      response.setHeader("Content-Type", "application/json; charset=utf-8");
      return response.end('{"message":"Hello from the Thaw API"}');
    }
    if (request.method === "GET" && thawHasAsset(request.url)) {
      response.setHeader("Content-Type", thawAssetContentType(request.url));
      return response.end(thawAsset(request.url));
    }
    response.statusCode = 404;
    return response.end("Not Found");
  });
  server.listen(port, (): void => {
    console.log(`http://127.0.0.1:${port}`);
  });
}
