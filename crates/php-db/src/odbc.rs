//! SQL Server and Oracle adapter through the platform ODBC driver manager.

use crate::{
    AcquireError, ConnectionConfig, Connector, DatabaseDriver, DatabaseKind, DatabaseSession,
    DbColumnMeta, DbCommand, DbErrorInfo, DbResult, DbResultSet, DbValue, Pool, PooledConnection,
};
use libc::{c_char, c_int, c_short, c_ushort, c_void};
use std::{
    collections::HashMap,
    ffi::{CStr, CString},
    ptr::NonNull,
    sync::Arc,
    time::Duration,
};

type Handle = *mut c_void;
type AllocHandle = unsafe extern "C" fn(c_short, Handle, *mut Handle) -> c_short;
type FreeHandle = unsafe extern "C" fn(c_short, Handle) -> c_short;
type SetEnvAttr = unsafe extern "C" fn(Handle, c_int, *mut c_void, c_int) -> c_short;
type DriverConnect = unsafe extern "C" fn(
    Handle,
    Handle,
    *const c_char,
    c_short,
    *mut c_char,
    c_short,
    *mut c_short,
    c_ushort,
) -> c_short;
type Disconnect = unsafe extern "C" fn(Handle) -> c_short;
type ExecDirect = unsafe extern "C" fn(Handle, *const c_char, c_int) -> c_short;
type Prepare = unsafe extern "C" fn(Handle, *const c_char, c_int) -> c_short;
type BindParameter = unsafe extern "C" fn(
    Handle,
    c_ushort,
    c_short,
    c_short,
    c_short,
    usize,
    c_short,
    *mut c_void,
    isize,
    *mut isize,
) -> c_short;
type Execute = unsafe extern "C" fn(Handle) -> c_short;
type FreeStmt = unsafe extern "C" fn(Handle, c_ushort) -> c_short;
type NumResultCols = unsafe extern "C" fn(Handle, *mut c_short) -> c_short;
type DescribeCol = unsafe extern "C" fn(
    Handle,
    c_ushort,
    *mut c_char,
    c_short,
    *mut c_short,
    *mut c_short,
    *mut usize,
    *mut c_short,
    *mut c_short,
) -> c_short;
type Fetch = unsafe extern "C" fn(Handle) -> c_short;
type MoreResults = unsafe extern "C" fn(Handle) -> c_short;
type GetData =
    unsafe extern "C" fn(Handle, c_ushort, c_short, *mut c_void, isize, *mut isize) -> c_short;
type RowCount = unsafe extern "C" fn(Handle, *mut isize) -> c_short;
type GetDiagRec = unsafe extern "C" fn(
    c_short,
    Handle,
    c_short,
    *mut c_char,
    *mut c_int,
    *mut c_char,
    c_short,
    *mut c_short,
) -> c_short;
type GetInfo =
    unsafe extern "C" fn(Handle, c_ushort, *mut c_void, c_short, *mut c_short) -> c_short;

const SUCCESS: c_short = 0;
const SUCCESS_INFO: c_short = 1;
const NO_DATA: c_short = 100;
const HANDLE_ENV: c_short = 1;
const HANDLE_DBC: c_short = 2;
const HANDLE_STMT: c_short = 3;
const ATTR_ODBC_VERSION: c_int = 200;
const ODBC_VERSION_3: isize = 3;
const DRIVER_NOPROMPT: c_ushort = 0;
const C_CHAR: c_short = 1;
const C_BINARY: c_short = -2;
const SQL_PARAM_INPUT: c_short = 1;
const SQL_VARCHAR: c_short = 12;
const SQL_LONGVARBINARY: c_short = -4;
const SQL_NULL_DATA: isize = -1;
const SQL_CLOSE: c_ushort = 0;
const SQL_RESET_PARAMS: c_ushort = 3;
const SQL_DRIVER_VER: c_ushort = 7;

struct OdbcApi {
    library: Handle,
    alloc: AllocHandle,
    free: FreeHandle,
    set_env_attr: SetEnvAttr,
    connect: DriverConnect,
    disconnect: Disconnect,
    exec: ExecDirect,
    prepare: Prepare,
    bind_parameter: BindParameter,
    execute: Execute,
    free_stmt: FreeStmt,
    num_cols: NumResultCols,
    describe_col: DescribeCol,
    fetch: Fetch,
    more_results: MoreResults,
    get_data: GetData,
    row_count: RowCount,
    diag: GetDiagRec,
    get_info: GetInfo,
}
unsafe impl Send for OdbcApi {}
unsafe impl Sync for OdbcApi {}
impl Drop for OdbcApi {
    fn drop(&mut self) {
        unsafe {
            libc::dlclose(self.library);
        }
    }
}

