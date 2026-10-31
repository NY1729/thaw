//! Host tests for the plain-TCP, zlib and misc hosts the registry's node:net / node:zlib shims call.
use super::*;

fn run_ok(src: &str) -> String {
    let mut it = Interp::new();
    it.run(src).unwrap_or_else(|_| panic!("script must run"));
    it.run_event_loop().unwrap_or_else(|_| panic!("event loop"));
    it.out.join("|")
}

#[test]
fn tcp_loopback_listen_connect_write_read_and_tls_stub() {
    let out = run_ok(
        r#"
        var l = __thaw_net_listen('127.0.0.1', 0).split(':'); // ok:<handle>:<port>
        var op = __thaw_net_connect_start('127.0.0.1', Number(l[2]));
        var conn;
        (function wait() {
          var r = __thaw_async_poll(op);
          if (r === 'pending') return setTimeout(wait, 1);
          conn = r.split(':'); // ok:<handle>:<ip>:<port>
          var acc;
          (function accept() {
            acc = __thaw_net_poll_accept(Number(l[1]));
            if (acc === 'err:pending') return setTimeout(accept, 1);
            acc = acc.split(':');
            console.log(__thaw_net_write(Number(conn[1]), Buffer.from('ping').toString('hex')));
            (function read() {
              var d = __thaw_net_poll_read(Number(acc[1]));
              if (d === 'pending') return setTimeout(read, 1);
              console.log(Buffer.from(d.slice(3), 'hex').toString());
              __thaw_net_destroy(Number(conn[1])); __thaw_net_destroy(Number(acc[1])); __thaw_net_close_listener(Number(l[1]));
            })();
          })();
        })();
        console.log(__thaw_net_connect('127.0.0.1', 70000));
        console.log(__thaw_tls_connect('h', 443).slice(0, 29));
        "#,
    );
    assert_eq!(
        out,
        "err:port should be >= 0 and < 65536. Received 70000|err:TLS support is not linked|ok|ping"
    );
}

#[test]
fn zlib_one_shot_and_async_jobs_round_trip() {
    let out = run_ok(
        r#"
        var hex = __thaw_zlib_hex('compress', 'gzip', Buffer.from('hello hello hello').toString('hex'));
        console.log(Buffer.from(__thaw_zlib_hex('decompress', 'gzip', hex), 'hex').toString());
        var br = __thaw_zlib_hex('compress', 'brotli', Buffer.from('brotli!').toString('hex'));
        console.log(Buffer.from(__thaw_zlib_hex('decompress', 'brotli', br), 'hex').toString());
        var job = __thaw_zlib_async_start('compress', 'deflate', new Uint8Array([1, 2, 3, 4]), '{"level":9}');
        (function poll() {
          var done = JSON.parse(__thaw_zlib_async_poll());
          if (!done.length) return setTimeout(poll, 1);
          var packed = new Uint8Array(__thaw_zlib_async_take(done[0].handle));
          var back = __thaw_zlib_hex('decompress', 'deflate', Buffer.from(packed).toString('hex'));
          console.log(done[0].ok, back);
        })();
        "#,
    );
    assert_eq!(out, "hello hello hello|brotli!|true 01020304");
}

#[test]
fn hpack_hash_and_timer_ref_hosts() {
    let out = run_ok(
        r#"
        var enc = __thaw_hpack_huffman_encode(Buffer.from('www.example.com').toString('hex'));
        console.log(enc, Buffer.from(__thaw_hpack_huffman_decode(enc), 'hex').toString());
        console.log(__thaw_crypto_hash_hex('sha256', Buffer.from('abc').toString('hex')).slice(0, 16));
        var t = __thaw_set_timeout_ref(function () { console.log('never'); }, 10000, false);
        __thaw_set_timer_ref(t, false);
        console.log(Object.prototype.toString.call(process));
        "#,
    );
    assert_eq!(
        out,
        "f1e3c2e5f23a6ba0ab90f4ff www.example.com|ba7816bf8f01cfea|[object process]"
    );
}
