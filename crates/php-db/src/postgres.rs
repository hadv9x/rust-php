//! PostgreSQL adapter loaded dynamically through libpq's stable C API.

use crate::{AcquireError, ConnectionConfig, Connector, DatabaseDriver, DatabaseKind, DatabaseSession, DbColumnMeta, DbCommand, DbErrorInfo, DbResult, DbResultSet, DbValue, Pool, PooledConnection};
use libc::{c_char, c_int, c_void};
use std::{ffi::{CStr, CString}, ptr::NonNull, sync::Arc, time::Duration};

type ConnectDb = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type ConnectionStatus = unsafe extern "C" fn(*const c_void) -> c_int;
type ConnectionError = unsafe extern "C" fn(*const c_void) -> *const c_char;
type Finish = unsafe extern "C" fn(*mut c_void);
type ExecParams = unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *const u32, *const *const c_char, *const c_int, *const c_int, c_int) -> *mut c_void;
type Exec = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;
type SendQuery = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
type GetResult = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type ResultStatus = unsafe extern "C" fn(*const c_void) -> c_int;
type ResultError = unsafe extern "C" fn(*const c_void) -> *const c_char;
type ResultErrorField = unsafe extern "C" fn(*const c_void, c_int) -> *const c_char;
type TupleCount = unsafe extern "C" fn(*const c_void) -> c_int;
type FieldCount = unsafe extern "C" fn(*const c_void) -> c_int;
type FieldName = unsafe extern "C" fn(*const c_void, c_int) -> *const c_char;
type FieldOid = unsafe extern "C" fn(*const c_void, c_int) -> u32;
type FieldSize = unsafe extern "C" fn(*const c_void, c_int) -> c_int;
type FieldModifier = unsafe extern "C" fn(*const c_void, c_int) -> c_int;
type IsNull = unsafe extern "C" fn(*const c_void, c_int, c_int) -> c_int;
type GetValue = unsafe extern "C" fn(*const c_void, c_int, c_int) -> *const c_char;
type CommandTuples = unsafe extern "C" fn(*const c_void) -> *const c_char;
type Clear = unsafe extern "C" fn(*mut c_void);
type LibVersion = unsafe extern "C" fn() -> c_int;

struct PgApi {
    library: *mut c_void,
    lib_version: Option<LibVersion>,
    connect_db: ConnectDb,
    connection_status: ConnectionStatus,
    connection_error: ConnectionError,
    finish: Finish,
    exec_params: ExecParams,
    exec: Exec,
    send_query: SendQuery,
    get_result: GetResult,
    result_status: ResultStatus,
    result_error: ResultError,
    result_error_field: ResultErrorField,
    tuple_count: TupleCount,
    field_count: FieldCount,
    field_name: FieldName,
    field_oid: FieldOid,
    field_size: FieldSize,
    field_modifier: FieldModifier,
    is_null: IsNull,
    get_value: GetValue,
    command_tuples: CommandTuples,
    clear: Clear,
}

unsafe impl Send for PgApi {}
unsafe impl Sync for PgApi {}

impl Drop for PgApi {
    fn drop(&mut self) { unsafe { libc::dlclose(self.library); } }
}

impl PgApi {
    fn load() -> Result<Arc<Self>, String> {
        let names = ["libpq.so.5", "libpq.so"];
        let mut library = std::ptr::null_mut();
        for name in names {
            let name = CString::new(name).expect("library soname has no NUL");
            library = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
            if !library.is_null() { break; }
        }
        if library.is_null() { return Err("PostgreSQL driver requires libpq.so.5 or libpq.so".into()); }
        macro_rules! sym {
            ($name:literal, $ty:ty) => {{
                let symbol_name = concat!($name, "\0");
                let address = unsafe { libc::dlsym(library, symbol_name.as_ptr().cast()) };
                if address.is_null() {
                    unsafe { libc::dlclose(library); }
                    return Err(concat!("libpq is missing symbol ", $name).into());
                }
                unsafe { std::mem::transmute::<*mut c_void, $ty>(address) }
            }};
        }
        let lib_version_address = unsafe { libc::dlsym(library, c"PQlibVersion".as_ptr()) };
        let lib_version = (!lib_version_address.is_null()).then(|| unsafe {
            std::mem::transmute::<*mut c_void, LibVersion>(lib_version_address)
        });
        Ok(Arc::new(Self {
            library,
            lib_version,
            connect_db: sym!("PQconnectdb", ConnectDb),
            connection_status: sym!("PQstatus", ConnectionStatus),
            connection_error: sym!("PQerrorMessage", ConnectionError),
            finish: sym!("PQfinish", Finish),
            exec_params: sym!("PQexecParams", ExecParams),
            exec: sym!("PQexec", Exec),
            send_query: sym!("PQsendQuery", SendQuery),
            get_result: sym!("PQgetResult", GetResult),
            result_status: sym!("PQresultStatus", ResultStatus),
            result_error: sym!("PQresultErrorMessage", ResultError),
            result_error_field: sym!("PQresultErrorField", ResultErrorField),
            tuple_count: sym!("PQntuples", TupleCount),
            field_count: sym!("PQnfields", FieldCount),
            field_name: sym!("PQfname", FieldName),
            field_oid: sym!("PQftype", FieldOid),
            field_size: sym!("PQfsize", FieldSize),
            field_modifier: sym!("PQfmod", FieldModifier),
            is_null: sym!("PQgetisnull", IsNull),
            get_value: sym!("PQgetvalue", GetValue),
            command_tuples: sym!("PQcmdTuples", CommandTuples),
            clear: sym!("PQclear", Clear),
        }))
    }
}