impl OdbcApi {
    fn load() -> Result<Arc<Self>, String> {
        let mut library = std::ptr::null_mut();
        for name in ["libodbc.so.2", "libodbc.so"] {
            let name = CString::new(name).unwrap();
            library = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
            if !library.is_null() {
                break;
            }
        }
        if library.is_null() {
            return Err("SQL Server/Oracle adapter requires unixODBC or iODBC".into());
        }
        macro_rules! sym {
            ($name:literal, $ty:ty) => {{
                let address = unsafe { libc::dlsym(library, concat!($name, "\0").as_ptr().cast()) };
                if address.is_null() {
                    unsafe {
                        libc::dlclose(library);
                    }
                    return Err(concat!("ODBC manager is missing symbol ", $name).into());
                }
                unsafe { std::mem::transmute::<*mut c_void, $ty>(address) }
            }};
        }
        Ok(Arc::new(Self {
            library,
            alloc: sym!("SQLAllocHandle", AllocHandle),
            free: sym!("SQLFreeHandle", FreeHandle),
            set_env_attr: sym!("SQLSetEnvAttr", SetEnvAttr),
            connect: sym!("SQLDriverConnect", DriverConnect),
            disconnect: sym!("SQLDisconnect", Disconnect),
            exec: sym!("SQLExecDirect", ExecDirect),
            prepare: sym!("SQLPrepare", Prepare),
            bind_parameter: sym!("SQLBindParameter", BindParameter),
            execute: sym!("SQLExecute", Execute),
            free_stmt: sym!("SQLFreeStmt", FreeStmt),
            num_cols: sym!("SQLNumResultCols", NumResultCols),
            describe_col: sym!("SQLDescribeCol", DescribeCol),
            fetch: sym!("SQLFetch", Fetch),
            more_results: sym!("SQLMoreResults", MoreResults),
            get_data: sym!("SQLGetData", GetData),
            row_count: sym!("SQLRowCount", RowCount),
            diag: sym!("SQLGetDiagRec", GetDiagRec),
            get_info: sym!("SQLGetInfo", GetInfo),
        }))
    }
}

#[derive(Clone)]
struct OdbcOptions {
    connection_string: CString,
    kind: DatabaseKind,
}
struct OdbcConnection {
    env: NonNull<c_void>,
    dbc: NonNull<c_void>,
    api: Arc<OdbcApi>,
    kind: DatabaseKind,
}
unsafe impl Send for OdbcConnection {}
impl Drop for OdbcConnection {
    fn drop(&mut self) {
        unsafe {
            (self.api.disconnect)(self.dbc.as_ptr());
            (self.api.free)(HANDLE_DBC, self.dbc.as_ptr());
            (self.api.free)(HANDLE_ENV, self.env.as_ptr());
        }
    }
}

struct OdbcConnector {
    options: OdbcOptions,
    api: Arc<OdbcApi>,
}
impl Connector for OdbcConnector {
    type Connection = OdbcConnection;
    type Error = String;
    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        let mut env = std::ptr::null_mut();
        check(
            unsafe { (self.api.alloc)(HANDLE_ENV, std::ptr::null_mut(), &mut env) },
            "allocate ODBC environment",
        )?;
        let env = NonNull::new(env).ok_or("ODBC returned a null environment")?;
        if let Err(error) = check(
            unsafe {
                (self.api.set_env_attr)(
                    env.as_ptr(),
                    ATTR_ODBC_VERSION,
                    ODBC_VERSION_3 as *mut c_void,
                    0,
                )
            },
            "configure ODBC version",
        ) {
            unsafe {
                (self.api.free)(HANDLE_ENV, env.as_ptr());
            }
            return Err(error);
        }
        let mut dbc = std::ptr::null_mut();
        if let Err(error) = check(
            unsafe { (self.api.alloc)(HANDLE_DBC, env.as_ptr(), &mut dbc) },
            "allocate ODBC connection",
        ) {
            unsafe {
                (self.api.free)(HANDLE_ENV, env.as_ptr());
            }
            return Err(error);
        }
        let dbc = NonNull::new(dbc).ok_or("ODBC returned a null connection")?;
        let mut out = [0i8; 1024];
        let mut out_len = 0;
        let result = unsafe {
            (self.api.connect)(
                dbc.as_ptr(),
                std::ptr::null_mut(),
                self.options.connection_string.as_ptr(),
                -3,
                out.as_mut_ptr(),
                out.len() as c_short,
                &mut out_len,
                DRIVER_NOPROMPT,
            )
        };
        if let Err(error) = check(result, "connect through ODBC") {
            let error = diag(&self.api, HANDLE_DBC, dbc.as_ptr()).unwrap_or(error);
            unsafe {
                (self.api.free)(HANDLE_DBC, dbc.as_ptr());
                (self.api.free)(HANDLE_ENV, env.as_ptr());
            }
            return Err(error);
        }
        Ok(OdbcConnection {
            env,
            dbc,
            api: Arc::clone(&self.api),
            kind: self.options.kind,
        })
    }
}

