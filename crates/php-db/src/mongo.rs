//! MongoDB document driver loaded from the official libmongoc runtime.

use crate::{DatabaseDriver, DatabaseKind, DatabaseSession, DbCommand, DbResult, DbValue};
use libc::{c_char, c_int, c_void};
use std::{
    ffi::{CStr, CString},
    ptr::NonNull,
    sync::{Arc, OnceLock},
};

type Bool = c_int;
type Init = unsafe extern "C" fn();
type UriNew = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type UriSetInt = unsafe extern "C" fn(*mut c_void, *const c_char, i32) -> Bool;
type UriSetString = unsafe extern "C" fn(*mut c_void, *const c_char) -> Bool;
type UriDestroy = unsafe extern "C" fn(*mut c_void);
type PoolNew = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type PoolDestroy = unsafe extern "C" fn(*mut c_void);
type PoolPop = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type PoolPush = unsafe extern "C" fn(*mut c_void, *mut c_void);
type GetCollection = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> *mut c_void;
type CollectionDestroy = unsafe extern "C" fn(*mut c_void);
type BsonFromJson = unsafe extern "C" fn(*const u8, isize, *mut c_void) -> *mut c_void;
type BsonNew = unsafe extern "C" fn() -> *mut c_void;
type BsonDestroy = unsafe extern "C" fn(*mut c_void);
type BsonAsJson = unsafe extern "C" fn(*const c_void, *mut usize) -> *mut c_char;
type BsonFree = unsafe extern "C" fn(*mut c_void);
type Find =
    unsafe extern "C" fn(*mut c_void, *const c_void, *const c_void, *const c_void) -> *mut c_void;
type CountDocuments = unsafe extern "C" fn(
    *mut c_void,
    *const c_void,
    *const c_void,
    *const c_void,
    *mut c_void,
    *mut c_void,
) -> i64;
type EstimatedDocumentCount = unsafe extern "C" fn(
    *mut c_void,
    *const c_void,
    *const c_void,
    *mut c_void,
    *mut c_void,
) -> i64;
type Aggregate = unsafe extern "C" fn(
    *mut c_void,
    c_int,
    *const c_void,
    *const c_void,
    *const c_void,
) -> *mut c_void;
type CursorNext = unsafe extern "C" fn(*mut c_void, *mut *const c_void) -> Bool;
type CursorError = unsafe extern "C" fn(*const c_void, *mut c_void) -> Bool;
type CursorDestroy = unsafe extern "C" fn(*mut c_void);
type InsertOne = unsafe extern "C" fn(
    *mut c_void,
    *const c_void,
    *const c_void,
    *mut c_void,
    *mut c_void,
) -> Bool;
type InsertMany = unsafe extern "C" fn(
    *mut c_void,
    *const *const c_void,
    usize,
    *const c_void,
    *mut c_void,
    *mut c_void,
) -> Bool;
type Update = unsafe extern "C" fn(
    *mut c_void,
    *const c_void,
    *const c_void,
    *const c_void,
    *mut c_void,
    *mut c_void,
) -> Bool;
type Delete = unsafe extern "C" fn(
    *mut c_void,
    *const c_void,
    *const c_void,
    *mut c_void,
    *mut c_void,
) -> Bool;
type CommandSimple = unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_void,
    *const c_void,
    *mut c_void,
    *mut c_void,
) -> Bool;
type ClientStartSession =
    unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void) -> *mut c_void;
type SessionStartTransaction =
    unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void) -> Bool;
type SessionAppend = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> Bool;
type SessionEnd = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> Bool;
type SessionAbort = unsafe extern "C" fn(*mut c_void, *mut c_void) -> Bool;
type SessionDestroy = unsafe extern "C" fn(*mut c_void);

#[repr(C)]
struct BsonError {
    domain: u32,
    code: u32,
    message: [c_char; 504],
}

impl BsonError {
    fn new() -> Self {
        unsafe { std::mem::zeroed() }
    }
    fn message(&self) -> String {
        let message = unsafe { CStr::from_ptr(self.message.as_ptr()) }
            .to_string_lossy()
            .trim()
            .to_owned();
        if message.is_empty() {
            format!("libmongoc error domain={} code={}", self.domain, self.code)
        } else {
            format!(
                "libmongoc error domain={} code={}: {message}",
                self.domain, self.code
            )
        }
    }
}

struct Api {
    library: *mut c_void,
    init: Init,
    uri_new: UriNew,
    uri_set_int: UriSetInt,
    uri_set_username: UriSetString,
    uri_set_password: UriSetString,
    uri_destroy: UriDestroy,
    pool_new: PoolNew,
    pool_destroy: PoolDestroy,
    pool_pop: PoolPop,
    pool_push: PoolPush,
    get_collection: GetCollection,
    collection_destroy: CollectionDestroy,
    bson_from_json: BsonFromJson,
    bson_new: BsonNew,
    bson_destroy: BsonDestroy,
    bson_as_json: BsonAsJson,
    bson_free: BsonFree,
    find: Find,
    count_documents: CountDocuments,
    estimated_document_count: EstimatedDocumentCount,
    aggregate: Aggregate,
    cursor_next: CursorNext,
    cursor_error: CursorError,
    cursor_destroy: CursorDestroy,
    insert_one: InsertOne,
    insert_many: InsertMany,
    replace_one: Update,
    update_one: Update,
    update_many: Update,
    delete_one: Delete,
    delete_many: Delete,
    command_simple: CommandSimple,
    client_start_session: ClientStartSession,
    session_start_transaction: SessionStartTransaction,
    session_append: SessionAppend,
    session_commit: SessionEnd,
    session_abort: SessionAbort,
    session_destroy: SessionDestroy,
}
unsafe impl Send for Api {}
unsafe impl Sync for Api {}
impl Drop for Api {
    fn drop(&mut self) {
        unsafe {
            libc::dlclose(self.library);
        }
    }
}

