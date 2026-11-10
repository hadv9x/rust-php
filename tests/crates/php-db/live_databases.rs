use php_db::{
    ConnectionConfig, DatabaseDriver, DatabaseKind, DbCommand, DbErrorInfo, DbValue, MongoAction,
    MongoDriver, MysqlDriver, OdbcDriver, PostgresDriver,
};
use std::sync::Arc;

fn run_live_backend(prefix: &str, expected_kind: DatabaseKind) {
    let dsn_key = format!("PHP_RUST_TEST_{}_DSN", prefix);
    let dsn = std::env::var(&dsn_key).unwrap_or_default();
    if dsn.trim().is_empty() {
        if prefix != "ORACLE" && std::env::var_os("PHP_RUST_REQUIRE_LIVE_DBS").is_some() {
            panic!("{prefix} live database check is required but {dsn_key} is empty");
        }
        eprintln!("skip {prefix}: set {dsn_key} to run the live database check");
        return;
    }
    let mut config = ConnectionConfig::new(prefix.to_ascii_lowercase(), dsn)
        .unwrap_or_else(|error| panic!("{prefix} DSN: {error}"));
    assert_eq!(
        config.kind, expected_kind,
        "{prefix} DSN has the wrong scheme"
    );
    config.username = std::env::var(format!("PHP_RUST_TEST_{prefix}_USERNAME")).ok();
    config.password = std::env::var(format!("PHP_RUST_TEST_{prefix}_PASSWORD")).ok();
    config.driver = std::env::var(format!("PHP_RUST_TEST_{prefix}_ODBC_DRIVER")).ok();
    config.pool.max_connections = 2;
    config.pool.acquire_timeout = std::time::Duration::from_secs(10);
    let driver: Arc<dyn DatabaseDriver> = match expected_kind {
        DatabaseKind::PostgreSql => Arc::new(
            PostgresDriver::new(&config)
                .unwrap_or_else(|error| panic!("{prefix} driver init: {error}")),
        ),
        DatabaseKind::Mysql | DatabaseKind::MariaDb => Arc::new(
            MysqlDriver::new(&config)
                .unwrap_or_else(|error| panic!("{prefix} driver init: {error}")),
        ),
        DatabaseKind::SqlServer | DatabaseKind::Oracle => Arc::new(
            OdbcDriver::new(&config)
                .unwrap_or_else(|error| panic!("{prefix} driver init: {error}")),
        ),
        DatabaseKind::MongoDb => Arc::new(
            MongoDriver::new(&config)
                .unwrap_or_else(|error| panic!("{prefix} driver init: {error}")),
        ),
    };
    driver
        .ping()
        .unwrap_or_else(|error| panic!("{prefix} live ping failed: {error}"));
    if matches!(
        expected_kind,
        DatabaseKind::Mysql
            | DatabaseKind::MariaDb
            | DatabaseKind::PostgreSql
            | DatabaseKind::SqlServer
    ) {
        assert!(
            driver
                .client_version()
                .is_some_and(|version| !version.is_empty()),
            "{prefix} client library version should be available"
        );
    }
    if expected_kind == DatabaseKind::MongoDb {
        verify_live_mongodb(&driver);
        eprintln!("PASS {prefix}: connection, ping, document counts, bulk insert, single-document update/replace/delete, and replica-set rollback succeeded (MongoDB has no PDO SQL surface)");
        return;
    }

    verify_live_sql_backend(prefix, expected_kind, &driver);
    eprintln!("PASS {prefix}: connection, prepared binding, metadata, diagnostics, and supported transaction/result-set checks succeeded");
}