/// SQL Server and Oracle database driver using configured ODBC vendor drivers.
pub struct OdbcDriver {
    kind: DatabaseKind,
    connector: OdbcConnector,
    pool: Pool<OdbcConnection>,
    acquire_timeout: Duration,
    max_result_rows: usize,
    max_column_bytes: usize,
}
impl OdbcDriver {
    pub fn new(config: &ConnectionConfig) -> Result<Self, String> {
        config.validate()?;
        if !matches!(config.kind, DatabaseKind::SqlServer | DatabaseKind::Oracle) {
            return Err("OdbcDriver supports SQL Server or Oracle DSNs".into());
        }
        let api = OdbcApi::load()?;
        let options = OdbcOptions {
            connection_string: odbc_connection_string(config)?,
            kind: config.kind,
        };
        let connector = OdbcConnector { options, api };
        let pool = Pool::with_options(config.pool)?;
        pool.warm(&connector).map_err(acquire_error)?;
        Ok(Self {
            kind: config.kind,
            connector,
            pool,
            acquire_timeout: config.pool.acquire_timeout,
            max_result_rows: config.max_result_rows,
            max_column_bytes: config.max_column_bytes,
        })
    }
    fn execute_sql(&self, sql: &str, parameters: &[DbValue]) -> Result<DbResult, String> {
        let connection = self
            .pool
            .acquire(&self.connector, self.acquire_timeout)
            .map_err(acquire_error)?;
        let result = execute_on(
            &connection,
            sql,
            parameters,
            self.max_result_rows,
            self.max_column_bytes,
        );
        if result.is_err() {
            connection.discard();
        }
        result
    }
    fn execute_prepared_sql(&self, sql: &str, parameters: &[DbValue]) -> Result<DbResult, String> {
        let connection = self
            .pool
            .acquire(&self.connector, self.acquire_timeout)
            .map_err(acquire_error)?;
        let result = execute_prepared_on(
            &connection,
            sql,
            parameters,
            self.max_result_rows,
            self.max_column_bytes,
        );
        if result.is_err() {
            connection.discard();
        }
        result
    }
}
impl DatabaseDriver for OdbcDriver {
    fn kind(&self) -> DatabaseKind {
        self.kind
    }
    fn client_version(&self) -> Option<String> {
        let connection = self
            .pool
            .acquire(&self.connector, self.acquire_timeout)
            .ok()?;
        let mut version = [0 as c_char; 256];
        let mut length = 0;
        let status = unsafe {
            (connection.api.get_info)(
                connection.dbc.as_ptr(),
                SQL_DRIVER_VER,
                version.as_mut_ptr().cast(),
                version.len() as c_short,
                &mut length,
            )
        };
        if status < 0 || length <= 0 {
            return None;
        }
        Some(
            unsafe { CStr::from_ptr(version.as_ptr()) }
                .to_string_lossy()
                .into_owned(),
        )
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
            DbCommand::Mongo { .. } => Err("MongoDB command cannot be sent through ODBC".into()),
        }
    }
    fn quote_binary(&self, value: &[u8]) -> Result<String, String> {
        quote_odbc_binary(self.kind, value)
    }
    fn start_session(&self) -> Result<Box<dyn DatabaseSession>, String> {
        Ok(Box::new(OdbcSession {
            connection: Some(
                self.pool
                    .acquire(&self.connector, self.acquire_timeout)
                    .map_err(acquire_error)?,
            ),
            prepared: HashMap::new(),
            max_result_rows: self.max_result_rows,
            max_column_bytes: self.max_column_bytes,
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
        Ok(Box::new(OdbcSession {
            connection: Some(connection),
            prepared: HashMap::new(),
            max_result_rows: self.max_result_rows,
            max_column_bytes: self.max_column_bytes,
        }))
    }
}

struct OdbcSession {
    connection: Option<PooledConnection<OdbcConnection>>,
    prepared: HashMap<String, usize>,
    max_result_rows: usize,
    max_column_bytes: usize,
}
impl OdbcSession {
    fn clear_prepared(&mut self) {
        if let Some(connection) = self.connection.as_ref() {
            for (_, handle) in self.prepared.drain() {
                unsafe {
                    (connection.api.free)(HANDLE_STMT, handle as Handle);
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
        if parameters.len() > c_ushort::MAX as usize {
            return Err("ODBC prepared statement has too many parameters".into());
        }
        let connection = self
            .connection
            .as_ref()
            .ok_or("ODBC transaction connection is unusable")?;
        let handle = if let Some(handle) = self.prepared.get(sql) {
            *handle as Handle
        } else {
            let handle = allocate_prepared(connection, sql)?;
            self.prepared.insert(sql.to_owned(), handle as usize);
            handle
        };
        execute_prepared_handle(
            connection,
            handle,
            sql,
            parameters,
            self.max_result_rows,
            self.max_column_bytes,
        )
    }
}
impl Drop for OdbcSession {
    fn drop(&mut self) {
        self.clear_prepared();
    }
}
impl DatabaseSession for OdbcSession {
    fn client_version(&self) -> Option<String> {
        let connection = self.connection.as_ref()?;
        let mut version = [0 as c_char; 256];
        let mut length = 0;
        let status = unsafe {
            (connection.api.get_info)(
                connection.dbc.as_ptr(),
                SQL_DRIVER_VER,
                version.as_mut_ptr().cast(),
                version.len() as c_short,
                &mut length,
            )
        };
        if status < 0 || length <= 0 {
            return None;
        }
        Some(
            unsafe { CStr::from_ptr(version.as_ptr()) }
                .to_string_lossy()
                .into_owned(),
        )
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
                return Err("MongoDB command cannot be sent through ODBC".into())
            }
        };
        let result = if prepared {
            self.execute_cached_prepared(statement, parameters)
        } else {
            let connection = self
                .connection
                .as_ref()
                .ok_or("ODBC transaction connection is unusable")?;
            execute_on(
                connection,
                statement,
                parameters,
                self.max_result_rows,
                self.max_column_bytes,
            )
        };
        if result.is_err() {
            self.clear_prepared();
            self.connection
                .take()
                .expect("session connection")
                .discard();
        }
        result
    }
    fn quote_binary(&mut self, value: &[u8]) -> Result<String, String> {
        let connection = self
            .connection
            .as_ref()
            .ok_or("ODBC PDO connection is unusable")?;
        quote_odbc_binary(connection.kind, value)
    }
}

fn quote_odbc_binary(kind: DatabaseKind, value: &[u8]) -> Result<String, String> {
    let hex = value
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    match kind {
        DatabaseKind::SqlServer => Ok(format!("0x{hex}")),
        DatabaseKind::Oracle => Ok(format!("HEXTORAW('{hex}')")),
        _ => Err("binary quoting is only available for SQL Server and Oracle ODBC drivers".into()),
    }
}

fn execute_on(
    connection: &OdbcConnection,
    sql: &str,
    parameters: &[DbValue],
    max_result_rows: usize,
    max_column_bytes: usize,
) -> Result<DbResult, String> {
    let statement = CString::new(interpolate(sql, parameters, connection.kind)?)
        .map_err(|_| "SQL statement contains NUL")?;
    let mut handle = std::ptr::null_mut();
    check(
        unsafe { (connection.api.alloc)(HANDLE_STMT, connection.dbc.as_ptr(), &mut handle) },
        "allocate ODBC statement",
    )?;
    let handle = NonNull::new(handle).ok_or("ODBC returned a null statement")?;
    let executed = unsafe { (connection.api.exec)(handle.as_ptr(), statement.as_ptr(), -3) };
    if let Err(error) = check(executed, "execute ODBC query") {
        let error = diag(&connection.api, HANDLE_STMT, handle.as_ptr()).unwrap_or(error);
        unsafe {
            (connection.api.free)(HANDLE_STMT, handle.as_ptr());
        }
        return Err(error);
    }
    let result: Result<DbResult, String> = (|| {
        let mut first = read_result(&connection.api, handle, max_result_rows, max_column_bytes)?;
        loop {
            let status = unsafe { (connection.api.more_results)(handle.as_ptr()) };
            if status == NO_DATA {
                break;
            }
            check(status, "advance ODBC result set")?;
            let next = read_result(&connection.api, handle, max_result_rows, max_column_bytes)?;
            first.additional_result_sets.push(DbResultSet {
                columns: next.columns,
                column_metadata: next.column_metadata,
                rows: next.rows,
                affected_rows: next.affected_rows,
            });
        }
        Ok(first)
    })();
    unsafe {
        (connection.api.free)(HANDLE_STMT, handle.as_ptr());
    }
    let mut result = result?;
    attach_last_insert_id(connection, sql, &mut result);
    Ok(result)
}

fn allocate_prepared(connection: &OdbcConnection, sql: &str) -> Result<Handle, String> {
    let sql = CString::new(sql).map_err(|_| "SQL statement contains NUL")?;
    let mut raw = std::ptr::null_mut();
    check(
        unsafe { (connection.api.alloc)(HANDLE_STMT, connection.dbc.as_ptr(), &mut raw) },
        "allocate ODBC statement",
    )?;
    let statement = NonNull::new(raw).ok_or("ODBC returned a null statement")?;
    if let Err(error) = check_statement(
        &connection.api,
        statement.as_ptr(),
        unsafe { (connection.api.prepare)(statement.as_ptr(), sql.as_ptr(), -3) },
        "prepare ODBC statement",
    ) {
        unsafe {
            (connection.api.free)(HANDLE_STMT, statement.as_ptr());
        }
        return Err(error);
    }
    Ok(statement.as_ptr())
}

fn execute_prepared_on(
    connection: &OdbcConnection,
    sql: &str,
    parameters: &[DbValue],
    max_result_rows: usize,
    max_column_bytes: usize,
) -> Result<DbResult, String> {
    if parameters.len() > c_ushort::MAX as usize {
        return Err("ODBC prepared statement has too many parameters".into());
    }
    let handle = allocate_prepared(connection, sql)?;
    let result = execute_prepared_handle(
        connection,
        handle,
        sql,
        parameters,
        max_result_rows,
        max_column_bytes,
    );
    unsafe {
        (connection.api.free)(HANDLE_STMT, handle);
    }
    result
}

fn execute_prepared_handle(
    connection: &OdbcConnection,
    handle: Handle,
    sql: &str,
    parameters: &[DbValue],
    max_result_rows: usize,
    max_column_bytes: usize,
) -> Result<DbResult, String> {
    if parameters.len() > c_ushort::MAX as usize {
        return Err("ODBC prepared statement has too many parameters".into());
    }
    check_statement(
        &connection.api,
        handle,
        unsafe { (connection.api.free_stmt)(handle, SQL_CLOSE) },
        "close previous ODBC result",
    )?;
    check_statement(
        &connection.api,
        handle,
        unsafe { (connection.api.free_stmt)(handle, SQL_RESET_PARAMS) },
        "reset ODBC parameters",
    )?;
    let mut buffers = Vec::<Vec<u8>>::with_capacity(parameters.len());
    let mut indicators = Vec::<isize>::with_capacity(parameters.len());
    let mut types = Vec::<(c_short, c_short, usize, isize)>::with_capacity(parameters.len());
    for value in parameters {
        let (mut bytes, c_type, sql_type) = match value {
            DbValue::Null => (Vec::new(), C_CHAR, SQL_VARCHAR),
            DbValue::Bool(value) => (
                if *value { b"1".to_vec() } else { b"0".to_vec() },
                C_CHAR,
                SQL_VARCHAR,
            ),
            DbValue::Integer(value) => (value.to_string().into_bytes(), C_CHAR, SQL_VARCHAR),
            DbValue::Float(value) if value.is_finite() => {
                (value.to_string().into_bytes(), C_CHAR, SQL_VARCHAR)
            }
            DbValue::Float(_) => {
                return Err("non-finite floats cannot be bound through ODBC".into())
            }
            DbValue::String(value) => (value.as_bytes().to_vec(), C_CHAR, SQL_VARCHAR),
            DbValue::Bytes(value) => (value.clone(), C_BINARY, SQL_LONGVARBINARY),
            DbValue::Array(_) | DbValue::Document(_) => {
                return Err("composite ODBC parameters must be encoded by the caller".into())
            }
        };
        let is_null = matches!(value, DbValue::Null);
        indicators.push(if is_null {
            SQL_NULL_DATA
        } else {
            bytes.len() as isize
        });
        let buffer_length = if is_null {
            0
        } else {
            bytes.len().max(1) as isize
        };
        if !is_null && bytes.is_empty() {
            bytes.push(0);
        }
        types.push((c_type, sql_type, bytes.len().max(1), buffer_length));
        buffers.push(bytes);
    }
    for index in 0..parameters.len() {
        let buffer = if indicators[index] == SQL_NULL_DATA {
            std::ptr::null_mut()
        } else {
            buffers[index].as_mut_ptr().cast()
        };
        let (c_type, sql_type, column_size, buffer_length) = types[index];
        check_statement(
            &connection.api,
            handle,
            unsafe {
                (connection.api.bind_parameter)(
                    handle,
                    (index + 1) as c_ushort,
                    SQL_PARAM_INPUT,
                    c_type,
                    sql_type,
                    column_size,
                    0,
                    buffer,
                    buffer_length,
                    &mut indicators[index],
                )
            },
            "bind ODBC parameter",
        )?;
    }
    check_statement(
        &connection.api,
        handle,
        unsafe { (connection.api.execute)(handle) },
        "execute prepared ODBC statement",
    )?;
    let mut first = read_result(
        &connection.api,
        NonNull::new(handle).ok_or("ODBC returned a null statement")?,
        max_result_rows,
        max_column_bytes,
    )?;
    loop {
        let status = unsafe { (connection.api.more_results)(handle) };
        if status == NO_DATA {
            break;
        }
        check_statement(
            &connection.api,
            handle,
            status,
            "advance ODBC prepared result set",
        )?;
        let next = read_result(
            &connection.api,
            NonNull::new(handle).ok_or("ODBC returned a null statement")?,
            max_result_rows,
            max_column_bytes,
        )?;
        first.additional_result_sets.push(DbResultSet {
            columns: next.columns,
            column_metadata: next.column_metadata,
            rows: next.rows,
            affected_rows: next.affected_rows,
        });
    }
    check_statement(
        &connection.api,
        handle,
        unsafe { (connection.api.free_stmt)(handle, SQL_RESET_PARAMS) },
        "release ODBC parameter buffers",
    )?;
    attach_last_insert_id(connection, sql, &mut first);
    Ok(first)
}

fn attach_last_insert_id(connection: &OdbcConnection, sql: &str, result: &mut DbResult) {
    if connection.kind == DatabaseKind::SqlServer
        && sql
            .trim_start()
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("insert"))
    {
        if let Ok(identity) = execute_on(
            connection,
            "SELECT CONVERT(varchar(100), SCOPE_IDENTITY()) AS id",
            &[],
            1_000_000,
            8 * 1024 * 1024,
        ) {
            result.last_insert_id =
                identity
                    .rows
                    .first()
                    .and_then(|row| row.first())
                    .and_then(|value| match value {
                        DbValue::String(value) if !value.is_empty() => Some(value.clone()),
                        DbValue::Integer(value) => Some(value.to_string()),
                        _ => None,
                    });
        }
    }
}

fn odbc_connection_string(config: &ConnectionConfig) -> Result<CString, String> {
    let (scheme, fields) = config
        .dsn
        .split_once(':')
        .ok_or("ODBC DSN must include a scheme")?;
    let mut options = Vec::<(String, String)>::new();
    for field in fields.split(';').filter(|field| !field.trim().is_empty()) {
        let (key, value) = field
            .split_once('=')
            .ok_or_else(|| format!("invalid ODBC DSN field '{field}'"))?;
        options.push((key.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    let mut parts = Vec::new();
    match scheme.to_ascii_lowercase().as_str() {
        "sqlsrv" | "sqlserver" | "mssql" => {
            let mut server = None;
            let mut database = None;
            for (key, value) in options {
                match key.as_str() {
                    "server" | "host" => server = Some(value),
                    "database" | "dbname" => database = Some(value),
                    "port" | "encrypt" | "trustservercertificate" => {
                        parts.push(format!("{}={}", key, brace(&value)))
                    }
                    other => return Err(format!("unsupported SQL Server DSN option '{other}'")),
                }
            }
            parts.push(format!(
                "DRIVER={}",
                brace(
                    config
                        .driver
                        .as_deref()
                        .unwrap_or("ODBC Driver 18 for SQL Server")
                )
            ));
            if let Some(server) = server {
                parts.push(format!("SERVER={}", brace(&server)));
            }
            if let Some(database) = database {
                parts.push(format!("DATABASE={}", brace(&database)));
            }
        }
        "oci" | "oracle" => {
            let mut dbq = None;
            for (key, value) in options {
                match key.as_str() {
                    "dbname" | "dbq" => dbq = Some(value),
                    "host" | "port" | "service_name" => {
                        parts.push(format!("{}={}", key, brace(&value)))
                    }
                    other => return Err(format!("unsupported Oracle DSN option '{other}'")),
                }
            }
            parts.push(format!(
                "DRIVER={}",
                brace(config.driver.as_deref().unwrap_or("Oracle"))
            ));
            if let Some(dbq) = dbq {
                let dbq = if dbq.starts_with("//") {
                    dbq
                } else {
                    brace(&dbq)
                };
                parts.push(format!("DBQ={dbq}"));
            }
        }
        _ => return Err("OdbcDriver supports SQL Server or Oracle DSNs".into()),
    }
    if let Some(user) = &config.username {
        parts.push(format!("UID={}", brace(user)));
    }
    if let Some(password) = &config.password {
        parts.push(format!("PWD={}", brace(password)));
    }
    CString::new(parts.join(";"))
        .map_err(|_| "ODBC connection settings cannot contain NUL bytes".into())
}
fn brace(value: &str) -> String {
    format!("{{{}}}", value.replace('}', "}}"))
}

fn interpolate(sql: &str, parameters: &[DbValue], kind: DatabaseKind) -> Result<String, String> {
    let mut output = Vec::with_capacity(sql.len() + parameters.len() * 8);
    let mut cursor = 0;
    let mut index = 0;
    let mut quote = 0u8;
    let mut bracket_identifier = false;
    let mut block_comment = false;
    let bytes = sql.as_bytes();
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
        } else if bracket_identifier {
            output.push(byte);
            if byte == b']' {
                if bytes.get(cursor + 1) == Some(&b']') {
                    output.push(b']');
                    cursor += 2;
                    continue;
                }
                bracket_identifier = false;
            }
            cursor += 1;
        } else if quote != 0 {
            output.push(byte);
            if byte == quote {
                if bytes.get(cursor + 1) == Some(&quote) {
                    output.push(quote);
                    cursor += 2;
                    continue;
                }
                quote = 0;
            }
            cursor += 1;
        } else if byte == b'[' && kind == DatabaseKind::SqlServer {
            bracket_identifier = true;
            output.push(byte);
            cursor += 1;
        } else if matches!(byte, b'\'' | b'"') {
            quote = byte;
            output.push(byte);
            cursor += 1;
        } else if bytes.get(cursor..cursor + 2) == Some(b"--") || byte == b'#' {
            let end = bytes[cursor..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |offset| cursor + offset);
            output.extend_from_slice(&bytes[cursor..end]);
            cursor = end;
        } else if bytes.get(cursor..cursor + 2) == Some(b"/*") {
            output.extend_from_slice(b"/*");
            cursor += 2;
            block_comment = true;
        } else if byte == b'?' {
            let value = parameters
                .get(index)
                .ok_or("parameter count does not match ODBC placeholders")?;
            index += 1;
            match value {
                DbValue::Null => output.extend_from_slice(b"NULL"),
                DbValue::Bool(value) => output.extend_from_slice(if *value { b"1" } else { b"0" }),
                DbValue::Integer(value) => output.extend_from_slice(value.to_string().as_bytes()),
                DbValue::Float(value) if value.is_finite() => {
                    output.extend_from_slice(value.to_string().as_bytes())
                }
                DbValue::Float(_) => {
                    return Err("non-finite floats cannot be bound through ODBC".into())
                }
                DbValue::String(value) => {
                    output.push(b'\'');
                    output.extend_from_slice(value.replace('\'', "''").as_bytes());
                    output.push(b'\'');
                }
                DbValue::Bytes(value) => {
                    let hex = value
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    match kind {
                        DatabaseKind::SqlServer => {
                            output.extend_from_slice(format!("0x{hex}").as_bytes())
                        }
                        DatabaseKind::Oracle => {
                            output.extend_from_slice(format!("HEXTORAW('{hex}')").as_bytes())
                        }
                        _ => {
                            return Err("binary ODBC parameters require SQL Server or Oracle".into())
                        }
                    }
                }
                DbValue::Array(_) | DbValue::Document(_) => {
                    return Err("composite ODBC parameters must be encoded by the caller".into())
                }
            }
            cursor += 1;
        } else {
            output.push(byte);
            cursor += 1;
        }
    }
    if index != parameters.len() {
        return Err("parameter count does not match ODBC placeholders".into());
    }
    String::from_utf8(output).map_err(|_| "SQL statement must be valid UTF-8".into())
}

fn read_result(
    api: &OdbcApi,
    statement: NonNull<c_void>,
    max_result_rows: usize,
    max_column_bytes: usize,
) -> Result<DbResult, String> {
    let mut count: c_short = 0;
    check(
        unsafe { (api.num_cols)(statement.as_ptr(), &mut count) },
        "read ODBC result columns",
    )?;
    let mut columns = Vec::new();
    let mut column_metadata = Vec::new();
    for column in 1..=count.max(0) as c_ushort {
        let mut name = [0i8; 512];
        let mut name_len = 0;
        let mut data_type = 0;
        let mut size = 0;
        let mut scale = 0;
        let mut nullable = 0;
        check(
            unsafe {
                (api.describe_col)(
                    statement.as_ptr(),
                    column,
                    name.as_mut_ptr(),
                    name.len() as c_short,
                    &mut name_len,
                    &mut data_type,
                    &mut size,
                    &mut scale,
                    &mut nullable,
                )
            },
            "read ODBC column name",
        )?;
        columns.push(unsafe { CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned() });
        column_metadata.push(DbColumnMeta {
            native_type: Some(odbc_type_name(data_type).into()),
            length: Some(size),
            precision: matches!(data_type, 2 | 3).then_some(size.min(i16::MAX as usize) as i16),
            nullable: match nullable {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            },
            table_name: None,
            catalog_name: None,
        });
    }
    let mut rows = Vec::new();
    if !columns.is_empty() {
        loop {
            let status = unsafe { (api.fetch)(statement.as_ptr()) };
            if status == NO_DATA {
                break;
            }
            check(status, "fetch ODBC row")?;
            if rows.len() >= max_result_rows {
                return Err(format!(
                    "ODBC result exceeds configured max_result_rows ({max_result_rows})"
                ));
            }
            let mut row = Vec::with_capacity(columns.len());
            for column in 1..=columns.len() {
                let mut value = Vec::<u8>::new();
                let mut is_null = false;
                loop {
                    let mut chunk = [0u8; 4096];
                    let mut indicator: isize = 0;
                    let status = unsafe {
                        (api.get_data)(
                            statement.as_ptr(),
                            column as c_ushort,
                            C_CHAR,
                            chunk.as_mut_ptr().cast(),
                            chunk.len() as isize,
                            &mut indicator,
                        )
                    };
                    if status < 0 {
                        return Err(diag(api, HANDLE_STMT, statement.as_ptr())
                            .unwrap_or_else(|| "failed to read ODBC value".into()));
                    }
                    if indicator == -1 {
                        is_null = true;
                        break;
                    }
                    let len = chunk
                        .iter()
                        .position(|byte| *byte == 0)
                        .unwrap_or(chunk.len());
                    if value.len().saturating_add(len) > max_column_bytes {
                        return Err(format!(
                            "ODBC column exceeds configured max_column_bytes ({max_column_bytes})"
                        ));
                    }
                    value.extend_from_slice(&chunk[..len]);
                    if status == SUCCESS {
                        break;
                    }
                    if status != SUCCESS_INFO {
                        check(status, "read ODBC value")?;
                    }
                }
                row.push(if is_null {
                    DbValue::Null
                } else {
                    DbValue::String(String::from_utf8_lossy(&value).into_owned())
                });
            }
            rows.push(row);
        }
    }
    let mut affected: isize = 0;
    let _ = unsafe { (api.row_count)(statement.as_ptr(), &mut affected) };
    let affected_rows = (affected.max(0) as u64).max(rows.len() as u64);
    Ok(DbResult {
        columns,
        column_metadata,
        rows,
        affected_rows,
        last_insert_id: None,
        ..DbResult::default()
    })
}

fn odbc_type_name(data_type: c_short) -> &'static str {
    match data_type {
        -10 => "longnvarchar",
        -9 => "nvarchar",
        -8 => "nchar",
        -7 => "bit",
        -6 => "tinyint",
        -5 => "bigint",
        -4 => "longvarbinary",
        -3 => "varbinary",
        -2 => "binary",
        -1 => "longvarchar",
        1 => "char",
        2 => "numeric",
        3 => "decimal",
        4 => "integer",
        5 => "smallint",
        6 => "float",
        7 => "real",
        8 => "double",
        9 => "datetime",
        12 => "varchar",
        91 => "date",
        92 => "time",
        93 => "timestamp",
        _ => "unknown",
    }
}

fn check(status: c_short, operation: &str) -> Result<(), String> {
    if status == SUCCESS || status == SUCCESS_INFO {
        Ok(())
    } else {
        Err(format!("{operation} failed (ODBC status {status})"))
    }
}
fn check_statement(
    api: &OdbcApi,
    handle: Handle,
    status: c_short,
    operation: &str,
) -> Result<(), String> {
    check(status, operation).map_err(|error| diag(api, HANDLE_STMT, handle).unwrap_or(error))
}
fn diag(api: &OdbcApi, kind: c_short, handle: Handle) -> Option<String> {
    let mut state = [0i8; 6];
    let mut native = 0;
    let mut message = [0i8; 1024];
    let mut length = 0;
    let status = unsafe {
        (api.diag)(
            kind,
            handle,
            1,
            state.as_mut_ptr(),
            &mut native,
            message.as_mut_ptr(),
            message.len() as c_short,
            &mut length,
        )
    };
    (status == SUCCESS || status == SUCCESS_INFO).then(|| {
        let sql_state = unsafe {
            CStr::from_ptr(state.as_ptr())
                .to_string_lossy()
                .into_owned()
        };
        let message = unsafe {
            CStr::from_ptr(message.as_ptr())
                .to_string_lossy()
                .into_owned()
        };
        DbErrorInfo {
            sql_state,
            driver_code: Some(native as i64),
            message,
        }
        .encode()
    })
}
fn acquire_error(error: AcquireError<String>) -> String {
    match error {
        AcquireError::Timeout => "timed out waiting for an ODBC connection".into(),
        AcquireError::Closed => "ODBC connection pool is closed".into(),
        AcquireError::Connect(error) => error,
    }
}

#[cfg(test)]
#[path = "../../../tests/crates/php-db/db/odbc.rs"]
mod tests;
