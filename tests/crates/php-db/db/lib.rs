use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct TestConnector(AtomicUsize);
impl Connector for TestConnector {
    type Connection = usize;
    type Error = &'static str;
    fn connect(&self) -> Result<usize, Self::Error> {
        Ok(self.0.fetch_add(1, Ordering::SeqCst))
    }
}

struct TestDriver(DatabaseKind);
impl DatabaseDriver for TestDriver {
    fn kind(&self) -> DatabaseKind {
        self.0
    }
    fn ping(&self) -> Result<(), String> {
        Ok(())
    }
    fn execute(&self, _: &DbCommand) -> Result<DbResult, String> {
        Ok(DbResult::default())
    }
}

#[test]
fn parses_requested_database_families() {
    for (dsn, kind) in [
        ("mysql:host=db", DatabaseKind::Mysql),
        ("mariadb:host=db", DatabaseKind::MariaDb),
        ("pgsql:host=db", DatabaseKind::PostgreSql),
        ("mongodb://db", DatabaseKind::MongoDb),
        ("sqlsrv:Server=db", DatabaseKind::SqlServer),
        ("oci:dbname=db", DatabaseKind::Oracle),
    ] {
        assert_eq!(DatabaseKind::from_dsn(dsn).unwrap(), kind);
    }
    assert!(DatabaseKind::from_dsn("unknown:db").is_err());
}

#[test]
fn reuses_connection_and_obeys_capacity() {
    let connector = TestConnector(AtomicUsize::new(0));
    let pool = Pool::new(1).unwrap();
    let first = pool.acquire(&connector, Duration::from_millis(20)).unwrap();
    assert_eq!(*first, 0);
    drop(first);
    let reused = pool.acquire(&connector, Duration::from_millis(20)).unwrap();
    assert_eq!(*reused, 0);
    assert_eq!(connector.0.load(Ordering::SeqCst), 1);
    assert!(matches!(
        pool.acquire(&connector, Duration::from_millis(1)),
        Err(AcquireError::Timeout)
    ));
}

#[test]
fn credentials_are_redacted_from_debug_output() {
    let mut config = ConnectionConfig::new("main", "mysql:host=db").unwrap();
    config.username = Some("user".into());
    config.password = Some("secret".into());
    let debug = format!("{config:?}");
    assert!(!debug.contains("secret"));
    assert!(!debug.contains("host=db"));
}

#[test]
fn validates_configurable_result_limits() {
    let mut config = ConnectionConfig::new("main", "pgsql:host=db").unwrap();
    assert_eq!(config.max_result_rows, 1_000_000);
    assert_eq!(config.max_column_bytes, 8 * 1024 * 1024);
    config.max_result_rows = 0;
    assert!(config.validate().unwrap_err().contains("max_result_rows"));
    config.max_result_rows = 1;
    config.max_column_bytes = 0;
    assert!(config.validate().unwrap_err().contains("max_column_bytes"));
}

#[test]
fn registry_shares_handles_and_rejects_duplicate_or_wrong_drivers() {
    let registry = DatabaseRegistry::default();
    let config = ConnectionConfig::new("main", "pgsql:host=db").unwrap();
    assert!(registry
        .register(&config, Arc::new(TestDriver(DatabaseKind::Mysql)))
        .is_err());
    registry
        .register(&config, Arc::new(TestDriver(DatabaseKind::PostgreSql)))
        .unwrap();
    assert!(registry
        .register(&config, Arc::new(TestDriver(DatabaseKind::PostgreSql)))
        .is_err());
    let other_request = registry.clone();
    assert!(Arc::ptr_eq(
        &registry.get("main").unwrap(),
        &other_request.get("main").unwrap()
    ));
    assert_eq!(registry.names(), vec!["main"]);
}

#[test]
fn dsn_resolution_rejects_ambiguous_named_connections() {
    let registry = DatabaseRegistry::default();
    for name in ["first", "second"] {
        let mut config = ConnectionConfig::new(name, "pgsql:host=db").unwrap();
        config.username = Some(name.into());
        config.password = Some("secret".into());
        registry
            .register(&config, Arc::new(TestDriver(DatabaseKind::PostgreSql)))
            .unwrap();
    }
    assert!(registry
        .resolve_dsn("pgsql:host=db", None, None)
        .err()
        .unwrap()
        .contains("multiple"));
    assert_eq!(
        registry
            .resolve_dsn("pgsql:host=db", Some("first"), Some("secret"))
            .unwrap()
            .0,
        "first"
    );
}

#[test]
fn warmup_and_idle_expiration_follow_pool_options() {
    let connector = TestConnector(AtomicUsize::new(0));
    let pool = Pool::with_options(PoolOptions {
        min_connections: 2,
        max_connections: 3,
        acquire_timeout: Duration::from_millis(50),
        idle_timeout: Some(Duration::from_millis(1)),
    })
    .unwrap();
    pool.warm(&connector).unwrap();
    assert_eq!(pool.size(), (2, 2));
    let first = pool.acquire(&connector, Duration::from_millis(50)).unwrap();
    let second = pool.acquire(&connector, Duration::from_millis(50)).unwrap();
    let extra = pool.acquire(&connector, Duration::from_millis(50)).unwrap();
    drop((first, second, extra));
    assert_eq!(pool.size(), (3, 3));
    std::thread::sleep(Duration::from_millis(3));
    let _lease = pool.acquire(&connector, Duration::from_millis(50)).unwrap();
    assert_eq!(pool.size(), (2, 1));
    assert_eq!(connector.0.load(Ordering::SeqCst), 3);
}

#[test]
fn discarded_connections_are_not_reused() {
    let connector = TestConnector(AtomicUsize::new(0));
    let pool = Pool::new(1).unwrap();
    let broken = pool.acquire(&connector, Duration::from_millis(20)).unwrap();
    broken.discard();
    assert_eq!(pool.size(), (0, 0));
    assert_eq!(
        *pool.acquire(&connector, Duration::from_millis(20)).unwrap(),
        1
    );
}