struct PgConnection {
    raw: NonNull<c_void>,
    api: Arc<PgApi>,
}

unsafe impl Send for PgConnection {}

impl Drop for PgConnection {
    fn drop(&mut self) { unsafe { (self.api.finish)(self.raw.as_ptr()); } }
}

struct PgConnector {
    api: Arc<PgApi>,
    dsn: CString,
}

impl Connector for PgConnector {
    type Connection = PgConnection;
    type Error = String;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        let raw = unsafe { (self.api.connect_db)(self.dsn.as_ptr()) };
        let raw = NonNull::new(raw).ok_or_else(|| "libpq could not allocate a connection".to_owned())?;
        if unsafe { (self.api.connection_status)(raw.as_ptr()) } != 0 {
            let message = unsafe { c_string((self.api.connection_error)(raw.as_ptr())) };
            unsafe { (self.api.finish)(raw.as_ptr()); }
            return Err(format!("PostgreSQL connection failed: {message}"));
        }
        Ok(PgConnection { raw, api: Arc::clone(&self.api) })
    }
}

/// PostgreSQL driver backed by a bounded pool of libpq connections.
pub struct PostgresDriver {
    connector: PgConnector,
    pool: Pool<PgConnection>,
    acquire_timeout: Duration,
    max_result_rows: usize,
}

impl PostgresDriver {
    pub fn new(config: &ConnectionConfig) -> Result<Self, String> {
        config.validate()?;
        if config.kind != DatabaseKind::PostgreSql { return Err("PostgresDriver requires a pgsql/postgres DSN".into()); }
        let api = PgApi::load()?;
        let mut dsn = postgres_conninfo(&config.dsn)?;
        if let Some(user) = &config.username { dsn.push_str(&format!(" user={}", quote_conninfo(user))); }
        if let Some(password) = &config.password { dsn.push_str(&format!(" password={}", quote_conninfo(password))); }
        let dsn = CString::new(dsn).map_err(|_| "PostgreSQL DSN cannot contain a NUL byte".to_owned())?;
        let connector = PgConnector { api, dsn };
        let pool = Pool::with_options(config.pool)?;
        pool.warm(&connector).map_err(acquire_error)?;
        Ok(Self { connector, pool, acquire_timeout: config.pool.acquire_timeout, max_result_rows: config.max_result_rows })
    }

    fn acquire(&self) -> Result<PooledConnection<PgConnection>, String> {
        self.pool.acquire(&self.connector, self.acquire_timeout).map_err(acquire_error)
    }

    fn execute_sql(&self, sql: &str, parameters: &[DbValue]) -> Result<DbResult, String> {
        let connection = self.acquire()?;
        let result = execute_on(&connection, sql, parameters, self.max_result_rows);
        if result.is_err() && unsafe { (connection.api.connection_status)(connection.raw.as_ptr()) } != 0 { connection.discard(); }
        result
    }

    fn ping_db(&self) -> Result<(), String> {
        let connection = self.acquire()?;
        let sql = c"SELECT 1";
        let result = unsafe { (connection.api.exec)(connection.raw.as_ptr(), sql.as_ptr()) };
        let result = NonNull::new(result).ok_or_else(|| connection_error(&connection))?;
        let status = unsafe { (connection.api.result_status)(result.as_ptr()) };
        unsafe { (connection.api.clear)(result.as_ptr()); }
        if status == 2 { Ok(()) } else { Err("PostgreSQL health check failed".into()) }
    }
}

