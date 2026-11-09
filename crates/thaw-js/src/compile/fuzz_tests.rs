//! Deterministic differential fuzzer for the compiled subset (plan TJ150 section 7, PR5). Each seed makes a program
//! of helper functions in the PR1-4 grammar (no calls, no closures, no object literals) and a driver that calls each
//! helper with corner-value arguments and traces every return value and thrown error. Off, On and Check must print
//! the same trace, and most helpers must really compile, so a broken generator cannot pass vacuously.
//! `THAW_JS_FUZZ_SEEDS` sets the seed count (default 60; each run leaks about 4 MB of interpreter, so 300 seeds need 4 GB).

use super::*;
use crate::Interp;
use std::time::{Duration, Instant};

const HELPERS: usize = 3;

/// xorshift64 with a per-seed start state: the same seed always yields the same program.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

/// Corner values as source literals: the table of plan section 7 PR2, reduced to what the driver passes.
const CORNERS: &[&str] = &[
    "0",
    "-0",
    "1",
    "-1",
    "0.5",
    "-1.5",
    "2147483647",
    "2147483648",
    "-2147483649",
    "4294967295",
    "4294967296",
    "1e19",
    "9007199254740993",
    "NaN",
    "Infinity",
    "-Infinity",
    "\"5\"",
    "\"\"",
    "\"abc\"",
    "true",
    "false",
    "null",
    "undefined",
    "3n",
];

const BIN_OPS: &[&str] = &[
    "+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>", ">>>", "<", "<=", ">", ">=", "===", "!==",
    "==", "!=", "**",
];

const ASSIGN_OPS: &[&str] = &["=", "+=", "-=", "*=", "|=", "^=", "&=", "<<=", ">>>="];

/// Source generator for one seed. `scopes` mirrors the block structure: each name is (source name, mutable).
/// Loop counters are immutable, so no generated loop can run forever.
struct Gen {
    rng: Rng,
    scopes: Vec<Vec<(String, bool)>>,
    next: usize,
    loops: usize,
}