impl Api {
    fn load() -> Result<Arc<Self>, String> {
        static API_CACHE: OnceLock<Result<Arc<Api>, String>> = OnceLock::new();
        API_CACHE.get_or_init(Self::load_and_init).clone()
    }

    fn load_and_init() -> Result<Arc<Self>, String> {
        let mut library = std::ptr::null_mut();
        for name in ["libmongoc-1.0.so.0", "libmongoc-1.0.so"] {
            let name = CString::new(name).unwrap();
            library = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
            if !library.is_null() {
                break;
            }
        }
        if library.is_null() {
            return Err("MongoDB driver requires the official libmongoc runtime".into());
        }
        macro_rules! sym {
            ($name:literal, $ty:ty) => {{
                let address = unsafe { libc::dlsym(library, concat!($name, "\0").as_ptr().cast()) };
                if address.is_null() {
                    unsafe {
                        libc::dlclose(library);
                    }
                    return Err(concat!("libmongoc is missing symbol ", $name).into());
                }
                unsafe { std::mem::transmute::<*mut c_void, $ty>(address) }
            }};
        }
        let api = Arc::new(Self {
            library,
            init: sym!("mongoc_init", Init),
            uri_new: sym!("mongoc_uri_new", UriNew),
            uri_set_int: sym!("mongoc_uri_set_option_as_int32", UriSetInt),
            uri_set_username: sym!("mongoc_uri_set_username", UriSetString),
            uri_set_password: sym!("mongoc_uri_set_password", UriSetString),
            uri_destroy: sym!("mongoc_uri_destroy", UriDestroy),
            pool_new: sym!("mongoc_client_pool_new", PoolNew),
            pool_destroy: sym!("mongoc_client_pool_destroy", PoolDestroy),
            pool_pop: sym!("mongoc_client_pool_pop", PoolPop),
            pool_push: sym!("mongoc_client_pool_push", PoolPush),
            get_collection: sym!("mongoc_client_get_collection", GetCollection),
            collection_destroy: sym!("mongoc_collection_destroy", CollectionDestroy),
            bson_from_json: sym!("bson_new_from_json", BsonFromJson),
            bson_new: sym!("bson_new", BsonNew),
            bson_destroy: sym!("bson_destroy", BsonDestroy),
            bson_as_json: sym!("bson_as_canonical_extended_json", BsonAsJson),
            bson_free: sym!("bson_free", BsonFree),
            find: sym!("mongoc_collection_find_with_opts", Find),
            count_documents: sym!("mongoc_collection_count_documents", CountDocuments),
            estimated_document_count: sym!(
                "mongoc_collection_estimated_document_count",
                EstimatedDocumentCount
            ),
            aggregate: sym!("mongoc_collection_aggregate", Aggregate),
            cursor_next: sym!("mongoc_cursor_next", CursorNext),
            cursor_error: sym!("mongoc_cursor_error", CursorError),
            cursor_destroy: sym!("mongoc_cursor_destroy", CursorDestroy),
            insert_one: sym!("mongoc_collection_insert_one", InsertOne),
            insert_many: sym!("mongoc_collection_insert_many", InsertMany),
            replace_one: sym!("mongoc_collection_replace_one", Update),
            update_one: sym!("mongoc_collection_update_one", Update),
            update_many: sym!("mongoc_collection_update_many", Update),
            delete_one: sym!("mongoc_collection_delete_one", Delete),
            delete_many: sym!("mongoc_collection_delete_many", Delete),
            command_simple: sym!("mongoc_client_command_simple", CommandSimple),
            client_start_session: sym!("mongoc_client_start_session", ClientStartSession),
            session_start_transaction: sym!(
                "mongoc_client_session_start_transaction",
                SessionStartTransaction
            ),
            session_append: sym!("mongoc_client_session_append", SessionAppend),
            session_commit: sym!("mongoc_client_session_commit_transaction", SessionEnd),
            session_abort: sym!("mongoc_client_session_abort_transaction", SessionAbort),
            session_destroy: sym!("mongoc_client_session_destroy", SessionDestroy),
        });
        unsafe {
            (api.init)();
        }
        Ok(api)
    }
}

/// Driver uses MongoDB's own topology-aware per-server connection pool.
struct MongoClientPool {
    api: Arc<Api>,
    raw: NonNull<c_void>,
}
unsafe impl Send for MongoClientPool {}
unsafe impl Sync for MongoClientPool {}
impl Drop for MongoClientPool {
    fn drop(&mut self) {
        unsafe {
            (self.api.pool_destroy)(self.raw.as_ptr());
        }
    }
}

