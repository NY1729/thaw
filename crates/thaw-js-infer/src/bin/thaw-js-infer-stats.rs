//! `thaw-js-infer-stats [--hint] [--optimistic] [--detail] <file.js>...` : per-file inference census.
use thaw_js_infer::{analyze_source, Class, Config, Reason};

fn main() {
    let mut cfg = Config::default();
    let mut dts_root: Option<String> = None;
    let (mut detail, mut annotate, mut files) = (false, false, Vec::new());
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--hint" => cfg.hint_params = true,
            "--optimistic" => cfg.optimistic_uninit = true,
            "--detail" => detail = true,
            "--annotate" => annotate = true,
            a if a.starts_with("--dts-root=") => {
                dts_root = Some(a["--dts-root=".len()..].to_string())
            }
            _ => files.push(a),
        }
    }
    if annotate {
        // `--annotate <file.js>`: print the TypeScript-annotated source to stdout
        let src = std::fs::read_to_string(&files[0]).unwrap();
        print!("{}", thaw_js_infer::annotate(&src, cfg.clone()).unwrap());
        return;
    }
    println!("pkg\tfns\tconcrete\tnullable\tunion\tany\tdead\tclosed\tany_exprs/exprs\tpasses\tdts_matched\tdts_typed\tthis_matched\tthis_typed");
    let mut tot = thaw_js_infer::Summary::default();
    for path in files {
        let name = std::path::Path::new(&path)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let src = std::fs::read_to_string(&path).unwrap();
        let mut cfg = cfg.clone();
        if let Some(root) = &dts_root {
            let dir = format!("{root}/{name}/thaw_modules");
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    let p = e.path().join("package.d.ts");
                    if let Ok(t) = std::fs::read_to_string(&p) {
                        match thaw_js_infer::dts::DtsIndex::parse(&t) {
                            Ok(i) => cfg.dts = Some(std::sync::Arc::new(i)),
                            Err(e) => eprintln!("{name}: d.ts parse error: {e}"),
                        }
                    }
                }
            }
        }
        // deep ASTs: run on a big stack
        let r = std::thread::Builder::new()
            .stack_size(512 << 20)
            .spawn(move || analyze_source(&src, cfg))
            .unwrap()
            .join()
            .unwrap();
        let (fns, s) = match r {
            Ok(x) => x,
            Err(e) => {
                println!("{name}\tERROR\t{e}");
                continue;
            }
        };
        let c = |k: Class| s.by_class.get(&k).copied().unwrap_or(0);
        println!(
            "{name}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}/{}\t{}\t{}\t{}\t{}\t{}",
            s.functions,
            c(Class::Concrete),
            c(Class::Nullable),
            c(Class::Union),
            c(Class::Any),
            c(Class::Dead),
            s.closed,
            s.any_exprs,
            s.exprs,
            s.passes,
            s.dts_matched,
            s.dts_typed,
            s.this_matched,
            s.this_typed
        );
        let hist = |m: &std::collections::BTreeMap<Reason, usize>| {
            let mut v: Vec<_> = m.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1));
            v.iter()
                .take(8)
                .map(|(r, n)| format!("{r:?}={n}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        println!("  fn-any-reasons: {}", hist(&s.fn_reason));
        println!("  slot-any-reasons: {}", hist(&s.slot_reason));
        if detail {
            for f in fns.iter().filter(|f| f.closed).take(40) {
                println!(
                    "  CLOSED {}@{} ({}) -> {}",
                    f.name,
                    f.lo,
                    f.slots
                        .iter()
                        .map(|s| format!("{}:{}", s.0, s.1))
                        .collect::<Vec<_>>()
                        .join(", "),
                    f.ret
                );
            }
        }
        macro_rules! add { ($($f:ident),*) => {$( tot.$f += s.$f; )*} }
        add!(
            functions,
            closed,
            slots,
            exprs,
            any_exprs,
            dts_matched,
            dts_typed,
            this_matched,
            this_typed
        );
        for (k, v) in &s.by_class {
            *tot.by_class.entry(*k).or_default() += v;
        }
        for (k, v) in &s.dts_reason {
            *tot.dts_reason.entry(*k).or_default() += v;
        }
        for (k, v) in &s.fn_reason {
            *tot.fn_reason.entry(*k).or_default() += v;
        }
        for (k, v) in &s.slot_reason {
            *tot.slot_reason.entry(*k).or_default() += v;
        }
        for (k, v) in &s.slots_by_class {
            *tot.slots_by_class.entry(*k).or_default() += v;
        }
    }
    let c = |k: Class| tot.by_class.get(&k).copied().unwrap_or(0);
    println!(
        "TOTAL\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}/{}\t-\t{}\t{}\t{}\t{}",
        tot.functions,
        c(Class::Concrete),
        c(Class::Nullable),
        c(Class::Union),
        c(Class::Any),
        c(Class::Dead),
        tot.closed,
        tot.any_exprs,
        tot.exprs,
        tot.dts_matched,
        tot.dts_typed,
        tot.this_matched,
        tot.this_typed
    );
    let mut v: Vec<_> = tot.fn_reason.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1));
    println!(
        "TOTAL fn-any-reasons: {}",
        v.iter()
            .map(|(r, n)| format!("{r:?}={n}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let mut v: Vec<_> = tot.slot_reason.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1));
    println!(
        "TOTAL slot-any-reasons: {}",
        v.iter()
            .map(|(r, n)| format!("{r:?}={n}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let mut v: Vec<_> = tot.dts_reason.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1));
    println!(
        "TOTAL dts-matched-but-any reasons: {}",
        v.iter()
            .map(|(r, n)| format!("{r:?}={n}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!("TOTAL slots by class: {:?}", tot.slots_by_class);
}
