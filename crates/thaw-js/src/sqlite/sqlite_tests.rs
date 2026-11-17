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