struct MongoUri {
    api: Arc<Api>,
    raw: NonNull<c_void>,
}
unsafe impl Send for MongoUri {}
impl Drop for MongoUri {
    fn drop(&mut self) {
        unsafe {
            (self.api.uri_destroy)(self.raw.as_ptr());
        }
    }
}

struct MongoClientLease {
    pool: Arc<MongoClientPool>,
    raw: NonNull<c_void>,
}
unsafe impl Send for MongoClientLease {}
impl Drop for MongoClientLease {
    fn drop(&mut self) {
        unsafe {
            (self.pool.api.pool_push)(self.pool.raw.as_ptr(), self.raw.as_ptr());
        }
    }
}

struct BsonOwner {
    raw: NonNull<c_void>,
    destroy: BsonDestroy,
}
impl BsonOwner {
    fn as_ptr(&self) -> *mut c_void {
        self.raw.as_ptr()
    }
}
impl Drop for BsonOwner {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)(self.raw.as_ptr());
        }
    }
}

struct CollectionOwner {
    raw: NonNull<c_void>,
    destroy: CollectionDestroy,
}
impl CollectionOwner {
    fn as_ptr(&self) -> *mut c_void {
        self.raw.as_ptr()
    }
}
impl Drop for CollectionOwner {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)(self.raw.as_ptr());
        }
    }
}

struct CursorOwner {
    raw: NonNull<c_void>,
    destroy: CursorDestroy,
}
impl CursorOwner {
    fn as_ptr(&self) -> *mut c_void {
        self.raw.as_ptr()
    }
}
impl Drop for CursorOwner {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)(self.raw.as_ptr());
        }
    }
}

pub struct MongoDriver {
    api: Arc<Api>,
    _uri: MongoUri,
    pool: Arc<MongoClientPool>,
    max_result_rows: usize,
}
unsafe impl Send for MongoDriver {}
unsafe impl Sync for MongoDriver {}

impl MongoDriver {
    pub fn new(config: &crate::ConnectionConfig) -> Result<Self, String> {
        config.validate()?;
        if config.kind != DatabaseKind::MongoDb {
            return Err("MongoDriver requires a mongodb:// DSN".into());
        }
        let api = Api::load()?;
        let uri_text =
            CString::new(config.dsn.clone()).map_err(|_| "MongoDB URI cannot contain NUL bytes")?;
        let uri = MongoUri {
            api: Arc::clone(&api),
            raw: NonNull::new(unsafe { (api.uri_new)(uri_text.as_ptr()) })
                .ok_or("invalid MongoDB URI")?,
        };
        if let Some(username) = &config.username {
            let username = CString::new(username.as_str())
                .map_err(|_| "MongoDB username cannot contain NUL bytes")?;
            if unsafe { (api.uri_set_username)(uri.raw.as_ptr(), username.as_ptr()) } == 0 {
                return Err("invalid MongoDB username".into());
            }
        }
        if let Some(password) = &config.password {
            let password = CString::new(password.as_str())
                .map_err(|_| "MongoDB password cannot contain NUL bytes")?;
            if unsafe { (api.uri_set_password)(uri.raw.as_ptr(), password.as_ptr()) } == 0 {
                return Err("invalid MongoDB password".into());
            }
        }
        set_uri_int(
            &api,
            uri.raw.as_ptr(),
            "maxPoolSize",
            config.pool.max_connections,
        )?;
        if config.pool.min_connections > 0 {
            set_uri_int(
                &api,
                uri.raw.as_ptr(),
                "minPoolSize",
                config.pool.min_connections,
            )?;
        }
        set_uri_int(
            &api,
            uri.raw.as_ptr(),
            "waitQueueTimeoutMS",
            config
                .pool
                .acquire_timeout
                .as_millis()
                .min(i32::MAX as u128) as usize,
        )?;
        if let Some(timeout) = config.pool.idle_timeout {
            set_uri_int(
                &api,
                uri.raw.as_ptr(),
                "maxIdleTimeMS",
                timeout.as_millis().min(i32::MAX as u128) as usize,
            )?;
        }
        let raw_pool = NonNull::new(unsafe { (api.pool_new)(uri.raw.as_ptr()) })
            .ok_or("could not create MongoDB client pool")?;
        let pool = Arc::new(MongoClientPool {
            api: Arc::clone(&api),
            raw: raw_pool,
        });
        Ok(Self {
            api,
            _uri: uri,
            pool,
            max_result_rows: config.max_result_rows,
        })
    }

    fn execute_mongo(
        &self,
        database: &str,
        collection: &str,
        action: &crate::MongoAction,
    ) -> Result<DbResult, String> {
        let database =
            CString::new(database).map_err(|_| "MongoDB database name contains NUL bytes")?;
        let collection_name =
            CString::new(collection).map_err(|_| "MongoDB collection name contains NUL bytes")?;
        let client = lease_client(Arc::clone(&self.pool))?;
        execute_action(
            &self.api,
            client.raw.as_ptr(),
            database.as_ptr(),
            collection_name.as_ptr(),
            action,
            None,
            self.max_result_rows,
        )
    }
}

