//! MySQL and MariaDB adapter loaded through the mysqlclient/MariaDB C API.

use crate::{
    AcquireError, ConnectionConfig, Connector, DatabaseDriver, DatabaseKind, DatabaseSession,
    DbColumnMeta, DbCommand, DbErrorInfo, DbResult, DbResultSet, DbValue, Pool, PooledConnection,
};
use libc::{c_char, c_int, c_uint, c_ulong, c_ulonglong, c_void};
use std::{
    collections::HashMap,
    ffi::{CStr, CString},
    ptr::NonNull,
    sync::{Arc, OnceLock},
    time::Duration,
};

type LibraryInit = unsafe extern "C" fn(c_int, *mut *mut c_char, *mut *mut c_char) -> c_int;
type ClientInfo = unsafe extern "C" fn() -> *const c_char;
type Init = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type RealConnect = unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
    c_uint,
    *const c_char,
    c_ulong,
) -> *mut c_void;
type Close = unsafe extern "C" fn(*mut c_void);
type Query = unsafe extern "C" fn(*mut c_void, *const c_char, c_ulong) -> c_int;
type NextResult = unsafe extern "C" fn(*mut c_void) -> c_int;
type StoreResult = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type NumFields = unsafe extern "C" fn(*mut c_void) -> c_uint;
type FetchFields = unsafe extern "C" fn(*mut c_void) -> *mut MysqlField;
type FetchRow = unsafe extern "C" fn(*mut c_void) -> *mut *mut c_char;
type FetchLengths = unsafe extern "C" fn(*mut c_void) -> *mut c_ulong;
type NumRows = unsafe extern "C" fn(*mut c_void) -> c_ulonglong;
type AffectedRows = unsafe extern "C" fn(*mut c_void) -> c_ulonglong;
type InsertId = unsafe extern "C" fn(*mut c_void) -> c_ulonglong;
type FreeResult = unsafe extern "C" fn(*mut c_void);
type Escape = unsafe extern "C" fn(*mut c_void, *mut c_char, *const c_char, c_ulong) -> c_ulong;
type Error = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type Errno = unsafe extern "C" fn(*mut c_void) -> c_uint;
type SqlState = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type Ping = unsafe extern "C" fn(*mut c_void) -> c_int;
type SetCharset = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
type StmtInit = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type StmtPrepare = unsafe extern "C" fn(*mut c_void, *const c_char, c_ulong) -> c_int;
type StmtExecute = unsafe extern "C" fn(*mut c_void) -> c_int;
type StmtBind = unsafe extern "C" fn(*mut c_void, *mut MysqlBind) -> c_char;
type StmtResultMetadata = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type StmtStoreResult = unsafe extern "C" fn(*mut c_void) -> c_int;
type StmtNumRows = unsafe extern "C" fn(*mut c_void) -> c_ulonglong;
type StmtParamCount = unsafe extern "C" fn(*mut c_void) -> c_ulong;
type StmtFieldCount = unsafe extern "C" fn(*mut c_void) -> c_uint;
type StmtFetch = unsafe extern "C" fn(*mut c_void) -> c_int;
type StmtFetchColumn = unsafe extern "C" fn(*mut c_void, *mut MysqlBind, c_uint, c_ulong) -> c_int;
type StmtNextResult = unsafe extern "C" fn(*mut c_void) -> c_int;
type StmtFreeResult = unsafe extern "C" fn(*mut c_void) -> c_char;
type StmtClose = unsafe extern "C" fn(*mut c_void) -> c_char;
type StmtAffectedRows = unsafe extern "C" fn(*mut c_void) -> c_ulonglong;
type StmtInsertId = unsafe extern "C" fn(*mut c_void) -> c_ulonglong;
type StmtError = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type StmtErrno = unsafe extern "C" fn(*mut c_void) -> c_uint;
type StmtSqlState = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type StmtReset = unsafe extern "C" fn(*mut c_void) -> c_char;

#[repr(C)]
union MysqlBindUnion {
    row_ptr: *mut u8,
    indicator: *mut c_char,
}

#[repr(C)]
struct MysqlBind {
    length: *mut c_ulong,
    is_null: *mut c_char,
    buffer: *mut c_void,
    error: *mut c_char,
    union_value: MysqlBindUnion,
    store_param_func: Option<unsafe extern "C" fn(*mut c_void, *mut MysqlBind)>,
    fetch_result: Option<unsafe extern "C" fn(*mut MysqlBind, *mut MysqlField, *mut *mut u8)>,
    skip_result: Option<unsafe extern "C" fn(*mut MysqlBind, *mut MysqlField, *mut *mut u8)>,
    buffer_length: c_ulong,
    offset: c_ulong,
    length_value: c_ulong,
    flags: c_uint,
    pack_length: c_uint,
    buffer_type: c_uint,
    error_value: c_char,
    is_unsigned: c_char,
    long_data_used: c_char,
    is_null_value: c_char,
    extension: *mut c_void,
}

