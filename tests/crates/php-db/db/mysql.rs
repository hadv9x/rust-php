use super::*;

#[test]
fn converts_mysql_and_mariadb_dsn_fields() {
    let config =
        ConnectionConfig::new("mysql", "mysql:host=127.0.0.1;port=3307;dbname=app").unwrap();
    let options = mysql_options(&config).unwrap();
    assert_eq!(unsafe { c_string(options.host.as_ptr()) }, "127.0.0.1");
    assert_eq!(options.port, 3307);
    assert_eq!(unsafe { c_string(options.database.as_ptr()) }, "app");
    let config = ConnectionConfig::new("maria", "mariadb:dbname=app").unwrap();
    assert_eq!(config.kind, DatabaseKind::MariaDb);
    assert!(mysql_options(&ConnectionConfig::new("bad", "mysql:unknown=value").unwrap()).is_err());
}

#[test]
fn loads_mysql_client_symbols_when_installed() {
    if let Ok(api) = MysqlApi::load() {
        assert!(api
            .client_info
            .is_some_and(|client_info| unsafe { !client_info().is_null() }));
    }
}

#[test]
fn constructs_mysql_and_mariadb_pools_without_connecting_until_acquire() {
    for dsn in [
        "mysql:host=127.0.0.1;dbname=test",
        "mariadb:host=127.0.0.1;dbname=test",
    ] {
        let config = ConnectionConfig::new("test", dsn).unwrap();
        if MysqlApi::load().is_ok() {
            assert!(MysqlDriver::new(&config).is_ok());
        }
    }
}