struct MongoSession {
    api: Arc<Api>,
    client: Option<MongoClientLease>,
    session: NonNull<c_void>,
    transaction_active: bool,
    max_result_rows: usize,
}
unsafe impl Send for MongoSession {}

impl Drop for MongoSession {
    fn drop(&mut self) {
        unsafe {
            if self.transaction_active {
                (self.api.session_abort)(self.session.as_ptr(), std::ptr::null_mut());
            }
            (self.api.session_destroy)(self.session.as_ptr());
            drop(self.client.take());
        }
    }
}

impl DatabaseSession for MongoSession {
    fn execute(&mut self, command: &DbCommand) -> Result<DbResult, String> {
        let DbCommand::Mongo {
            database,
            collection,
            action,
        } = command
        else {
            return Err("SQL command cannot be sent through a MongoDB session".into());
        };
        let database = CString::new(database.as_str())
            .map_err(|_| "MongoDB database name contains NUL bytes")?;
        let collection = CString::new(collection.as_str())
            .map_err(|_| "MongoDB collection name contains NUL bytes")?;
        execute_action(
            &self.api,
            self.client
                .as_ref()
                .ok_or("MongoDB session is closed")?
                .raw
                .as_ptr(),
            database.as_ptr(),
            collection.as_ptr(),
            action,
            Some(self.session.as_ptr()),
            self.max_result_rows,
        )
    }

    fn begin_transaction(&mut self) -> Result<(), String> {
        let mut error = BsonError::new();
        if unsafe {
            (self.api.session_start_transaction)(
                self.session.as_ptr(),
                std::ptr::null(),
                (&mut error as *mut BsonError).cast(),
            )
        } != 0
        {
            self.transaction_active = true;
            Ok(())
        } else {
            Err(format!("MongoDB could not start transaction: {} (transactions require a replica set or sharded cluster)", error.message()))
        }
    }

    fn commit_transaction(&mut self) -> Result<(), String> {
        let mut error = BsonError::new();
        if unsafe {
            (self.api.session_commit)(
                self.session.as_ptr(),
                std::ptr::null_mut(),
                (&mut error as *mut BsonError).cast(),
            )
        } != 0
        {
            self.transaction_active = false;
            Ok(())
        } else {
            Err(format!(
                "MongoDB transaction commit failed: {}",
                error.message()
            ))
        }
    }

    fn rollback_transaction(&mut self) -> Result<(), String> {
        let mut error = BsonError::new();
        if unsafe {
            (self.api.session_abort)(self.session.as_ptr(), (&mut error as *mut BsonError).cast())
        } != 0
        {
            self.transaction_active = false;
            Ok(())
        } else {
            Err(format!(
                "MongoDB transaction rollback failed: {}",
                error.message()
            ))
        }
    }
}

impl DatabaseDriver for MongoDriver {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::MongoDb
    }
    fn ping(&self) -> Result<(), String> {
        let client = lease_client(Arc::clone(&self.pool))?;
        let uri_text = CString::new("admin").unwrap();
        let command = bson(
            &self.api,
            &DbValue::Document(vec![("ping".into(), DbValue::Integer(1))]),
        )?;
        let mut error = BsonError::new();
        let ok = unsafe {
            (self.api.command_simple)(
                client.raw.as_ptr(),
                uri_text.as_ptr(),
                command.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                (&mut error as *mut BsonError).cast(),
            )
        };
        if ok != 0 {
            Ok(())
        } else {
            Err(format!("MongoDB ping command failed: {}", error.message()))
        }
    }
    fn execute(&self, command: &DbCommand) -> Result<DbResult, String> {
        match command {
            DbCommand::Mongo {
                database,
                collection,
                action,
            } => self.execute_mongo(database, collection, action),
            DbCommand::Sql { .. } | DbCommand::PreparedSql { .. } => {
                Err("SQL command cannot be sent to MongoDB".into())
            }
        }
    }
    fn start_session(&self) -> Result<Box<dyn DatabaseSession>, String> {
        let client = lease_client(Arc::clone(&self.pool))?;
        let mut error = BsonError::new();
        let session = NonNull::new(unsafe {
            (self.api.client_start_session)(
                client.raw.as_ptr(),
                std::ptr::null(),
                (&mut error as *mut BsonError).cast(),
            )
        })
        .ok_or_else(|| {
            format!(
                "MongoDB could not start a client session: {}",
                error.message()
            )
        })?;
        Ok(Box::new(MongoSession {
            api: Arc::clone(&self.api),
            client: Some(client),
            session,
            transaction_active: false,
            max_result_rows: self.max_result_rows,
        }))
    }
}