impl Default for MysqlBind {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
// Keep the complete stable MYSQL_FIELD prefix: pointer arithmetic over the
// client-returned field array depends on this ABI layout, not just `name`.
struct MysqlField {
    name: *const c_char,
    org_name: *const c_char,
    table: *const c_char,
    org_table: *const c_char,
    db: *const c_char,
    catalog: *const c_char,
    def: *const c_char,
    length: c_ulong,
    max_length: c_ulong,
    name_length: c_uint,
    org_name_length: c_uint,
    table_length: c_uint,
    org_table_length: c_uint,
    db_length: c_uint,
    catalog_length: c_uint,
    def_length: c_uint,
    flags: c_uint,
    decimals: c_uint,
    charsetnr: c_uint,
    field_type: c_uint,
    extension: *mut c_void,
}

struct MysqlApi {
    library: *mut c_void,
    library_init: Option<LibraryInit>,
    client_info: Option<ClientInfo>,
    init: Init,
    real_connect: RealConnect,
    close: Close,
    query: Query,
    next_result: NextResult,
    store_result: StoreResult,
    num_fields: NumFields,
    fetch_fields: FetchFields,
    fetch_row: FetchRow,
    fetch_lengths: FetchLengths,
    num_rows: NumRows,
    affected_rows: AffectedRows,
    insert_id: InsertId,
    free_result: FreeResult,
    escape: Escape,
    error: Error,
    errno: Errno,
    sqlstate: SqlState,
    ping: Ping,
    set_charset: SetCharset,
    stmt_init: StmtInit,
    stmt_prepare: StmtPrepare,
    stmt_execute: StmtExecute,
    stmt_bind_param: StmtBind,
    stmt_bind_result: StmtBind,
    stmt_result_metadata: StmtResultMetadata,
    stmt_store_result: StmtStoreResult,
    stmt_num_rows: StmtNumRows,
    stmt_param_count: StmtParamCount,
    stmt_field_count: StmtFieldCount,
    stmt_fetch: StmtFetch,
    stmt_fetch_column: StmtFetchColumn,
    stmt_next_result: StmtNextResult,
    stmt_free_result: StmtFreeResult,
    stmt_close: StmtClose,
    stmt_affected_rows: StmtAffectedRows,
    stmt_insert_id: StmtInsertId,
    stmt_error: StmtError,
    stmt_errno: StmtErrno,
    stmt_sqlstate: StmtSqlState,
    stmt_reset: StmtReset,
}

unsafe impl Send for MysqlApi {}
unsafe impl Sync for MysqlApi {}
impl Drop for MysqlApi {
    fn drop(&mut self) {
        unsafe {
            libc::dlclose(self.library);
        }
    }
}

impl MysqlApi {
    fn load() -> Result<Arc<Self>, String> {
        static API: OnceLock<Result<Arc<MysqlApi>, String>> = OnceLock::new();
        API.get_or_init(Self::load_once).clone()
    }

