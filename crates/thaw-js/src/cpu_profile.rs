//! `worker.startCpuProfile(name)`: the worker's own sampling profile in Node's cpuprofile shape. The worker takes the
//! samples itself, from its interrupt checks (`worker::serve_cpu`), so only this thread walks `Interp::call_frames`.
//! A timer thread does nothing but raise the `requested` and `sample_due` flags every `INTERVAL`.

use std::collections::HashMap;
use std::fmt::Write;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::builtins::json_quote;
use crate::worker::CpuProbe;
use crate::{FuncDef, Interp};

/// Node's default sampling interval (1000 us).
const INTERVAL: Duration = Duration::from_millis(1);
/// V8's "no position" line and column.
const NO_POSITION: u32 = u32::MAX;

/// Microseconds since the first profile clock read (Node's cpuprofile times are monotonic microseconds).
pub(crate) fn now_us() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_micros() as u64
}

/// The active profiles of one worker, by name, and the timer that samples them while any is active. Dropping it
/// stops and joins the timer.
#[derive(Default)]
pub(crate) struct Profiler {
    profiles: Vec<(String, Profile)>,
    timer: Option<(Arc<AtomicBool>, JoinHandle<()>)>,
}

impl Drop for Profiler {
    fn drop(&mut self) {
        self.halt_timer();
    }
}

impl Profiler {
    /// Starts a named profile: `"started"`, or `"exists"` when one of that name is already running.
    pub(crate) fn start(&mut self, name: &str, probe: &Arc<CpuProbe>) -> &'static str {
        if self.profiles.iter().any(|(n, _)| n == name) {
            return "exists";
        }
        self.profiles
            .push((name.to_string(), Profile::new(now_us())));
        if self.timer.is_none() {
            let stop = Arc::new(AtomicBool::new(false));
            let (flag, probe) = (stop.clone(), probe.clone());
            let join = std::thread::Builder::new()
                .name("thaw-js-cpu-profile".into())
                .spawn(move || {
                    while !flag.load(Ordering::Relaxed) {
                        std::thread::sleep(INTERVAL);
                        probe.tick();
                    }
                })
                .expect("cpu profile timer spawns");
            self.timer = Some((stop, join));
        }
        "started"
    }

    /// Stops a named profile and returns its JSON, or None when no profile has that name. Every active profile takes
    /// a sample first, so a profile always has at least one.
    pub(crate) fn finish(&mut self, it: &Interp, name: &str) -> Option<String> {
        let now = now_us();
        self.sample_all(it, now);
        let at = self.profiles.iter().position(|(n, _)| n == name)?;
        let (_, profile) = self.profiles.remove(at);
        if self.profiles.is_empty() {
            self.halt_timer();
        }
        Some(profile.json(now))
    }

    /// Adds one sample to every active profile, for the stack this thread is running now.
    pub(crate) fn sample_all(&mut self, it: &Interp, now: u64) {
        for (_, profile) in &mut self.profiles {
            profile.sample(it, now);
        }
    }

    fn halt_timer(&mut self) {
        if let Some((stop, join)) = self.timer.take() {
            stop.store(true, Ordering::Relaxed);
            let _ = join.join();
        }
    }
}

/// One profile: a trie of the stacks sampled so far (node 0 is `(root)`), the leaf of each sample, and the time
/// since the previous sample.
struct Profile {
    start: u64,
    last: u64,
    nodes: Vec<Node>,
    /// (parent node, function pointer or 0 for `(program)`) -> child node
    index: HashMap<(usize, usize), usize>,
    samples: Vec<u32>,
    deltas: Vec<u64>,
}

struct Node {
    frame: Frame,
    hits: u64,
    children: Vec<usize>,
    /// Keeps the function alive while the trie refers to its pointer.
    _def: Option<Rc<FuncDef>>,
}

struct Frame {
    name: String,
    script: u32,
    line: u32,
    column: u32,
}