fn execute_action(
    api: &Api,
    client: *mut c_void,
    database: *const c_char,
    collection_name: *const c_char,
    action: &crate::MongoAction,
    session: Option<*mut c_void>,
    max_result_rows: usize,
) -> Result<DbResult, String> {
    let collection = CollectionOwner {
        raw: NonNull::new(unsafe { (api.get_collection)(client, database, collection_name) })
            .ok_or("MongoDB could not open collection")?,
        destroy: api.collection_destroy,
    };
    let result = match action {
        crate::MongoAction::Find {
            filter,
            limit,
            options,
        } => {
            let filter = bson(api, filter)?;
            let effective_limit = limit.map_or(max_result_rows as u64, |limit| {
                limit.min(max_result_rows as u64)
            });
            let DbValue::Document(mut option_values) = options.clone() else {
                return Err("MongoDB find options must be a document".into());
            };
            if let Some((_, value)) = option_values.iter_mut().find(|(key, _)| key == "limit") {
                *value = DbValue::Integer(effective_limit as i64);
            } else {
                option_values.push(("limit".into(), DbValue::Integer(effective_limit as i64)));
            }
            let options = bson(api, &DbValue::Document(option_values))?;
            append_session(api, session, options.as_ptr())?;
            let cursor = CursorOwner {
                raw: NonNull::new(unsafe {
                    (api.find)(
                        collection.as_ptr(),
                        filter.as_ptr(),
                        options.as_ptr(),
                        std::ptr::null(),
                    )
                })
                .ok_or("MongoDB find command failed")?,
                destroy: api.cursor_destroy,
            };
            let mut rows = Vec::new();
            loop {
                let mut document = std::ptr::null();
                if unsafe { (api.cursor_next)(cursor.as_ptr(), &mut document) } == 0 {
                    break;
                }
                if rows.len() >= max_result_rows {
                    return Err(format!(
                        "MongoDB result exceeds configured max_result_rows ({max_result_rows})"
                    ));
                }
                let json = bson_json(api, document)?;
                rows.push(vec![json_to_value(&json)?]);
            }
            let mut error = BsonError::new();
            let failed = unsafe {
                (api.cursor_error)(cursor.as_ptr(), (&mut error as *mut BsonError).cast())
            } != 0;
            if failed {
                Err(format!(
                    "MongoDB cursor returned an error: {}",
                    error.message()
                ))
            } else {
                Ok(DbResult {
                    affected_rows: rows.len() as u64,
                    rows,
                    columns: vec!["document".into()],
                    last_insert_id: None,
                    ..DbResult::default()
                })
            }
        }
        crate::MongoAction::CountDocuments {
            filter,
            estimated,
            options,
        } => {
            let filter = bson(api, filter)?;
            let options = bson(api, options)?;
            append_session(api, session, options.as_ptr())?;
            let mut error = BsonError::new();
            let count = unsafe {
                if *estimated {
                    (api.estimated_document_count)(
                        collection.as_ptr(),
                        options.as_ptr(),
                        std::ptr::null(),
                        std::ptr::null_mut(),
                        (&mut error as *mut BsonError).cast(),
                    )
                } else {
                    (api.count_documents)(
                        collection.as_ptr(),
                        filter.as_ptr(),
                        options.as_ptr(),
                        std::ptr::null(),
                        std::ptr::null_mut(),
                        (&mut error as *mut BsonError).cast(),
                    )
                }
            };
            if count < 0 {
                Err(format!(
                    "MongoDB countDocuments failed: {}",
                    error.message()
                ))
            } else {
                Ok(DbResult {
                    affected_rows: count as u64,
                    ..DbResult::default()
                })
            }
        }
        crate::MongoAction::Aggregate { pipeline, options } => {
            let pipeline =
                DbValue::Document(vec![("pipeline".into(), DbValue::Array(pipeline.clone()))]);
            let pipeline = bson(api, &pipeline)?;
            let options = bson(api, options)?;
            append_session(api, session, options.as_ptr())?;
            let cursor = CursorOwner {
                raw: NonNull::new(unsafe {
                    (api.aggregate)(
                        collection.as_ptr(),
                        0,
                        pipeline.as_ptr(),
                        options.as_ptr(),
                        std::ptr::null(),
                    )
                })
                .ok_or("MongoDB aggregate command failed")?,
                destroy: api.cursor_destroy,
            };
            let rows_result = collect_cursor(api, cursor.as_ptr(), max_result_rows);
            rows_result.map(|rows| DbResult {
                affected_rows: rows.len() as u64,
                rows,
                columns: vec!["document".into()],
                last_insert_id: None,
                ..DbResult::default()
            })
        }
        crate::MongoAction::InsertOne { document, options } => {
            let document = bson(api, document)?;
            let options = bson(api, options)?;
            append_session(api, session, options.as_ptr())?;
            let mut error = BsonError::new();
            let ok = unsafe {
                (api.insert_one)(
                    collection.as_ptr(),
                    document.as_ptr(),
                    options.as_ptr(),
                    std::ptr::null_mut(),
                    (&mut error as *mut BsonError).cast(),
                )
            };
            if ok != 0 {
                Ok(DbResult {
                    affected_rows: 1,
                    ..DbResult::default()
                })
            } else {
                Err(format!("MongoDB insertOne failed: {}", error.message()))
            }
        }
        crate::MongoAction::InsertMany { documents, options } => {
            if documents.is_empty() {
                return Err("MongoDB insertMany requires at least one document".into());
            }
            let documents = documents
                .iter()
                .map(|document| bson(api, document))
                .collect::<Result<Vec<_>, _>>()?;
            let pointers = documents
                .iter()
                .map(BsonOwner::as_ptr)
                .map(|pointer| pointer.cast_const())
                .collect::<Vec<_>>();
            let options = bson(api, options)?;
            append_session(api, session, options.as_ptr())?;
            let reply = empty_bson(api)?;
            let mut error = BsonError::new();
            let ok = unsafe {
                (api.insert_many)(
                    collection.as_ptr(),
                    pointers.as_ptr(),
                    pointers.len(),
                    options.as_ptr(),
                    reply.as_ptr(),
                    (&mut error as *mut BsonError).cast(),
                )
            };
            if ok != 0 {
                Ok(DbResult {
                    affected_rows: reply_count(api, reply.as_ptr(), &["insertedCount", "n"]),
                    ..DbResult::default()
                })
            } else {
                Err(format!("MongoDB insertMany failed: {}", error.message()))
            }
        }
        crate::MongoAction::UpdateOne {
            filter,
            update,
            options,
        }
        | crate::MongoAction::UpdateMany {
            filter,
            update,
            options,
        } => {
            let update_function = if matches!(action, crate::MongoAction::UpdateOne { .. }) {
                api.update_one
            } else {
                api.update_many
            };
            let filter = bson(api, filter)?;
            let update = bson(api, update)?;
            let options = bson(api, options)?;
            append_session(api, session, options.as_ptr())?;
            let reply = empty_bson(api)?;
            let mut error = BsonError::new();
            let ok = unsafe {
                update_function(
                    collection.as_ptr(),
                    filter.as_ptr(),
                    update.as_ptr(),
                    options.as_ptr(),
                    reply.as_ptr(),
                    (&mut error as *mut BsonError).cast(),
                )
            };
            if ok != 0 {
                let matched = reply_count(api, reply.as_ptr(), &["matchedCount", "n"]);
                let modified = reply_count(api, reply.as_ptr(), &["modifiedCount", "n"]);
                Ok(DbResult {
                    affected_rows: modified,
                    matched_rows: Some(matched),
                    modified_rows: Some(modified),
                    ..DbResult::default()
                })
            } else {
                Err(format!("MongoDB updateMany failed: {}", error.message()))
            }
        }
        crate::MongoAction::ReplaceOne {
            filter,
            replacement,
            options,
        } => {
            let filter = bson(api, filter)?;
            let replacement = bson(api, replacement)?;
            let options = bson(api, options)?;
            append_session(api, session, options.as_ptr())?;
            let reply = empty_bson(api)?;
            let mut error = BsonError::new();
            let ok = unsafe {
                (api.replace_one)(
                    collection.as_ptr(),
                    filter.as_ptr(),
                    replacement.as_ptr(),
                    options.as_ptr(),
                    reply.as_ptr(),
                    (&mut error as *mut BsonError).cast(),
                )
            };
            if ok != 0 {
                let matched = reply_count(api, reply.as_ptr(), &["matchedCount", "n"]);
                let modified = reply_count(api, reply.as_ptr(), &["modifiedCount", "n"]);
                Ok(DbResult {
                    affected_rows: modified,
                    matched_rows: Some(matched),
                    modified_rows: Some(modified),
                    ..DbResult::default()
                })
            } else {
                Err(format!("MongoDB replaceOne failed: {}", error.message()))
            }
        }
        crate::MongoAction::DeleteOne { filter, options }
        | crate::MongoAction::DeleteMany { filter, options } => {
            let delete_function = if matches!(action, crate::MongoAction::DeleteOne { .. }) {
                api.delete_one
            } else {
                api.delete_many
            };
            let filter = bson(api, filter)?;
            let options = bson(api, options)?;
            append_session(api, session, options.as_ptr())?;
            let reply = empty_bson(api)?;
            let mut error = BsonError::new();
            let ok = unsafe {
                delete_function(
                    collection.as_ptr(),
                    filter.as_ptr(),
                    options.as_ptr(),
                    reply.as_ptr(),
                    (&mut error as *mut BsonError).cast(),
                )
            };
            if ok != 0 {
                Ok(DbResult {
                    affected_rows: reply_count(api, reply.as_ptr(), &["deletedCount", "n"]),
                    ..DbResult::default()
                })
            } else {
                Err(format!("MongoDB deleteMany failed: {}", error.message()))
            }
        }
    };
    result
}