    fn load_once() -> Result<Arc<Self>, String> {
        let mut library = std::ptr::null_mut();
        for name in [
            "libmariadb.so.3",
            "libmysqlclient.so.24",
            "libmysqlclient.so",
        ] {
            let name = CString::new(name).unwrap();
            library = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
            if !library.is_null() {
                break;
            }
        }
        if library.is_null() {
            return Err("MySQL/MariaDB driver requires libmariadb or libmysqlclient".into());
        }
        macro_rules! sym {
            ($name:literal, $ty:ty) => {{
                let address = unsafe { libc::dlsym(library, concat!($name, "\0").as_ptr().cast()) };
                if address.is_null() {
                    unsafe {
                        libc::dlclose(library);
                    }
                    return Err(concat!("MySQL client library is missing symbol ", $name).into());
                }
                unsafe { std::mem::transmute::<*mut c_void, $ty>(address) }
            }};
        }
        let library_init_address = unsafe { libc::dlsym(library, c"mysql_library_init".as_ptr()) };
        let library_init_address = if library_init_address.is_null() {
            unsafe { libc::dlsym(library, c"mysql_server_init".as_ptr()) }
        } else {
            library_init_address
        };
        let library_init = (!library_init_address.is_null()).then(|| unsafe {
            std::mem::transmute::<*mut c_void, LibraryInit>(library_init_address)
        });
        let client_info_address =
            unsafe { libc::dlsym(library, c"mysql_get_client_info".as_ptr()) };
        let client_info = (!client_info_address.is_null()).then(|| unsafe {
            std::mem::transmute::<*mut c_void, ClientInfo>(client_info_address)
        });
        let api = Arc::new(Self {
            library,
            library_init,
            client_info,
            init: sym!("mysql_init", Init),
            real_connect: sym!("mysql_real_connect", RealConnect),
            close: sym!("mysql_close", Close),
            query: sym!("mysql_real_query", Query),
            next_result: sym!("mysql_next_result", NextResult),
            store_result: sym!("mysql_store_result", StoreResult),
            num_fields: sym!("mysql_num_fields", NumFields),
            fetch_fields: sym!("mysql_fetch_fields", FetchFields),
            fetch_row: sym!("mysql_fetch_row", FetchRow),
            fetch_lengths: sym!("mysql_fetch_lengths", FetchLengths),
            num_rows: sym!("mysql_num_rows", NumRows),
            affected_rows: sym!("mysql_affected_rows", AffectedRows),
            free_result: sym!("mysql_free_result", FreeResult),
            insert_id: sym!("mysql_insert_id", InsertId),
            escape: sym!("mysql_real_escape_string", Escape),
            error: sym!("mysql_error", Error),
            errno: sym!("mysql_errno", Errno),
            sqlstate: sym!("mysql_sqlstate", SqlState),
            ping: sym!("mysql_ping", Ping),
            set_charset: sym!("mysql_set_character_set", SetCharset),
            stmt_init: sym!("mysql_stmt_init", StmtInit),
            stmt_prepare: sym!("mysql_stmt_prepare", StmtPrepare),
            stmt_execute: sym!("mysql_stmt_execute", StmtExecute),
            stmt_bind_param: sym!("mysql_stmt_bind_param", StmtBind),
            stmt_bind_result: sym!("mysql_stmt_bind_result", StmtBind),
            stmt_result_metadata: sym!("mysql_stmt_result_metadata", StmtResultMetadata),
            stmt_store_result: sym!("mysql_stmt_store_result", StmtStoreResult),
            stmt_num_rows: sym!("mysql_stmt_num_rows", StmtNumRows),
            stmt_param_count: sym!("mysql_stmt_param_count", StmtParamCount),
            stmt_field_count: sym!("mysql_stmt_field_count", StmtFieldCount),
            stmt_fetch: sym!("mysql_stmt_fetch", StmtFetch),
            stmt_fetch_column: sym!("mysql_stmt_fetch_column", StmtFetchColumn),
            stmt_next_result: sym!("mysql_stmt_next_result", StmtNextResult),
            stmt_free_result: sym!("mysql_stmt_free_result", StmtFreeResult),
            stmt_close: sym!("mysql_stmt_close", StmtClose),
            stmt_affected_rows: sym!("mysql_stmt_affected_rows", StmtAffectedRows),
            stmt_insert_id: sym!("mysql_stmt_insert_id", StmtInsertId),
            stmt_error: sym!("mysql_stmt_error", StmtError),
            stmt_errno: sym!("mysql_stmt_errno", StmtErrno),
            stmt_sqlstate: sym!("mysql_stmt_sqlstate", StmtSqlState),
            stmt_reset: sym!("mysql_stmt_reset", StmtReset),
        });
        if api
            .library_init
            .is_some_and(|init| unsafe { init(0, std::ptr::null_mut(), std::ptr::null_mut()) } != 0)
        {
            return Err("MySQL client library initialization failed".into());
        }
        Ok(api)
    }
}

#[derive(Clone)]
struct MysqlOptions {
    host: CString,
    port: c_uint,
    database: CString,
    username: CString,
    password: CString,
    socket: Option<CString>,
    charset: Option<CString>,
}

struct MysqlConnection {
    raw: NonNull<c_void>,
    api: Arc<MysqlApi>,
}
unsafe impl Send for MysqlConnection {}
impl Drop for MysqlConnection {
    fn drop(&mut self) {
        unsafe {
            (self.api.close)(self.raw.as_ptr());
        }
    }
}

struct MysqlConnector {
    api: Arc<MysqlApi>,
    options: MysqlOptions,
}
impl Connector for MysqlConnector {
    type Connection = MysqlConnection;
    type Error = String;
    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        let raw = unsafe { (self.api.init)(std::ptr::null_mut()) };
        let raw = NonNull::new(raw)
            .ok_or_else(|| "mysql_init could not allocate a connection".to_owned())?;
        // Enable multi-statement execution so PDOStatement::nextRowset can expose MySQL result sets.
        let connected = unsafe {
            (self.api.real_connect)(
                raw.as_ptr(),
                self.options.host.as_ptr(),
                self.options.username.as_ptr(),
                self.options.password.as_ptr(),
                self.options.database.as_ptr(),
                self.options.port,
                self.options
                    .socket
                    .as_ref()
                    .map_or(std::ptr::null(), |value| value.as_ptr()),
                1 << 16,
            )
        };
        if connected.is_null() {
            let error = unsafe { c_string((self.api.error)(raw.as_ptr())) };
            unsafe {
                (self.api.close)(raw.as_ptr());
            }
            return Err(format!("MySQL/MariaDB connection failed: {error}"));
        }
        if let Some(charset) = &self.options.charset {
            if unsafe { (self.api.set_charset)(raw.as_ptr(), charset.as_ptr()) } != 0 {
                let error = unsafe { c_string((self.api.error)(raw.as_ptr())) };
                unsafe {
                    (self.api.close)(raw.as_ptr());
                }
                return Err(format!("cannot set MySQL connection charset: {error}"));
            }
        }
        Ok(MysqlConnection {
            raw,
            api: Arc::clone(&self.api),
        })
    }
}

/// MySQL or MariaDB driver backed by a bounded pool.
pub struct MysqlDriver {
    kind: DatabaseKind,
    connector: MysqlConnector,
    pool: Pool<MysqlConnection>,
    acquire_timeout: Duration,
    max_result_rows: usize,
}

impl MysqlDriver {
    pub fn new(config: &ConnectionConfig) -> Result<Self, String> {
        config.validate()?;
        if !matches!(config.kind, DatabaseKind::Mysql | DatabaseKind::MariaDb) {
            return Err("MysqlDriver requires a mysql: or mariadb: DSN".into());
        }
        let options = mysql_options(config)?;
        let connector = MysqlConnector {
            api: MysqlApi::load()?,
            options,
        };
        let pool = Pool::with_options(config.pool)?;
        pool.warm(&connector).map_err(acquire_error)?;
        Ok(Self {
            kind: config.kind,
            connector,
            pool,
            acquire_timeout: config.pool.acquire_timeout,
            max_result_rows: config.max_result_rows,
        })
    }

    fn acquire(&self) -> Result<PooledConnection<MysqlConnection>, String> {
        self.pool
            .acquire(&self.connector, self.acquire_timeout)
            .map_err(acquire_error)
    }

    fn execute_sql(&self, sql: &str, parameters: &[DbValue]) -> Result<DbResult, String> {
        let connection = self.acquire()?;
        let result = execute_on(&connection, sql, parameters, self.max_result_rows);
        if result.is_err() && unsafe { (connection.api.ping)(connection.raw.as_ptr()) } != 0 {
            connection.discard();
        }
        result
    }
    fn execute_prepared_sql(&self, sql: &str, parameters: &[DbValue]) -> Result<DbResult, String> {
        let connection = self.acquire()?;
        let result = execute_prepared_on(&connection, sql, parameters, self.max_result_rows);
        if result.is_err() && unsafe { (connection.api.ping)(connection.raw.as_ptr()) } != 0 {
            connection.discard();
        }
        result
    }
}

