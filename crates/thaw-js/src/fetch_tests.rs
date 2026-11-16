//! Host tests for the global `fetch` on thaw-js (Q9 plan-q9-thawjs-fetch.md, test 1).
//!
//! Scenario tests for the thaw-js realm `fetch` (Q9 plan-q9-thawjs-fetch.md, test 1). Each expected line is copied
//! from the node oracle `tools/parity/routed/fetch.expected` (same scenario, same output lines). Scenarios that
//! need the HTTP transport are `#[ignore]`d: the transport is the node:http bundle, which only a loaded bundle
//! provides; run them with `cargo test -p thaw-js fetch_ -- --ignored` once a bundle is loaded in the realm.
use super::*;

/// A loopback HTTP/1.1 server over the `__thaw_net_*` natives. `fx.listen(fn)` calls `fn(req)` per request;
/// `req` has `method`, `path`, `body`. `fn` returns a Buffer (the wire bytes, then the connection is shut
/// and closed) or `[buffer, true]` to write and keep the connection open.
const FIXTURE: &str = r#"
var fx = {};
fx.listen = function (respond) {
  var l = __thaw_net_listen('127.0.0.1', 0).split(':');
  fx.listener = Number(l[1]);
  fx.port = Number(l[2]);
  function accept() {
    var acc = __thaw_net_poll_accept(fx.listener);
    if (acc === 'err:pending') return setTimeout(accept, 1);
    if (acc.indexOf('ok:') !== 0) return;
    serve(Number(acc.split(':')[1]));
    accept();
  }
  function serve(conn) {
    var buf = '';
    (function read() {
      var d = __thaw_net_poll_read(conn);
      if (d === 'pending') return setTimeout(read, 1);
      if (d.indexOf('err:') === 0) return;
      if (d.indexOf('ok:') === 0) buf += Buffer.from(d.slice(3), 'hex').toString();
      var end = buf.indexOf('\r\n\r\n');
      if (end < 0) return d === 'eof' ? undefined : read();
      var head = buf.slice(0, end);
      var m = /content-length:\s*(\d+)/i.exec(head);
      var need = end + 4 + (m ? Number(m[1]) : 0);
      if (buf.length < need && d !== 'eof') return read();
      var first = head.split(' ');
      var out = respond({ method: first[0], path: first[1], body: buf.slice(end + 4, need) });
      __thaw_net_write(conn, out[0].toString('hex'));
      if (out[1]) return;
      __thaw_net_shutdown(conn);
      __thaw_net_destroy(conn);
    })();
  }
  accept();
};
fx.text = function (wire) { return [Buffer.from(wire), false]; };
"#;

fn fetch_lines(scenario: &str) -> Vec<String> {
    let mut it = Interp::new();
    let source = format!("{FIXTURE}\n{scenario}");
    it.run(&source)
        .unwrap_or_else(|e| panic!("script must run: {e:?}"));
    it.run_event_loop()
        .unwrap_or_else(|e| panic!("event loop: {e:?}"));
    it.out.clone()
}

#[test]
fn fetch_fixture_answers_a_raw_loopback_request() {
    let out = fetch_lines(
        r#"
        fx.listen(function (req) {
          return fx.text('HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello');
        });
        var c = __thaw_net_connect('127.0.0.1', fx.port).split(':');
        __thaw_net_write(Number(c[1]), Buffer.from('GET / HTTP/1.1\r\nHost: x\r\n\r\n').toString('hex'));
        var got = '';
        (function read() {
          var d = __thaw_net_poll_read(Number(c[1]));
          if (d === 'pending') return setTimeout(read, 1);
          if (d.indexOf('ok:') === 0) { got += Buffer.from(d.slice(3), 'hex').toString(); return read(); }
          console.log(got.slice(got.indexOf('\r\n\r\n') + 4));
          __thaw_net_destroy(Number(c[1]));
          __thaw_net_close_listener(fx.listener);
        })();
        "#,
    );
    assert_eq!(out, vec!["hello".to_string()]);
}