fn collect_cursor(
    api: &Api,
    cursor: *mut c_void,
    max_result_rows: usize,
) -> Result<Vec<Vec<DbValue>>, String> {
    let mut rows = Vec::new();
    loop {
        let mut document = std::ptr::null();
        if unsafe { (api.cursor_next)(cursor, &mut document) } == 0 {
            break;
        }
        if rows.len() >= max_result_rows {
            return Err(format!(
                "MongoDB result exceeds configured max_result_rows ({max_result_rows})"
            ));
        }
        let json = bson_json(api, document)?;
        rows.push(vec![json_to_value(&json)?]);
    }
    let mut error = BsonError::new();
    if unsafe { (api.cursor_error)(cursor, (&mut error as *mut BsonError).cast()) } != 0 {
        Err(format!(
            "MongoDB cursor returned an error: {}",
            error.message()
        ))
    } else {
        Ok(rows)
    }
}

fn set_uri_int(api: &Api, uri: *mut c_void, name: &str, value: usize) -> Result<(), String> {
    let name = CString::new(name).unwrap();
    if unsafe { (api.uri_set_int)(uri, name.as_ptr(), value.min(i32::MAX as usize) as i32) } == 0 {
        Err("invalid MongoDB connection pool option".into())
    } else {
        Ok(())
    }
}