enum Key<'a> {
    Program,
    Func(&'a Rc<FuncDef>),
}

impl Profile {
    fn new(start: u64) -> Self {
        let root = Node {
            frame: Frame {
                name: "(root)".into(),
                script: 0,
                line: NO_POSITION,
                column: NO_POSITION,
            },
            hits: 0,
            children: Vec::new(),
            _def: None,
        };
        Profile {
            start,
            last: start,
            nodes: vec![root],
            index: HashMap::new(),
            samples: Vec::new(),
            deltas: Vec::new(),
        }
    }

    fn sample(&mut self, it: &Interp, now: u64) {
        let mut at = 0;
        if it.call_frames.is_empty() {
            at = self.child(it, at, Key::Program);
        }
        for (def, _) in &it.call_frames {
            at = self.child(it, at, Key::Func(def));
        }
        self.nodes[at].hits += 1;
        self.samples.push(at as u32 + 1);
        self.deltas.push(now - self.last);
        self.last = now;
    }

    fn child(&mut self, it: &Interp, parent: usize, key: Key) -> usize {
        let (ptr, frame, def) = match key {
            Key::Program => (
                0,
                Frame {
                    name: "(program)".into(),
                    script: 0,
                    line: NO_POSITION,
                    column: NO_POSITION,
                },
                None,
            ),
            Key::Func(def) => {
                let (line, column) = position(it, def.src);
                let frame = Frame {
                    name: def.name.as_deref().unwrap_or("").to_string(),
                    script: def.src.0,
                    line,
                    column,
                };
                (Rc::as_ptr(def) as usize, frame, Some(def.clone()))
            }
        };
        if let Some(&node) = self.index.get(&(parent, ptr)) {
            return node;
        }
        let node = self.nodes.len();
        self.nodes.push(Node {
            frame,
            hits: 0,
            children: Vec::new(),
            _def: def,
        });
        self.nodes[parent].children.push(node);
        self.index.insert((parent, ptr), node);
        node
    }

    /// Node's cpuprofile JSON: `nodes` (ids are 1-based), `startTime`/`endTime`, `samples` and `timeDeltas`.
    fn json(&self, end: u64) -> String {
        let mut out = String::from("{\"nodes\":[");
        for (i, node) in self.nodes.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let f = &node.frame;
            let _ = write!(out, "{{\"id\":{},\"callFrame\":{{\"functionName\":", i + 1);
            json_quote(&f.name, &mut out);
            let _ = write!(
                out,
                ",\"scriptId\":{},\"url\":\"\",\"lineNumber\":{},\"columnNumber\":{}}},\"hitCount\":{},\"children\":",
                f.script, f.line, f.column, node.hits
            );
            if node.children.is_empty() {
                out.push_str("null");
            } else {
                let kids: Vec<String> = node.children.iter().map(|c| (c + 1).to_string()).collect();
                let _ = write!(out, "[{}]", kids.join(","));
            }
            out.push('}');
        }
        let samples: Vec<String> = self.samples.iter().map(u32::to_string).collect();
        let deltas: Vec<String> = self.deltas.iter().map(u64::to_string).collect();
        let _ = write!(
            out,
            "],\"startTime\":{},\"endTime\":{end},\"samples\":[{}],\"timeDeltas\":[{}]}}",
            self.start,
            samples.join(","),
            deltas.join(",")
        );
        out
    }
}

/// The 0-based line and column of a function's start (`src` is 1-based), from its script's text. Missing text (a
/// function without source) has no position, as in V8.
fn position(it: &Interp, src: (u32, u32, u32)) -> (u32, u32) {
    let (run, start, _) = src;
    let Some(text) = it.sources.get(&run) else {
        return (NO_POSITION, NO_POSITION);
    };
    let bytes = &text.as_bytes()[..(start.saturating_sub(1) as usize).min(text.len())];
    let line = bytes.iter().filter(|&&b| b == b'\n').count();
    let column = bytes.len() - bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    (line as u32, column as u32)
}
