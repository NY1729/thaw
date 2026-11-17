//! Interp-level tests of the `__thaw_sqlite_*` natives (wire shapes, handle lifetimes, error codes).
use super::*;

/// Runs `src` in a fresh realm and returns its console lines joined with `|`.
fn run(src: &str) -> String {
    let mut it = Interp::new();
    it.run(src).unwrap_or_else(|_| panic!("script must run"));
    it.out.join("|")
}

#[test]
fn create_insert_select_round_trips_every_value_type() {
    let out = run(r#"
        var db = __thaw_sqlite_open(':memory:', '{}');
        __thaw_sqlite_exec(db, 'create table t(a integer, b text, c real, d blob, e)');
        var ins = __thaw_sqlite_prepare(db, 'insert into t values (?,?,?,?,?)');
        console.log(__thaw_sqlite_stmt_run(ins, JSON.stringify([[1,1],[2,'x'],[3,1.5],[4,{b:'0102'}],[5,null]])));
        console.log(__thaw_sqlite_stmt_run(ins, JSON.stringify([[1,2],[2,'y'],[3,{f:'inf'}],[4,{b:''}],[5,{i:'5'}]])));
        var sel = __thaw_sqlite_prepare(db, 'select a,b,c,d,e from t order by rowid');
        console.log(__thaw_sqlite_stmt_all(sel, ''));
        console.log(__thaw_sqlite_stmt_get(sel, ''));
    "#);
    assert_eq!(
        out,
        concat!(
            r#"{"changes":1,"lastInsertRowid":1}|"#,
            r#"{"changes":1,"lastInsertRowid":2}|"#,
            r#"[[1,"x",1.5,{"b":"0102"},null],[2,"y",{"f":"inf"},{"b":""},5]]|"#,
            r#"[1,"x",1.5,{"b":"0102"},null]"#,
        )
    );
}

#[test]
fn bigint_cells_are_tagged_and_flagged_per_statement() {
    let out = run(r#"
        var db = __thaw_sqlite_open(':memory:', '{}');
        var s = __thaw_sqlite_prepare(db, 'select 9007199254740993 x, 7 y');
        console.log(__thaw_sqlite_stmt_get(s, ''));
        __thaw_sqlite_stmt_set_flag(s, 'readBigInts', true);
        console.log(__thaw_sqlite_stmt_get(s, ''));
        var big = __thaw_sqlite_prepare(db, 'select ? v');
        __thaw_sqlite_stmt_set_flag(big, 'readBigInts', true);
        console.log(__thaw_sqlite_stmt_get(big, JSON.stringify([[1,{i:'-9223372036854775808'}]])));
    "#);
    assert_eq!(
        out,
        concat!(
            r#"[{"i":"9007199254740993"},7]|"#,
            r#"[{"i":"9007199254740993"},{"i":"7"}]|"#,
            r#"[{"i":"-9223372036854775808"}]"#,
        )
    );
}

#[test]
fn positional_bare_and_prefixed_named_parameters() {
    let out = run(r#"
        var db = __thaw_sqlite_open(':memory:', '{}');
        var s = __thaw_sqlite_prepare(db, 'select :a x, @b y, $c z');
        console.log(__thaw_sqlite_stmt_all(s, JSON.stringify([['a','A'],['b',2],['$c',null]])));
        console.log(__thaw_sqlite_stmt_expanded_sql(s));
        var p = __thaw_sqlite_prepare(db, 'select ?1 a, ?2 b');
        console.log(__thaw_sqlite_stmt_get(p, JSON.stringify([[1,'z'],[2,3]])));
        try { __thaw_sqlite_stmt_get(s, JSON.stringify([['zz',1]])); } catch (e) { console.log(e.code, e.message); }
        __thaw_sqlite_stmt_set_flag(s, 'allowBareNamedParameters', false);
        try { __thaw_sqlite_stmt_get(s, JSON.stringify([['a',1]])); } catch (e) { console.log(e.code, e.message); }
        __thaw_sqlite_stmt_set_flag(s, 'allowUnknownNamedParameters', true);
        console.log(__thaw_sqlite_stmt_get(s, JSON.stringify([['zz',1],[':a',7]])));
        console.log(__thaw_sqlite_stmt_source_sql(s));
    "#);
    assert_eq!(
        out,
        concat!(
            r#"[["A",2.0,null]]|"#,
            r#"select 'A' x, 2.0 y, NULL z|"#,
            r#"["z",3.0]|"#,
            r#"ERR_INVALID_STATE Unknown named parameter 'zz'|"#,
            r#"ERR_INVALID_STATE Unknown named parameter 'a'|"#,
            r#"[7.0,null,null]|"#,
            r#"select :a x, @b y, $c z"#,
        )
    );
}

#[test]
fn unbound_parameters_are_null_on_each_run() {
    let out = run(r#"
        var db = __thaw_sqlite_open(':memory:', '{}');
        var s = __thaw_sqlite_prepare(db, 'select ? a, ? b');
        console.log(__thaw_sqlite_stmt_get(s, JSON.stringify([[1,1],[2,2]])));
        console.log(__thaw_sqlite_stmt_get(s, JSON.stringify([[1,3]])));
    "#);
    assert_eq!(out, "[1.0,2.0]|[3.0,null]");
}

#[test]
fn step_iterates_rows_then_reports_done() {
    let out = run(r#"
        var db = __thaw_sqlite_open(':memory:', '{}');
        var s = __thaw_sqlite_prepare(db, 'select 1 v union all select 2');
        __thaw_sqlite_stmt_start(s, '');
        console.log(__thaw_sqlite_stmt_step(s), __thaw_sqlite_stmt_step(s), __thaw_sqlite_stmt_step(s));
        __thaw_sqlite_stmt_reset(s);
        console.log(__thaw_sqlite_stmt_step(s));
    "#);
    assert_eq!(out, "[1] [2] null|[1]");
}

#[test]
fn constraint_and_syntax_errors_carry_sqlite_codes() {
    let out = run(r#"
        var db = __thaw_sqlite_open(':memory:', '{}');
        __thaw_sqlite_exec(db, 'create table u(id integer primary key)');
        try { __thaw_sqlite_exec(db, 'insert into u values (1),(1)'); }
        catch (e) { console.log(e.code, e.errcode, e.errstr, e.message); }
        try { __thaw_sqlite_exec(db, 'bad sql'); }
        catch (e) { console.log(e.code, e.errcode, e.errstr, e.message); }
        try { __thaw_sqlite_prepare(db, 'select * from nope'); }
        catch (e) { console.log(e.code, e.errcode, e.errstr, e.message); }
    "#);
    assert_eq!(
        out,
        concat!(
            "ERR_SQLITE_ERROR 1555 constraint failed UNIQUE constraint failed: u.id|",
            "ERR_SQLITE_ERROR 1 SQL logic error near \"bad\": syntax error|",
            "ERR_SQLITE_ERROR 1 SQL logic error no such table: nope",
        )
    );
}

#[test]
fn transactions_columns_and_location() {
    let out = run(r#"
        var db = __thaw_sqlite_open(':memory:', '{}');
        __thaw_sqlite_exec(db, 'create table t(a integer)');
        __thaw_sqlite_exec(db, 'begin');
        console.log(__thaw_sqlite_is_transaction(db));
        __thaw_sqlite_exec(db, 'rollback');
        console.log(__thaw_sqlite_is_transaction(db), __thaw_sqlite_location(db), __thaw_sqlite_is_open(db));
        var c = __thaw_sqlite_prepare(db, 'select a, 1 + 1 from t');
        console.log(__thaw_sqlite_stmt_columns(c));
    "#);
    assert_eq!(
        out,
        concat!(
            "true|",
            "false null true|",
            r#"[{"column":"a","database":"main","name":"a","table":"t","type":"INTEGER"},{"column":null,"database":null,"name":"1 + 1","table":null,"type":null}]"#,
        )
    );
}

#[test]
fn close_finalizes_statements_and_later_use_is_invalid_state() {
    let out = run(r#"
        var db = __thaw_sqlite_open(':memory:', '{}');
        var s = __thaw_sqlite_prepare(db, 'select 1');
        __thaw_sqlite_close(db);
        console.log(__thaw_sqlite_is_open(db));
        try { __thaw_sqlite_exec(db, 'select 1'); } catch (e) { console.log(e.code, e.message); }
        try { __thaw_sqlite_stmt_get(s, ''); } catch (e) { console.log(e.code, e.message); }
    "#);
    assert_eq!(
        out,
        "false|ERR_INVALID_STATE database is not open|ERR_INVALID_STATE statement has been finalized"
    );
}

/// Runs the node:sqlite shim (registry builtins/sqlite.js) in a fresh realm with `sqlite` bound to its exports;
/// returns the console lines joined by newlines.
fn run_shim(body: &str) -> String {
    let shim = include_str!("../../../thaw-registry/src/registry/builtins/sqlite.js");
    let src = format!(
        "var module = {{ exports: {{}} }};\n(function (module, exports) {{\n{shim}\n}})(module, module.exports);\nvar sqlite = module.exports;\n{body}"
    );
    let mut it = Interp::new();
    it.run(&src)
        .unwrap_or_else(|_| panic!("the node:sqlite shim must run"));
    it.out.join("\n")
}

#[test]
fn node_sqlite_shim_basic_run_get_all_columns_and_null_prototype_rows() {
    let out = run_shim(
        r#"var db = new sqlite.DatabaseSync(':memory:');
db.exec('create table t(a integer primary key, b text, c blob)');
var ins = db.prepare('insert into t(b,c) values (?,?)');
console.log(JSON.stringify(ins.run('x', new Uint8Array([1, 2]))));
console.log(JSON.stringify(ins.run(null, new Uint8Array([97, 98]))));
var sel = db.prepare('select * from t order by a');
var row = sel.get();
console.log(Object.getPrototypeOf(row) === null, row.a, row.b, Array.from(row.c).join(','));
console.log(sel.all().length, sel.all()[1].b, String(sel.all()[1].c));
console.log(JSON.stringify(sel.columns().map(function (c) { return c.name + ':' + c.type; })));
console.log(db.prepare('select 1 as a where 0').get() === undefined);
"#,
    );
    assert_eq!(
        out,
        r#"{"lastInsertRowid":1,"changes":1}
{"lastInsertRowid":2,"changes":1}
true 1 x 1,2
2 null 97,98
["a:INTEGER","b:TEXT","c:BLOB"]
true"#
    );
}

#[test]
fn node_sqlite_shim_bigint_mode_range_errors_and_bind_type_errors() {
    let out = run_shim(
        r#"var db = new sqlite.DatabaseSync(':memory:');
var s = db.prepare('select 9007199254740993 as n, 7 as m');
try { s.get(); } catch (e) { console.log(e.name, e.code, e.message); }
s.setReadBigInts(true);
var r = s.get();
console.log(String(r.n), String(r.m), typeof r.n);
try { db.prepare('select ?').get(true); } catch (e) { console.log(e.name, e.code, e.message); }
try { db.prepare('select ?').get(BigInt('1180591620717411303424')); } catch (e) { console.log(e.name, e.code, e.message); }
console.log(db.prepare('select ? as x, ? as y, ? as z').get(Infinity, -Infinity, 'a\u0000b').z.length);
"#,
    );
    assert_eq!(
        out,
        r#"RangeError ERR_OUT_OF_RANGE Value is too large to be represented as a JavaScript number: 9007199254740993
9007199254740993 7 bigint
TypeError ERR_INVALID_ARG_TYPE Provided value cannot be bound to SQLite parameter 1.
TypeError ERR_INVALID_ARG_VALUE BigInt value is too large to bind.
1"#
    );
}

#[test]
fn node_sqlite_shim_sqlite_and_closed_database_errors() {
    let out = run_shim(
        r#"var db = new sqlite.DatabaseSync(':memory:');
try { db.exec('selec 1'); } catch (e) { console.log(e.code, e.errcode, e.errstr, e.message); }
db.exec('create table p(a primary key); create table c(b references p(a))');
try { db.exec('insert into c values (9)'); } catch (e) { console.log(e.code, e.errcode, e.errstr, e.message); }
try { db.prepare('select * from nope'); } catch (e) { console.log(e.code, e.errcode, e.message); }
db.close();
try { db.exec('select 1'); } catch (e) { console.log(e.code, e.message); }
try { db.close(); } catch (e) { console.log(e.code, e.message); }
console.log(db.isOpen);
"#,
    );
    assert_eq!(
        out,
        r#"ERR_SQLITE_ERROR 1 SQL logic error near "selec": syntax error
ERR_SQLITE_ERROR 787 constraint failed FOREIGN KEY constraint failed
ERR_SQLITE_ERROR 1 no such table: nope
ERR_INVALID_STATE database is not open
ERR_INVALID_STATE database is not open
false"#
    );
}

#[test]
fn node_sqlite_shim_transactions_and_finalized_statements() {
    let out = run_shim(
        r#"var db = new sqlite.DatabaseSync(':memory:');
db.exec('create table t(a)');
db.exec('begin');
console.log(db.isTransaction);
db.exec('insert into t values (1)');
db.exec('rollback');
console.log(db.isTransaction, db.prepare('select count(*) as n from t').get().n);
var s = db.prepare('select 1 as x');
db.close();
try { s.get(); } catch (e) { console.log(e.code, e.message); }
"#,
    );
    assert_eq!(
        out,
        r#"true
false 0
ERR_INVALID_STATE statement has been finalized"#
    );
}

#[test]
fn node_sqlite_shim_iterator_early_return_and_reset() {
    let out = run_shim(
        r#"var db = new sqlite.DatabaseSync(':memory:');
db.exec('create table t(a); insert into t values (1),(2),(3)');
var s = db.prepare('select a from t order by a');
var seen = [];
for (var row of s.iterate()) { seen.push(row.a); if (seen.length === 2) break; }
console.log(seen.join(','));
var it = s.iterate();
console.log(it.next().value.a, it.return().done, it.next().done);
console.log(s.all().length);
"#,
    );
    assert_eq!(
        out,
        r#"1,2
1 true true
3"#
    );
}

#[test]
fn node_sqlite_shim_named_parameters_and_unknown_names() {
    let out = run_shim(
        r#"var db = new sqlite.DatabaseSync(':memory:');
console.log(JSON.stringify(db.prepare('select :a as a, @b as b, $c as c').get({ a: 1, b: 2, c: 3 })));
var s = db.prepare('select :a as a');
try { s.get({ zz: 1 }); } catch (e) { console.log(e.code, e.message); }
s.setAllowUnknownNamedParameters(true);
console.log(JSON.stringify(s.get({ zz: 1 })));
console.log(JSON.stringify(db.prepare('select ?1 as a, ?2 as b').get('p', 'q')));
console.log(JSON.stringify(db.prepare('select :x as x').get({ x: null })));
"#,
    );
    assert_eq!(
        out,
        r#"{"a":1,"b":2,"c":3}
ERR_INVALID_STATE Unknown named parameter 'zz'
{"a":null}
{"a":"p","b":"q"}
{"x":null}"#
    );
}

#[test]
fn node_sqlite_shim_blobs_constants_and_constructor_argument_errors() {
    let out = run_shim(
        r#"var db = new sqlite.DatabaseSync(':memory:');
var s = db.prepare('select ? as b, typeof(?) as t');
var r = s.get(new Uint8Array([0, 255]), new Uint8Array([0, 255]));
console.log(Array.from(r.b).join(','), r.t);
console.log(JSON.stringify(sqlite.constants));
console.log(Object.keys(sqlite).join(','));
console.log(db.prepare('select 5 as a').sourceSQL, '|', db.prepare('select 5 as a').expandedSQL);
try { new sqlite.DatabaseSync(); } catch (e) { console.log(e.name, e.code, e.message); }
try { sqlite.DatabaseSync(':memory:'); } catch (e) { console.log(e.name, e.code, e.message); }
try { new sqlite.DatabaseSync(':memory:', { readOnly: 1 }); } catch (e) { console.log(e.name, e.code, e.message); }
try { new sqlite.StatementSync(); } catch (e) { console.log(e.name, e.code, e.message); }
try { db.exec(5); } catch (e) { console.log(e.name, e.code, e.message); }
"#,
    );
    assert_eq!(
        out,
        r#"0,255 blob
{"SQLITE_CHANGESET_OMIT":0,"SQLITE_CHANGESET_REPLACE":1,"SQLITE_CHANGESET_ABORT":2,"SQLITE_CHANGESET_DATA":1,"SQLITE_CHANGESET_NOTFOUND":2,"SQLITE_CHANGESET_CONFLICT":3,"SQLITE_CHANGESET_CONSTRAINT":4,"SQLITE_CHANGESET_FOREIGN_KEY":5}
DatabaseSync,StatementSync,constants,backup
select 5 as a | select 5 as a
TypeError ERR_INVALID_ARG_TYPE The "path" argument must be a string, Uint8Array, or URL without null bytes.
TypeError ERR_CONSTRUCT_CALL_REQUIRED Cannot call constructor without `new`
TypeError ERR_INVALID_ARG_TYPE The "options.readOnly" argument must be a boolean.
Error ERR_ILLEGAL_CONSTRUCTOR Illegal constructor
TypeError ERR_INVALID_ARG_TYPE The "sql" argument must be a string."#
    );
}

#[test]
fn node_sqlite_shim_return_arrays_location_and_columns_of_a_subquery() {
    let out = run_shim(
        r#"var db = new sqlite.DatabaseSync(':memory:');
var s = db.prepare('select :n as n');
s.setReturnArrays(true);
console.log(JSON.stringify(s.get({ n: 5 })));
s.setReturnArrays(false);
console.log(JSON.stringify(s.all({ n: 6 })));
var u = db.prepare('select * from (select 1 as a union all select 2)');
console.log(u.all().length, JSON.stringify(u.columns()));
console.log(db.location(), db.prepare('select 1').iterate().next().done);
"#,
    );
    assert_eq!(
        out,
        r#"[5]
[{"n":6}]
2 [{"column":null,"database":null,"name":"a","table":null,"type":null}]
null false"#
    );
}
