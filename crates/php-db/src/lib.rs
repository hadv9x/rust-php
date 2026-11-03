//! Shared database configuration and bounded connection-pool primitives.
//!
//! Concrete database clients implement [`Connector`] and are registered by the
//! server integration. The pool blocks only the caller waiting for a slot, so
//! it is intended to be used from the PHP VM's blocking request worker.

mod postgres;
mod mysql;
mod odbc;
mod mongo;

pub use postgres::PostgresDriver;
pub use mysql::MysqlDriver;
pub use odbc::OdbcDriver;
pub use mongo::MongoDriver;

use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Condvar, Mutex, MutexGuard, RwLock},
    time::{Duration, Instant},
};

/// Database families supported by the server's connection configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatabaseKind {
    Mysql,
    MariaDb,
    PostgreSql,
    MongoDb,
    SqlServer,
    Oracle,
}

impl DatabaseKind {
    /// Parse PHP/PDO-style DSN prefixes and common URI schemes.
    pub fn from_dsn(dsn: &str) -> Result<Self, String> {
        let prefix = dsn.split_once(':').map(|(prefix, _)| prefix)
            .or_else(|| dsn.split_once("//").map(|(scheme, _)| scheme))
            .ok_or_else(|| "database DSN must include a driver scheme".to_owned())?
            .to_ascii_lowercase();
        match prefix.as_str() {
            "mysql" => Ok(Self::Mysql),
            "mariadb" => Ok(Self::MariaDb),
            "pgsql" | "postgres" | "postgresql" => Ok(Self::PostgreSql),
            "mongodb" | "mongodb+srv" => Ok(Self::MongoDb),
            "sqlsrv" | "sqlserver" | "mssql" => Ok(Self::SqlServer),
            "oci" | "oracle" => Ok(Self::Oracle),
            _ => Err(format!("unsupported database DSN scheme '{prefix}'")),
        }
    }
}

/// Pool limits for one configured database connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolOptions {
    pub min_connections: usize,
    pub max_connections: usize,
    pub acquire_timeout: Duration,
    pub idle_timeout: Option<Duration>,
}

impl Default for PoolOptions {
    fn default() -> Self {
        Self { min_connections: 0, max_connections: 10, acquire_timeout: Duration::from_secs(5), idle_timeout: Some(Duration::from_secs(600)) }
    }
}

impl PoolOptions {
    pub fn validate(self) -> Result<Self, String> {
        if self.max_connections == 0 { return Err("database pool max_connections must be greater than zero".into()); }
        if self.min_connections > self.max_connections { return Err("database pool min_connections cannot exceed max_connections".into()); }
        if self.acquire_timeout.is_zero() { return Err("database pool acquire_timeout must be greater than zero".into()); }
        Ok(self)
    }
}

/// Named connection definition. Debug intentionally omits DSN and credentials.
#[derive(Clone)]
pub struct ConnectionConfig {
    pub name: String,
    pub kind: DatabaseKind,
    pub dsn: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub driver: Option<String>,
    pub pool: PoolOptions,
    /// Maximum rows materialized per result set.
    pub max_result_rows: usize,
    /// Maximum bytes collected for one ODBC text column.
    pub max_column_bytes: usize,
}

impl ConnectionConfig {
    pub fn new(name: impl Into<String>, dsn: impl Into<String>) -> Result<Self, String> {
        let name = name.into();
        let dsn = dsn.into();
        if name.trim().is_empty() { return Err("database connection name cannot be empty".into()); }
        let kind = DatabaseKind::from_dsn(&dsn)?;
        Ok(Self { name, kind, dsn, username: None, password: None, driver: None, pool: PoolOptions::default(), max_result_rows: 1_000_000, max_column_bytes: 8 * 1024 * 1024 })
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() { return Err("database connection name cannot be empty".into()); }
        let detected = DatabaseKind::from_dsn(&self.dsn)?;
        if detected != self.kind { return Err("database connection kind does not match its DSN".into()); }
        self.pool.validate()?;
        if self.max_result_rows == 0 { return Err("database max_result_rows must be greater than zero".into()); }
        if self.max_result_rows > i64::MAX as usize { return Err("database max_result_rows cannot exceed signed 64-bit range".into()); }
        if self.max_column_bytes == 0 { return Err("database max_column_bytes must be greater than zero".into()); }
        Ok(())
    }
}

impl fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionConfig").field("name", &self.name).field("kind", &self.kind)
            .field("dsn", &"[redacted]").field("username", &self.username.as_ref().map(|_| "[redacted]"))
            .field("password", &self.password.as_ref().map(|_| "[redacted]")).field("pool", &self.pool).finish()
    }
}