impl DatabaseDriver for PostgresDriver {
    fn kind(&self) -> DatabaseKind { DatabaseKind::PostgreSql }
    fn client_version(&self) -> Option<String> {
        let version = unsafe { (self.connector.api.lib_version?)() };
        if version <= 0 { return None; }
        let version = version as u32;
        Some(format!("{}.{}.{}", version / 10000, (version / 100) % 100, version % 100))
    }
    fn ping(&self) -> Result<(), String> { self.ping_db() }
    fn execute(&self, command: &DbCommand) -> Result<DbResult, String> {
        match command {
            DbCommand::Sql { statement, parameters } => self.execute_sql(statement, parameters),
            DbCommand::PreparedSql { statement, parameters } => self.execute_sql(statement, parameters),
            DbCommand::Mongo { .. } => Err("MongoDB command cannot be sent to PostgreSQL".into()),
        }
    }
    fn quote_string(&self, value: &str) -> Result<String, String> {
        quote_postgres_string(value)
    }
    fn quote_binary(&self, value: &[u8]) -> Result<String, String> {
        Ok(format!("decode('{}', 'hex')", value.iter().map(|byte| format!("{byte:02x}")).collect::<String>()))
    }
    fn start_session(&self) -> Result<Box<dyn DatabaseSession>, String> {
        Ok(Box::new(PgSession { connection: Some(self.acquire()?), max_result_rows: self.max_result_rows }))
    }
    fn start_session_with_timeout(&self, acquire_timeout: Duration) -> Result<Box<dyn DatabaseSession>, String> {
        let timeout = if acquire_timeout.is_zero() { self.acquire_timeout } else { acquire_timeout };
        let connection = self.pool.acquire(&self.connector, timeout).map_err(acquire_error)?;
        Ok(Box::new(PgSession { connection: Some(connection), max_result_rows: self.max_result_rows }))
    }
}

struct PgSession { connection: Option<PooledConnection<PgConnection>>, max_result_rows: usize }
impl DatabaseSession for PgSession {
    fn client_version(&self) -> Option<String> {
        let version = unsafe { (self.connection.as_ref()?.api.lib_version?)() };
        if version <= 0 { return None; }
        let version = version as u32;
        Some(format!("{}.{}.{}", version / 10000, (version / 100) % 100, version % 100))
    }
    fn execute(&mut self, command: &DbCommand) -> Result<DbResult, String> {
        let (statement, parameters) = match command {
            DbCommand::Sql { statement, parameters } | DbCommand::PreparedSql { statement, parameters } => (statement, parameters),
            DbCommand::Mongo { .. } => return Err("MongoDB command cannot be sent to PostgreSQL".into()),
        };
        let connection = self.connection.as_ref().ok_or("PostgreSQL transaction connection is unusable")?;
        let result = execute_on(connection, statement, parameters, self.max_result_rows);
        if result.is_err() && unsafe { (connection.api.connection_status)(connection.raw.as_ptr()) } != 0 {
            self.connection.take().expect("session connection").discard();
        }
        result
    }
    fn quote_string(&mut self, value: &str) -> Result<String, String> { quote_postgres_string(value) }
    fn quote_binary(&mut self, value: &[u8]) -> Result<String, String> {
        Ok(format!("decode('{}', 'hex')", value.iter().map(|byte| format!("{byte:02x}")).collect::<String>()))
    }
}

fn quote_postgres_string(value: &str) -> Result<String, String> {
    if value.contains('\0') { return Err("PostgreSQL text values cannot contain NUL bytes".into()); }
    let mut escaped = String::with_capacity(value.len() + 3);
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\'' => escaped.push_str("\\'"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() && (character as u32) <= 0xff => escaped.push_str(&format!("\\{:03o}", character as u32)),
            character if character.is_control() && (character as u32) <= 0xffff => escaped.push_str(&format!("\\u{:04X}", character as u32)),
            character if character.is_control() => escaped.push_str(&format!("\\U{:08X}", character as u32)),
            character => escaped.push(character),
        }
    }
    Ok(format!("E'{escaped}'"))
}