fn append_session(
    api: &Api,
    session: Option<*mut c_void>,
    options: *mut c_void,
) -> Result<(), String> {
    if let Some(session) = session {
        let mut error = BsonError::new();
        if unsafe { (api.session_append)(session, options, (&mut error as *mut BsonError).cast()) }
            == 0
        {
            return Err(format!(
                "MongoDB could not attach the transaction session to the operation: {}",
                error.message()
            ));
        }
    }
    Ok(())
}

fn bson(api: &Api, value: &DbValue) -> Result<BsonOwner, String> {
    let json = value_json(value);
    let raw = NonNull::new(unsafe {
        (api.bson_from_json)(json.as_ptr(), json.len() as isize, std::ptr::null_mut())
    })
    .ok_or("invalid BSON document")?;
    Ok(BsonOwner {
        raw,
        destroy: api.bson_destroy,
    })
}

fn empty_bson(api: &Api) -> Result<BsonOwner, String> {
    let raw = NonNull::new(unsafe { (api.bson_new)() })
        .ok_or("cannot allocate MongoDB reply document")?;
    Ok(BsonOwner {
        raw,
        destroy: api.bson_destroy,
    })
}

fn lease_client(pool: Arc<MongoClientPool>) -> Result<MongoClientLease, String> {
    let raw = NonNull::new(unsafe { (pool.api.pool_pop)(pool.raw.as_ptr()) })
        .ok_or("MongoDB client pool is closed")?;
    Ok(MongoClientLease { pool, raw })
}

fn bson_json(api: &Api, bson: *const c_void) -> Result<String, String> {
    let mut length = 0usize;
    let value = NonNull::new(unsafe { (api.bson_as_json)(bson, &mut length) }.cast::<c_void>())
        .ok_or("cannot serialize MongoDB document")?;
    let json = unsafe { CStr::from_ptr(value.as_ptr().cast()) }
        .to_string_lossy()
        .into_owned();
    unsafe {
        (api.bson_free)(value.as_ptr());
    }
    Ok(json)
}

fn reply_count(api: &Api, reply: *const c_void, fields: &[&str]) -> u64 {
    bson_json(api, reply)
        .ok()
        .and_then(|json| json_to_value(&json).ok())
        .and_then(|value| match value {
            DbValue::Document(document) => fields.iter().find_map(|field| {
                document
                    .iter()
                    .find(|(key, _)| key == field)
                    .and_then(|(_, value)| match value {
                        DbValue::Integer(value) => u64::try_from(*value).ok(),
                        DbValue::Float(value) if *value >= 0.0 => Some(*value as u64),
                        DbValue::Document(number) => number.iter().find_map(|(kind, value)| {
                            if matches!(kind.as_str(), "$numberInt" | "$numberLong") {
                                match value {
                                    DbValue::String(value) => value.parse().ok(),
                                    DbValue::Integer(value) => u64::try_from(*value).ok(),
                                    _ => None,
                                }
                            } else {
                                None
                            }
                        }),
                        _ => None,
                    })
            }),
            _ => None,
        })
        .unwrap_or(0)
}

