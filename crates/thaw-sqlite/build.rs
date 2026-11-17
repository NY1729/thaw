// Compiles the vendored SQLite amalgamation with the compile options Node 22
// uses for node:sqlite. Sessions and the preupdate hook are needed by
// createSession/applyChangeset; COLUMN_METADATA by StatementSync.columns().
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    cc::Build::new()
        .file("vendor/sqlite3.c")
        .include("vendor")
        .opt_level(2)
        .warnings(false)
        .define("SQLITE_THREADSAFE", "1")
        .define("SQLITE_ENABLE_SESSION", None)
        .define("SQLITE_ENABLE_PREUPDATE_HOOK", None)
        .define("SQLITE_ENABLE_COLUMN_METADATA", None)
        .define("SQLITE_ENABLE_LOAD_EXTENSION", None)
        .define("SQLITE_ENABLE_FTS3", None)
        .define("SQLITE_ENABLE_FTS3_PARENTHESIS", None)
        .define("SQLITE_ENABLE_FTS5", None)
        .define("SQLITE_ENABLE_RTREE", None)
        .define("SQLITE_ENABLE_DBSTAT_VTAB", None)
        .define("SQLITE_ENABLE_DBPAGE_VTAB", None)
        .define("SQLITE_ENABLE_MATH_FUNCTIONS", None)
        .define("SQLITE_ENABLE_PERCENTILE", None)
        .define("SQLITE_ENABLE_UNLOCK_NOTIFY", None)
        .define("SQLITE_SECURE_DELETE", None)
        .define("SQLITE_DEFAULT_RECURSIVE_TRIGGERS", None)
        .compile("thaw_sqlite3");
}
