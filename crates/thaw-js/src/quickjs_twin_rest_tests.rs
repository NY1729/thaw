//! Twins of the thaw-quickjs host tests (processes, filesystem, os parsers, networking, ciphers) that check
//! Node-visible behaviour, run against the thaw-js implementation with the same inputs and expected values. Named
//! `twin_<theme>_<what>`; the engine-internal tests of the retired crate (C-API glue, handle leases) have no twin.

use crate::compile::CompileMode;
use crate::zlib::{hex_decode, hex_encode};
use crate::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("thaw-twin-rest-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Console lines of `src` on a fresh interpreter, after the event loop ran dry.
fn lines(src: &str) -> Vec<String> {
    let mut it = Interp::with_compile_mode(CompileMode::Off);
    match it.run(src) {
        Ok(_) => {}
        Err(Ctl::Throw(v)) => panic!("uncaught {}", it.to_str(&v)),
        Err(_) => panic!("stray completion"),
    }
    it.run_event_loop().expect("event loop runs");
    std::mem::take(&mut it.out)
}

// ---------------------------------------------------------------------------------------------------------------
// child_process: synchronous
// ---------------------------------------------------------------------------------------------------------------

mod child {
    use super::*;
    use crate::child;

    fn sync(cmd: &str, args: serde_json::Value, options: serde_json::Value) -> serde_json::Value {
        json(&child::sync(cmd, &args.to_string(), &options.to_string()))
    }

    fn out(record: &serde_json::Value, key: &str) -> Vec<u8> {
        hex_decode(record[key].as_str().unwrap())
    }

    #[test]
    fn twin_child_sync_input_and_both_output_pipes_make_progress() {
        let input = vec![b'x'; 256 * 1024];
        let record = sync(
            "/bin/sh",
            serde_json::json!(["-c", "exec 3>&2; dd if=/dev/zero bs=4096 count=64 2>/dev/null; dd if=/dev/zero bs=4096 count=64 1>&3 2>/dev/null; exec 3>&-; cat"]),
            serde_json::json!({"input": hex_encode(&input)}),
        );
        assert_eq!(record["status"], 0);
        assert!(record.get("error").is_none());
        let stdout = out(&record, "stdout");
        assert_eq!(stdout.len(), 512 * 1024);
        assert!(stdout[..256 * 1024].iter().all(|byte| *byte == 0));
        assert_eq!(&stdout[256 * 1024..], input.as_slice());
        assert_eq!(out(&record, "stderr"), vec![0; 256 * 1024]);
    }

    #[test]
    fn twin_child_sync_input_epipe_stops_the_child() {
        // Node 22: `{error: EPIPE}`; the child that stopped reading its input is not waited for.
        let started = Instant::now();
        let record = sync(
            "/bin/sh",
            serde_json::json!(["-c", "exec 0<&-; exec sleep 30"]),
            serde_json::json!({"input": hex_encode(&vec![b'x'; 4 * 1024 * 1024])}),
        );
        assert_eq!(record["code"], "EPIPE");
        assert!(record["pid"].as_u64().is_some());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn twin_child_sync_timeout_and_kill_signal_stop_a_child_that_never_exits() {
        let started = Instant::now();
        let record = sync(
            "/bin/sleep",
            serde_json::json!(["30"]),
            serde_json::json!({"timeout": 200, "killSignal": 9}),
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(record["timedOut"], true);
        assert_eq!(record["signal"], 9);
        assert!(record["status"].is_null());
    }

    #[test]
    fn twin_child_sync_max_buffer_stops_a_child_that_keeps_writing() {
        let started = Instant::now();
        let record = sync(
            "/bin/sh",
            serde_json::json!(["-c", "while :; do echo xxxxxxxxxxxxxxxx; done"]),
            serde_json::json!({"maxBuffer": 1000, "killSignal": 15}),
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(record["maxBufferExceeded"], true);
        assert_eq!(record["timedOut"], false);
        assert_eq!(record["signal"], 15);
        assert!(out(&record, "stdout").len() > 1000);
    }

    #[test]
    fn twin_child_sync_within_limits_reports_neither_flag() {
        let record = sync(
            "/bin/sh",
            serde_json::json!(["-c", "printf abc; printf def >&2"]),
            serde_json::json!({"timeout": 5000, "maxBuffer": 1000}),
        );
        assert_eq!(record["status"], 0);
        assert_eq!(record["timedOut"], false);
        assert_eq!(record["maxBufferExceeded"], false);
        assert_eq!(out(&record, "stdout"), b"abc");
        assert_eq!(out(&record, "stderr"), b"def");
    }

    #[test]
    fn twin_child_sync_kill_signal_outside_0_to_64_is_rejected_before_spawn() {
        let record = sync(
            "/bin/sh",
            serde_json::json!(["-c", "exit 0"]),
            serde_json::json!({"killSignal": 65}),
        );
        assert_eq!(record["code"], "EINVAL");
        assert!(record.get("pid").is_none());
    }

    #[test]
    fn twin_child_spawned_children_get_default_signal_dispositions() {
        // A host started under nohup (SIGINT ignored) must not hand that to children, as in libuv.
        // SAFETY: only this test touches SIGINT's disposition, and it restores the previous one.
        let previous = unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
        let record = sync(
            "/bin/sh",
            serde_json::json!(["-c", "kill -INT $$; echo survived"]),
            serde_json::json!({}),
        );
        unsafe { libc::signal(libc::SIGINT, previous) };
        assert_eq!(record["signal"], 2);
        assert!(out(&record, "stdout").is_empty());
    }

    #[test]
    fn twin_child_sync_without_input_gets_a_closed_stdin_pipe_not_the_parents() {
        let record = sync(
            "/bin/sh",
            serde_json::json!(["-c", "cat >/dev/null; printf eof"]),
            serde_json::json!({"timeout": 5000}),
        );
        assert_eq!(record["timedOut"], false);
        assert_eq!(out(&record, "stdout"), b"eof");
    }

    // The identity probe: real/effective uid and gid of the child as `uid,euid,gid,egid`.
    const IDENTITY: &str = r#"echo "$(id -u),$(id -ru),$(id -g),$(id -rg)""#;

    fn ids() -> (u32, u32, u32, u32) {
        unsafe {
            (
                libc::getuid(),
                libc::geteuid(),
                libc::getgid(),
                libc::getegid(),
            )
        }
    }

    #[test]
    fn twin_child_unprivileged_uid_zero_fails_before_the_command_runs() {
        if unsafe { libc::geteuid() } == 0 {
            return; // the EPERM fixture needs an unprivileged test process
        }
        let record = sync(
            "/bin/sh",
            serde_json::json!(["-c", IDENTITY]),
            serde_json::json!({"uid": 0}),
        );
        assert_eq!(record["code"], "EPERM");
        assert!(record["stdout"].as_str().unwrap_or_default().is_empty());
    }

    #[test]
    fn twin_child_negative_uid_is_a_real_request_and_fails_before_the_command_runs() {
        let record = sync(
            "/bin/sh",
            serde_json::json!(["-c", IDENTITY]),
            serde_json::json!({"uid": -1}),
        );
        assert!(record.get("error").is_some(), "uid=-1 must fail: {record}");
        assert!(record["stdout"].as_str().unwrap_or_default().is_empty());
    }

    #[test]
    fn twin_child_root_credentials_change_child_only_and_gid_only_clears_groups() {
        if unsafe { libc::geteuid() } != 0 {
            return; // a changed-credential fixture needs root
        }
        let before = ids();
        let identity = |options: serde_json::Value| -> String {
            let record = sync("/bin/sh", serde_json::json!(["-c", IDENTITY]), options);
            assert_eq!(record["status"], 0, "{record}");
            String::from_utf8(out(&record, "stdout"))
                .unwrap()
                .trim()
                .to_string()
        };
        assert_eq!(
            identity(serde_json::json!({"uid": 65534})),
            format!("65534,65534,{},{}", before.2, before.3)
        );
        assert_eq!(
            identity(serde_json::json!({"gid": 65534})),
            format!("{},{},65534,65534", before.0, before.1)
        );
        assert_eq!(
            identity(serde_json::json!({"uid": 65534, "gid": 65534})),
            "65534,65534,65534,65534"
        );
        assert_eq!(
            identity(serde_json::json!({})),
            format!("{},{},{},{}", before.0, before.1, before.2, before.3)
        );
        assert_eq!(ids(), before);
    }

    #[test]
    fn twin_child_signed_int32_ids_keep_the_standard_platform_cast() {
        let id = |source: &str, key: &str| {
            child::id_opt(&crate::json::parse_native(source).unwrap(), key)
        };
        assert_eq!(id(r#"{"uid":-1}"#, "uid"), Some(u32::MAX));
        assert_eq!(id(r#"{"uid":-2147483648}"#, "uid"), Some(2_147_483_648));
        assert_eq!(id(r#"{"gid":2147483647}"#, "gid"), Some(2_147_483_647));
        assert_eq!(id(r#"{"uid":null}"#, "uid"), None);
        assert_eq!(id(r#"{}"#, "gid"), None);
    }

    // ----- asynchronous children ---------------------------------------------------------------------------------

    /// Polls until every child finished; returns all events.
    fn drain(deadline: Duration) -> Vec<serde_json::Value> {
        let end = Instant::now() + deadline;
        let mut events = Vec::new();
        loop {
            events.extend(json(&child::poll()).as_array().unwrap().iter().cloned());
            if !child::active() {
                return events;
            }
            assert!(
                Instant::now() < end,
                "child did not finish in time: {events:?}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn twin_child_async_kill_is_handled_while_a_stdin_backlog_cannot_drain() {
        crate::sysio::ignore_sigpipe();
        let handle =
            child::spawn("/bin/sleep", &serde_json::json!(["30"]).to_string(), "{}").unwrap();
        // `sleep` never reads stdin: far more than a pipe buffer stays queued.
        for _ in 0..16 {
            let data = hex_encode(&vec![b'z'; 256 * 1024]);
            child::send_stdin(handle, &data, false);
        }
        let started = Instant::now();
        assert!(
            child::kill(handle, 0),
            "signal 0 probes a live child without stopping it"
        );
        assert!(child::kill(handle, 15));
        let mut exit = None;
        while exit.is_none() {
            for event in json(&child::poll()).as_array().unwrap() {
                if event["type"] == "exit" {
                    exit = Some(event.clone());
                }
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "kill was blocked by the stdin backlog"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(exit.unwrap()["signal"], 15);
        assert!(
            !child::kill(handle, 15),
            "a released handle reports no delivery"
        );
    }

    #[test]
    fn twin_child_async_unspawnable_command_queues_error_then_terminal_exit_only() {
        let handle = child::spawn("/path/not/started", "[]", "{}").unwrap();
        let events = drain(Duration::from_secs(2));
        let kinds: Vec<_> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["error", "exit"], "{events:?}");
        assert_eq!(events[0]["code"], "ENOENT");
        assert_eq!(events[0]["handle"], handle);
        assert!(events[1]["code"].is_null() && events[1]["signal"].is_null());
    }

    #[test]
    fn twin_child_spawning_a_file_still_open_for_writing_waits_for_it_instead_of_failing() {
        use std::os::unix::fs::PermissionsExt;
        let directory = scratch("text-busy");
        let script = directory.join("busy.sh");
        std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        // While any descriptor open for writing exists, exec fails with ETXTBSY.
        let writer = std::fs::OpenOptions::new()
            .append(true)
            .open(&script)
            .unwrap();
        assert_eq!(
            std::process::Command::new(&script)
                .spawn()
                .err()
                .and_then(|error| error.raw_os_error()),
            Some(libc::ETXTBSY),
            "the fixture must reproduce the busy file"
        );
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            drop(writer);
        });
        let mut command = std::process::Command::new(&script);
        let status = child::spawn_retrying(&mut command)
            .expect("spawn must outlast the busy window")
            .wait()
            .unwrap();
        release.join().unwrap();
        assert!(status.success());
    }

    #[test]
    fn twin_child_successful_async_identity_delivers_spawn_output_exit_and_releases_handle() {
        let parent = ids();
        let (uid, gid) = if parent.1 == 0 {
            (65_534, 65_534)
        } else if parent.0 == parent.1 && parent.2 == parent.3 {
            (parent.0, parent.2)
        } else {
            return; // needs matching real/effective ids or root
        };
        let options = serde_json::json!({"uid": uid, "gid": gid}).to_string();
        child::spawn(
            "/bin/sh",
            &serde_json::json!(["-c", IDENTITY]).to_string(),
            &options,
        )
        .unwrap();
        let events = drain(Duration::from_secs(2));
        assert_eq!(
            events.first().unwrap()["type"],
            "spawn",
            "events: {events:?}"
        );
        assert_eq!(events.last().unwrap()["type"], "exit", "events: {events:?}");
        assert_eq!(events.last().unwrap()["code"], 0);
        let stdout: Vec<u8> = events
            .iter()
            .filter(|e| e["type"] == "stdout")
            .flat_map(|e| hex_decode(e["value"].as_str().unwrap()))
            .collect();
        assert_eq!(
            String::from_utf8(stdout).unwrap().trim(),
            format!("{uid},{uid},{gid},{gid}")
        );
        assert_eq!(
            ids(),
            parent,
            "credential application must not mutate the parent"
        );
        assert!(!events.iter().any(|e| e["type"] == "error"));
        assert!(
            !child::active(),
            "terminal poll must release the child handle"
        );
    }

    #[test]
    fn twin_child_an_unreferenced_child_does_not_keep_the_event_loop_alive_until_referenced_again()
    {
        let handle = child::spawn(
            "sleep",
            &serde_json::json!(["30"]).to_string(),
            &serde_json::json!({ "stdio": ["ignore", "ignore", "ignore"] }).to_string(),
        )
        .unwrap();
        assert!(child::active(), "a fresh child is referenced");
        child::set_ref(handle, false);
        assert!(
            !child::active(),
            "an unreferenced child must not count as activity"
        );
        child::set_ref(handle, true);
        assert!(child::active());
        assert!(child::kill(handle, libc::SIGKILL));
        drain(Duration::from_secs(5));
        assert!(!child::active());
    }

    #[test]
    fn twin_child_stdout_then_stderr_output_keeps_write_order_across_pipes() {
        for round in 0..300 {
            child::spawn(
                "sh",
                &serde_json::json!(["-c", "printf abcd; printf x >&2"]).to_string(),
                "{}",
            )
            .unwrap();
            let events = drain(Duration::from_secs(10));
            let first = |kind: &str| events.iter().position(|event| event["type"] == kind);
            let (o, e) = (
                first("stdout").expect("stdout event"),
                first("stderr").expect("stderr event"),
            );
            assert!(o < e, "round {round}: stderr overtook stdout: {events:?}");
        }
    }

    #[test]
    fn twin_child_inherited_and_ignored_stdio_attach_no_pipes_to_the_host() {
        child::spawn(
            "sh",
            &serde_json::json!(["-c", "echo to-stdout; echo to-stderr >&2"]).to_string(),
            &serde_json::json!({ "stdio": ["ignore", "ignore", "ignore"] }).to_string(),
        )
        .unwrap();
        let events = drain(Duration::from_secs(5));
        assert!(
            !events
                .iter()
                .any(|event| event["type"] == "stdout" || event["type"] == "stderr"),
            "ignored stdio must not be piped: {events:?}"
        );
        child::spawn(
            "sh",
            &serde_json::json!(["-c", "echo piped-out"]).to_string(),
            "{}",
        )
        .unwrap();
        let events = drain(Duration::from_secs(5));
        let output: Vec<u8> = events
            .iter()
            .filter(|event| event["type"] == "stdout")
            .flat_map(|event| hex_decode(event["value"].as_str().unwrap()))
            .collect();
        assert_eq!(output, b"piped-out\n", "the default stays piped");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// worker_threads
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn twin_worker_late_ref_keeps_queued_terminal_events_alive_until_poll() {
    // Node 22: an unref'd worker's queued message and exit still arrive once the parent refs it again.
    let src = "const wt = __thaw_worker_threads_exports;
      (async () => {
        const w = new wt.Worker(`const { parentPort } = require('node:worker_threads'); parentPort.postMessage('queued');`, { eval: true });
        w.on('message', (m) => console.log('msg ' + m));
        w.on('exit', (c) => console.log('exit ' + c));
        w.unref();
        await new Promise((r) => setTimeout(r, 100));
        w.ref();
        await new Promise((r) => setTimeout(r, 1500));
        console.log('done');
      })().catch((e) => console.log('failed ' + e));";
    assert_eq!(lines(src), ["msg queued", "exit 0", "done"]);
}

/// Runs a Worker with `source` under `resourceLimits` and prints its error as `code|message`, or `none`.
fn worker_error(source: &str, limits: &str) -> String {
    let src = format!(
        "const wt = __thaw_worker_threads_exports;
         (async () => {{
           const worker = new wt.Worker({source:?}, {{ eval: true{limits} }});
           const e = await new Promise((resolve) => {{ worker.on('error', resolve); worker.on('exit', () => resolve(null)); }});
           console.log(e ? e.code + '|' + e.message : 'none');
         }})();"
    );
    lines(&src).join("\n")
}

const OOM: &str = "ERR_WORKER_OUT_OF_MEMORY|Worker terminated due to reaching memory limit: JS heap out of memory";

#[test]
fn twin_worker_memory_limit_is_reported_as_out_of_memory_every_time() {
    let source = "const keep = []; while (true) { keep.push(new Array(100000).fill(1)); }";
    for run in 0..10 {
        let error = worker_error(source, ", resourceLimits: { maxOldGenerationSizeMb: 16 }");
        assert_eq!(error, OOM, "run {run}");
    }
}

#[test]
fn twin_worker_memory_limit_covers_other_heap_shapes() {
    let small_objects = "const keep = []; let i = 0; while (true) { keep.push({ a: i++ }); }";
    // The retired engine also capped string growth; thaw-js does not count strings (heap_limit.rs: the approved
    // byte-accounting approximation), so that shape is not twinned.
    let arrays = "const keep = []; while (true) { keep.push(new Array(100000).fill(1)); }";
    for (source, mb) in [(small_objects, 16), (arrays, 1), (arrays, 4), (arrays, 64)] {
        for _ in 0..3 {
            let limits = format!(", resourceLimits: {{ maxOldGenerationSizeMb: {mb} }}");
            assert_eq!(worker_error(source, &limits), OOM, "{mb} MB {source}");
        }
    }
}

#[test]
fn twin_worker_ordinary_memory_errors_are_not_reported_as_worker_out_of_memory() {
    // Node 22: both print `undefined|out of memory`: the user's own error, not the memory cap.
    let source = "throw new Error('out of memory');";
    assert_eq!(worker_error(source, ""), "undefined|out of memory");
    assert_eq!(
        worker_error(source, ", resourceLimits: { maxOldGenerationSizeMb: 64 }"),
        "undefined|out of memory"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// os: /proc parsers and priority
// ---------------------------------------------------------------------------------------------------------------

mod os {
    use crate::sysio::{get_priority, linux_cpu_metadata, linux_cpu_times};

    #[test]
    fn twin_os_parses_numbered_cpus_in_milliseconds_without_affinity_or_guest_double_counting() {
        let stat = "cpu 999 999 999 999 999 999 999 999 999 999\n\
                    cpu1 25 50 75 100 125 150 175 200 225 250\n\
                    cpu3 250 500 750 1000 1250 1500 1750 2000 2250 2500\n";
        assert_eq!(
            linux_cpu_times(stat, 250),
            vec![
                (1, [100, 200, 300, 400, 600]),
                (3, [1000, 2000, 3000, 4000, 6000])
            ]
        );
        assert_eq!(
            linux_cpu_times("cpu7 18446744073709551615 1 2 3 4 5 6 7 8 9\n", 100),
            vec![(7, [u64::MAX, 10, 20, 30, 50])]
        );
        assert!(linux_cpu_times(stat, 0).is_empty());
    }

    #[test]
    fn twin_os_matches_core_metadata_by_processor_number() {
        let info = "processor : 3\nmodel name : fast\ncpu MHz : 3200.5\n\n\
                    processor : 1\nmodel name : efficient\ncpu MHz : 1700.25\n";
        let metadata = linux_cpu_metadata(info);
        assert_eq!(metadata.get(&1), Some(&("efficient".to_string(), 1700)));
        assert_eq!(metadata.get(&3), Some(&("fast".to_string(), 3201)));
    }

    #[test]
    fn twin_os_getpriority_accepts_valid_minus_one() {
        let current = get_priority(0).unwrap();
        assert!((-20..=19).contains(&current));
        let absent = get_priority(i32::MAX).unwrap_err();
        assert_eq!(absent.raw_os_error(), Some(libc::ESRCH));
    }

    #[test]
    fn twin_os_cpus_report_one_entry_per_core_with_times() {
        // The live end of the parsers above: os.cpus() on this host.
        let out = super::lines(
            "const c = JSON.parse(__thaw_os_info()).cpus; console.log(c.length > 0, c.every((x) => typeof x.model === 'string' && x.times.user >= 0 && x.times.idle >= 0));",
        );
        assert_eq!(out, ["true true"]);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// filesystem
// ---------------------------------------------------------------------------------------------------------------

mod fs {
    use super::*;
    use crate::sysio::{
        fs_code, fs_copy_recursive, fs_copy_unsafe_relation, fs_err, fs_statfs_record, fs_timespec,
        host_fs,
    };
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

    fn call(op: &str, path: &Path, value: &str) -> serde_json::Value {
        json(&host_fs(op, path.to_str().unwrap(), value, false))
    }

    #[test]
    fn twin_fs_metadata_wire_preserves_integer_fields_and_nanoseconds() {
        let d = scratch("metadata");
        let path = d.join("f");
        std::fs::write(&path, b"data").unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        let record = call("stat", &path, "");
        assert_eq!(record["exact"]["length"], metadata.len().to_string());
        assert_eq!(record["exact"]["ino"], metadata.ino().to_string());
        assert_eq!(record["exact"]["dev"], metadata.dev().to_string());
        assert_eq!(
            record["exact"]["mtimeNs"],
            (i128::from(metadata.mtime()) * 1_000_000_000 + i128::from(metadata.mtime_nsec()))
                .to_string()
        );
        // a time one nanosecond before the epoch stays exact and negative
        assert_eq!(
            call("utimes", &path, "-0.000000001,-0.000000001")["ok"],
            true
        );
        assert_eq!(call("stat", &path, "")["exact"]["mtimeNs"], "-1");
    }

    #[test]
    fn twin_fs_error_preserves_native_errno_and_path_operation() {
        let code = |raw| {
            json(&fs_err(
                "rmdir",
                "/example",
                &std::io::Error::from_raw_os_error(raw),
            ))["code"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(code(libc::EPERM), "EPERM");
        assert_eq!(code(libc::EACCES), "EACCES");
        assert_eq!(code(libc::ELOOP), "ELOOP");
        assert_eq!(code(libc::ENOSPC), "ENOSPC");
        assert_eq!(code(libc::EXDEV), "EXDEV");
        assert_eq!(code(libc::ENOTSUP), "ENOTSUP");
        assert_eq!(code(libc::EOPNOTSUPP), "ENOTSUP");
        assert_eq!(code(123456), "ERRNO_123456");
        assert_eq!(
            fs_code(&std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid path"
            )),
            "EINVAL"
        );
        let d = scratch("errno");
        std::fs::write(d.join("child"), b"x").unwrap();
        let result = call("rmdir", &d, "");
        assert_eq!(result["code"], "ENOTEMPTY");
        assert_eq!(result["operation"], "rmdir");
        assert_eq!(result["path"], d.to_string_lossy().as_ref());
    }

    #[test]
    fn twin_fs_statfs_reports_linux_type_for_path_and_descriptor() {
        let d = std::env::temp_dir();
        let opened = call("fd_open", &d, "r,438");
        assert_eq!(opened["ok"], true, "{opened}");
        let fd = opened["fd"].as_u64().unwrap().to_string();
        let path_result = call("statfs", &d, "");
        let fd_result = json(&host_fs("fd_statfs", &fd, "", false));
        assert_eq!(path_result["ok"], true);
        assert_eq!(fd_result["ok"], true, "{fd_result}");
        assert_ne!(path_result["type"], 0);
        assert_eq!(fd_result["type"], path_result["type"]);
        assert_eq!(fd_result["blocks"], path_result["blocks"]);
        host_fs("fd_close", &fd, "", false);
    }

    #[test]
    fn twin_fs_timespec_normalizes_negative_fraction_and_bounds() {
        let tiny = fs_timespec(Some("-0.00000000000000001")).unwrap();
        assert_eq!((tiny.tv_sec, tiny.tv_nsec), (0, 0));
        let negative = fs_timespec(Some("-0.000000001")).unwrap();
        assert_eq!((negative.tv_sec, negative.tv_nsec), (-1, 999_999_999));
        let subnanosecond = fs_timespec(Some("0.0000000006")).unwrap();
        assert_eq!((subnanosecond.tv_sec, subnanosecond.tv_nsec), (0, 0));
        let milliseconds = fs_timespec(Some("1.234")).unwrap();
        assert_eq!(
            (milliseconds.tv_sec, milliseconds.tv_nsec),
            (1, 234_000_000)
        );
        assert!(fs_timespec(Some("NaN")).is_err());
        assert!(fs_timespec(Some("9223372036854775808")).is_err());
        assert!(fs_timespec(Some("-1e300")).is_err());
        assert!(fs_timespec(Some("1e300")).is_err());
    }

    #[test]
    fn twin_fs_utimes_updates_permissionless_target_and_preserves_link_behavior() {
        let d = scratch("utimes");
        let (target, link) = (d.join("target"), d.join("link"));
        std::fs::write(&target, b"x").unwrap();
        symlink(&target, &link).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert_eq!(call("utimes", &target, "-0.000000001,1.234")["ok"], true);
        let stat = std::fs::metadata(&target).unwrap();
        assert_eq!((stat.atime(), stat.atime_nsec()), (-1, 999_999_999));
        assert_eq!((stat.mtime(), stat.mtime_nsec()), (1, 234_000_000));
        assert_eq!(call("lutimes", &link, "2.5,3.5")["ok"], true);
        assert_eq!(std::fs::symlink_metadata(&link).unwrap().mtime(), 3);
        assert_eq!(std::fs::metadata(&target).unwrap().mtime(), 1);
        assert_eq!(call("utimes", &link, "4,5")["ok"], true);
        assert_eq!(std::fs::metadata(&target).unwrap().mtime(), 5);
        assert_eq!(std::fs::symlink_metadata(&link).unwrap().mtime(), 3);
    }

    #[test]
    fn twin_fs_raw_path_transport_rejects_malformed_hex_and_nul() {
        // "ff2ffe" is the path bytes [0xff, '/', 0xfe]: it decodes and is simply absent (ENOENT).
        assert_eq!(
            json(&host_fs("stat_raw", "ff2ffe", "", false))["code"],
            "ENOENT"
        );
        for bad in ["f", "fg", "610062"] {
            assert_eq!(
                json(&host_fs("stat_raw", bad, "", false))["code"],
                "EINVAL",
                "{bad}"
            );
        }
    }

    #[test]
    fn twin_fs_statfs_record_preserves_large_counters_as_decimal_strings() {
        let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
        stats.f_bsize = 9_007_199_254_740_995;
        stats.f_blocks = 9_007_199_254_740_997;
        stats.f_bfree = 9_007_199_254_740_999;
        stats.f_bavail = 9_007_199_254_741_001;
        stats.f_files = 9_007_199_254_741_003;
        stats.f_ffree = 9_007_199_254_741_005;
        let record = json(&fs_statfs_record(&stats, 9_007_199_254_740_993));
        for (key, expected) in [
            ("type", "9007199254740993"),
            ("bsize", "9007199254740995"),
            ("blocks", "9007199254740997"),
            ("bfree", "9007199254740999"),
            ("bavail", "9007199254741001"),
            ("files", "9007199254741003"),
            ("ffree", "9007199254741005"),
        ] {
            assert_eq!(record["exact"][key], expected);
        }
        assert!(record["blocks"].is_number());
    }

    #[test]
    fn twin_fs_copy_relation_resolves_alias_parents_before_descendant_mutation() {
        let root = scratch("copy-relation");
        let (source, outside) = (root.join("source"), root.join("outside"));
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(outside.join("deep")).unwrap();
        symlink(&source, root.join("source-alias")).unwrap();
        symlink(outside.join("deep"), root.join("outside-alias")).unwrap();
        assert!(fs_copy_unsafe_relation(&source, &root.join("source-alias/new"), false).unwrap());
        assert!(
            fs_copy_unsafe_relation(&source, &root.join("missing/../source/new"), false).unwrap()
        );
        assert!(!fs_copy_unsafe_relation(
            &source,
            &root.join("outside-alias/../source/new"),
            false
        )
        .unwrap());
        assert!(fs_copy_recursive(&source, &root.join("source-alias/new")).is_err());
        assert!(!root.join("missing").exists());
        assert!(!source.join("new").exists());
    }

    /// A directory entry that vanishes between the listing and its `lstat` (a busy `/tmp`) must not fail the whole
    /// listing: `glob` walks absolute patterns from `/` and treats ENOENT as "empty".
    #[test]
    fn twin_fs_readdir_entries_survive_entries_removed_while_listing() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let root = scratch("readdir-churn");
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let churn = {
            let (root, stop) = (root.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut index = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let path = root.join(format!("e{}", index % 64));
                    if index.is_multiple_of(3) {
                        let _ = std::fs::create_dir_all(path.join("inner"));
                        let _ = std::fs::remove_dir_all(&path);
                    } else {
                        let _ = std::fs::write(&path, b"x");
                        let _ = std::fs::remove_file(&path);
                    }
                    index += 1;
                }
            })
        };
        let mut failure = None;
        for round in 0..4000 {
            for recursive in [false, true] {
                let result = json(&host_fs(
                    "readdir",
                    root.to_str().unwrap(),
                    "typed",
                    recursive,
                ));
                if result["ok"] != true {
                    failure = Some((round, recursive, result.to_string()));
                }
            }
            if failure.is_some() {
                break;
            }
        }
        stop.store(true, Ordering::Relaxed);
        churn.join().unwrap();
        assert!(
            failure.is_none(),
            "typed readdir failed on a vanishing entry: {failure:?}"
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// networking
// ---------------------------------------------------------------------------------------------------------------

mod net {
    use super::*;
    use crate::net::{net_connect, net_destroy, net_poll_read, net_set_option, With, NET_STREAMS};
    use crate::tls::{
        certificate_metadata, server_client_auth_status, tls_ca_roots, tls_client_roots,
        RecordingServerVerifier,
    };
    use base64::Engine;
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::RootCertStore;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    const ROOT: &[u8] = include_bytes!("fixtures/tubitak-root.der");

    fn spec(bytes: &[u8]) -> String {
        hex_encode(bytes)
    }

    fn connected() -> (TcpListener, u32) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let connected = net_connect("127.0.0.1", port);
        let handle = connected
            .strip_prefix("ok:")
            .unwrap()
            .split(':')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        (listener, handle)
    }

    #[test]
    fn twin_tls_explicit_ca_replaces_defaults_and_preserves_all_pem_certificates() {
        let defaults = tls_client_roots(None).unwrap();
        assert_eq!(defaults.roots.as_slice(), webpki_roots::TLS_SERVER_ROOTS);
        assert!(defaults.len() > 1);
        assert_eq!(tls_ca_roots(None).unwrap().len(), 0);
        assert_eq!(tls_client_roots(Some("")).unwrap().len(), 0);
        let root = spec(ROOT);
        let explicit = tls_client_roots(Some(&root)).unwrap();
        let mut expected = RootCertStore::empty();
        expected.add(CertificateDer::from(ROOT.to_vec())).unwrap();
        assert_eq!(explicit.roots.as_slice(), expected.roots.as_slice());
        assert_eq!(explicit.len(), 1);
        let comma = tls_ca_roots(Some(&format!("{root},{root}"))).unwrap();
        assert_eq!(
            comma.roots.as_slice(),
            [expected.roots[0].clone(), expected.roots[0].clone()]
        );
        let encoded = base64::engine::general_purpose::STANDARD.encode(ROOT);
        let pem = format!(
            "-----BEGIN CERTIFICATE-----\n{encoded}\n-----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\n{encoded}\n-----END CERTIFICATE-----"
        );
        let pem_roots = tls_ca_roots(Some(&spec(pem.as_bytes()))).unwrap();
        assert_eq!(
            pem_roots.roots.as_slice(),
            [expected.roots[0].clone(), expected.roots[0].clone()]
        );
    }

    #[test]
    fn twin_tls_public_root_metadata_exposes_safe_subject_and_san_shapes() {
        let metadata = json(&certificate_metadata(ROOT));
        assert!(metadata["subject"]["CN"].is_array());
        assert!(metadata["issuer"]["CN"].is_array());
        assert!(metadata["dnsNames"].is_array());
        assert!(metadata["hasDnsSan"].is_boolean());
        assert!(metadata["ipAddresses"].is_array());
        assert!(metadata["subjectaltname"].is_string());
        assert!(metadata["sanError"].is_boolean());
        let canonical = lines(
            "console.log(__thaw_tls_canonical_ip('2001:0db8:0:0:0:0:0:1') + '|' + __thaw_tls_canonical_ip('name.example') + '|');",
        );
        assert_eq!(canonical, ["2001:db8::1||"]);
    }

    #[test]
    fn twin_tls_invalid_server_certificate_chain_is_recorded_before_strict_or_permissive_result() {
        let name = ServerName::try_from("identity.example").unwrap();
        let certificate = CertificateDer::from(b"not an X.509 certificate".to_vec());
        let timestamp = UnixTime::since_unix_epoch(Duration::from_secs(1));
        let verifier = |allow_invalid: bool| {
            let result = Arc::new(Mutex::new(None));
            let verifier = RecordingServerVerifier {
                roots: RootCertStore::empty(),
                defer_name: true,
                allow_invalid,
                result: Arc::clone(&result),
                algorithms: rustls::crypto::ring::default_provider()
                    .signature_verification_algorithms,
            };
            (verifier, result)
        };
        let (strict, strict_result) = verifier(false);
        assert!(
            rustls::client::danger::ServerCertVerifier::verify_server_cert(
                &strict,
                &certificate,
                &[],
                &name,
                &[],
                timestamp,
            )
            .is_err()
        );
        assert!(strict_result.lock().unwrap().as_ref().is_some());
        let (permissive, permissive_result) = verifier(true);
        assert!(
            rustls::client::danger::ServerCertVerifier::verify_server_cert(
                &permissive,
                &certificate,
                &[],
                &name,
                &[],
                timestamp,
            )
            .is_ok()
        );
        assert!(permissive_result.lock().unwrap().as_ref().is_some());
    }

    #[test]
    fn twin_tls_server_authorization_requires_requested_and_verified_client_identity() {
        assert_eq!(
            server_client_auth_status(false, false, true),
            (false, Some("TLS client authentication was not requested"))
        );
        assert_eq!(
            server_client_auth_status(true, false, true),
            (false, Some("CERT_NOT_PROVIDED"))
        );
        assert_eq!(
            server_client_auth_status(true, true, false),
            (false, Some("CERT_REJECTED"))
        );
        assert_eq!(server_client_auth_status(true, true, true), (true, None));
    }

    #[test]
    fn twin_tls_explicit_ca_rejects_malformed_or_partial_inputs() {
        for invalid in ["z", "abc", "00", "zz", "00,"] {
            assert!(tls_ca_roots(Some(invalid)).is_err(), "accepted {invalid:?}");
        }
        for pem in [
            b"-----BEGIN CERTIFICATE-----bad".as_slice(),
            b"-----BEGIN CERTIFICATE-----\n\n-----END CERTIFICATE-----".as_slice(),
            b"-----BEGIN PRIVATE KEY-----\nAA==\n-----END PRIVATE KEY-----".as_slice(),
        ] {
            assert!(tls_ca_roots(Some(&spec(pem))).is_err());
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(ROOT);
        let valid_pem =
            format!("-----BEGIN CERTIFICATE-----\n{encoded}\n-----END CERTIFICATE-----");
        for invalid_bundle in [
            format!("{valid_pem}\n-----BEGIN CERTIFICATE-----\nbad"),
            format!("{valid_pem}\n-----BEGIN PRIVATE KEY-----\nAA==\n-----END PRIVATE KEY-----"),
            format!("{valid_pem}\ntrailing"),
            format!("\u{2003}{valid_pem}"),
            format!("{valid_pem}\n\u{2003}{valid_pem}"),
            format!("-----BEGIN CERTIFICATE-----\n{encoded}\u{2003}\n-----END CERTIFICATE-----"),
        ] {
            assert!(
                tls_ca_roots(Some(&spec(invalid_bundle.as_bytes()))).is_err(),
                "{invalid_bundle:?}"
            );
        }
        assert!(tls_ca_roots(Some(&format!("{},zz", spec(ROOT)))).is_err());
        assert!(tls_ca_roots(Some(&format!("{},00", spec(ROOT)))).is_err());
        assert!(tls_ca_roots(Some(&format!(
            "{},{}",
            spec(ROOT),
            spec(b"-----BEGIN CERTIFICATE-----bad")
        )))
        .is_err());
    }

    #[test]
    fn twin_net_set_option_applies_nodelay_and_keepalive_to_the_socket() {
        use std::os::fd::AsRawFd;
        let (listener, handle) = connected();
        let (_peer, _) = listener.accept().unwrap();
        let read = |level: libc::c_int, name: libc::c_int| -> libc::c_int {
            NET_STREAMS.with(|streams| {
                let streams = streams.borrow();
                let stream = streams.1.get(&handle).unwrap();
                let mut value: libc::c_int = -1;
                let mut length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
                let result = unsafe {
                    libc::getsockopt(
                        stream.as_raw_fd(),
                        level,
                        name,
                        (&mut value as *mut libc::c_int).cast(),
                        &mut length,
                    )
                };
                assert_eq!(result, 0);
                value
            })
        };
        assert_eq!(read(libc::SOL_SOCKET, libc::SO_KEEPALIVE), 0);
        assert_eq!(net_set_option("tcp", handle, "keepalive", true, 4500), "ok");
        assert_eq!(read(libc::SOL_SOCKET, libc::SO_KEEPALIVE), 1);
        assert_eq!(read(libc::IPPROTO_TCP, libc::TCP_KEEPIDLE), 4);
        assert_eq!(net_set_option("tcp", handle, "keepalive", false, 0), "ok");
        assert_eq!(read(libc::SOL_SOCKET, libc::SO_KEEPALIVE), 0);
        assert_eq!(net_set_option("tcp", handle, "nodelay", true, 0), "ok");
        assert_eq!(read(libc::IPPROTO_TCP, libc::TCP_NODELAY), 1);
        assert_eq!(net_set_option("tcp", handle, "nodelay", false, 0), "ok");
        assert_eq!(read(libc::IPPROTO_TCP, libc::TCP_NODELAY), 0);
        assert_eq!(
            net_set_option("tcp", 4_000_000, "nodelay", true, 0),
            "err:socket is closed"
        );
        net_destroy(handle);
    }

    #[test]
    fn twin_net_poll_read_retires_stream_after_peer_reset() {
        use std::os::fd::AsRawFd;
        let (listener, handle) = connected();
        let (peer, _) = listener.accept().unwrap();
        let linger = libc::linger {
            l_onoff: 1,
            l_linger: 0,
        };
        let configured = unsafe {
            libc::setsockopt(
                peer.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_LINGER,
                (&linger as *const libc::linger).cast(),
                std::mem::size_of::<libc::linger>() as libc::socklen_t,
            )
        };
        assert_eq!(configured, 0);
        drop(peer);
        let deadline = Instant::now() + Duration::from_secs(1);
        let outcome = loop {
            let outcome = net_poll_read(handle);
            if outcome != "pending" || Instant::now() >= deadline {
                break outcome;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        assert!(outcome.starts_with("err:"), "{outcome}");
        assert_eq!(net_poll_read(handle), "err:socket is closed");
        NET_STREAMS.with(|streams| assert!(!streams.borrow().1.contains_key(&handle)));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// symmetric ciphers
// ---------------------------------------------------------------------------------------------------------------

mod cipher {
    use crate::node_crypto::cipher::{sym_block_cipher, sym_ctr_xor};

    #[test]
    fn twin_cipher_ctr_keystream_is_stateless_across_offsets() {
        let (key, iv) = ([7u8; 32], [9u8; 16]);
        let whole = sym_ctr_xor(&key, &iv, 0, &[0u8; 40], true).unwrap();
        let tail = sym_ctr_xor(&key, &iv, 5, &[0u8; 35], true).unwrap();
        assert_eq!(&whole[5..], &tail[..]);
    }

    #[test]
    fn twin_cipher_cbc_round_trips_with_padding_and_rejects_a_bad_final_block() {
        let (key, iv) = ([1u8; 24], [2u8; 16]);
        let encrypted =
            sym_block_cipher(false, &key, &iv, b"padded text", false, true, true).unwrap();
        assert_eq!(encrypted.len(), 16);
        assert_eq!(
            sym_block_cipher(false, &key, &iv, &encrypted, true, true, true).unwrap(),
            b"padded text"
        );
        let unpadded = sym_block_cipher(false, &key, &iv, &[0u8; 16], false, true, false).unwrap();
        assert_eq!(
            sym_block_cipher(false, &key, &iv, &unpadded, true, true, true),
            Err("bad decrypt".to_string())
        );
    }
}
