Manual end-to-end check of axios over the thaw-js net/zlib hosts (no network needed):
  node server.mjs > port.txt &                      # prints the listening port
  THAW_JS_RUNTIME=1 thaw build axioshttp.ts -o axioshttp.bin   (in a project with `thaw add axios`)
  AXIOS_PORT=$(cat port.txt) ./axioshttp.bin
Expected (matches Node except the last line, see designs/quickjs-removal/slice5-hono-axios-cheerio.md):
  200 hello world / 200 {"method":"POST","got":{"a":1}} / 200 {"z":"compressed"} / err true 404 nope