/// Typed values shared by SQL and document database adapters.
#[derive(Clone, Debug, PartialEq)]
pub enum DbValue {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<DbValue>),
    Document(Vec<(String, DbValue)>),
}

/// Driver-neutral command passed from the PHP extension layer to a backend.
#[derive(Clone, Debug, PartialEq)]
pub enum DbCommand {
    Sql { statement: String, parameters: Vec<DbValue> },
    /// SQL with values passed through the backend's native parameter API.
    PreparedSql { statement: String, parameters: Vec<DbValue> },
    Mongo { database: String, collection: String, action: MongoAction },
}

/// Supported collection operations for MongoDB adapters.
#[derive(Clone, Debug, PartialEq)]
pub enum MongoAction {
    Find { filter: DbValue, limit: Option<u64>, options: DbValue },
    CountDocuments { filter: DbValue, estimated: bool, options: DbValue },
    InsertOne { document: DbValue, options: DbValue },
    InsertMany { documents: Vec<DbValue>, options: DbValue },
    ReplaceOne { filter: DbValue, replacement: DbValue, options: DbValue },
    UpdateOne { filter: DbValue, update: DbValue, options: DbValue },
    UpdateMany { filter: DbValue, update: DbValue, options: DbValue },
    DeleteOne { filter: DbValue, options: DbValue },
    DeleteMany { filter: DbValue, options: DbValue },
    Aggregate { pipeline: Vec<DbValue>, options: DbValue },
}

/// Driver-neutral rows and write metadata returned to the PHP runtime.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DbColumnMeta {
    pub native_type: Option<String>,
    pub length: Option<usize>,
    pub precision: Option<i16>,
    pub nullable: Option<bool>,
    pub table_name: Option<String>,
    pub catalog_name: Option<String>,
}

/// A subsequent result returned by a multi-statement SQL execution.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DbResultSet {
    pub columns: Vec<String>,
    pub column_metadata: Vec<DbColumnMeta>,
    pub rows: Vec<Vec<DbValue>>,
    pub affected_rows: u64,
}

/// Driver-neutral rows and write metadata returned to the PHP runtime.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DbResult {
    pub columns: Vec<String>,
    pub column_metadata: Vec<DbColumnMeta>,
    pub additional_result_sets: Vec<DbResultSet>,
    pub rows: Vec<Vec<DbValue>>,
    pub affected_rows: u64,
    pub matched_rows: Option<u64>,
    pub modified_rows: Option<u64>,
    pub last_insert_id: Option<String>,
}

/// Optional vendor diagnostics encoded into the existing driver error channel.
/// The marker keeps driver error plumbing backwards-compatible while PDO can
/// recover SQLSTATE and native codes where a client library exposes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbErrorInfo {
    pub sql_state: String,
    pub driver_code: Option<i64>,
    pub message: String,
}

impl DbErrorInfo {
    pub fn encode(&self) -> String {
        format!("\0DBERR:{}:{}:{}", self.sql_state, self.driver_code.map(|code| code.to_string()).unwrap_or_default(), self.message)
    }

    pub fn decode(error: &str) -> Self {
        if let Some(rest) = error.strip_prefix("\0DBERR:") {
            let mut fields = rest.splitn(3, ':');
            let sql_state = fields.next().filter(|state| state.len() == 5).unwrap_or("HY000").to_owned();
            let driver_code = fields.next().and_then(|code| code.parse().ok());
            let message = fields.next().unwrap_or(rest).to_owned();
            Self { sql_state, driver_code, message }
        } else {
            Self { sql_state: "HY000".into(), driver_code: None, message: error.to_owned() }
        }
    }
}

/// A concrete backend. Implementations own their bounded pool and return
/// connections to it after each operation; SQL and Mongo commands share this
/// dispatch surface while retaining backend-specific semantics.
pub trait DatabaseDriver: Send + Sync + 'static {
    fn kind(&self) -> DatabaseKind;
    fn ping(&self) -> Result<(), String>;
    fn execute(&self, command: &DbCommand) -> Result<DbResult, String>;
    /// Client library version, when exposed by the native backend API.
    fn client_version(&self) -> Option<String> { None }
    /// Return a safely quoted SQL string literal for this backend.
    fn quote_string(&self, value: &str) -> Result<String, String> {
        Ok(format!("'{}'", value.replace('\'', "''")))
    }
    fn quote_binary(&self, value: &[u8]) -> Result<String, String> {
        Ok(format!("X'{}'", value.iter().map(|byte| format!("{byte:02x}")).collect::<String>()))
    }
    fn start_session(&self) -> Result<Box<dyn DatabaseSession>, String> {
        Err("transactions are not supported by this database driver".into())
    }
    fn start_session_with_timeout(&self, _acquire_timeout: Duration) -> Result<Box<dyn DatabaseSession>, String> {
        self.start_session()
    }
}