impl DatabaseDriver for MysqlDriver {
    fn kind(&self) -> DatabaseKind {
        self.kind
    }
    fn client_version(&self) -> Option<String> {
        let function = self.connector.api.client_info?;
        let version = unsafe { function() };
        (!version.is_null()).then(|| unsafe { c_string(version) })
    }
    fn ping(&self) -> Result<(), String> {
        self.execute_sql("SELECT 1", &[]).map(|_| ())
    }
    fn execute(&self, command: &DbCommand) -> Result<DbResult, String> {
        match command {
            DbCommand::Sql {
                statement,
                parameters,
            } => self.execute_sql(statement, parameters),
            DbCommand::PreparedSql {
                statement,
                parameters,
            } => self.execute_prepared_sql(statement, parameters),
            DbCommand::Mongo { .. } => {
                Err("MongoDB command cannot be sent to MySQL/MariaDB".into())
            }
        }
    }
    fn quote_string(&self, value: &str) -> Result<String, String> {
        let connection = self.acquire()?;
        escape_mysql_string(&connection, value)
    }
    fn quote_binary(&self, value: &[u8]) -> Result<String, String> {
        Ok(format!(
            "X'{}'",
            value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ))
    }
    fn start_session(&self) -> Result<Box<dyn DatabaseSession>, String> {
        Ok(Box::new(MysqlSession {
            connection: Some(self.acquire()?),
            prepared: HashMap::new(),
            max_result_rows: self.max_result_rows,
        }))
    }
    fn start_session_with_timeout(
        &self,
        acquire_timeout: Duration,
    ) -> Result<Box<dyn DatabaseSession>, String> {
        let timeout = if acquire_timeout.is_zero() {
            self.acquire_timeout
        } else {
            acquire_timeout
        };
        let connection = self
            .pool
            .acquire(&self.connector, timeout)
            .map_err(acquire_error)?;
        Ok(Box::new(MysqlSession {
            connection: Some(connection),
            prepared: HashMap::new(),
            max_result_rows: self.max_result_rows,
        }))
    }
}

struct MysqlSession {
    connection: Option<PooledConnection<MysqlConnection>>,
    prepared: HashMap<String, usize>,
    max_result_rows: usize,
}
impl MysqlSession {
    fn clear_prepared(&mut self) {
        if let Some(connection) = self.connection.as_ref() {
            for (_, stmt) in self.prepared.drain() {
                unsafe {
                    (connection.api.stmt_close)(stmt as *mut c_void);
                }
            }
        } else {
            self.prepared.clear();
        }
    }
    fn execute_cached_prepared(
        &mut self,
        sql: &str,
        parameters: &[DbValue],
    ) -> Result<DbResult, String> {
        let connection = self
            .connection
            .as_ref()
            .ok_or("MySQL transaction connection is unusable")?;
        let stmt = if let Some(stmt) = self.prepared.get(sql) {
            *stmt as *mut c_void
        } else {
            let stmt = prepare_statement(connection, sql)?;
            self.prepared.insert(sql.to_owned(), stmt as usize);
            stmt
        };
        execute_prepared_statement(connection, stmt, parameters, self.max_result_rows)
    }
}
impl Drop for MysqlSession {
    fn drop(&mut self) {
        self.clear_prepared();
    }
}
impl DatabaseSession for MysqlSession {
    fn client_version(&self) -> Option<String> {
        let function = self.connection.as_ref()?.api.client_info?;
        let version = unsafe { function() };
        (!version.is_null()).then(|| unsafe { c_string(version) })
    }
    fn execute(&mut self, command: &DbCommand) -> Result<DbResult, String> {
        let (statement, parameters, prepared) = match command {
            DbCommand::Sql {
                statement,
                parameters,
            } => (statement, parameters, false),
            DbCommand::PreparedSql {
                statement,
                parameters,
            } => (statement, parameters, true),
            DbCommand::Mongo { .. } => {
                return Err("MongoDB command cannot be sent to MySQL/MariaDB".into())
            }
        };
        let result = if prepared {
            self.execute_cached_prepared(statement, parameters)
        } else {
            let connection = self
                .connection
                .as_ref()
                .ok_or("MySQL transaction connection is unusable")?;
            execute_on(connection, statement, parameters, self.max_result_rows)
        };
        if result.is_err() {
            self.clear_prepared();
            let is_connected = self.connection.as_ref().is_some_and(
                |connection| unsafe { (connection.api.ping)(connection.raw.as_ptr()) } == 0,
            );
            if !is_connected {
                self.connection
                    .take()
                    .expect("MySQL session connection")
                    .discard();
            }
        }
        result
    }
    fn quote_string(&mut self, value: &str) -> Result<String, String> {
        let connection = self
            .connection
            .as_ref()
            .ok_or("MySQL PDO connection is unusable")?;
        escape_mysql_string(connection, value)
    }
    fn quote_binary(&mut self, value: &[u8]) -> Result<String, String> {
        Ok(format!(
            "X'{}'",
            value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ))
    }
}

fn escape_mysql_string(connection: &MysqlConnection, value: &str) -> Result<String, String> {
    let mut escaped = vec![0 as c_char; value.len().saturating_mul(2).saturating_add(1)];
    let length = unsafe {
        (connection.api.escape)(
            connection.raw.as_ptr(),
            escaped.as_mut_ptr(),
            value.as_ptr().cast(),
            value.len() as c_ulong,
        )
    } as usize;
    String::from_utf8(escaped[..length].iter().map(|byte| *byte as u8).collect())
        .map(|escaped| format!("'{escaped}'"))
        .map_err(|_| "MySQL escaped string is not valid UTF-8".into())
}

