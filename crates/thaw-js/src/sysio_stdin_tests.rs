//! Main-thread `process.stdin` plumbing in sysio.rs: the reader thread and terminal raw mode.

use std::time::{Duration, Instant};

use crate::sysio::{poll_stdin, set_fd_raw_mode, start_stdin_fd};

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