fn verify_live_mongodb(driver: &Arc<dyn DatabaseDriver>) {
    let mut session = driver
        .start_session()
        .expect("MongoDB session should start");
    session
        .begin_transaction()
        .expect("MongoDB transaction should start; configure a replica set");
    let marker = format!("live-check-{}", std::process::id());
    session
        .execute(&DbCommand::Mongo {
            database: "php_rust_integration".into(),
            collection: "rollback_probe".into(),
            action: MongoAction::InsertOne {
                document: DbValue::Document(vec![(
                    "marker".into(),
                    DbValue::String(marker.clone()),
                )]),
                options: DbValue::Document(Vec::new()),
            },
        })
        .expect("MongoDB insert inside transaction should succeed");
    let inserted = session
        .execute(&DbCommand::Mongo {
            database: "php_rust_integration".into(),
            collection: "rollback_probe".into(),
            action: MongoAction::InsertMany {
                documents: vec![
                    DbValue::Document(vec![
                        ("batch".into(), DbValue::String(marker.clone())),
                        ("item".into(), DbValue::Integer(1)),
                    ]),
                    DbValue::Document(vec![
                        ("batch".into(), DbValue::String(marker.clone())),
                        ("item".into(), DbValue::Integer(2)),
                    ]),
                ],
                options: DbValue::Document(Vec::new()),
            },
        })
        .expect("MongoDB insertMany inside transaction should succeed");
    assert_eq!(inserted.affected_rows, 2);
    let count = session
        .execute(&DbCommand::Mongo {
            database: "php_rust_integration".into(),
            collection: "rollback_probe".into(),
            action: MongoAction::CountDocuments {
                filter: DbValue::Document(vec![("batch".into(), DbValue::String(marker.clone()))]),
                estimated: false,
                options: DbValue::Document(Vec::new()),
            },
        })
        .expect("MongoDB countDocuments inside transaction should succeed");
    assert_eq!(count.affected_rows, 2);
    let update = session
        .execute(&DbCommand::Mongo {
            database: "php_rust_integration".into(),
            collection: "rollback_probe".into(),
            action: MongoAction::UpdateOne {
                filter: DbValue::Document(vec![("marker".into(), DbValue::String(marker.clone()))]),
                update: DbValue::Document(vec![(
                    "$set".into(),
                    DbValue::Document(vec![("updated".into(), DbValue::Bool(true))]),
                )]),
                options: DbValue::Document(Vec::new()),
            },
        })
        .expect("MongoDB updateOne inside transaction should succeed");
    assert_eq!(update.matched_rows, Some(1));
    assert_eq!(update.modified_rows, Some(1));
    let replacement = session
        .execute(&DbCommand::Mongo {
            database: "php_rust_integration".into(),
            collection: "rollback_probe".into(),
            action: MongoAction::ReplaceOne {
                filter: DbValue::Document(vec![("marker".into(), DbValue::String(marker.clone()))]),
                replacement: DbValue::Document(vec![
                    ("marker".into(), DbValue::String(marker.clone())),
                    ("replaced".into(), DbValue::Bool(true)),
                ]),
                options: DbValue::Document(Vec::new()),
            },
        })
        .expect("MongoDB replaceOne inside transaction should succeed");
    assert_eq!(replacement.matched_rows, Some(1));
    assert_eq!(replacement.modified_rows, Some(1));
    let delete = session
        .execute(&DbCommand::Mongo {
            database: "php_rust_integration".into(),
            collection: "rollback_probe".into(),
            action: MongoAction::DeleteOne {
                filter: DbValue::Document(vec![("marker".into(), DbValue::String(marker.clone()))]),
                options: DbValue::Document(Vec::new()),
            },
        })
        .expect("MongoDB deleteOne inside transaction should succeed");
    assert_eq!(delete.affected_rows, 1);
    session
        .rollback_transaction()
        .expect("MongoDB transaction rollback should succeed");
    let result = driver
        .execute(&DbCommand::Mongo {
            database: "php_rust_integration".into(),
            collection: "rollback_probe".into(),
            action: MongoAction::Find {
                filter: DbValue::Document(vec![("marker".into(), DbValue::String(marker))]),
                limit: Some(1),
                options: DbValue::Document(Vec::new()),
            },
        })
        .expect("MongoDB query after transaction rollback should succeed");
    assert!(
        result.rows.is_empty(),
        "MongoDB rollback must remove the transaction's insert"
    );
}

