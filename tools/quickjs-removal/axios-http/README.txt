Manual end-to-end check of axios over the thaw-js net/zlib hosts (no network needed):
  node server.mjs > port.txt &                      # prints the listening port
  THAW_JS_RUNTIME=1 thaw build axioshttp.ts -o axioshttp.bin   (in a project with `thaw add axios`)
  AXIOS_PORT=$(cat port.txt) ./axioshttp.bin
Expected (matches Node exactly):
  200 hello world / 200 {"method":"POST","got":{"a":1}} / 200 {"z":"compressed"} / err true 404 nope

https (hermetic; the fixture CA in tls/ is injected into the TLS connect call because the compiled program has no CA option):
  node https-server.mjs > sport.txt &
  HTTPS_PORT=$(cat sport.txt) CA_HEX=$(od -An -v -tx1 tls/ca.pem | tr -d ' \n') \
    thaw-js-node --pkg axios=<axios bundle.js> axios-https.js      # -> 200 {"secure":true,"url":"/ping?x=1"}