fn value_json(value: &DbValue) -> String {
    match value {
        DbValue::Null => "null".into(),
        DbValue::Bool(value) => value.to_string(),
        DbValue::Integer(value) => value.to_string(),
        DbValue::Float(value) if value.is_finite() => value.to_string(),
        DbValue::Float(_) => "null".into(),
        DbValue::String(value) => json_string(value),
        DbValue::Bytes(value) => json_string(
            &value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        ),
        DbValue::Array(values) => format!(
            "[{}]",
            values.iter().map(value_json).collect::<Vec<_>>().join(",")
        ),
        DbValue::Document(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!("{}:{}", json_string(key), value_json(value)))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_to_value(source: &str) -> Result<DbValue, String> {
    let mut parser = JsonParser { source, cursor: 0 };
    let value = parser.value()?;
    parser.space();
    if parser.cursor != source.len() {
        return Err("invalid JSON in MongoDB result".into());
    }
    Ok(value)
}

struct JsonParser<'a> {
    source: &'a str,
    cursor: usize,
}
impl JsonParser<'_> {
    fn space(&mut self) {
        while self
            .source
            .as_bytes()
            .get(self.cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.cursor += 1;
        }
    }
    fn value(&mut self) -> Result<DbValue, String> {
        self.space();
        let bytes = self.source.as_bytes();
        match bytes.get(self.cursor).copied() {
            Some(b'n') if self.take_word("null") => Ok(DbValue::Null),
            Some(b't') if self.take_word("true") => Ok(DbValue::Bool(true)),
            Some(b'f') if self.take_word("false") => Ok(DbValue::Bool(false)),
            Some(b'"') => self.string().map(DbValue::String),
            Some(b'[') => {
                self.cursor += 1;
                let mut values = Vec::new();
                self.space();
                if self.take_byte(b']') {
                    return Ok(DbValue::Array(values));
                }
                loop {
                    values.push(self.value()?);
                    self.space();
                    if self.take_byte(b']') {
                        break;
                    }
                    self.expect_byte(b',')?;
                }
                Ok(DbValue::Array(values))
            }
            Some(b'{') => {
                self.cursor += 1;
                let mut values = Vec::new();
                self.space();
                if self.take_byte(b'}') {
                    return Ok(DbValue::Document(values));
                }
                loop {
                    self.space();
                    let key = self.string()?;
                    self.space();
                    self.expect_byte(b':')?;
                    values.push((key, self.value()?));
                    self.space();
                    if self.take_byte(b'}') {
                        break;
                    }
                    self.expect_byte(b',')?;
                }
                Ok(DbValue::Document(values))
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err("invalid JSON in MongoDB result".into()),
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.expect_byte(b'"')?;
        let mut output = String::new();
        while let Some(byte) = self.source.as_bytes().get(self.cursor).copied() {
            if byte == b'"' {
                self.cursor += 1;
                return Ok(output);
            }
            if byte == b'\\' {
                self.cursor += 1;
                let escaped = *self
                    .source
                    .as_bytes()
                    .get(self.cursor)
                    .ok_or("invalid JSON escape")?;
                self.cursor += 1;
                match escaped {
                    b'"' => output.push('"'),
                    b'\\' => output.push('\\'),
                    b'/' => output.push('/'),
                    b'b' => output.push('\u{0008}'),
                    b'f' => output.push('\u{000c}'),
                    b'n' => output.push('\n'),
                    b'r' => output.push('\r'),
                    b't' => output.push('\t'),
                    b'u' => output.push(self.unicode_escape()?),
                    _ => return Err("invalid JSON escape".into()),
                }
            } else {
                let ch = self.source[self.cursor..]
                    .chars()
                    .next()
                    .ok_or("invalid JSON string")?;
                output.push(ch);
                self.cursor += ch.len_utf8();
            }
        }
        Err("unterminated JSON string".into())
    }
    fn unicode_escape(&mut self) -> Result<char, String> {
        let end = self.cursor.checked_add(4).ok_or("invalid Unicode escape")?;
        let code = u16::from_str_radix(
            self.source
                .get(self.cursor..end)
                .ok_or("invalid Unicode escape")?,
            16,
        )
        .map_err(|_| "invalid Unicode escape")?;
        self.cursor = end;
        if (0xD800..=0xDBFF).contains(&code) {
            if self.source.as_bytes().get(self.cursor..self.cursor + 2) != Some(b"\\u") {
                return Err("invalid Unicode surrogate pair".into());
            }
            self.cursor += 2;
            let low_end = self.cursor + 4;
            let low = u16::from_str_radix(
                self.source
                    .get(self.cursor..low_end)
                    .ok_or("invalid Unicode escape")?,
                16,
            )
            .map_err(|_| "invalid Unicode escape")?;
            self.cursor = low_end;
            if !(0xDC00..=0xDFFF).contains(&low) {
                return Err("invalid Unicode surrogate pair".into());
            }
            let scalar = 0x10000 + (((code as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
            char::from_u32(scalar).ok_or("invalid Unicode scalar".into())
        } else {
            char::from_u32(code as u32).ok_or("invalid Unicode scalar".into())
        }
    }
    fn number(&mut self) -> Result<DbValue, String> {
        let start = self.cursor;
        let bytes = self.source.as_bytes();
        if bytes.get(self.cursor) == Some(&b'-') {
            self.cursor += 1;
        }
        while bytes.get(self.cursor).is_some_and(u8::is_ascii_digit) {
            self.cursor += 1;
        }
        if bytes.get(self.cursor) == Some(&b'.') {
            self.cursor += 1;
            while bytes.get(self.cursor).is_some_and(u8::is_ascii_digit) {
                self.cursor += 1;
            }
        }
        if bytes
            .get(self.cursor)
            .is_some_and(|byte| matches!(byte, b'e' | b'E'))
        {
            self.cursor += 1;
            if bytes
                .get(self.cursor)
                .is_some_and(|byte| matches!(byte, b'+' | b'-'))
            {
                self.cursor += 1;
            }
            while bytes.get(self.cursor).is_some_and(u8::is_ascii_digit) {
                self.cursor += 1;
            }
        }
        let number = &self.source[start..self.cursor];
        if let Ok(integer) = number.parse::<i64>() {
            Ok(DbValue::Integer(integer))
        } else {
            number
                .parse::<f64>()
                .map(DbValue::Float)
                .map_err(|_| "invalid JSON number".into())
        }
    }
    fn take_word(&mut self, word: &str) -> bool {
        if self.source[self.cursor..].starts_with(word) {
            self.cursor += word.len();
            true
        } else {
            false
        }
    }
    fn take_byte(&mut self, byte: u8) -> bool {
        if self.source.as_bytes().get(self.cursor) == Some(&byte) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }
    fn expect_byte(&mut self, byte: u8) -> Result<(), String> {
        if self.take_byte(byte) {
            Ok(())
        } else {
            Err("invalid JSON structure".into())
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/crates/php-db/db/mongo.rs"]
mod tests;