fn execute_on(
    connection: &MysqlConnection,
    sql: &str,
    parameters: &[DbValue],
    max_result_rows: usize,
) -> Result<DbResult, String> {
    let query = interpolate_parameters(connection, sql, parameters)?;
    let status = unsafe {
        (connection.api.query)(
            connection.raw.as_ptr(),
            query.as_ptr().cast(),
            query.len() as c_ulong,
        )
    };
    if status != 0 {
        return Err(connection_error(connection));
    }
    let mut first = read_current_result(connection, max_result_rows)?;
    loop {
        let status = unsafe { (connection.api.next_result)(connection.raw.as_ptr()) };
        if status == -1 {
            break;
        }
        if status != 0 {
            return Err(connection_error(connection));
        }
        let next = read_current_result(connection, max_result_rows)?;
        first.additional_result_sets.push(DbResultSet {
            columns: next.columns,
            column_metadata: next.column_metadata,
            rows: next.rows,
            affected_rows: next.affected_rows,
        });
    }
    Ok(first)
}

fn read_current_result(
    connection: &MysqlConnection,
    max_result_rows: usize,
) -> Result<DbResult, String> {
    let result = unsafe { (connection.api.store_result)(connection.raw.as_ptr()) };
    if result.is_null() {
        let affected = unsafe { (connection.api.affected_rows)(connection.raw.as_ptr()) };
        let insert_id = unsafe { (connection.api.insert_id)(connection.raw.as_ptr()) };
        if affected == c_ulonglong::MAX {
            return Ok(DbResult::default());
        }
        return Ok(DbResult {
            affected_rows: affected,
            last_insert_id: (insert_id != 0).then(|| insert_id.to_string()),
            ..DbResult::default()
        });
    }
    read_result(
        connection,
        NonNull::new(result).expect("non-null result"),
        max_result_rows,
    )
}

fn mysql_options(config: &ConnectionConfig) -> Result<MysqlOptions, String> {
    let (scheme, dsn) = config
        .dsn
        .split_once(':')
        .ok_or("MySQL DSN must include a scheme")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "mysql" | "mariadb") {
        return Err("invalid MySQL/MariaDB DSN".into());
    }
    let mut host = "localhost".to_owned();
    let mut port = 3306u32;
    let mut database = String::new();
    let mut socket = None;
    let mut charset = None;
    for field in dsn.split(';').filter(|field| !field.trim().is_empty()) {
        let (key, value) = field
            .split_once('=')
            .ok_or_else(|| format!("invalid MySQL DSN field '{field}'"))?;
        match key.trim() {
            "host" => host = value.trim().to_owned(),
            "port" => port = value.trim().parse().map_err(|_| "invalid MySQL DSN port")?,
            "dbname" => database = value.trim().to_owned(),
            "charset" => charset = Some(value.trim().to_owned()),
            "unix_socket" => socket = Some(value.trim().to_owned()),
            other => return Err(format!("unsupported MySQL DSN option '{other}'")),
        }
    }
    Ok(MysqlOptions {
        host: CString::new(host).map_err(|_| "MySQL host cannot contain a NUL byte")?,
        port,
        database: CString::new(database).map_err(|_| "MySQL database cannot contain a NUL byte")?,
        username: CString::new(config.username.clone().unwrap_or_default())
            .map_err(|_| "MySQL username cannot contain a NUL byte")?,
        password: CString::new(config.password.clone().unwrap_or_default())
            .map_err(|_| "MySQL password cannot contain a NUL byte")?,
        socket: socket
            .map(CString::new)
            .transpose()
            .map_err(|_| "MySQL socket path cannot contain NUL bytes")?,
        charset: charset
            .map(CString::new)
            .transpose()
            .map_err(|_| "MySQL charset cannot contain NUL bytes")?,
    })
}

fn interpolate_parameters(
    connection: &MysqlConnection,
    sql: &str,
    parameters: &[DbValue],
) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(sql.len() + parameters.len() * 8);
    let mut parameter = 0usize;
    let mut cursor = 0usize;
    let bytes = sql.as_bytes();
    let mut quote = 0u8;
    let mut block_comment = false;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if block_comment {
            if bytes.get(cursor..cursor + 2) == Some(b"*/") {
                output.extend_from_slice(b"*/");
                cursor += 2;
                block_comment = false;
            } else {
                output.push(byte);
                cursor += 1;
            }
        } else if quote != 0 {
            output.push(byte);
            if byte == b'\\' && quote != b'`' && cursor + 1 < bytes.len() {
                output.push(bytes[cursor + 1]);
                cursor += 2;
                continue;
            }
            if byte == quote {
                if bytes.get(cursor + 1) == Some(&quote) {
                    output.push(quote);
                    cursor += 2;
                    continue;
                }
                quote = 0;
            }
            cursor += 1;
        } else if byte == b'\'' || byte == b'"' || byte == b'`' {
            quote = byte;
            output.push(byte);
            cursor += 1;
        } else if bytes.get(cursor..cursor + 2) == Some(b"--")
            && bytes
                .get(cursor + 2)
                .is_none_or(|byte| byte.is_ascii_whitespace() || *byte < 0x20)
        {
            let end = bytes[cursor..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |index| cursor + index);
            output.extend_from_slice(&bytes[cursor..end]);
            cursor = end;
        } else if byte == b'#' {
            let end = bytes[cursor..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |index| cursor + index);
            output.extend_from_slice(&bytes[cursor..end]);
            cursor = end;
        } else if bytes.get(cursor..cursor + 2) == Some(b"/*") {
            output.extend_from_slice(b"/*");
            cursor += 2;
            block_comment = true;
        } else if byte == b'?' {
            let value = parameters
                .get(parameter)
                .ok_or("parameter count does not match MySQL placeholders")?;
            parameter += 1;
            append_literal(connection, value, &mut output)?;
            cursor += 1;
        } else {
            output.push(byte);
            cursor += 1;
        }
    }
    if parameter != parameters.len() {
        return Err("parameter count does not match MySQL placeholders".into());
    }
    Ok(output)
}