#[test]
fn fetch_global_is_a_function() {
    let out = fetch_lines(
        r#"
        (async function () {
          console.log(typeof fetch);
        })();
        "#,
    );
    assert_eq!(out, vec!["function".to_string()]);
}

#[test]
#[ignore = "needs a node:http client loaded in the realm (bundle hook); a bare Interp has none. Routed run: tools/parity/routed/fetch.js"]
fn fetch_get_returns_status_headers_and_text() {
    let out = fetch_lines(
        r#"
        fx.listen(function (req) {
          return fx.text('HTTP/1.1 200 OK\r\nx-a: b\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello');
        });
        (async function () {
          var r = await fetch('http://127.0.0.1:' + fx.port + '/plain');
          console.log(r.status, r.ok, r.statusText, r.headers.get('x-a'), r.redirected, r.type, new URL(r.url).pathname);
          console.log(JSON.stringify(await r.text()));
          __thaw_net_close_listener(fx.listener);
        })();
        "#,
    );
    assert_eq!(
        out,
        vec![
            "200 true OK b false basic /plain".to_string(),
            "\"hello\"".to_string()
        ]
    );
}

#[test]
#[ignore = "needs a node:http client loaded in the realm (bundle hook); a bare Interp has none. Routed run: tools/parity/routed/fetch.js"]
fn fetch_post_sends_the_request_body() {
    let out = fetch_lines(
        r#"
        fx.listen(function (req) {
          var body = req.method + ':' + req.body;
          return fx.text('HTTP/1.1 200 OK\r\nContent-Length: ' + body.length + '\r\nConnection: close\r\n\r\n' + body);
        });
        (async function () {
          var r = await fetch('http://127.0.0.1:' + fx.port + '/echo', { method: 'POST', body: 'payload' });
          console.log(await r.text());
          __thaw_net_close_listener(fx.listener);
        })();
        "#,
    );
    assert_eq!(out, vec!["POST:payload".to_string()]);
}

#[test]
#[ignore = "needs a node:http client loaded in the realm (bundle hook); a bare Interp has none. Routed run: tools/parity/routed/fetch.js"]
fn fetch_follows_302_and_sets_redirected_and_url() {
    let out = fetch_lines(
        r#"
        fx.listen(function (req) {
          if (req.path === '/r302') return fx.text('HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n');
          return fx.text('HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nfinal');
        });
        (async function () {
          var r = await fetch('http://127.0.0.1:' + fx.port + '/r302');
          console.log(r.status, r.redirected, new URL(r.url).pathname, await r.text());
          __thaw_net_close_listener(fx.listener);
        })();
        "#,
    );
    assert_eq!(out, vec!["200 true /final final".to_string()]);
}

#[test]
#[ignore = "needs a node:http client loaded in the realm (bundle hook); a bare Interp has none. Routed run: tools/parity/routed/fetch.js"]
fn fetch_redirect_manual_returns_the_302() {
    let out = fetch_lines(
        r#"
        fx.listen(function (req) {
          return fx.text('HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n');
        });
        (async function () {
          var r = await fetch('http://127.0.0.1:' + fx.port + '/r302', { redirect: 'manual' });
          console.log(r.status, r.redirected, r.headers.get('location'));
          __thaw_net_close_listener(fx.listener);
        })();
        "#,
    );
    assert_eq!(out, vec!["302 false /final".to_string()]);
}

#[test]
// Note: without a transport the fetch rejects with a TypeError, so this expectation passes by accident here. Re-check
// the redirect rejection (not the missing client) once a bundle is loaded.
#[ignore = "needs a node:http client loaded in the realm (bundle hook); a bare Interp has none. Routed run: tools/parity/routed/fetch.js"]
fn fetch_redirect_error_rejects_with_type_error() {
    let out = fetch_lines(
        r#"
        fx.listen(function (req) {
          return fx.text('HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n');
        });
        (async function () {
          try {
            await fetch('http://127.0.0.1:' + fx.port + '/r302', { redirect: 'error' });
            console.log('no error');
          } catch (e) {
            console.log(e.name);
          }
          __thaw_net_close_listener(fx.listener);
        })();
        "#,
    );
    assert_eq!(out, vec!["TypeError".to_string()]);
}

