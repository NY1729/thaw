use crate::{backup, Db, OpenOptions, Value, OPEN_CREATE, OPEN_READWRITE};

fn mem() -> Db {
    Db::open(
        ":memory:",
        OPEN_READWRITE | OPEN_CREATE,
        &OpenOptions::default(),
    )
    .unwrap()
}

fn one_int(db: &Db, sql: &str) -> i64 {
    let mut s = db.prepare(sql).unwrap();
    assert!(s.step().unwrap());
    match s.column(0) {
        Value::Int(i) => i,
        v => panic!("expected int, got {v:?}"),
    }
}

#[test]
fn create_insert_select() {
    let db = mem();
    db.exec("create table t(id integer primary key, name text)")
        .unwrap();
    let mut ins = db.prepare("insert into t(name) values (?)").unwrap();
    ins.bind(1, &Value::Text("ann".into())).unwrap();
    assert!(!ins.step().unwrap());
    assert_eq!(db.changes(), 1);
    assert_eq!(db.last_insert_rowid(), 1);

    let mut sel = db.prepare("select id, name as n from t").unwrap();
    assert_eq!(sel.column_count(), 2);
    assert_eq!(sel.column_name(0), "id");
    assert_eq!(sel.column_name(1), "n");
    assert!(sel.step().unwrap());
    assert_eq!(sel.column(0), Value::Int(1));
    assert_eq!(sel.column(1), Value::Text("ann".into()));
    assert!(!sel.step().unwrap());
}

#[test]
fn value_round_trip() {
    let db = mem();
    db.exec("create table t(v)").unwrap();
    let values = vec![
        Value::Null,
        Value::Int(0),
        Value::Int(-1),
        Value::Int(i64::MIN),
        Value::Int(i64::MAX),
        Value::Int((1 << 53) + 1),
        Value::Real(1.5),
        Value::Real(f64::INFINITY),
        Value::Text(String::new()),
        Value::Text("héllo".into()),
        Value::Text("a\0b".into()),
        Value::Blob(vec![]),
        Value::Blob(vec![0, 1, 0, 255, 0]),
    ];
    let mut ins = db.prepare("insert into t values (?)").unwrap();
    for v in &values {
        ins.bind(1, v).unwrap();
        ins.step().unwrap();
        ins.reset().unwrap();
    }
    let mut sel = db.prepare("select v from t order by rowid").unwrap();
    for v in &values {
        assert!(sel.step().unwrap());
        assert_eq!(&sel.column(0), v);
    }
    assert!(!sel.step().unwrap());
}

#[test]
fn named_and_positional_binding() {
    let db = mem();
    db.exec("create table t(a, b)").unwrap();

    let mut named = db.prepare("insert into t values (:a, @b)").unwrap();
    named.bind_name(":a", &Value::Int(1)).unwrap();
    named.bind_name("@b", &Value::Text("x".into())).unwrap();
    named.step().unwrap();
    let err = named.bind_name(":nope", &Value::Null).unwrap_err();
    assert_eq!(err.primary(), 25);

    let mut pos = db.prepare("insert into t values (?, ?)").unwrap();
    pos.bind(1, &Value::Int(2)).unwrap();
    pos.bind(2, &Value::Text("y".into())).unwrap();
    pos.step().unwrap();

    let mut sel = db.prepare("select a, b from t order by a").unwrap();
    assert!(sel.step().unwrap());
    assert_eq!(sel.column(1), Value::Text("x".into()));
    assert!(sel.step().unwrap());
    assert_eq!(sel.column(1), Value::Text("y".into()));
}

#[test]
fn transactions() {
    let db = mem();
    db.exec("create table t(a)").unwrap();
    db.exec("begin").unwrap();
    assert!(db.in_transaction());
    db.exec("insert into t values (1)").unwrap();
    db.exec("rollback").unwrap();
    assert!(!db.in_transaction());
    assert_eq!(one_int(&db, "select count(*) from t"), 0);

    db.exec("begin; insert into t values (2); commit").unwrap();
    assert!(!db.in_transaction());
    assert_eq!(one_int(&db, "select count(*) from t"), 1);
}

#[test]
fn constraint_errors() {
    let db = mem();
    db.exec("create table p(id integer primary key)").unwrap();
    db.exec("create table c(pid integer references p(id))")
        .unwrap();

    let fk = db.exec("insert into c values (99)").unwrap_err();
    assert_eq!(fk.errcode, 787);
    assert_eq!(fk.primary(), 19);
    assert_eq!(fk.errstr, "constraint failed");

    db.exec("insert into p values (1)").unwrap();
    let pk = db.exec("insert into p values (1)").unwrap_err();
    assert_eq!(pk.errcode, 1555);
    assert_eq!(pk.primary(), 19);
    assert!(pk.message.contains("UNIQUE constraint failed"));
}

#[test]
fn syntax_error_code() {
    let db = mem();
    let err = db.prepare("selec 1").err().unwrap();
    assert_eq!(err.errcode, 1);
    assert_eq!(err.primary(), 1);
}

#[test]
fn fk_disabled_by_option() {
    let opts = OpenOptions {
        foreign_keys: false,
        ..OpenOptions::default()
    };
    let db = Db::open(":memory:", OPEN_READWRITE | OPEN_CREATE, &opts).unwrap();
    db.exec("create table p(id integer primary key); create table c(pid references p(id))")
        .unwrap();
    db.exec("insert into c values (99)").unwrap();
}

#[test]
fn busy_timeout_option_accepted() {
    let opts = OpenOptions {
        timeout_ms: 250,
        ..OpenOptions::default()
    };
    let db = Db::open(":memory:", OPEN_READWRITE | OPEN_CREATE, &opts).unwrap();
    assert_eq!(one_int(&db, "select 7"), 7);
}

#[test]
fn backup_between_memory_dbs() {
    let src = mem();
    src.exec("create table t(a); insert into t values (1), (2), (3)")
        .unwrap();
    let dst = mem();
    let pages = backup(&src, &dst).unwrap();
    assert!(pages > 0);
    assert_eq!(one_int(&dst, "select sum(a) from t"), 6);
}

#[test]
fn session_changeset_round_trip() {
    let a = mem();
    let b = mem();
    for db in [&a, &b] {
        db.exec("create table t(id integer primary key, v text)")
            .unwrap();
    }
    let session = a.create_session().unwrap();
    a.exec("insert into t values (1, 'one')").unwrap();
    let cs = session.changeset().unwrap();
    drop(session);

    assert!(b.apply_changeset(&cs).unwrap());
    let mut sel = b.prepare("select v from t where id = 1").unwrap();
    assert!(sel.step().unwrap());
    assert_eq!(sel.column(0), Value::Text("one".into()));
}

#[test]
fn drop_db_with_live_statement_is_safe() {
    let db = mem();
    db.exec("create table t(a); insert into t values (5)")
        .unwrap();
    let mut stmt = db.prepare("select a from t").unwrap();
    drop(db);
    assert!(stmt.step().unwrap());
    assert_eq!(stmt.column(0), Value::Int(5));
    drop(stmt);
}
