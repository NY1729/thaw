// The whole-bundle blocker gate (designs/blocked-bundles-gate.md, PR1). Each fixture bundle is scanned with the
// build's verdict (`thaw_registry::scan_bundle_for_build`, the call `thaw-js-scan` makes), and its blocker rows must
// be exactly the rows `tools/blocked-bundles.txt` lists for that bundle. Rows of bundles without a fixture are
// checked by the nightly full-set run instead.

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The reasons a row may carry. `scanner-precision` and `missing-node-api` are open rows that the D1 gate forbids.
const REASONS: [&str; 4] = [
    "other-runtime-only",
    "native-addon-dlopen",
    "scanner-precision",
    "missing-node-api",
];

struct Fixture {
    name: String,
    sha256: String,
    installed: HashSet<String>,
    addon: bool,
}

fn repo_file(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn data_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
}

fn read_manifest() -> Vec<Fixture> {
    let text = std::fs::read_to_string(repo_file("tools/blocked-bundles-manifest.tsv")).unwrap();
    data_lines(&text)
        .map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            assert_eq!(cols.len(), 4, "manifest row needs 4 columns: {line}");
            let installed = cols[2]
                .strip_prefix("installed=")
                .unwrap_or_else(|| panic!("third manifest column must be installed=: {line}"));
            let addon = match cols[3] {
                "addon=yes" => true,
                "addon=no" => false,
                other => {
                    panic!("fourth manifest column must be addon=yes or addon=no, got {other:?}")
                }
            };
            Fixture {
                name: cols[0].to_string(),
                sha256: cols[1].to_string(),
                installed: installed
                    .split(',')
                    .filter(|p| !p.is_empty())
                    .map(str::to_string)
                    .collect(),
                addon,
            }
        })
        .collect()
}

/// `bundle<TAB>blocker` -> reason, for every listed row. Rows must be unique and sorted by that key.
fn read_list() -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(repo_file("tools/blocked-bundles.txt")).unwrap();
    let mut rows = BTreeMap::new();
    let mut previous: Option<String> = None;
    for line in data_lines(&text) {
        let cols: Vec<&str> = line.split('\t').collect();
        assert!(
            cols.len() >= 3,
            "row needs bundle, blocker and reason: {line}"
        );
        assert!(
            REASONS.contains(&cols[2]),
            "unknown reason {:?} in row: {line}",
            cols[2]
        );
        let key = format!("{}\t{}", cols[0], cols[1]);
        if let Some(prev) = &previous {
            assert!(
                *prev < key,
                "rows must be unique and sorted by bundle and blocker: {line}"
            );
        }
        previous = Some(key.clone());
        rows.insert(key, cols[2].to_string());
    }
    rows
}

#[test]
fn blocked_bundles_fixtures_scan_to_exactly_the_listed_rows() {
    let manifest = read_manifest();
    let list = read_list();
    let dir = repo_file("crates/thaw-cli/tests/fixtures/bundles");

    let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let in_manifest: BTreeSet<String> = manifest
        .iter()
        .map(|f| format!("{}.js.gz", f.name))
        .collect();
    assert_eq!(
        on_disk, in_manifest,
        "fixture files and tools/blocked-bundles-manifest.tsv disagree (run tools/refresh-blocked-bundle-fixtures.sh)"
    );

    let mut problems = Vec::new();
    for fixture in &manifest {
        let gz = std::fs::read(dir.join(format!("{}.js.gz", fixture.name))).unwrap();
        let digest: String = Sha256::digest(&gz)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            digest, fixture.sha256,
            "fixture {} drifted from its manifest hash (refresh with tools/refresh-blocked-bundle-fixtures.sh)",
            fixture.name
        );

        let mut source = String::new();
        GzDecoder::new(&gz[..])
            .read_to_string(&mut source)
            .unwrap_or_else(|e| panic!("fixture {} is not valid gzip text: {e}", fixture.name));
        // the served peers a build of a program importing this bundle has (see `served_peers_of`)
        let served: HashSet<String> = thaw_registry::served_peers_of(&source, &fixture.installed)
            .unwrap_or_default()
            .into_iter()
            .collect();
        let report = thaw_registry::scan_bundle_for_build(
            &source,
            fixture.addon,
            &fixture.installed,
            &served,
        )
        .unwrap_or_else(|e| panic!("scan of {} failed: {e}", fixture.name));

        let actual: BTreeSet<String> = report
            .blockers
            .keys()
            .map(|blocker| format!("{}\t{blocker}", fixture.name))
            .collect();
        let expected: BTreeSet<String> = list
            .keys()
            .filter(|key| key.split('\t').next() == Some(fixture.name.as_str()))
            .cloned()
            .collect();
        for row in actual.difference(&expected) {
            problems.push(format!(
                "scan produced an unlisted blocker {row:?}: list it in tools/blocked-bundles.txt with its reason, or fix the regression"
            ));
        }
        for row in expected.difference(&actual) {
            problems.push(format!(
                "remove this line (shrink): {row:?} no longer blocks the bundle"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "blocked-bundles gate:\n  {}",
        problems.join("\n  ")
    );
}