fn append_literal(
    connection: &MysqlConnection,
    value: &DbValue,
    output: &mut Vec<u8>,
) -> Result<(), String> {
    match value {
        DbValue::Null => output.extend_from_slice(b"NULL"),
        DbValue::Bool(value) => output.extend_from_slice(if *value { b"1" } else { b"0" }),
        DbValue::Integer(value) => output.extend_from_slice(value.to_string().as_bytes()),
        DbValue::Float(value) if value.is_finite() => {
            output.extend_from_slice(value.to_string().as_bytes())
        }
        DbValue::Float(_) => return Err("non-finite floats cannot be bound to MySQL".into()),
        DbValue::String(value) => {
            let mut escaped = vec![0; value.len().saturating_mul(2).saturating_add(1)];
            let count = unsafe {
                (connection.api.escape)(
                    connection.raw.as_ptr(),
                    escaped.as_mut_ptr().cast(),
                    value.as_ptr().cast(),
                    value.len() as c_ulong,
                )
            } as usize;
            escaped.truncate(count);
            output.push(b'\'');
            output.extend_from_slice(&escaped);
            output.push(b'\'');
        }
        DbValue::Bytes(value) => {
            output.extend_from_slice(b"X'");
            const HEX: &[u8; 16] = b"0123456789abcdef";
            for byte in value {
                output.push(HEX[(*byte >> 4) as usize]);
                output.push(HEX[(*byte & 15) as usize]);
            }
            output.push(b'\'');
        }
        DbValue::Array(_) | DbValue::Document(_) => {
            return Err("composite MySQL parameters must be encoded by the caller".into())
        }
    }
    Ok(())
}

fn read_result(
    connection: &MysqlConnection,
    result: NonNull<c_void>,
    max_result_rows: usize,
) -> Result<DbResult, String> {
    let api = &connection.api;
    let fields = unsafe { (api.num_fields)(result.as_ptr()) } as usize;
    let field_names = unsafe { (api.fetch_fields)(result.as_ptr()) };
    if fields > 0 && field_names.is_null() {
        unsafe {
            (api.free_result)(result.as_ptr());
        }
        return Err("MySQL client returned no metadata for a non-empty result".into());
    }
    let mut columns = Vec::with_capacity(fields);
    let mut column_metadata = Vec::with_capacity(fields);
    for index in 0..fields {
        let field = unsafe { &*field_names.add(index) };
        columns.push(unsafe { c_string(field.name) });
        column_metadata.push(DbColumnMeta {
            native_type: Some(mysql_type_name(field.field_type).into()),
            length: Some(field.length as usize),
            precision: Some(field.decimals.min(i16::MAX as c_uint) as i16),
            nullable: Some(field.flags & 1 == 0),
            table_name: Some(unsafe { c_string(field.table) }),
            catalog_name: Some(unsafe { c_string(field.db) }),
        });
    }
    let total_rows = unsafe { (api.num_rows)(result.as_ptr()) };
    if total_rows > max_result_rows as u64 {
        unsafe {
            (api.free_result)(result.as_ptr());
        }
        return Err(format!(
            "MySQL result exceeds configured max_result_rows ({max_result_rows})"
        ));
    }
    let count = total_rows as usize;
    let mut rows = Vec::with_capacity(count);
    loop {
        let row = unsafe { (api.fetch_row)(result.as_ptr()) };
        if row.is_null() {
            break;
        }
        let lengths = unsafe { (api.fetch_lengths)(result.as_ptr()) };
        let mut values = Vec::with_capacity(fields);
        for column in 0..fields {
            let value = unsafe { *row.add(column) };
            if value.is_null() {
                values.push(DbValue::Null);
            } else {
                let length = unsafe { *lengths.add(column) } as usize;
                let bytes = unsafe { std::slice::from_raw_parts(value.cast::<u8>(), length) };
                values.push(DbValue::String(String::from_utf8_lossy(bytes).into_owned()));
            }
        }
        rows.push(values);
    }
    unsafe {
        (api.free_result)(result.as_ptr());
    }
    let insert_id = unsafe { (api.insert_id)(connection.raw.as_ptr()) };
    Ok(DbResult {
        columns,
        column_metadata,
        rows,
        affected_rows: count as u64,
        last_insert_id: (insert_id != 0).then(|| insert_id.to_string()),
        ..DbResult::default()
    })
}

fn prepare_statement(connection: &MysqlConnection, sql: &str) -> Result<*mut c_void, String> {
    let stmt = NonNull::new(unsafe { (connection.api.stmt_init)(connection.raw.as_ptr()) })
        .ok_or_else(|| connection_error(connection))?;
    let prepared = (|| {
        let sql = CString::new(sql).map_err(|_| "SQL statement contains NUL")?;
        if unsafe {
            (connection.api.stmt_prepare)(
                stmt.as_ptr(),
                sql.as_ptr(),
                sql.as_bytes().len() as c_ulong,
            )
        } != 0
        {
            return Err(statement_error(connection, stmt.as_ptr()));
        }
        Ok(())
    })();
    if let Err(error) = prepared {
        unsafe {
            (connection.api.stmt_close)(stmt.as_ptr());
        }
        return Err(error);
    }
    Ok(stmt.as_ptr())
}

fn execute_prepared_on(
    connection: &MysqlConnection,
    sql: &str,
    parameters: &[DbValue],
    max_result_rows: usize,
) -> Result<DbResult, String> {
    let stmt = prepare_statement(connection, sql)?;
    let result = execute_prepared_statement(connection, stmt, parameters, max_result_rows);
    unsafe {
        (connection.api.stmt_close)(stmt);
    }
    result
}