/// A pinned database connection used for a transaction scope.
pub trait DatabaseSession: Send + 'static {
    fn execute(&mut self, command: &DbCommand) -> Result<DbResult, String>;
    /// Client library or ODBC driver version for this checked-out connection.
    fn client_version(&self) -> Option<String> { None }
    fn quote_string(&mut self, value: &str) -> Result<String, String> {
        Ok(format!("'{}'", value.replace('\'', "''")))
    }
    fn quote_binary(&mut self, value: &[u8]) -> Result<String, String> {
        Ok(format!("X'{}'", value.iter().map(|byte| format!("{byte:02x}")).collect::<String>()))
    }
    fn begin_transaction(&mut self) -> Result<(), String> { Err("transactions are not supported by this database session".into()) }
    fn commit_transaction(&mut self) -> Result<(), String> { Err("transactions are not supported by this database session".into()) }
    fn rollback_transaction(&mut self) -> Result<(), String> { Err("transactions are not supported by this database session".into()) }
}

/// Process-wide named database handles shared by concurrent request workers.
#[derive(Clone, Default)]
pub struct DatabaseRegistry {
    drivers: Arc<RwLock<HashMap<String, RegisteredDriver>>>,
}

#[derive(Clone)]
struct RegisteredDriver {
    config: ConnectionConfig,
    driver: Arc<dyn DatabaseDriver>,
}

impl DatabaseRegistry {
    pub fn register(&self, config: &ConnectionConfig, driver: Arc<dyn DatabaseDriver>) -> Result<(), String> {
        config.validate()?;
        if driver.kind() != config.kind { return Err(format!("database driver kind does not match connection '{}'", config.name)); }
        let mut drivers = self.drivers.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if drivers.contains_key(&config.name) { return Err(format!("database connection '{}' is already registered", config.name)); }
        drivers.insert(config.name.clone(), RegisteredDriver { config: config.clone(), driver });
        Ok(())
    }

    pub fn get(&self, name: &str) -> Result<Arc<dyn DatabaseDriver>, String> {
        self.drivers.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(name).map(|entry| Arc::clone(&entry.driver))
            .ok_or_else(|| format!("database connection '{name}' is not configured"))
    }

    /// Resolve a PHP PDO DSN to a preconfigured named handle.
    pub fn resolve_dsn(&self, dsn: &str, username: Option<&str>, password: Option<&str>) -> Result<(String, Arc<dyn DatabaseDriver>), String> {
        let drivers = self.drivers.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut matches = drivers.values().filter(|entry| entry.config.dsn == dsn
            && username.is_none_or(|username| entry.config.username.as_deref() == Some(username))
            && password.is_none_or(|password| entry.config.password.as_deref() == Some(password)));
        let entry = matches.next()
            .ok_or_else(|| "PDO DSN does not match a configured database connection".to_owned())?;
        if matches.next().is_some() {
            return Err("PDO DSN matches multiple configured connections; use a unique DSN or credentials".into());
        }
        Ok((entry.config.name.clone(), Arc::clone(&entry.driver)))
    }

    pub fn names(&self) -> Vec<String> {
        let mut names = self.drivers.read().unwrap_or_else(std::sync::PoisonError::into_inner).keys().cloned().collect::<Vec<_>>();
        names.sort();
        names
    }
}

impl fmt::Debug for DatabaseRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseRegistry").field("connections", &self.names()).finish()
    }
}

/// Establishes connections for a particular concrete database client.
pub trait Connector: Send + Sync + 'static {
    type Connection: Send + 'static;
    type Error: fmt::Display;

    fn connect(&self) -> Result<Self::Connection, Self::Error>;
}

#[derive(Debug)]
pub enum AcquireError<E> {
    Timeout,
    Closed,
    Connect(E),
}

struct State<C> {
    idle: Vec<(C, Instant)>,
    total: usize,
    closed: bool,
}

struct Inner<C> {
    max: usize,
    state: Mutex<State<C>>,
    available: Condvar,
}

/// A cloneable bounded pool. Acquired connections return to the pool on drop.
pub struct Pool<C: Send + 'static> {
    inner: Arc<Inner<C>>,
    options: PoolOptions,
}

impl<C: Send + 'static> Clone for Pool<C> {
    fn clone(&self) -> Self { Self { inner: Arc::clone(&self.inner), options: self.options } }
}

