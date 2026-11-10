use super::*;

#[test]
fn converts_php_postgres_dsn_into_libpq_conninfo() {
    assert_eq!(
        postgres_conninfo("pgsql:host=db;port=5432;dbname=app").unwrap(),
        "host='db' port='5432' dbname='app'"
    );
    assert_eq!(
        postgres_conninfo("postgres://db/app").unwrap(),
        "postgres://db/app"
    );
    assert!(postgres_conninfo("pgsql:host=db;unknown=x").is_err());
}

#[test]
fn loads_libpq_symbols_when_the_runtime_library_is_installed() {
    if let Ok(api) = PgApi::load() {
        assert!(api
            .lib_version
            .is_some_and(|lib_version| unsafe { lib_version() > 0 }));
    }
}

#[test]
fn constructs_a_postgres_pool_without_connecting_until_acquire() {
    let config = ConnectionConfig::new("test", "pgsql:host=127.0.0.1;dbname=test").unwrap();
    if PgApi::load().is_ok() {
        assert!(PostgresDriver::new(&config).is_ok());
    }
}
