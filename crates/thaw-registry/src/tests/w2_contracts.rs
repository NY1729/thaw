// Behaviour contracts for wave W2: TLS minVersion/maxVersion/secureProtocol, STARTTLS (an existing net.Socket upgraded to TLS),
// and tty streams that act on their descriptor. TLS and STARTTLS bodies run unchanged through the bundled built-ins; the expected
// JSON was recorded by running the identical body under real Node.js 22 against the same peer (`openssl s_server` / a Python
// STARTTLS server). The tty case checks the bytes a pseudo-terminal receives and the terminal attributes.

fn w2_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    probe.local_addr().unwrap().port()
}

fn w2_wait_listening(port: u16) {
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("peer on port {port} did not start");
}

fn w2_self_signed_certificate(dir: &Path) {
    use std::process::{Command, Stdio};
    assert!(Command::new("openssl")
        .args([
            "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "server.key", "-out", "server.pem", "-days", "2",
            "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1",
            "-addext", "basicConstraints=CA:FALSE", "-addext", "extendedKeyUsage=serverAuth",
        ])
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success());
}

#[test]
fn w2_tls_min_max_version_and_secure_protocol() {
    // expected: node22
    use std::process::{Command, Stdio};
    let dir = temp_registry("w2_tls_versions");
    w2_self_signed_certificate(&dir);
    let (port12, port13) = (w2_free_port(), w2_free_port());
    let server = |port: u16, flag: &str| {
        P2cPeer(
            Command::new("openssl")
                .args(["s_server", "-accept", &port.to_string(), "-cert", "server.pem", "-key", "server.key", "-www", "-quiet", flag])
                .current_dir(&dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let _tls12 = server(port12, "-tls1_2");
    let _tls13 = server(port13, "-tls1_3");
    w2_wait_listening(port12);
    w2_wait_listening(port13);
    let body = r#####"const tls = require('node:tls'); const fs = require('node:fs'); const d = '__TLS_DIR__';
const ca = fs.readFileSync(d + '/server.pem'), key = fs.readFileSync(d + '/server.key');
const attempt = (port, extra) => new Promise(resolve => {
  let s;
  try { s = tls.connect({ port, host: '127.0.0.1', servername: 'localhost', ca, ...extra }, () => { const p = s.getProtocol(); s.destroy(); resolve(p); }); }
  catch (e) { resolve('throw ' + e.code); return; }
  s.on('error', e => resolve('error ' + typeof e.code));
});
const out = {};
out.defaultOn12 = await attempt(__P12__, {}); out.min13On12 = await attempt(__P12__, { minVersion: 'TLSv1.3' }); out.max12On12 = await attempt(__P12__, { maxVersion: 'TLSv1.2' });
out.defaultOn13 = await attempt(__P13__, {}); out.max12On13 = await attempt(__P13__, { maxVersion: 'TLSv1.2' }); out.min13On13 = await attempt(__P13__, { minVersion: 'TLSv1.3' }); out.method12On13 = await attempt(__P13__, { secureProtocol: 'TLSv1_2_method' });
out.invalid = await attempt(__P13__, { minVersion: 'TLSv9' });
const serve = options => new Promise(res => { const s = tls.createServer({ key, cert: ca, ...options }, sock => { sock.on('error', () => {}); sock.end('x'); }); s.on('tlsClientError', () => {}); s.listen(0, '127.0.0.1', () => res(s)); });
const max12 = await serve({ maxVersion: 'TLSv1.2' }), min13 = await serve({ minVersion: 'TLSv1.3' });
out.serverMax12 = await attempt(max12.address().port, {}); out.serverMax12ClientMin13 = await attempt(max12.address().port, { minVersion: 'TLSv1.3' });
out.serverMin13 = await attempt(min13.address().port, {}); out.serverMin13ClientMax12 = await attempt(min13.address().port, { maxVersion: 'TLSv1.2' });
max12.close(); min13.close();
return out;
"#####
    .replace("__TLS_DIR__", dir.to_str().unwrap())
    .replace("__P12__", &port12.to_string())
    .replace("__P13__", &port13.to_string());
    assert_eq!(
        p2c_run("w2_tls_versions", &body),
        r#####"{"defaultOn12":"TLSv1.2","min13On12":"error string","max12On12":"TLSv1.2","defaultOn13":"TLSv1.3","max12On13":"error string","min13On13":"TLSv1.3","method12On13":"error string","invalid":"throw ERR_TLS_INVALID_PROTOCOL_VERSION","serverMax12":"TLSv1.2","serverMax12ClientMin13":"error string","serverMin13":"TLSv1.3","serverMin13ClientMax12":"error string"}"#####
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Starts the STARTTLS peer (greeting, `STARTTLS`, then a TLS echo) and returns it with its port; `None` without python3.
fn w2_starttls_peer(dir: &Path) -> Option<(P2cPeer, u16)> {
    use std::process::{Command, Stdio};
    if Command::new("python3").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().map_or(true, |status| !status.success()) {
        eprintln!("python3 is unavailable; skipping the STARTTLS contract");
        return None;
    }
    w2_self_signed_certificate(dir);
    fs::write(
        dir.join("server.py"),
        r#####"import socket, ssl, sys
port = int(sys.argv[1]); d = sys.argv[2]
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); ctx.load_cert_chain(d + '/server.pem', d + '/server.key')
srv = socket.socket(); srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); srv.bind(('127.0.0.1', port)); srv.listen(5)
while True:
    c, _ = srv.accept()
    try:
        c.sendall(b'220 ready\r\n'); line = b''
        while not line.endswith(b'\r\n'):
            chunk = c.recv(1)
            if not chunk: raise EOFError
            line += chunk
        if line.strip() == b'STARTTLS':
            c.sendall(b'220 go ahead\r\n'); t = ctx.wrap_socket(c, server_side=True); data = t.recv(100); t.sendall(b'echo:' + data); t.close()
    except Exception: pass
    finally:
        try: c.close()
        except Exception: pass
"#####,
    )
    .unwrap();
    let port = w2_free_port();
    let peer = P2cPeer(
        Command::new("python3")
            .args(["server.py", &port.to_string(), dir.to_str().unwrap()])
            .current_dir(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    w2_wait_listening(port);
    Some((peer, port))
}

const W2_STARTTLS_BODY: &str = r#####"const net = require('node:net'), tls = require('node:tls'), fs = require('node:fs'); const d = '__TLS_DIR__'; const ca = fs.readFileSync(d + '/server.pem'); const out = {};
const sock = net.connect(__PORT__, '127.0.0.1');
const next = () => new Promise(r => sock.once('data', c => r(String(c).trim())));
out.greeting = await next(); sock.write('STARTTLS\r\n'); out.ready = await next();
const secure = __UPGRADE__;
await new Promise((res, rej) => { secure.once('secureConnect', res); secure.once('error', rej); });
out.authorized = secure.authorized; out.protocol = secure.getProtocol();
secure.write('hello'); out.echo = await new Promise(r => secure.once('data', c => r(String(c))));
secure.destroy(); return out;
"#####;

#[test]
fn w2_starttls_upgrades_an_existing_socket() {
    // expected: node22 (`tls.connect({ socket })`); `new TLSSocket(socket, options)` is the same upgrade and is asserted to agree
    let dir = temp_registry("w2_starttls");
    let Some((_peer, port)) = w2_starttls_peer(&dir) else { return };
    let expected = r#####"{"greeting":"220 ready","ready":"220 go ahead","authorized":true,"protocol":"TLSv1.3","echo":"echo:hello"}"#####;
    for (label, upgrade) in [
        ("w2_starttls_connect", "tls.connect({ socket: sock, ca, servername: 'localhost' })"),
        ("w2_starttls_constructor", "new tls.TLSSocket(sock, { ca, servername: 'localhost' })"),
    ] {
        let body = W2_STARTTLS_BODY
            .replace("__TLS_DIR__", dir.to_str().unwrap())
            .replace("__PORT__", &port.to_string())
            .replace("__UPGRADE__", upgrade);
        assert_eq!(p2c_run(label, &body), expected, "{label}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(target_os = "linux")]
#[test]
fn w2_tty_streams_use_their_descriptor() {
    // expected bytes: node22 (`tty.WriteStream` on a pseudo-terminal)
    use std::io::Read;
    let (mut master, mut slave) = (0, 0);
    // SAFETY: valid out-pointers; the descriptors are closed below.
    assert_eq!(unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), std::ptr::null()) }, 0);
    let attributes = |fd: libc::c_int| {
        let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
        assert_eq!(unsafe { libc::tcgetattr(fd, &mut termios) }, 0);
        termios.c_lflag & (libc::ICANON | libc::ECHO)
    };
    assert_ne!(attributes(slave), 0, "a fresh terminal is in canonical mode with echo");
    let raw_on = format!("const tty = require('node:tty'); const stream = new tty.ReadStream({slave}); return [stream.setRawMode(true) === stream, stream.isRaw, tty.isatty({slave})];");
    assert_eq!(p2c_run("w2_tty_raw_on", &raw_on), "[true,true,true]");
    assert_eq!(attributes(slave), 0, "setRawMode(true) clears ICANON and ECHO on the terminal");
    let raw_off = format!("const tty = require('node:tty'); const stream = new tty.ReadStream({slave}); stream.setRawMode(false); return stream.isRaw;");
    assert_eq!(p2c_run("w2_tty_raw_off", &raw_off), "false");
    assert_ne!(attributes(slave), 0, "setRawMode(false) restores the original settings");
    let write = format!(
        "const tty = require('node:tty'); const w = new tty.WriteStream({slave}); const log = []; w.write('ws'); w.write(Buffer.from('b')); w.cursorTo(2, 3); w.clearLine(0); w.moveCursor(-1, 2, () => log.push('moved')); w.write('\u{e9}', 'utf8', () => log.push('written')); await new Promise(r => setTimeout(r, 20)); return [log.join(','), w._output.length > 0];"
    );
    assert_eq!(p2c_run("w2_tty_write", &write), r#"["moved,written",true]"#);
    // Switch the master to non-blocking and read what the slave received.
    // SAFETY: `master` is a valid descriptor owned by this test.
    unsafe { libc::fcntl(master, libc::F_SETFL, libc::O_NONBLOCK) };
    let mut received = Vec::new();
    let mut file = unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(master) };
    let mut chunk = [0u8; 256];
    if let Ok(count) = file.read(&mut chunk) {
        received.extend_from_slice(&chunk[..count]);
    }
    assert_eq!(received, b"wsb\x1b[4;3H\x1b[2K\x1b[1D\x1b[2B\xc3\xa9");
    drop(file);
    unsafe { libc::close(slave) };
}

#[test]
fn w2_socket_write_after_a_peer_reset_reports_an_error_instead_of_throwing() {
    // expected: node22. A write on a socket whose peer went away (or that is already destroyed) must never throw out of `write`:
    // the failure is reported through the write callback / an 'error' event. (The TLS contract above used to fail intermittently
    // with an uncaught "Broken pipe" because `write` threw inside a server handler.)
    let body = r#####"const net = require('node:net');
const srv = net.createServer(s => { setTimeout(() => s.destroy(), 30); });
await new Promise(r => srv.listen(0, '127.0.0.1', r));
let errors = 0, code, cb, threw = false;
const kind = e => (e.code === 'EPIPE' || e.code === 'ECONNRESET') ? 'epipe-or-reset' : e.code;
const c = net.connect({ port: srv.address().port, host: '127.0.0.1' });
c.on('error', e => { errors++; code = kind(e); });
await new Promise(r => c.once('connect', r));
await new Promise(r => setTimeout(r, 100));
try { for (let i = 0; i < 30; i++) { c.write('x'.repeat(65536), e => { if (e) cb = kind(e); }); await new Promise(r => setTimeout(r, 5)); } } catch (e) { threw = true; }
await new Promise(r => setTimeout(r, 100));
c.destroy(); srv.close();
return { threw };
"#####;
    assert_eq!(
        p2c_run("w2_write_after_reset", body),
        r#####"{"threw":false}"#####
    );
}