fn execute_on(connection: &PgConnection, statement: &str, parameters: &[DbValue], max_result_rows: usize) -> Result<DbResult, String> {
    let sql = CString::new(statement).map_err(|_| "SQL statement cannot contain a NUL byte".to_owned())?;
    if parameters.is_empty() {
        let mut result = execute_simple_query(connection, &sql, max_result_rows)?;
        let normalized = statement.trim_start().to_ascii_lowercase();
        if normalized.starts_with("insert") && normalized.split(|character: char| !character.is_ascii_alphabetic()).any(|word| word == "returning") {
            result.last_insert_id = result.rows.first().and_then(|row| row.first()).and_then(|value| match value {
                DbValue::String(value) => Some(value.clone()), DbValue::Integer(value) => Some(value.to_string()), _ => None,
            });
        }
        return Ok(result);
    }
    let encoded = parameters.iter().map(parameter_text).collect::<Result<Vec<_>, _>>()?;
    const BYTEA_OID: u32 = 17;
    let parameter_types = parameters.iter().map(|value| if matches!(value, DbValue::Bytes(_)) { BYTEA_OID } else { 0 }).collect::<Vec<_>>();
    let parameter_types = parameter_types.iter().any(|oid| *oid != 0).then_some(parameter_types.as_ptr()).unwrap_or(std::ptr::null());
    let pointers = encoded.iter().map(|value| value.as_ref().map_or(std::ptr::null(), |value| value.as_ptr())).collect::<Vec<_>>();
    let result = unsafe { (connection.api.exec_params)(connection.raw.as_ptr(), sql.as_ptr(), pointers.len() as c_int, parameter_types, pointers.as_ptr(), std::ptr::null(), std::ptr::null(), 0) };
    let Some(result) = NonNull::new(result) else { return Err(connection_error(connection)); };
    let mut result = read_result(&connection.api, result, max_result_rows)?;
    let normalized = statement.trim_start().to_ascii_lowercase();
    if normalized.starts_with("insert") && normalized.split(|character: char| !character.is_ascii_alphabetic()).any(|word| word == "returning") {
        result.last_insert_id = result.rows.first().and_then(|row| row.first()).and_then(|value| match value {
            DbValue::String(value) => Some(value.clone()), DbValue::Integer(value) => Some(value.to_string()), _ => None,
        });
    }
    Ok(result)
}

fn execute_simple_query(connection: &PgConnection, sql: &CString, max_result_rows: usize) -> Result<DbResult, String> {
    if unsafe { (connection.api.send_query)(connection.raw.as_ptr(), sql.as_ptr()) } == 0 {
        return Err(connection_error(connection));
    }
    let mut first: Option<DbResult> = None;
    let mut failure = None;
    loop {
        let raw = unsafe { (connection.api.get_result)(connection.raw.as_ptr()) };
        let Some(raw) = NonNull::new(raw) else { break; };
        match read_result(&connection.api, raw, max_result_rows) {
            Ok(next) => {
                if let Some(first) = &mut first {
                    first.additional_result_sets.push(DbResultSet { columns: next.columns, column_metadata: next.column_metadata,
                        rows: next.rows, affected_rows: next.affected_rows });
                } else { first = Some(next); }
            }
            Err(error) => { failure.get_or_insert(error); }
        }
    }
    if let Some(error) = failure { return Err(error); }
    first.ok_or_else(|| "PostgreSQL command returned no result set".into())
}