fn verify_live_sql_backend(prefix: &str, kind: DatabaseKind, driver: &Arc<dyn DatabaseDriver>) {
    let (sql, parameter) = match kind {
        DatabaseKind::PostgreSql => ("SELECT $1::int AS value", DbValue::Integer(42)),
        DatabaseKind::Mysql | DatabaseKind::MariaDb | DatabaseKind::SqlServer => {
            ("SELECT ? AS value", DbValue::Integer(42))
        }
        DatabaseKind::Oracle => ("SELECT ? AS value FROM DUAL", DbValue::Integer(42)),
        DatabaseKind::MongoDb => unreachable!(),
    };
    let mut session = driver
        .start_session()
        .unwrap_or_else(|error| panic!("{prefix} session: {error}"));
    let prepared = session
        .execute(&DbCommand::PreparedSql {
            statement: sql.into(),
            parameters: vec![parameter],
        })
        .unwrap_or_else(|error| panic!("{prefix} prepared query: {error}"));
    assert_eq!(
        prepared.columns.len(),
        1,
        "{prefix} prepared query column count"
    );
    assert_eq!(prepared.rows.len(), 1, "{prefix} prepared query row count");
    assert_eq!(
        prepared.rows[0][0],
        DbValue::String("42".into()),
        "{prefix} prepared parameter value"
    );
    assert!(
        prepared
            .column_metadata
            .first()
            .and_then(|meta| meta.native_type.as_ref())
            .is_some(),
        "{prefix} should return native column type metadata"
    );

    let bad_sql = match kind {
        DatabaseKind::Oracle => "SELECT FROM DUAL",
        _ => "SELECT FROM",
    };
    let error = driver
        .execute(&DbCommand::Sql {
            statement: bad_sql.into(),
            parameters: Vec::new(),
        })
        .expect_err("intentionally invalid SQL must report a backend error");
    let info = DbErrorInfo::decode(&error);
    assert_eq!(
        info.sql_state.len(),
        5,
        "{prefix} SQLSTATE should be five characters: {info:?}"
    );
    assert_ne!(
        info.sql_state, "HY000",
        "{prefix} should retain the backend SQLSTATE: {info:?}"
    );
    if matches!(
        kind,
        DatabaseKind::Mysql
            | DatabaseKind::MariaDb
            | DatabaseKind::SqlServer
            | DatabaseKind::Oracle
    ) {
        assert!(
            info.driver_code.is_some_and(|code| code != 0),
            "{prefix} should retain its native driver code: {info:?}"
        );
    }

    if kind != DatabaseKind::Oracle {
        let begin = match kind {
            DatabaseKind::Mysql | DatabaseKind::MariaDb => "START TRANSACTION",
            DatabaseKind::PostgreSql => "BEGIN",
            DatabaseKind::SqlServer => "BEGIN TRANSACTION",
            _ => unreachable!(),
        };
        session
            .execute(&DbCommand::Sql {
                statement: begin.into(),
                parameters: Vec::new(),
            })
            .unwrap_or_else(|error| panic!("{prefix} begin transaction: {error}"));
        session
            .execute(&DbCommand::Sql {
                statement: "ROLLBACK".into(),
                parameters: Vec::new(),
            })
            .unwrap_or_else(|error| panic!("{prefix} rollback transaction: {error}"));
    }

    if matches!(
        kind,
        DatabaseKind::Mysql
            | DatabaseKind::MariaDb
            | DatabaseKind::PostgreSql
            | DatabaseKind::SqlServer
    ) {
        let multi = session
            .execute(&DbCommand::Sql {
                statement: "SELECT 1 AS result_a; SELECT 2 AS result_b".into(),
                parameters: Vec::new(),
            })
            .unwrap_or_else(|error| panic!("{prefix} multi-result query: {error}"));
        assert_eq!(
            multi.additional_result_sets.len(),
            1,
            "{prefix} should expose its second result set"
        );
        assert_eq!(
            multi.additional_result_sets[0].rows.len(),
            1,
            "{prefix} second result set row count"
        );
    }
}

#[test]
fn live_mysql() {
    run_live_backend("MYSQL", DatabaseKind::Mysql);
}
#[test]
fn live_mariadb() {
    run_live_backend("MARIADB", DatabaseKind::MariaDb);
}
#[test]
fn live_postgresql() {
    run_live_backend("POSTGRESQL", DatabaseKind::PostgreSql);
}
#[test]
fn live_mongodb() {
    run_live_backend("MONGODB", DatabaseKind::MongoDb);
}
#[test]
fn live_sql_server() {
    run_live_backend("SQLSERVER", DatabaseKind::SqlServer);
}
#[test]
fn live_oracle() {
    run_live_backend("ORACLE", DatabaseKind::Oracle);
}