#[test]
#[ignore = "needs a node:http client loaded in the realm (bundle hook); a bare Interp has none. Routed run: tools/parity/routed/fetch.js"]
fn fetch_gzip_body_is_decoded() {
    let out = fetch_lines(
        r#"
        fx.listen(function (req) {
          var gz = __thaw_zlib_hex('compress', 'gzip', Buffer.from('gzipped body').toString('hex'));
          var head = 'HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: ' + (gz.length / 2) + '\r\nConnection: close\r\n\r\n';
          return [Buffer.concat([Buffer.from(head), Buffer.from(gz, 'hex')]), false];
        });
        (async function () {
          var r = await fetch('http://127.0.0.1:' + fx.port + '/gzip');
          console.log(await r.text());
          __thaw_net_close_listener(fx.listener);
        })();
        "#,
    );
    assert_eq!(out, vec!["gzipped body".to_string()]);
}

#[test]
#[ignore = "needs a node:http client loaded in the realm (bundle hook); a bare Interp has none. Routed run: tools/parity/routed/fetch.js"]
fn fetch_chunked_body_is_read() {
    let out = fetch_lines(
        r#"
        fx.listen(function (req) {
          return fx.text('HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n9\r\nchunk-one,\r\n9\r\nchunk-two\r\n0\r\n\r\n');
        });
        (async function () {
          var r = await fetch('http://127.0.0.1:' + fx.port + '/chunked');
          console.log(await r.text());
          __thaw_net_close_listener(fx.listener);
        })();
        "#,
    );
    assert_eq!(out, vec!["chunk-one,chunk-two".to_string()]);
}

#[test]
#[ignore = "needs a node:http client loaded in the realm (bundle hook); a bare Interp has none. Routed run: tools/parity/routed/fetch.js"]
fn fetch_abort_mid_body_rejects_with_abort_error() {
    let out = fetch_lines(
        r#"
        fx.listen(function (req) {
          return [fx.text('HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\npart')[0], true];
        });
        (async function () {
          var ac = new AbortController();
          var r = await fetch('http://127.0.0.1:' + fx.port + '/slow', { signal: ac.signal });
          setTimeout(function () { ac.abort(); }, 20);
          try {
            await r.text();
            console.log('no abort');
          } catch (e) {
            console.log(e.name);
          }
          __thaw_net_close_listener(fx.listener);
        })();
        "#,
    );
    assert_eq!(out, vec!["AbortError".to_string()]);
}

#[test]
fn fetch_already_aborted_signal_rejects_with_abort_error() {
    let out = fetch_lines(
        r#"
        (async function () {
          try {
            await fetch('http://127.0.0.1:1/plain', { signal: AbortSignal.abort() });
            console.log('no abort');
          } catch (e) {
            console.log(e.name);
          }
        })();
        "#,
    );
    assert_eq!(out, vec!["AbortError".to_string()]);
}

#[test]
#[ignore = "needs a node:http client loaded in the realm (bundle hook); a bare Interp has none. Routed run: tools/parity/routed/fetch.js"]
fn fetch_connection_refused_is_fetch_failed() {
    let out = fetch_lines(
        r#"
        var l = __thaw_net_listen('127.0.0.1', 0).split(':');
        var port = Number(l[2]);
        __thaw_net_close_listener(Number(l[1]));
        (async function () {
          try {
            await fetch('http://127.0.0.1:' + port + '/');
            console.log('no error');
          } catch (e) {
            console.log(e.name, e.message, e.cause && e.cause.code);
          }
        })();
        "#,
    );
    assert_eq!(out, vec!["TypeError fetch failed ECONNREFUSED".to_string()]);
}