impl<C: Send + 'static> Pool<C> {
    pub fn new(max_connections: usize) -> Result<Self, String> {
        if max_connections == 0 { return Err("pool max_connections must be greater than zero".into()); }
        Self::with_options(PoolOptions { max_connections, ..PoolOptions::default() })
    }

    pub fn with_options(options: PoolOptions) -> Result<Self, String> {
        let options = options.validate()?;
        Ok(Self { inner: Arc::new(Inner { max: options.max_connections, state: Mutex::new(State { idle: Vec::new(), total: 0, closed: false }), available: Condvar::new() }), options })
    }

    pub fn acquire<T: Connector<Connection = C>>(&self, connector: &T, timeout: Duration) -> Result<PooledConnection<C>, AcquireError<String>> {
        let deadline = Instant::now() + timeout;
        loop {
            let mut state = lock(&self.inner.state);
            if state.closed { return Err(AcquireError::Closed); }
            if let Some(idle_timeout) = self.options.idle_timeout {
                let now = Instant::now();
                let expirable = state.total.saturating_sub(self.options.min_connections);
                let mut expired = 0;
                state.idle.retain(|(_, since)| {
                    if expired < expirable && now.duration_since(*since) >= idle_timeout {
                        expired += 1;
                        false
                    } else {
                        true
                    }
                });
                state.total = state.total.saturating_sub(expired);
            }
            if let Some((connection, _)) = state.idle.pop() { return Ok(PooledConnection { connection: Some(connection), pool: Arc::clone(&self.inner), reusable: true }); }
            if state.total < self.inner.max {
                state.total += 1;
                drop(state);
                match connector.connect() {
                    Ok(connection) => {
                        let state = lock(&self.inner.state);
                        if state.closed {
                            drop(state);
                            let mut state = lock(&self.inner.state);
                            state.total = state.total.saturating_sub(1);
                            self.inner.available.notify_one();
                            return Err(AcquireError::Closed);
                        }
                        drop(state);
                        return Ok(PooledConnection { connection: Some(connection), pool: Arc::clone(&self.inner), reusable: true });
                    }
                    Err(error) => {
                        let mut state = lock(&self.inner.state);
                        state.total = state.total.saturating_sub(1);
                        self.inner.available.notify_one();
                        return Err(AcquireError::Connect(error.to_string()));
                    }
                }
            }
            let now = Instant::now();
            if now >= deadline { return Err(AcquireError::Timeout); }
            let waited = self.inner.available.wait_timeout(state, deadline - now);
            let (next, result) = waited.unwrap_or_else(std::sync::PoisonError::into_inner);
            drop(next);
            if result.timed_out() { return Err(AcquireError::Timeout); }
        }
    }

    /// Open the configured minimum number of connections up front.
    pub fn warm<T: Connector<Connection = C>>(&self, connector: &T) -> Result<(), AcquireError<String>> {
        let mut leases = Vec::with_capacity(self.options.min_connections);
        for _ in 0..self.options.min_connections {
            leases.push(self.acquire(connector, self.options.acquire_timeout)?);
        }
        drop(leases);
        Ok(())
    }

    pub fn close(&self) {
        let mut state = lock(&self.inner.state);
        state.closed = true;
        state.total = state.total.saturating_sub(state.idle.len());
        state.idle.clear();
        self.inner.available.notify_all();
    }

    pub fn size(&self) -> (usize, usize) {
        let state = lock(&self.inner.state);
        (state.total, state.idle.len())
    }
}

/// RAII lease for a pooled connection.
pub struct PooledConnection<C: Send + 'static> {
    connection: Option<C>,
    pool: Arc<Inner<C>>,
    reusable: bool,
}

impl<C: Send + 'static> PooledConnection<C> {
    /// Mark a connection unusable so it is dropped instead of returned to the idle pool.
    pub fn discard(mut self) {
        self.reusable = false;
    }
}

impl<C: Send + 'static> std::ops::Deref for PooledConnection<C> {
    type Target = C;
    fn deref(&self) -> &C { self.connection.as_ref().expect("pooled connection lease is present") }
}

impl<C: Send + 'static> std::ops::DerefMut for PooledConnection<C> {
    fn deref_mut(&mut self) -> &mut C { self.connection.as_mut().expect("pooled connection lease is present") }
}

impl<C: Send + 'static> Drop for PooledConnection<C> {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else { return; };
        let mut state = lock(&self.pool.state);
        if state.closed || !self.reusable { state.total = state.total.saturating_sub(1); }
        else { state.idle.push((connection, Instant::now())); }
        self.pool.available.notify_one();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> { mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner) }

#[cfg(test)]
#[path = "../../../tests/crates/php-db/db/lib.rs"]
mod tests;
