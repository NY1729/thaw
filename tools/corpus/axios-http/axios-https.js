// thaw-js-node --pkg axios=<axios bundle.js> axios-https.js <ca.pem hex via CA_HEX> -- axios GET over TLS (hermetic: the
// test CA is injected into the TLS connect host call, since the compiled program has no CA option for axios).
const connect = globalThis.__thaw_tls_connect_start;
globalThis.__thaw_tls_connect_start = (host, port, name, ca, cert, key, options) => connect(host, port, name, process.env.CA_HEX, cert, key, options);
const axios = require('axios');
axios.get('https://127.0.0.1:' + process.env.HTTPS_PORT + '/ping?x=1').then(
  (r) => console.log(r.status, JSON.stringify(r.data)),
  (e) => console.log('FAIL', e.message, e.code));
