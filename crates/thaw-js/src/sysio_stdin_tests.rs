//! Main-thread `process.stdin` plumbing in sysio.rs: the reader thread and terminal raw mode.

use std::time::{Duration, Instant};

use crate::sysio::{poll_stdin, set_fd_raw_mode, start_stdin_fd};
use crate::Interp;

#[cfg(unix)]
#[test]
fn stdin_reader_delivers_data_then_end() {
    let mut fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let (read_end, write_end) = (fds[0], fds[1]);
    let text = b"Ada\n";
    assert_eq!(
        unsafe { libc::write(write_end, text.as_ptr().cast(), text.len()) },
        text.len() as isize
    );
    unsafe { libc::close(write_end) };
    start_stdin_fd(read_end);
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut seen = String::new();
    while !seen.contains(r#"{"type":"end"}"#) {
        assert!(Instant::now() < deadline, "no end event; saw {seen}");
        seen.push_str(&poll_stdin());
        std::thread::sleep(Duration::from_millis(1));
    }
    let data = seen.find(r#"{"type":"data","value":"4164610a"}"#);
    assert!(data.is_some(), "{seen}");
    assert!(data < seen.find(r#"{"type":"end"}"#), "{seen}");
    assert_eq!(poll_stdin(), "[]");
    unsafe { libc::close(read_end) };
}

#[cfg(unix)]
#[test]
fn raw_mode_clears_icanon_and_restores() {
    let (mut master, mut slave) = (0i32, 0i32);
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        },
        0
    );
    let lflag = |fd: i32| {
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::tcgetattr(fd, &mut t) }, 0);
        t.c_lflag
    };
    let original = lflag(slave);
    assert_ne!(original & libc::ICANON, 0, "a fresh pty is canonical");
    assert!(set_fd_raw_mode(slave, true));
    let raw = lflag(slave);
    assert_eq!(raw & libc::ICANON, 0);
    assert_eq!(raw & libc::ISIG, 0);
    assert!(set_fd_raw_mode(slave, false));
    assert_eq!(lflag(slave), original);
    unsafe {
        libc::close(master);
        libc::close(slave);
    }
}

/// Runs `body` on its own thread and fails, instead of hanging the whole test binary, if it takes over 5s.
#[cfg(unix)]
fn guarded<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let value = body();
        let _ = done.send(value);
    });
    match finished.recv_timeout(Duration::from_secs(5)) {
        Ok(value) => value,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!("hung past 5s"),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => panic!("body panicked"),
    }
}

#[cfg(unix)]
fn pipe() -> (i32, i32) {
    let mut fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    (fds[0], fds[1])
}

#[cfg(unix)]
fn write_all(fd: i32, bytes: &[u8]) {
    assert_eq!(
        unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) },
        bytes.len() as isize
    );
}

#[test]
fn stdout_write_calls_its_trailing_callback_after_the_sync_code() {
    let mut it = Interp::new();
    let src = r#"
        let order = [];
        process.stdout.write('x\n', () => order.push('cb1'));
        process.stdout.write('y\n', 'utf8', () => order.push('cb2'));
        process.stdout.write('z\n', undefined);
        order.push('sync');
        process.on('exit', () => console.log('order=' + order.join(',')));
    "#;
    it.run_main(src)
        .unwrap_or_else(|_| panic!("script must run"));
    let out = it.out.join("|");
    assert!(out.contains("order=sync,cb1,cb2"), "{out}");
}

#[cfg(unix)]
#[test]
fn unref_stdin_lets_the_loop_end() {
    guarded(|| {
        let (read_end, write_end) = pipe();
        start_stdin_fd(read_end);
        let mut it = Interp::new();
        it.rt.stdin_started = true;
        assert!(
            it.work_pending(),
            "an open, referenced stdin keeps the loop alive"
        );
        it.run("process.stdin.unref()")
            .unwrap_or_else(|_| panic!("unref must run"));
        assert!(
            !it.work_pending(),
            "an unref'd stdin does not keep the loop alive"
        );
        // closing the write end lets the reader thread reach EOF and exit
        unsafe { libc::close(write_end) };
    });
}

#[cfg(unix)]
#[test]
fn stdin_eof_ends_the_loop() {
    let out = guarded(|| {
        let (read_end, write_end) = pipe();
        write_all(write_end, b"hi\n");
        unsafe { libc::close(write_end) };
        start_stdin_fd(read_end);
        let mut it = Interp::new();
        it.run(
            "process.stdin.on('data', d => console.log('d=' + d)); \
             process.stdin.on('end', () => console.log('end'));",
        )
        .unwrap_or_else(|_| panic!("script must run"));
        it.run_event_loop()
            .unwrap_or_else(|_| panic!("event loop must finish"));
        it.out.join("|")
    });
    let data = out.find("d=hi").unwrap_or_else(|| panic!("{out}"));
    assert!(
        data < out.find("end").unwrap_or_else(|| panic!("{out}")),
        "{out}"
    );
}

#[cfg(unix)]
#[test]
fn stdin_data_callback_then_stdout_callback_unref_ends_the_loop() {
    guarded(|| {
        let (read_end, write_end) = pipe();
        // the write end stays open: only the unref in the write callback can end the loop
        write_all(write_end, b"hi\n");
        start_stdin_fd(read_end);
        let mut it = Interp::new();
        it.run(
            "process.stdin.on('data', () => \
             process.stdout.write('a\\n', () => process.stdin.unref()));",
        )
        .unwrap_or_else(|_| panic!("script must run"));
        it.run_event_loop()
            .unwrap_or_else(|_| panic!("event loop must finish"));
        assert!(it.out.join("|").contains('a'), "{:?}", it.out);
        unsafe { libc::close(write_end) };
    });
}