fn read_result(api: &PgApi, result: NonNull<c_void>, max_result_rows: usize) -> Result<DbResult, String> {
    let status = unsafe { (api.result_status)(result.as_ptr()) };
    if status == 0 { // PGRES_EMPTY_QUERY
        unsafe { (api.clear)(result.as_ptr()); }
        return Ok(DbResult::default());
    }
    if status != 1 && status != 2 {
        let message = unsafe { c_string((api.result_error)(result.as_ptr())) };
        let sql_state = unsafe { c_string((api.result_error_field)(result.as_ptr(), b'C' as c_int)) };
        unsafe { (api.clear)(result.as_ptr()); }
        return Err(DbErrorInfo { sql_state: if sql_state.len() != 5 || sql_state == "00000" { "HY000".into() } else { sql_state }, driver_code: None, message }.encode());
    }
    let field_count = unsafe { (api.field_count)(result.as_ptr()) }.max(0) as usize;
    let row_count = unsafe { (api.tuple_count)(result.as_ptr()) }.max(0) as usize;
    if row_count > max_result_rows {
        unsafe { (api.clear)(result.as_ptr()); }
        return Err(format!("PostgreSQL result exceeds configured max_result_rows ({max_result_rows})"));
    }
    let columns = (0..field_count).map(|column| unsafe { c_string((api.field_name)(result.as_ptr(), column as c_int)) }).collect();
    let column_metadata = (0..field_count).map(|column| {
        let oid = unsafe { (api.field_oid)(result.as_ptr(), column as c_int) };
        let native_type = match oid {
            16 => "bool", 17 => "bytea", 20 => "int8", 21 => "int2", 23 => "int4", 25 => "text",
            700 => "float4", 701 => "float8", 1082 => "date", 1083 => "time", 1114 => "timestamp", 1184 => "timestamptz",
            1700 => "numeric", 2950 => "uuid", 114 => "json", 3802 => "jsonb",
            _ => "unknown",
        };
        let size = unsafe { (api.field_size)(result.as_ptr(), column as c_int) };
        let modifier = unsafe { (api.field_modifier)(result.as_ptr(), column as c_int) };
        let _ = modifier;
        DbColumnMeta { native_type: Some(native_type.into()), length: (size >= 0).then_some(size as usize), precision: None, nullable: None, table_name: None, catalog_name: None }
    }).collect();
    let mut rows = Vec::with_capacity(row_count);
    for row in 0..row_count {
        let mut values = Vec::with_capacity(field_count);
        for column in 0..field_count {
            let value = if unsafe { (api.is_null)(result.as_ptr(), row as c_int, column as c_int) } != 0 {
                DbValue::Null
            } else {
                DbValue::String(unsafe { c_string((api.get_value)(result.as_ptr(), row as c_int, column as c_int)) })
            };
            values.push(value);
        }
        rows.push(values);
    }
    let affected_rows = unsafe { c_string((api.command_tuples)(result.as_ptr())) }.parse().unwrap_or(0);
    unsafe { (api.clear)(result.as_ptr()); }
    Ok(DbResult { columns, column_metadata, rows, affected_rows, last_insert_id: None, ..DbResult::default() })
}

fn parameter_text(value: &DbValue) -> Result<Option<CString>, String> {
    let value = match value {
        DbValue::Null => return Ok(None),
        DbValue::Bool(value) => if *value { "true".to_owned() } else { "false".to_owned() },
        DbValue::Integer(value) => value.to_string(),
        DbValue::Float(value) if value.is_finite() => value.to_string(),
        DbValue::Float(_) => return Err("non-finite floats cannot be bound to PostgreSQL".into()),
        DbValue::String(value) => value.clone(),
        DbValue::Bytes(value) => format!("\\x{}", value.iter().map(|byte| format!("{byte:02x}")).collect::<String>()),
        DbValue::Array(_) | DbValue::Document(_) => return Err("composite parameters must be encoded by the caller".into()),
    };
    CString::new(value).map(Some).map_err(|_| "PostgreSQL text parameters cannot contain NUL bytes".into())
}

fn connection_error(connection: &PgConnection) -> String {
    unsafe { c_string((connection.api.connection_error)(connection.raw.as_ptr())) }
}

unsafe fn c_string(value: *const c_char) -> String {
    if value.is_null() { String::new() } else { CStr::from_ptr(value).to_string_lossy().into_owned() }
}

fn quote_conninfo(value: &str) -> String { format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'")) }

fn postgres_conninfo(dsn: &str) -> Result<String, String> {
    let Some((scheme, rest)) = dsn.split_once(':') else { return Err("PostgreSQL DSN must include a scheme".into()); };
    if matches!(scheme.to_ascii_lowercase().as_str(), "postgres" | "postgresql") { return Ok(dsn.to_owned()); }
    if !scheme.eq_ignore_ascii_case("pgsql") { return Err("PostgresDriver requires a pgsql/postgres DSN".into()); }
    let mut fields = Vec::new();
    for field in rest.split(';').filter(|field| !field.trim().is_empty()) {
        let (key, value) = field.split_once('=').ok_or_else(|| format!("invalid PostgreSQL DSN field '{field}'"))?;
        let key = key.trim();
        if !matches!(key, "host" | "port" | "dbname" | "user" | "password" | "sslmode" | "connect_timeout" | "application_name" | "options" | "service" | "target_session_attrs") {
            return Err(format!("unsupported PostgreSQL DSN option '{key}'"));
        }
        fields.push(format!("{key}={}", quote_conninfo(value.trim())));
    }
    Ok(fields.join(" "))
}

fn acquire_error(error: AcquireError<String>) -> String {
    match error {
        AcquireError::Timeout => "timed out waiting for a PostgreSQL connection".into(),
        AcquireError::Closed => "PostgreSQL connection pool is closed".into(),
        AcquireError::Connect(error) => error,
    }
}

#[cfg(test)]
#[path = "../../../tests/crates/php-db/db/postgres.rs"]
mod tests;