fn execute_prepared_statement(
    connection: &MysqlConnection,
    stmt: *mut c_void,
    parameters: &[DbValue],
    max_result_rows: usize,
) -> Result<DbResult, String> {
    let result = (|| {
        if unsafe { (connection.api.stmt_reset)(stmt) } != 0 {
            return Err(statement_error(connection, stmt));
        }
        let expected = unsafe { (connection.api.stmt_param_count)(stmt) } as usize;
        if expected != parameters.len() {
            return Err("parameter count does not match MySQL prepared statement".into());
        }
        let mut buffers = Vec::<Vec<u8>>::with_capacity(parameters.len());
        let mut lengths = Vec::<c_ulong>::with_capacity(parameters.len());
        let mut nulls = Vec::<c_char>::with_capacity(parameters.len());
        for value in parameters {
            let (bytes, is_null) = match value {
                DbValue::Null => (Vec::new(), true),
                DbValue::Bool(value) => (if *value { b"1".to_vec() } else { b"0".to_vec() }, false),
                DbValue::Integer(value) => (value.to_string().into_bytes(), false),
                DbValue::Float(value) if value.is_finite() => {
                    (value.to_string().into_bytes(), false)
                }
                DbValue::Float(_) => {
                    return Err("non-finite floats cannot be bound to MySQL".into())
                }
                DbValue::String(value) => (value.as_bytes().to_vec(), false),
                DbValue::Bytes(value) => (value.clone(), false),
                DbValue::Array(_) | DbValue::Document(_) => {
                    return Err("composite MySQL parameters must be encoded by the caller".into())
                }
            };
            lengths.push(bytes.len() as c_ulong);
            nulls.push(if is_null { 1 } else { 0 });
            buffers.push(bytes);
        }
        let mut binds = Vec::<MysqlBind>::with_capacity(parameters.len());
        for index in 0..parameters.len() {
            let buffer_type = match &parameters[index] {
                DbValue::Null => 6,
                DbValue::Bytes(_) => 252,
                _ => 254,
            };
            let mut bind = MysqlBind::default();
            bind.length = &mut lengths[index];
            bind.is_null = &mut nulls[index];
            bind.buffer = buffers[index].as_ptr() as *mut c_void;
            bind.buffer_length = buffers[index].len() as c_ulong;
            bind.buffer_type = buffer_type;
            binds.push(bind);
        }
        if !binds.is_empty()
            && unsafe { (connection.api.stmt_bind_param)(stmt, binds.as_mut_ptr()) } != 0
        {
            return Err(statement_error(connection, stmt));
        }
        if unsafe { (connection.api.stmt_execute)(stmt) } != 0 {
            return Err(statement_error(connection, stmt));
        }
        let mut first = read_prepared_result(connection, stmt, max_result_rows)?;
        loop {
            // The client API requires freeing the current result before
            // advancing a prepared CALL to its next result set.
            if unsafe { (connection.api.stmt_field_count)(stmt) } > 0
                && unsafe { (connection.api.stmt_free_result)(stmt) } != 0
            {
                return Err(statement_error(connection, stmt));
            }
            let next = unsafe { (connection.api.stmt_next_result)(stmt) };
            if next == -1 {
                break;
            }
            if next != 0 {
                return Err(statement_error(connection, stmt));
            }
            let result = read_prepared_result(connection, stmt, max_result_rows)?;
            first.additional_result_sets.push(DbResultSet {
                columns: result.columns,
                column_metadata: result.column_metadata,
                rows: result.rows,
                affected_rows: result.affected_rows,
            });
        }
        if unsafe { (connection.api.stmt_reset)(stmt) } != 0 {
            return Err(statement_error(connection, stmt));
        }
        Ok(first)
    })();
    result
}