impl Gen {
    fn fresh(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    fn names(&self, mutable_only: bool) -> Vec<String> {
        self.scopes
            .iter()
            .flatten()
            .filter(|(_, m)| !mutable_only || *m)
            .map(|(n, _)| n.clone())
            .collect()
    }

    fn pick_name(&mut self, mutable_only: bool) -> Option<String> {
        let names = self.names(mutable_only);
        if names.is_empty() {
            None
        } else {
            Some(names[self.rng.below(names.len())].clone())
        }
    }

    fn atom(&mut self) -> String {
        if self.rng.below(2) == 0 {
            if let Some(n) = self.pick_name(false) {
                return n;
            }
        }
        // parenthesised: a negative literal cannot be the left operand of `**`
        format!("({})", self.rng.pick(CORNERS))
    }

    fn expr(&mut self, depth: usize) -> String {
        if depth == 0 || self.rng.below(3) == 0 {
            return self.atom();
        }
        let d = depth - 1;
        match self.rng.below(10) {
            0..=2 => {
                let op = self.rng.pick(BIN_OPS);
                let (a, b) = (self.expr(d), self.expr(d));
                format!("({a} {op} {b})")
            }
            3 => {
                let op = self.rng.pick(&["&&", "||", "??"]);
                let (a, b) = (self.expr(d), self.expr(d));
                format!("({a} {op} {b})")
            }
            4 => {
                let op = self.rng.pick(&["-", "+", "~", "!"]);
                let a = self.expr(d);
                format!("({op}({a}))")
            }
            5 => {
                let a = self.expr(d);
                format!("(typeof ({a}))")
            }
            6 => {
                let (c, a, b) = (self.expr(d), self.expr(d), self.expr(d));
                format!("({c} ? {a} : {b})")
            }
            7 => {
                let arr = self.rng.pick(&["p0", "p1", "p2"]);
                let i = self.expr(d);
                format!("{arr}[{i}]")
            }
            8 => match self.pick_name(true) {
                Some(n) if self.rng.below(2) == 0 => format!("({n} = {})", self.expr(d)),
                Some(n) => format!("({n}++)"),
                None => self.atom(),
            },
            _ => "p1.length".to_string(),
        }
    }

    /// `return [every name visible here]`: the trace sees every local at the point of return.
    fn ret(&self) -> String {
        format!("return [{}];", self.names(false).join(", "))
    }

    fn block(&mut self, depth: usize, out: &mut String) {
        self.scopes.push(Vec::new());
        for _ in 0..self.rng.below(3) + 1 {
            self.stmt(depth, out);
        }
        self.scopes.pop();
    }

    fn stmt(&mut self, depth: usize, out: &mut String) {
        let mut c = self.rng.below(14);
        if depth == 0 && matches!(c, 6 | 7 | 8 | 12 | 13) {
            c = 11;
        }
        match c {
            0 | 1 => {
                let e = self.expr(2);
                let n = self.fresh("v");
                *out += &format!("let {n} = {e};\n");
                self.scopes.last_mut().unwrap().push((n, true));
            }
            2 => {
                let e = self.expr(2);
                let n = self.fresh("c");
                *out += &format!("const {n} = {e};\n");
                self.scopes.last_mut().unwrap().push((n, false));
            }
            3 => {
                let e = self.expr(2);
                let n = self.fresh("w");
                *out += &format!("var {n} = {e};\n");
                self.scopes[0].push((n, true));
            }
            4 => {
                // a const is assigned now and then: the TypeError must match the AST
                // only consts: a loop counter assigned here could make the loop run forever
                let consts: Vec<String> = self
                    .scopes
                    .iter()
                    .flatten()
                    .filter(|(n, m)| !*m && n.starts_with('c'))
                    .map(|(n, _)| n.clone())
                    .collect();
                let n = match self.rng.below(8) {
                    0 if !consts.is_empty() => Some(consts[self.rng.below(consts.len())].clone()),
                    _ => self.pick_name(true),
                };
                if let Some(n) = n {
                    let op = self.rng.pick(ASSIGN_OPS);
                    let e = self.expr(2);
                    *out += &format!("{n} {op} {e};\n");
                }
            }
            5 => match self.pick_name(true) {
                Some(n) if self.rng.below(2) == 0 => *out += &format!("{n}++;\n"),
                Some(n) => *out += &format!("--{n};\n"),
                None => {
                    let arr = self.rng.pick(&["p0", "p1", "p2"]);
                    let (i, e) = (self.expr(1), self.expr(2));
                    let op = self.rng.pick(&["=", "+="]);
                    *out += &format!("{arr}[{i}] {op} {e};\n");
                }
            },
            6 => {
                let c = self.expr(2);
                *out += &format!("if ({c}) {{\n");
                self.block(depth - 1, out);
                *out += "} else {\n";
                self.block(depth - 1, out);
                *out += "}\n";
            }
            7 => {
                let i = self.fresh("i");
                let k = self.rng.below(4);
                *out += &format!("for (var {i} = 0; {i} < {k}; {i}++) {{\n");
                self.scopes[0].push((i, false));
                self.loops += 1;
                self.block(depth - 1, out);
                self.loops -= 1;
                *out += "}\n";
            }
            8 => {
                let w = self.fresh("w");
                let k = self.rng.below(4);
                *out += &format!("var {w} = 0;\nwhile ({w} < {k}) {{\n{w}++;\n");
                self.scopes[0].push((w, false));
                self.loops += 1;
                self.block(depth - 1, out);
                self.loops -= 1;
                *out += "}\n";
            }
            9 if self.loops > 0 => {
                let e = self.expr(2);
                let kw = self.rng.pick(&["break", "continue"]);
                *out += &format!("if ({e}) {kw};\n");
            }
            10 => {
                let e = self.expr(2);
                *out += &format!("if ({e}) {{ {} }}\n", self.ret());
            }
            12 => {
                let e = self.expr(2);
                *out += &format!("if ({e}) {{\n");
                self.block(depth - 1, out);
                *out += "}\n";
            }
            13 => {
                *out += "{\n";
                self.block(depth - 1, out);
                *out += "}\n";
            }
            _ => {
                let e = self.expr(2);
                *out += &format!("{e};\n");
            }
        }
    }

    fn helper(&mut self, k: usize) -> String {
        self.scopes = vec![vec![
            ("p0".to_string(), true),
            ("p1".to_string(), true),
            ("p2".to_string(), true),
        ]];
        self.loops = 0;
        let mut body = String::new();
        for _ in 0..self.rng.below(5) + 3 {
            self.stmt(2, &mut body);
        }
        body += &self.ret();
        format!("function h{k}(p0, p1, p2) {{\n{body}\n}}\n")
    }

    fn arg(&mut self) -> String {
        match self.rng.below(5) {
            0 => self
                .rng
                .pick(&[
                    "[1, -0, 2147483647]",
                    "[1, 2, 3, 4]",
                    "[, 1]",
                    "[\"5\", null]",
                ])
                .to_string(),
            1 => self
                .rng
                .pick(&["new Int32Array([1, -2, 3, 4])", "new Uint8Array([255, 1])"])
                .to_string(),
            _ => self.rng.pick(CORNERS).to_string(),
        }
    }

    /// Each helper is called three times with random corner arguments; every result or error is traced.
    fn driver(&mut self) -> String {
        let mut s = String::from("var out = [];\nvar r;\n");
        for k in 0..HELPERS {
            for _ in 0..3 {
                let args: Vec<String> = (0..3).map(|_| self.arg()).collect();
                s += &format!(
                    "try {{ r = h{k}({}); for (var j = 0; j < r.length; j++) out.push(String(r[j])); }} catch (e) {{ out.push(\"E \" + e.name + \": \" + e.message); }}\n",
                    args.join(", ")
                );
            }
        }
        s += "for (var q = 0; q < out.length; q++) console.log(out[q]);\n";
        s
    }
}

fn program(seed: u64) -> String {
    let mut g = Gen {
        rng: Rng::new(seed),
        scopes: Vec::new(),
        next: 0,
        loops: 0,
    };
    let mut src = String::new();
    for k in 0..HELPERS {
        src += &g.helper(k);
    }
    src + &g.driver()
}

/// One run of a program: its output, whether it completed, how many functions it compiled, and whether the
/// wall-clock budget ended it first (a generated program can only run long through a bug or a generator fault).
struct Run {
    out: Vec<String>,
    ok: bool,
    compiled: u64,
    deadline: bool,
}

/// Wall budget per run. Generated programs finish in milliseconds; the budget only stops a non-terminating one.
const BUDGET: Duration = Duration::from_secs(2);

fn trace(src: &str, mode: CompileMode) -> Run {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    it.deadline = Some(Instant::now() + BUDGET);
    let ok = it.run_sloppy(src).is_ok();
    Run {
        out: it.out.clone(),
        ok,
        compiled: it.cstats.compiled(),
        deadline: it.dl_hit,
    }
}

#[test]
fn fuzz_compiled_subset_matches_the_ast() {
    let seeds = std::env::var("THAW_JS_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(60);
    let (mut generated, mut compiled, mut skipped) = (0u64, 0u64, 0u64);
    for seed in 1..=seeds {
        let src = program(seed);
        if std::env::var("THAW_JS_FUZZ_DUMP")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            == Some(seed)
        {
            eprintln!("--- program for seed {seed} ---\n{src}\n--- end ---");
        }
        let off = trace(&src, CompileMode::Off);
        let on = trace(&src, CompileMode::On);
        let check = trace(&src, CompileMode::Check);
        eprintln!(
            "fuzz seed {seed}: off={} on={} check={}",
            outcome(&off),
            outcome(&on),
            outcome(&check)
        );
        if off.deadline && on.deadline && check.deadline {
            // non-terminating in every mode: allowed, but counted and bounded below
            skipped += 1;
            generated += HELPERS as u64;
            continue;
        }
        for (mode, run) in [(CompileMode::On, &on), (CompileMode::Check, &check)] {
            assert!(
                !run.deadline && !off.deadline && run.out == off.out && run.ok == off.ok,
                "REAL BUG? seed {seed}: {mode:?} differs from Off (off deadline={}, {mode:?} deadline={}, ok {} vs {})\n\
                 --- program ---\n{src}\n--- off ---\n{:#?}\n--- {mode:?} ---\n{:#?}",
                off.deadline, run.deadline, off.ok, run.ok, off.out, run.out
            );
        }
        compiled += on.compiled;
        generated += HELPERS as u64;
    }
    eprintln!("fuzz: seeds={seeds} generated={generated} compiled(on)={compiled} skipped-nonterminating={skipped}");
    assert!(
        skipped * 10 < seeds,
        "{skipped} of {seeds} seeds did not terminate: the generator is broken"
    );
    assert!(
        compiled * 10 >= generated * 9,
        "compiled {compiled} of {generated} generated functions (< 90%): the generator is broken"
    );
}

fn outcome(r: &Run) -> &'static str {
    if r.deadline {
        "deadline"
    } else if r.ok {
        "ok"
    } else {
        "threw"
    }
}