fn read_prepared_result(
    connection: &MysqlConnection,
    stmt: *mut c_void,
    max_result_rows: usize,
) -> Result<DbResult, String> {
    let api = &connection.api;
    let fields = unsafe { (api.stmt_field_count)(stmt) } as usize;
    let mut columns = Vec::with_capacity(fields);
    let mut column_metadata = Vec::with_capacity(fields);
    let mut metadata = None;
    if fields > 0 {
        let Some(result) = NonNull::new(unsafe { (api.stmt_result_metadata)(stmt) }) else {
            return Err(statement_error(connection, stmt));
        };
        let field_ptr = unsafe { (api.fetch_fields)(result.as_ptr()) };
        if field_ptr.is_null() {
            unsafe {
                (api.free_result)(result.as_ptr());
            }
            return Err("MySQL prepared statement returned no column metadata".into());
        }
        for index in 0..fields {
            let field = unsafe { &*field_ptr.add(index) };
            columns.push(unsafe { c_string(field.name) });
            column_metadata.push(DbColumnMeta {
                native_type: Some(mysql_type_name(field.field_type).into()),
                length: Some(field.length as usize),
                precision: Some(field.decimals.min(i16::MAX as c_uint) as i16),
                nullable: Some(field.flags & 1 == 0),
                table_name: Some(unsafe { c_string(field.table) }),
                catalog_name: Some(unsafe { c_string(field.db) }),
            });
        }
        metadata = Some(result);
        if unsafe { (api.stmt_store_result)(stmt) } != 0 {
            unsafe {
                (api.free_result)(result.as_ptr());
            }
            return Err(statement_error(connection, stmt));
        }
    }
    let total_rows = if fields > 0 {
        unsafe { (api.stmt_num_rows)(stmt) }
    } else {
        0
    };
    if total_rows > max_result_rows as u64 {
        if let Some(metadata) = metadata {
            unsafe {
                (api.free_result)(metadata.as_ptr());
            }
        }
        return Err(format!(
            "MySQL result exceeds configured max_result_rows ({max_result_rows})"
        ));
    }
    let row_count = total_rows as usize;
    let mut rows = Vec::with_capacity(row_count);
    if fields > 0 {
        const INITIAL_COLUMN_BUFFER: usize = 1;
        let mut buffers = (0..fields)
            .map(|_| vec![0u8; INITIAL_COLUMN_BUFFER])
            .collect::<Vec<_>>();
        let mut lengths = vec![0 as c_ulong; fields];
        let mut nulls = vec![0 as c_char; fields];
        let mut binds = Vec::<MysqlBind>::with_capacity(fields);
        for index in 0..fields {
            let mut bind = MysqlBind::default();
            bind.length = &mut lengths[index];
            bind.is_null = &mut nulls[index];
            bind.buffer = buffers[index].as_mut_ptr().cast();
            bind.buffer_length = buffers[index].len() as c_ulong;
            bind.buffer_type = 254;
            binds.push(bind);
        }
        if unsafe { (api.stmt_bind_result)(stmt, binds.as_mut_ptr()) } != 0 {
            if let Some(metadata) = metadata {
                unsafe {
                    (api.free_result)(metadata.as_ptr());
                }
            }
            return Err(statement_error(connection, stmt));
        }
        loop {
            let status = unsafe { (api.stmt_fetch)(stmt) };
            if status == 100 {
                break;
            }
            if status != 0 && status != 101 {
                if let Some(metadata) = metadata {
                    unsafe {
                        (api.free_result)(metadata.as_ptr());
                    }
                }
                return Err(statement_error(connection, stmt));
            }
            let mut row = Vec::with_capacity(fields);
            for index in 0..fields {
                if nulls[index] != 0 {
                    row.push(DbValue::Null);
                    continue;
                }
                let length = lengths[index] as usize;
                if length > buffers[index].len() {
                    buffers[index].resize(length.max(1), 0);
                    binds[index].buffer = buffers[index].as_mut_ptr().cast();
                    binds[index].buffer_length = buffers[index].len() as c_ulong;
                    let fetched = unsafe {
                        (api.stmt_fetch_column)(stmt, &mut binds[index], index as c_uint, 0)
                    };
                    if fetched != 0 {
                        if let Some(metadata) = metadata {
                            unsafe {
                                (api.free_result)(metadata.as_ptr());
                            }
                        }
                        return Err(statement_error(connection, stmt));
                    }
                }
                row.push(DbValue::String(
                    String::from_utf8_lossy(&buffers[index][..length]).into_owned(),
                ));
            }
            rows.push(row);
        }
    }
    if let Some(metadata) = metadata {
        unsafe {
            (api.free_result)(metadata.as_ptr());
        }
    }
    let affected_rows = if fields == 0 {
        unsafe { (api.stmt_affected_rows)(stmt) }
    } else {
        rows.len() as c_ulonglong
    };
    let insert_id = unsafe { (api.stmt_insert_id)(stmt) };
    Ok(DbResult {
        columns,
        column_metadata,
        rows,
        affected_rows,
        last_insert_id: (insert_id != 0).then(|| insert_id.to_string()),
        ..DbResult::default()
    })
}

fn statement_error(connection: &MysqlConnection, stmt: *mut c_void) -> String {
    unsafe {
        let state = c_string((connection.api.stmt_sqlstate)(stmt));
        let code = (connection.api.stmt_errno)(stmt);
        let message = c_string((connection.api.stmt_error)(stmt));
        DbErrorInfo {
            sql_state: if state.len() == 5 && state != "00000" {
                state
            } else {
                "HY000".into()
            },
            driver_code: Some(code as i64),
            message,
        }
        .encode()
    }
}

fn mysql_type_name(field_type: c_uint) -> &'static str {
    match field_type {
        0 => "decimal",
        1 => "tinyint",
        2 => "smallint",
        3 => "int",
        4 => "float",
        5 => "double",
        6 => "null",
        7 | 17 => "timestamp",
        8 => "bigint",
        9 => "mediumint",
        10 | 14 => "date",
        11 | 19 => "time",
        12 | 18 => "datetime",
        13 => "year",
        15 => "varchar",
        16 => "bit",
        245 => "json",
        246 => "decimal",
        247 => "enum",
        248 => "set",
        249 => "tinyblob",
        250 => "mediumblob",
        251 => "longblob",
        252 => "blob",
        253 => "var_string",
        254 => "string",
        255 => "geometry",
        _ => "unknown",
    }
}

unsafe fn c_string(value: *const c_char) -> String {
    if value.is_null() {
        String::new()
    } else {
        CStr::from_ptr(value).to_string_lossy().trim().to_owned()
    }
}
fn connection_error(connection: &MysqlConnection) -> String {
    unsafe {
        let state = c_string((connection.api.sqlstate)(connection.raw.as_ptr()));
        let code = (connection.api.errno)(connection.raw.as_ptr());
        let message = c_string((connection.api.error)(connection.raw.as_ptr()));
        DbErrorInfo {
            sql_state: if state.len() == 5 && state != "00000" {
                state
            } else {
                "HY000".into()
            },
            driver_code: Some(code as i64),
            message,
        }
        .encode()
    }
}
fn acquire_error(error: AcquireError<String>) -> String {
    match error {
        AcquireError::Timeout => "timed out waiting for a MySQL connection".into(),
        AcquireError::Closed => "MySQL connection pool is closed".into(),
        AcquireError::Connect(error) => error,
    }
}

#[cfg(test)]
#[path = "../../../tests/crates/php-db/db/mysql.rs"]
mod tests;
