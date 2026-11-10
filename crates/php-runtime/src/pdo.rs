//! Small PDO-compatible surface backed by process-wide database drivers.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use php_db::{DatabaseKind, DatabaseSession, DbCommand, DbErrorInfo, DbResult, DbValue};

use crate::{database_registry, ArrayKey, PhpObject, Value};

static NEXT_PDO_ID: AtomicU64 = AtomicU64::new(1);
thread_local! { static PDO_SESSIONS: RefCell<HashMap<u64, Box<dyn DatabaseSession>>> = RefCell::new(HashMap::new()); }
thread_local! { static LAST_INSERT_IDS: RefCell<HashMap<u64, String>> = RefCell::new(HashMap::new()); }
thread_local! { static LAST_ERRORS: RefCell<HashMap<u64, String>> = RefCell::new(HashMap::new()); }
thread_local! { static ERROR_MODES: RefCell<HashMap<u64, i64>> = RefCell::new(HashMap::new()); }
thread_local! { static PDO_TIMEOUTS: RefCell<HashMap<u64, u64>> = RefCell::new(HashMap::new()); }
thread_local! { static PDO_EMULATE_PREPARES: RefCell<HashMap<u64, bool>> = RefCell::new(HashMap::new()); }
thread_local! { static PDO_AUTOCOMMIT: RefCell<HashMap<u64, bool>> = RefCell::new(HashMap::new()); }
thread_local! { static PDO_DEFAULT_STR_PARAMS: RefCell<HashMap<u64, i64>> = RefCell::new(HashMap::new()); }
thread_local! { static ACTIVE_TRANSACTIONS: RefCell<HashSet<u64>> = RefCell::new(HashSet::new()); }

fn cleanup_sessions() {
    let active =
        ACTIVE_TRANSACTIONS.with(|transactions| std::mem::take(&mut *transactions.borrow_mut()));
    PDO_SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        for id in active {
            if let Some(session) = sessions.get_mut(&id) {
                let _ = session.execute(&DbCommand::Sql {
                    statement: "ROLLBACK".into(),
                    parameters: Vec::new(),
                });
            }
        }
        sessions.clear();
    });
}

pub(crate) struct ExecutionGuard;
impl ExecutionGuard {
    pub(crate) fn enter() -> Self {
        cleanup_sessions();
        LAST_INSERT_IDS.with(|ids| ids.borrow_mut().clear());
        LAST_ERRORS.with(|errors| errors.borrow_mut().clear());
        ERROR_MODES.with(|modes| modes.borrow_mut().clear());
        PDO_TIMEOUTS.with(|timeouts| timeouts.borrow_mut().clear());
        PDO_EMULATE_PREPARES.with(|modes| modes.borrow_mut().clear());
        PDO_AUTOCOMMIT.with(|modes| modes.borrow_mut().clear());
        PDO_DEFAULT_STR_PARAMS.with(|modes| modes.borrow_mut().clear());
        ACTIVE_TRANSACTIONS.with(|transactions| transactions.borrow_mut().clear());
        crate::mongo_php::clear_sessions();
        Self
    }
}
impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        cleanup_sessions();
        LAST_INSERT_IDS.with(|ids| ids.borrow_mut().clear());
        LAST_ERRORS.with(|errors| errors.borrow_mut().clear());
        ERROR_MODES.with(|modes| modes.borrow_mut().clear());
        PDO_TIMEOUTS.with(|timeouts| timeouts.borrow_mut().clear());
        PDO_EMULATE_PREPARES.with(|modes| modes.borrow_mut().clear());
        PDO_AUTOCOMMIT.with(|modes| modes.borrow_mut().clear());
        PDO_DEFAULT_STR_PARAMS.with(|modes| modes.borrow_mut().clear());
        ACTIVE_TRANSACTIONS.with(|transactions| transactions.borrow_mut().clear());
        crate::mongo_php::clear_sessions();
    }
}

pub(crate) fn construct(arguments: &[Value]) -> Result<Value, String> {
    if arguments.is_empty() || arguments.len() > 4 {
        return Err(
            "PDO::__construct() expects a DSN, optional username, password, and options".into(),
        );
    }
    let dsn = arguments[0].show();
    let username = arguments
        .get(1)
        .filter(|value| !matches!(value.dereference(), Value::Null))
        .map(Value::show);
    let password = arguments
        .get(2)
        .filter(|value| !matches!(value.dereference(), Value::Null))
        .map(Value::show);
    let (name, driver) =
        database_registry().resolve_dsn(&dsn, username.as_deref(), password.as_deref())?;
    if driver.kind() == DatabaseKind::MongoDb {
        return Err("MongoDB uses the mongo_* document functions, not PDO SQL methods".into());
    }
    let mut properties = HashMap::new();
    properties.insert("\0database_name".into(), Value::String(name));
    let id = NEXT_PDO_ID.fetch_add(1, Ordering::Relaxed);
    properties.insert("\0pdo_id".into(), Value::Integer(id as i64));
    properties.insert("\0transaction_active".into(), Value::Bool(false));
    properties.insert("\0pdo_attr_0".into(), Value::Bool(true)); // AUTOCOMMIT
    properties.insert("\0pdo_attr_12".into(), Value::Bool(false)); // PERSISTENT
    properties.insert("\0pdo_attr_3".into(), Value::Number(2.0)); // ERRMODE_EXCEPTION
    properties.insert("\0pdo_attr_2".into(), Value::Integer(0)); // use configured pool timeout
    properties.insert("\0pdo_attr_8".into(), Value::Number(0.0)); // CASE_NATURAL
    properties.insert("\0pdo_attr_11".into(), Value::Number(0.0)); // NULL_NATURAL
    properties.insert("\0pdo_attr_17".into(), Value::Bool(false)); // STRINGIFY_FETCHES
    properties.insert("\0pdo_attr_19".into(), Value::Number(4.0)); // FETCH_BOTH
    let default_emulation = matches!(driver.kind(), DatabaseKind::Mysql | DatabaseKind::MariaDb);
    properties.insert("\0pdo_attr_20".into(), Value::Bool(default_emulation)); // EMULATE_PREPARES
    properties.insert("\0pdo_attr_21".into(), Value::Integer(1 << 29)); // DEFAULT_STR_PARAM = PARAM_STR_CHAR
    properties.insert("\0pdo_attr_1000".into(), Value::Integer(0)); // MAX_COLUMN_LEN; zero means unlimited
    properties.insert("\0pdo_attr_14".into(), Value::Bool(false)); // FETCH_TABLE_NAMES
    properties.insert("\0pdo_attr_15".into(), Value::Bool(false)); // FETCH_CATALOG_NAMES
    if arguments
        .get(3)
        .is_some_and(|value| !matches!(value.dereference(), Value::Array(_) | Value::Null))
    {
        return Err("PDO::__construct(): argument #4 ($options) must be of type ?array".into());
    }
    if let Some(Value::Array(options)) = arguments.get(3).map(Value::dereference) {
        for (attribute, value) in options.iter() {
            let key = match attribute {
                ArrayKey::Integer(key) => *key,
                _ => continue,
            };
            if key == 2
                && (!value.number().is_finite()
                    || value.number() < 0.0
                    || value.number().fract() != 0.0)
            {
                return Err("PDO::__construct(): invalid PDO::ATTR_TIMEOUT option".into());
            }
            if key == 3 && !matches!(value.number() as i64, 0..=2) {
                return Err("PDO::__construct(): invalid PDO::ATTR_ERRMODE option".into());
            }
            if key == 19 && !valid_fetch_mode(value.number() as i64) {
                return Err(
                    "PDO::__construct(): invalid PDO::ATTR_DEFAULT_FETCH_MODE option".into(),
                );
            }
            if key == 8 && !matches!(value.number() as i64, 0..=2) {
                return Err("PDO::__construct(): invalid PDO::ATTR_CASE option".into());
            }
            if key == 11 && !matches!(value.number() as i64, 0..=2) {
                return Err("PDO::__construct(): invalid PDO::ATTR_ORACLE_NULLS option".into());
            }
            if key == 21
                && !((value.number() as i64) == (1 << 30) || (value.number() as i64) == (1 << 29))
            {
                return Err(
                    "PDO::__construct(): invalid PDO::ATTR_DEFAULT_STR_PARAM option".into(),
                );
            }
            if key == 1000
                && (!value.number().is_finite()
                    || value.number() < 0.0
                    || value.number().fract() != 0.0)
            {
                return Err("PDO::__construct(): invalid PDO::ATTR_MAX_COLUMN_LEN option".into());
            }
            if matches!(key, 14 | 15) {
                properties.insert(format!("\0pdo_attr_{key}"), Value::Bool(value.truthy()));
            }
            if matches!(key, 3 | 8 | 11 | 17 | 19 | 2 | 21 | 1000) {
                properties.insert(format!("\0pdo_attr_{key}"), value.clone());
            }
            if key == 0 {
                properties.insert("\0pdo_attr_0".into(), Value::Bool(value.truthy()));
            }
            if key == 12 {
                properties.insert("\0pdo_attr_12".into(), Value::Bool(value.truthy()));
            }
            if key == 20 {
                properties.insert("\0pdo_attr_20".into(), Value::Bool(value.truthy()));
            }
        }
    }
    let error_mode = properties
        .get("\0pdo_attr_3")
        .map(Value::number)
        .unwrap_or(2.0) as i64;
    ERROR_MODES.with(|modes| {
        modes.borrow_mut().insert(id, error_mode);
    });
    let timeout = properties
        .get("\0pdo_attr_2")
        .map(Value::number)
        .unwrap_or(0.0)
        .max(0.0) as u64;
    PDO_TIMEOUTS.with(|timeouts| {
        timeouts.borrow_mut().insert(id, timeout);
    });
    PDO_EMULATE_PREPARES.with(|modes| {
        modes.borrow_mut().insert(
            id,
            properties.get("\0pdo_attr_20").is_some_and(Value::truthy),
        );
    });
    PDO_AUTOCOMMIT.with(|modes| {
        modes.borrow_mut().insert(
            id,
            properties.get("\0pdo_attr_0").is_some_and(Value::truthy),
        );
    });
    PDO_DEFAULT_STR_PARAMS.with(|modes| {
        modes.borrow_mut().insert(
            id,
            properties
                .get("\0pdo_attr_21")
                .map(Value::number)
                .unwrap_or((1 << 29) as f64) as i64,
        );
    });
    let acquire_timeout = std::time::Duration::from_secs(timeout);
    let session = driver
        .start_session_with_timeout(acquire_timeout)
        .map_err(|error| {
            ERROR_MODES.with(|modes| {
                modes.borrow_mut().remove(&id);
            });
            PDO_TIMEOUTS.with(|timeouts| {
                timeouts.borrow_mut().remove(&id);
            });
            PDO_EMULATE_PREPARES.with(|modes| {
                modes.borrow_mut().remove(&id);
            });
            PDO_AUTOCOMMIT.with(|modes| {
                modes.borrow_mut().remove(&id);
            });
            PDO_DEFAULT_STR_PARAMS.with(|modes| {
                modes.borrow_mut().remove(&id);
            });
            error
        })?;
    PDO_SESSIONS.with(|sessions| {
        sessions.borrow_mut().insert(id, session);
    });
    Ok(Value::Object(Rc::new(RefCell::new(PhpObject {
        class_name: "PDO".into(),
        properties,
        property_order: Vec::new(),
        active_property_hooks: Default::default(),
        exception: None,
    }))))
}

pub(crate) fn call_method(
    object: &Rc<RefCell<PhpObject>>,
    method: &str,
    arguments: &[Value],
    fetch_class: &mut dyn FnMut(&Value, &str, &[Value]) -> Result<Value, String>,
) -> Result<Value, String> {
    let class = object.borrow().class_name.to_ascii_lowercase();
    match (class.as_str(), method.to_ascii_lowercase().as_str()) {
        ("pdo", "query") => {
            let sql = required_string(arguments, 0, "PDO::query() expects a SQL statement")?;
            if let Some(mode) = arguments
                .get(1)
                .filter(|value| !matches!(value.dereference(), Value::Null))
            {
                validate_fetch_setup(mode, arguments.get(2..).unwrap_or_default(), "PDO::query")?;
            }
            let name = database_name(object)?;
            let id = pdo_id(object)?;
            let result = match execute(&name, &sql, Vec::new(), Some(id)) {
                Ok(result) => result,
                Err(error) => return report_error(object, error),
            };
            let case_mode = object
                .borrow()
                .properties
                .get("\0pdo_attr_8")
                .map(Value::number)
                .unwrap_or(0.0) as i64;
            let oracle_nulls = object
                .borrow()
                .properties
                .get("\0pdo_attr_11")
                .map(Value::number)
                .unwrap_or(0.0) as i64;
            let stringify = object
                .borrow()
                .properties
                .get("\0pdo_attr_17")
                .is_some_and(Value::truthy);
            let max_column_len = object
                .borrow()
                .properties
                .get("\0pdo_attr_1000")
                .map(Value::number)
                .unwrap_or(0.0)
                .max(0.0) as usize;
            let table_names = object
                .borrow()
                .properties
                .get("\0pdo_attr_14")
                .is_some_and(Value::truthy);
            let catalog_names = object
                .borrow()
                .properties
                .get("\0pdo_attr_15")
                .is_some_and(Value::truthy);
            let statement = statement(
                name,
                id,
                Some(sql),
                result,
                case_mode,
                oracle_nulls,
                stringify,
                max_column_len,
                table_names,
                catalog_names,
            );
            let mode = arguments
                .get(1)
                .filter(|value| !matches!(value.dereference(), Value::Null))
                .cloned()
                .or_else(|| object.borrow().properties.get("\0pdo_attr_19").cloned());
            if let Some(mode) = mode {
                set_statement_fetch_mode(&statement, mode, arguments.get(2..).unwrap_or_default());
            }
            Ok(statement)
        }
        ("pdo", "exec") => {
            let sql = required_string(arguments, 0, "PDO::exec() expects a SQL statement")?;
            let result = match execute(
                &database_name(object)?,
                &sql,
                Vec::new(),
                Some(pdo_id(object)?),
            ) {
                Ok(result) => result,
                Err(error) => return report_error(object, error),
            };
            Ok(Value::Integer(result.affected_rows as i64))
        }
        ("pdo", "prepare") => {
            let sql = required_string(arguments, 0, "PDO::prepare() expects a SQL statement")?;
            if arguments
                .get(1)
                .is_some_and(|value| !matches!(value.dereference(), Value::Array(_)))
            {
                return Err("PDO::prepare(): argument #2 ($options) must be of type array".into());
            }
            let case_mode = object
                .borrow()
                .properties
                .get("\0pdo_attr_8")
                .map(Value::number)
                .unwrap_or(0.0) as i64;
            let oracle_nulls = object
                .borrow()
                .properties
                .get("\0pdo_attr_11")
                .map(Value::number)
                .unwrap_or(0.0) as i64;
            let stringify = object
                .borrow()
                .properties
                .get("\0pdo_attr_17")
                .is_some_and(Value::truthy);
            let max_column_len = object
                .borrow()
                .properties
                .get("\0pdo_attr_1000")
                .map(Value::number)
                .unwrap_or(0.0)
                .max(0.0) as usize;
            let table_names = object
                .borrow()
                .properties
                .get("\0pdo_attr_14")
                .is_some_and(Value::truthy);
            let catalog_names = object
                .borrow()
                .properties
                .get("\0pdo_attr_15")
                .is_some_and(Value::truthy);
            let statement = statement(
                database_name(object)?,
                pdo_id(object)?,
                Some(sql),
                DbResult::default(),
                case_mode,
                oracle_nulls,
                stringify,
                max_column_len,
                table_names,
                catalog_names,
            );
            if let Some(Value::Array(options)) = arguments.get(1).map(Value::dereference) {
                for (attribute, value) in options.iter() {
                    if !matches!(attribute, ArrayKey::Integer(10)) || value.number() as i64 != 0 {
                        return Ok(Value::Bool(false));
                    }
                    if let Value::Object(statement) = &statement {
                        statement
                            .borrow_mut()
                            .properties
                            .insert("\0stmt_attr_10".into(), value.clone());
                    }
                }
            }
            if let Some(mode) = object.borrow().properties.get("\0pdo_attr_19").cloned() {
                set_statement_fetch_mode(&statement, mode, &[]);
            }
            Ok(statement)
        }
        ("pdo", "begintransaction") => {
            if transaction_active(object) {
                return begin_nested_transaction(object);
            }
            if let Err(error) = start_transaction(object) {
                return report_error(object, error);
            }
            let id = pdo_id(object)?;
            ACTIVE_TRANSACTIONS.with(|transactions| {
                transactions.borrow_mut().insert(id);
            });
            object
                .borrow_mut()
                .properties
                .insert("\0transaction_active".into(), Value::Bool(true));
            object
                .borrow_mut()
                .properties
                .insert("\0transaction_depth".into(), Value::Integer(1));
            Ok(Value::Bool(true))
        }
        ("pdo", "commit") => match finish_transaction(object, "COMMIT") {
            Ok(value) => Ok(value),
            Err(error) => report_error(object, error),
        },
        ("pdo", "rollback") => match finish_transaction(object, "ROLLBACK") {
            Ok(value) => Ok(value),
            Err(error) => report_error(object, error),
        },
        ("pdo", "intransaction") => Ok(Value::Bool(transaction_active(object))),
        ("pdo", "lastinsertid") => {
            let id = pdo_id(object)?;
            if let Some(sequence) = arguments
                .first()
                .filter(|value| !matches!(value.dereference(), Value::Null))
            {
                let sequence = sequence.show();
                let name = database_name(object)?;
                let driver = database_registry().get(&name)?;
                let sql = match driver.kind() {
                    DatabaseKind::PostgreSql => Some((
                        "SELECT currval(?)::text".to_owned(),
                        vec![(ArrayKey::Integer(0), DbValue::String(sequence))],
                    )),
                    DatabaseKind::Oracle if valid_sequence_name(&sequence) => {
                        Some((format!("SELECT {sequence}.CURRVAL FROM DUAL"), Vec::new()))
                    }
                    DatabaseKind::Oracle => {
                        return Err("PDO::lastInsertId(): invalid Oracle sequence name".into())
                    }
                    _ => None,
                };
                if let Some((sql, parameters)) = sql {
                    let result = match execute(&name, &sql, parameters, Some(id)) {
                        Ok(result) => result,
                        Err(error) => return report_error(object, error),
                    };
                    return Ok(result
                        .rows
                        .first()
                        .and_then(|row| row.first())
                        .map(|value| {
                            Value::String(match value {
                                DbValue::String(value) => value.clone(),
                                DbValue::Integer(value) => value.to_string(),
                                _ => return Value::Bool(false),
                            })
                        })
                        .unwrap_or(Value::Bool(false)));
                }
            }
            Ok(LAST_INSERT_IDS.with(|ids| {
                ids.borrow()
                    .get(&id)
                    .cloned()
                    .map(Value::String)
                    .unwrap_or(Value::Bool(false))
            }))
        }
        ("pdo", "quote") => {
            let value = required_string(arguments, 0, "PDO::quote() expects a string")?;
            let lob = arguments
                .get(1)
                .is_some_and(|kind| kind.number() as i64 == 3);
            let id = pdo_id(object)?;
            let quoted = PDO_SESSIONS
                .with(|sessions| {
                    sessions.borrow_mut().get_mut(&id).map(|session| {
                        if lob {
                            session.quote_binary(value.as_bytes())
                        } else {
                            session.quote_string(&value)
                        }
                    })
                })
                .transpose();
            let quoted = match quoted {
                Ok(Some(quoted)) => quoted,
                Ok(None) => {
                    let driver = match database_registry().get(&database_name(object)?) {
                        Ok(driver) => driver,
                        Err(error) => return report_error(object, error),
                    };
                    match if lob {
                        driver.quote_binary(value.as_bytes())
                    } else {
                        driver.quote_string(&value)
                    } {
                        Ok(quoted) => quoted,
                        Err(error) => return report_error(object, error),
                    }
                }
                Err(error) => return report_error(object, error),
            };
            Ok(Value::String(quoted))
        }
        ("pdo", "setattribute") => {
            let attribute = arguments
                .first()
                .map(Value::number)
                .ok_or("PDO::setAttribute() expects an attribute")?
                as i64;
            let Some(value) = arguments.get(1) else {
                return Err("PDO::setAttribute() expects a value".into());
            };
            if !matches!(
                attribute,
                0 | 2 | 3 | 8 | 11 | 12 | 14 | 15 | 17 | 19 | 20 | 21 | 1000
            ) {
                return Ok(Value::Bool(false));
            }
            if attribute == 2
                && (!value.number().is_finite()
                    || value.number() < 0.0
                    || value.number().fract() != 0.0)
            {
                return Ok(Value::Bool(false));
            }
            if attribute == 3 && !matches!(value.number() as i64, 0..=2) {
                return Ok(Value::Bool(false));
            }
            if attribute == 8 && !matches!(value.number() as i64, 0..=2) {
                return Ok(Value::Bool(false));
            }
            if attribute == 11 && !matches!(value.number() as i64, 0..=2) {
                return Ok(Value::Bool(false));
            }
            if attribute == 19 && !valid_fetch_mode(value.number() as i64) {
                return Ok(Value::Bool(false));
            }
            if attribute == 21
                && !((value.number() as i64) == (1 << 30) || (value.number() as i64) == (1 << 29))
            {
                return Ok(Value::Bool(false));
            }
            if attribute == 1000
                && (!value.number().is_finite()
                    || value.number() < 0.0
                    || value.number().fract() != 0.0)
            {
                return Ok(Value::Bool(false));
            }
            if attribute != 0 {
                object.borrow_mut().properties.insert(
                    format!("\0pdo_attr_{attribute}"),
                    if attribute == 12 {
                        Value::Bool(value.truthy())
                    } else {
                        value.clone()
                    },
                );
            }
            if attribute == 3 {
                let id = pdo_id(object)?;
                ERROR_MODES.with(|modes| {
                    modes.borrow_mut().insert(id, value.number() as i64);
                });
            }
            if attribute == 2 {
                let id = pdo_id(object)?;
                PDO_TIMEOUTS.with(|timeouts| {
                    timeouts.borrow_mut().insert(id, value.number() as u64);
                });
            }
            if attribute == 20 {
                let id = pdo_id(object)?;
                PDO_EMULATE_PREPARES.with(|modes| {
                    modes.borrow_mut().insert(id, value.truthy());
                });
                object
                    .borrow_mut()
                    .properties
                    .insert("\0pdo_attr_20".into(), Value::Bool(value.truthy()));
            }
            if attribute == 21 {
                PDO_DEFAULT_STR_PARAMS.with(|modes| {
                    modes
                        .borrow_mut()
                        .insert(pdo_id(object).unwrap_or_default(), value.number() as i64);
                });
            }
            if attribute == 0 {
                let id = pdo_id(object)?;
                let autocommit = value.truthy();
                if autocommit
                    && ACTIVE_TRANSACTIONS.with(|transactions| transactions.borrow().contains(&id))
                {
                    if let Err(error) = command_status(object, "COMMIT") {
                        return report_error(object, error);
                    }
                    ACTIVE_TRANSACTIONS.with(|transactions| {
                        transactions.borrow_mut().remove(&id);
                    });
                    object
                        .borrow_mut()
                        .properties
                        .insert("\0transaction_active".into(), Value::Bool(false));
                    object
                        .borrow_mut()
                        .properties
                        .insert("\0transaction_depth".into(), Value::Integer(0));
                } else if !autocommit
                    && PDO_SESSIONS.with(|sessions| sessions.borrow().contains_key(&id))
                    && !ACTIVE_TRANSACTIONS.with(|transactions| transactions.borrow().contains(&id))
                {
                    if let Err(error) = start_transaction(object) {
                        return report_error(object, error);
                    }
                    ACTIVE_TRANSACTIONS.with(|transactions| {
                        transactions.borrow_mut().insert(id);
                    });
                    object
                        .borrow_mut()
                        .properties
                        .insert("\0transaction_active".into(), Value::Bool(true));
                    object
                        .borrow_mut()
                        .properties
                        .insert("\0transaction_depth".into(), Value::Integer(1));
                }
                PDO_AUTOCOMMIT.with(|modes| {
                    modes.borrow_mut().insert(id, autocommit);
                });
                object
                    .borrow_mut()
                    .properties
                    .insert("\0pdo_attr_0".into(), Value::Bool(autocommit));
            }
            Ok(Value::Bool(true))
        }
        ("pdo", "getattribute") => {
            let attribute = arguments
                .first()
                .map(Value::number)
                .ok_or("PDO::getAttribute() expects an attribute")?
                as i64;
            if attribute == 5 {
                let version = PDO_SESSIONS.with(|sessions| {
                    sessions
                        .borrow()
                        .get(&pdo_id(object).ok()?)
                        .and_then(|session| session.client_version())
                });
                return Ok(version.map(Value::String).unwrap_or(Value::Bool(false)));
            }
            if matches!(attribute, 4 | 6) {
                let name = database_name(object)?;
                let kind = database_registry().get(&name)?.kind();
                let Some(sql) = server_version_query(kind) else {
                    return Ok(Value::Bool(false));
                };
                let result = match execute(&name, sql, Vec::new(), Some(pdo_id(object)?)) {
                    Ok(result) => result,
                    Err(error) => return report_error(object, error),
                };
                let Some(value) = result.rows.first().and_then(|row| row.first()) else {
                    return Ok(Value::Bool(false));
                };
                return Ok(Value::String(database_value_string(value)));
            }
            if attribute == 16 {
                let driver = database_registry().get(&database_name(object)?)?;
                let name = match driver.kind() {
                    DatabaseKind::Mysql | DatabaseKind::MariaDb => "mysql",
                    DatabaseKind::PostgreSql => "pgsql",
                    DatabaseKind::SqlServer => "sqlsrv",
                    DatabaseKind::Oracle => "oci",
                    DatabaseKind::MongoDb => return Err("MongoDB has no PDO driver".into()),
                };
                return Ok(Value::String(name.into()));
            }
            Ok(object
                .borrow()
                .properties
                .get(&format!("\0pdo_attr_{attribute}"))
                .cloned()
                .unwrap_or(Value::Bool(false)))
        }
        ("pdo", "errorcode") => Ok(error_code(pdo_id(object)?)),
        ("pdo", "errorinfo") => Ok(error_info(pdo_id(object)?)),
        ("pdo", _) => Err(format!("Call to undefined PDO method {method}()")),
        ("pdostatement", "execute") => {
            let (name, sql, id) = statement_context(object)?;
            let parameters = match arguments.first() {
                Some(value) if matches!(value.dereference(), Value::Null) => {
                    bound_parameters(object)?
                }
                Some(value) => bind_parameters(value)?,
                None => bound_parameters(object)?,
            };
            let result = match execute(&name, &sql, parameters, Some(id)) {
                Ok(result) => result,
                Err(error) => return report_error(object, error),
            };
            set_result(object, result);
            Ok(Value::Bool(true))
        }
        ("pdostatement", "fetch") => fetch(object, arguments.first(), fetch_class),
        ("pdostatement", "fetchall") => fetch_all(object, arguments, fetch_class),
        ("pdostatement", "fetchcolumn") => fetch_column(object, arguments.first()),
        ("pdostatement", "fetchobject") => fetch_object(object, arguments, fetch_class),
        ("pdostatement", "bindvalue") => bind_value(object, arguments),
        ("pdostatement", "bindparam") => bind_param(object, arguments),
        ("pdostatement", "bindcolumn") => bind_column(object, arguments),
        ("pdostatement", "setfetchmode") => {
            let mode = arguments
                .first()
                .cloned()
                .ok_or("PDOStatement::setFetchMode() expects a fetch mode")?;
            if !valid_fetch_mode(mode.number() as i64) {
                return Ok(Value::Bool(false));
            }
            if (mode.number() as i64 & 0xffff) == 9 {
                let Some(Value::Object(target)) = arguments.get(1) else {
                    return Err(
                        "PDOStatement::setFetchMode(PDO::FETCH_INTO) expects an object".into(),
                    );
                };
                object
                    .borrow_mut()
                    .properties
                    .insert("\0fetch_into".into(), Value::Object(target.clone()));
            }
            if (mode.number() as i64 & 0xffff) == 7 {
                let column = arguments.get(1).map(|value| value.number()).unwrap_or(0.0);
                if !column.is_finite() || column < 0.0 || column.fract() != 0.0 {
                    return Ok(Value::Bool(false));
                }
                object
                    .borrow_mut()
                    .properties
                    .insert("\0fetch_column".into(), Value::Integer(column as i64));
            }
            if (mode.number() as i64 & 0xffff) == 8 {
                let Some(class_name) = arguments.get(1) else {
                    return Err(
                        "PDOStatement::setFetchMode(PDO::FETCH_CLASS) expects a class name".into(),
                    );
                };
                object.borrow_mut().properties.insert(
                    "\0fetch_class_name".into(),
                    Value::String(class_name.show()),
                );
                object.borrow_mut().properties.insert(
                    "\0fetch_class_args".into(),
                    arguments
                        .get(2)
                        .cloned()
                        .unwrap_or_else(|| Value::from_array_entries(Vec::new())),
                );
                object.borrow_mut().properties.insert(
                    "\0fetch_props_late".into(),
                    Value::Bool((mode.number() as i64 & 0x100000) != 0),
                );
            }
            if (mode.number() as i64 & 0xffff) == 10 {
                let Some(callback) = arguments.get(1).cloned() else {
                    return Ok(Value::Bool(false));
                };
                object
                    .borrow_mut()
                    .properties
                    .insert("\0fetch_func".into(), callback);
                object
                    .borrow_mut()
                    .properties
                    .insert("\0fetch_mode".into(), mode);
                return Ok(Value::Bool(true));
            }
            object
                .borrow_mut()
                .properties
                .insert("\0fetch_mode".into(), mode);
            Ok(Value::Bool(true))
        }
        ("pdostatement", "rowcount") => {
            let object = object.borrow();
            Ok(object
                .properties
                .get("\0row_count")
                .cloned()
                .unwrap_or(Value::Integer(0)))
        }
        ("pdostatement", "columncount") => Ok(object
            .borrow()
            .properties
            .get("\0column_count")
            .cloned()
            .unwrap_or(Value::Integer(0))),
        ("pdostatement", "getcolumnmeta") => {
            let index = arguments.first().map(Value::number).unwrap_or(-1.0) as i64;
            let object = object.borrow();
            let name = object.properties.get("\0column_names").and_then(|value| {
                match value.dereference() {
                    Value::Array(columns) => columns
                        .iter()
                        .find(
                            |(key, _)| matches!(key, ArrayKey::Integer(column) if *column == index),
                        )
                        .map(|(_, value)| value.clone()),
                    _ => None,
                }
            });
            match name {
                Some(name) => {
                    let mut metadata = vec![(ArrayKey::String("name".into()), name)];
                    if let Some(Value::Array(columns)) = object
                        .properties
                        .get("\0column_metadata")
                        .map(Value::dereference)
                    {
                        if let Some(Value::Array(column)) =
                            columns.iter().find_map(|(key, value)| {
                                matches!(key, ArrayKey::Integer(column) if *column == index)
                                    .then_some(value.dereference())
                            })
                        {
                            metadata.extend(
                                column
                                    .iter()
                                    .map(|(key, value)| (key.clone(), value.clone())),
                            );
                        }
                    }
                    Ok(Value::from_array_entries(metadata))
                }
                None => Ok(Value::Bool(false)),
            }
        }
        ("pdostatement", "nextrowset") => next_rowset(object),
        ("pdostatement", "getattribute") => {
            let attribute = arguments
                .first()
                .map(Value::number)
                .ok_or("PDOStatement::getAttribute() expects an attribute")?
                as i64;
            if attribute == 10 {
                return Ok(object
                    .borrow()
                    .properties
                    .get("\0stmt_attr_10")
                    .cloned()
                    .unwrap_or(Value::Integer(0)));
            }
            Ok(Value::Bool(false))
        }
        ("pdostatement", "setattribute") => {
            let attribute = arguments
                .first()
                .map(Value::number)
                .ok_or("PDOStatement::setAttribute() expects an attribute")?
                as i64;
            let value = arguments
                .get(1)
                .ok_or("PDOStatement::setAttribute() expects a value")?;
            if attribute != 10 || value.number() as i64 != 0 {
                return Ok(Value::Bool(false));
            }
            object
                .borrow_mut()
                .properties
                .insert("\0stmt_attr_10".into(), value.clone());
            Ok(Value::Bool(true))
        }
        ("pdostatement", "debugdumpparams") => {
            let (name, sql, _) = statement_context(object)?;
            let parameter_count = object
                .borrow()
                .properties
                .get("\0bound_params")
                .map(|value| match value.dereference() {
                    Value::Array(values) => values.len(),
                    _ => 0,
                })
                .unwrap_or(0);
            eprintln!("PDOStatement for {name}: SQL={sql}; bound parameters={parameter_count}");
            Ok(Value::Null)
        }
        ("pdostatement", "getiterator") => {
            if !arguments.is_empty() {
                return Err("PDOStatement::getIterator() expects no arguments".into());
            }
            Ok(Value::Object(object.clone()))
        }
        ("pdostatement", "errorcode") => {
            let (_, _, id) = statement_context(object)?;
            Ok(error_code(id))
        }
        ("pdostatement", "errorinfo") => {
            let (_, _, id) = statement_context(object)?;
            Ok(error_info(id))
        }
        ("pdostatement", "closecursor") => {
            let mut object = object.borrow_mut();
            let row_count = object
                .properties
                .get("\0rows")
                .map(|value| match value.dereference() {
                    Value::Array(rows) => rows.len(),
                    _ => 0,
                })
                .unwrap_or(0);
            object
                .properties
                .insert("\0cursor".into(), Value::Integer(row_count as i64));
            Ok(Value::Bool(true))
        }
        ("pdostatement", _) => Err(format!("Call to undefined PDOStatement method {method}()")),
        _ => Err("Invalid PDO object".into()),
    }
}

pub(crate) fn class_constant(class: &str, constant: &str) -> Option<Value> {
    if !matches!(class.to_ascii_lowercase().as_str(), "pdo" | "pdostatement") {
        return None;
    }
    let value = match constant.to_ascii_uppercase().as_str() {
        "FETCH_DEFAULT" => 0.0,
        "FETCH_LAZY" => 1.0,
        "FETCH_ASSOC" => 2.0,
        "FETCH_NUM" => 3.0,
        "FETCH_BOTH" => 4.0,
        "FETCH_OBJ" => 5.0,
        "FETCH_BOUND" => 6.0,
        "FETCH_COLUMN" => 7.0,
        "FETCH_CLASS" => 8.0,
        "FETCH_INTO" => 9.0,
        "FETCH_FUNC" => 10.0,
        "FETCH_NAMED" => 11.0,
        "FETCH_KEY_PAIR" => 12.0,
        "FETCH_GROUP" => 65536.0,
        "FETCH_UNIQUE" => 196608.0,
        "FETCH_PROPS_LATE" => 1048576.0,
        "PARAM_NULL" => 0.0,
        "PARAM_INT" => 1.0,
        "PARAM_STR" => 2.0,
        "PARAM_LOB" => 3.0,
        "PARAM_BOOL" => 5.0,
        "PARAM_INPUT_OUTPUT" => 2147483648.0,
        "PARAM_STR_NATL" => 1073741824.0,
        "PARAM_STR_CHAR" => 536870912.0,
        "PARAM_EVT_ALLOC" => 0.0,
        "PARAM_EVT_FREE" => 1.0,
        "PARAM_EVT_EXEC_PRE" => 2.0,
        "PARAM_EVT_EXEC_POST" => 3.0,
        "PARAM_EVT_FETCH_PRE" => 4.0,
        "PARAM_EVT_FETCH_POST" => 5.0,
        "PARAM_EVT_NORMALIZE" => 6.0,
        "ATTR_AUTOCOMMIT" => 0.0,
        "ATTR_PREFETCH" => 1.0,
        "ATTR_SERVER_VERSION" => 4.0,
        "ATTR_CLIENT_VERSION" => 5.0,
        "ATTR_SERVER_INFO" => 6.0,
        "ATTR_CONNECTION_STATUS" => 7.0,
        "ATTR_CURSOR_NAME" => 9.0,
        "ATTR_PERSISTENT" => 12.0,
        "ATTR_STATEMENT_CLASS" => 13.0,
        "ATTR_FETCH_TABLE_NAMES" => 14.0,
        "ATTR_FETCH_CATALOG_NAMES" => 15.0,
        "ATTR_MAX_COLUMN_LEN" => 1000.0,
        "ATTR_DEFAULT_STR_PARAM" => 21.0,
        "ATTR_ERRMODE" => 3.0,
        "ERRMODE_SILENT" => 0.0,
        "ERRMODE_WARNING" => 1.0,
        "ERRMODE_EXCEPTION" => 2.0,
        "ATTR_CASE" => 8.0,
        "CASE_NATURAL" => 0.0,
        "CASE_UPPER" => 1.0,
        "CASE_LOWER" => 2.0,
        "ATTR_ORACLE_NULLS" => 11.0,
        "NULL_NATURAL" => 0.0,
        "NULL_EMPTY_STRING" => 1.0,
        "NULL_TO_STRING" => 2.0,
        "ATTR_STRINGIFY_FETCHES" => 17.0,
        "ATTR_DRIVER_NAME" => 16.0,
        "ATTR_DEFAULT_FETCH_MODE" => 19.0,
        "ATTR_TIMEOUT" => 2.0,
        "ATTR_CURSOR" => 10.0,
        "CURSOR_FWDONLY" => 0.0,
        "CURSOR_SCROLL" => 1.0,
        "ATTR_EMULATE_PREPARES" => 20.0,
        _ => return None,
    };
    Some(Value::Integer(value as i64))
}

pub(crate) fn method_exists(class: &str, method: &str) -> bool {
    match class.to_ascii_lowercase().as_str() {
        "pdo" => matches!(
            method.to_ascii_lowercase().as_str(),
            "connect"
                | "query"
                | "exec"
                | "prepare"
                | "begintransaction"
                | "commit"
                | "rollback"
                | "intransaction"
                | "lastinsertid"
                | "quote"
                | "errorcode"
                | "errorinfo"
                | "setattribute"
                | "getattribute"
                | "getavailabledrivers"
        ),
        "pdostatement" => matches!(
            method.to_ascii_lowercase().as_str(),
            "execute"
                | "fetch"
                | "fetchall"
                | "fetchcolumn"
                | "fetchobject"
                | "bindvalue"
                | "bindparam"
                | "bindcolumn"
                | "setfetchmode"
                | "rowcount"
                | "columncount"
                | "getcolumnmeta"
                | "getattribute"
                | "setattribute"
                | "nextrowset"
                | "debugdumpparams"
                | "closecursor"
                | "errorcode"
                | "errorinfo"
                | "getiterator"
        ),
        _ => false,
    }
}

/// Take the unread result rows for PDOStatement's IteratorAggregate behavior.
pub(crate) fn statement_iterator_rows(
    value: &Value,
    fetch_class: &mut dyn FnMut(&Value, &str, &[Value]) -> Result<Value, String>,
) -> Result<Vec<(ArrayKey, Value)>, String> {
    let Value::Object(object) = value else {
        return Err("Invalid PDOStatement iterator".into());
    };
    let mut statement = object.borrow_mut();
    if !statement.class_name.eq_ignore_ascii_case("PDOStatement") {
        return Err("Invalid PDOStatement iterator".into());
    }
    let rows = statement
        .properties
        .get("\0rows")
        .map(|value| match value.dereference() {
            Value::Array(rows) => rows.iter().cloned().collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .unwrap_or_default();
    let cursor = statement
        .properties
        .get("\0cursor")
        .map(Value::number)
        .unwrap_or(0.0)
        .max(0.0) as usize;
    let mode = statement
        .properties
        .get("\0fetch_mode")
        .map(Value::number)
        .unwrap_or(4.0) as i64;
    let column = statement
        .properties
        .get("\0fetch_column")
        .map(Value::number)
        .unwrap_or(0.0)
        .max(0.0) as usize;
    statement
        .properties
        .insert("\0cursor".into(), Value::Integer(rows.len() as i64));
    drop(statement);
    rows.into_iter()
        .skip(cursor)
        .map(|(key, row)| {
            let flags = mode & !0xffff;
            let fetched = match mode & 0xffff {
                0 | 4 => row_as_fetch_mode(row, 4, 0)?,
                2 | 3 | 5 | 7 => row_as_fetch_mode(row, mode & 0xffff, column)?,
                6 => fetch_bound(object, row)?,
                9 => fetch_into(object, row)?,
                11 => fetch_named(object, row)?,
                1 => row_as_fetch_mode(row, 1, 0)?,
                8 => {
                    let (class_name, constructor_args) = stored_fetch_class(object)?;
                    let args = value_array(&constructor_args)?;
                    let class_name = if flags & 0x100000 != 0
                        || object
                            .borrow()
                            .properties
                            .get("\0fetch_props_late")
                            .is_some_and(Value::truthy)
                    {
                        format!("\0PDO_FETCH_PROPS_LATE:{class_name}")
                    } else {
                        class_name
                    };
                    fetch_class(&row, &class_name, &args)?
                }
                10 => {
                    let callback = object
                        .borrow()
                        .properties
                        .get("\0fetch_func")
                        .cloned()
                        .ok_or("PDO::FETCH_FUNC requires a callback passed to setFetchMode()")?;
                    let Value::Array(fields) = row.dereference() else {
                        return Err("PDO::FETCH_FUNC row must be an array".into());
                    };
                    let args = fields
                        .iter()
                        .filter_map(|(key, value)| {
                            matches!(key, ArrayKey::Integer(_)).then(|| value.clone())
                        })
                        .collect::<Vec<_>>();
                    fetch_class(&callback, "\0PDO_FETCH_FUNC", &args)?
                }
                other => {
                    return Err(format!(
                        "PDOStatement iteration does not support fetch mode {other}"
                    ))
                }
            };
            Ok((key, fetched))
        })
        .collect()
}

pub(crate) fn call_static_method(
    class: &str,
    method: &str,
    arguments: &[Value],
) -> Option<Result<Value, String>> {
    if !class.eq_ignore_ascii_case("PDO") {
        return None;
    }
    if method.eq_ignore_ascii_case("connect") {
        return Some(construct(arguments));
    }
    if !method.eq_ignore_ascii_case("getAvailableDrivers") {
        return None;
    }
    Some((|| {
        if !arguments.is_empty() {
            return Err("PDO::getAvailableDrivers() expects no arguments".into());
        }
        let registry = database_registry();
        let mut names = registry
            .names()
            .into_iter()
            .filter_map(|name| {
                registry
                    .get(&name)
                    .ok()
                    .and_then(|driver| match driver.kind() {
                        DatabaseKind::Mysql | DatabaseKind::MariaDb => Some("mysql"),
                        DatabaseKind::PostgreSql => Some("pgsql"),
                        DatabaseKind::SqlServer => Some("sqlsrv"),
                        DatabaseKind::Oracle => Some("oci"),
                        DatabaseKind::MongoDb => None,
                    })
            })
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        Ok(Value::from_array_entries(
            names
                .into_iter()
                .enumerate()
                .map(|(index, name)| (ArrayKey::Integer(index as i64), Value::String(name.into())))
                .collect(),
        ))
    })())
}

fn command_status(object: &Rc<RefCell<PhpObject>>, sql: &str) -> Result<Value, String> {
    execute(
        &database_name(object)?,
        sql,
        Vec::new(),
        Some(pdo_id(object)?),
    )?;
    Ok(Value::Bool(true))
}

fn begin_sql(kind: DatabaseKind) -> Option<&'static str> {
    match kind {
        DatabaseKind::Mysql | DatabaseKind::MariaDb => Some("START TRANSACTION"),
        DatabaseKind::SqlServer => Some("BEGIN TRANSACTION"),
        DatabaseKind::PostgreSql => Some("BEGIN"),
        DatabaseKind::Oracle => None,
        DatabaseKind::MongoDb => unreachable!("MongoDB is not a PDO SQL driver"),
    }
}

fn start_transaction(object: &Rc<RefCell<PhpObject>>) -> Result<(), String> {
    let id = pdo_id(object)?;
    let driver = database_registry().get(&database_name(object)?)?;
    let timeout = PDO_TIMEOUTS.with(|timeouts| timeouts.borrow().get(&id).copied().unwrap_or(0));
    PDO_SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        if !sessions.contains_key(&id) {
            sessions.insert(
                id,
                driver.start_session_with_timeout(std::time::Duration::from_secs(timeout))?,
            );
        }
        let begin_result = if let Some(begin) = begin_sql(driver.kind()) {
            sessions
                .get_mut(&id)
                .expect("PDO session was initialized")
                .execute(&DbCommand::Sql {
                    statement: begin.into(),
                    parameters: Vec::new(),
                })
        } else {
            Ok(DbResult::default())
        };
        if let Err(error) = begin_result {
            sessions.remove(&id);
            return Err(error);
        }
        Ok(())
    })
}

fn transaction_active(object: &Rc<RefCell<PhpObject>>) -> bool {
    let Ok(id) = pdo_id(object) else {
        return false;
    };
    ACTIVE_TRANSACTIONS.with(|transactions| transactions.borrow().contains(&id))
}

fn finish_transaction(object: &Rc<RefCell<PhpObject>>, sql: &str) -> Result<Value, String> {
    if !transaction_active(object) {
        return Ok(Value::Bool(false));
    }
    let depth = object
        .borrow()
        .properties
        .get("\0transaction_depth")
        .map(Value::number)
        .unwrap_or(1.0) as usize;
    if depth > 1 {
        return finish_nested_transaction(object, depth, sql == "ROLLBACK");
    }
    command_status(object, sql)?;
    let id = pdo_id(object)?;
    ACTIVE_TRANSACTIONS.with(|transactions| {
        transactions.borrow_mut().remove(&id);
    });
    object
        .borrow_mut()
        .properties
        .insert("\0transaction_active".into(), Value::Bool(false));
    object
        .borrow_mut()
        .properties
        .insert("\0transaction_depth".into(), Value::Integer(0));
    Ok(Value::Bool(true))
}

fn savepoint_commands(kind: DatabaseKind, name: &str) -> (String, Option<String>, String) {
    match kind {
        DatabaseKind::Mysql | DatabaseKind::MariaDb | DatabaseKind::PostgreSql => (
            format!("SAVEPOINT {name}"),
            Some(format!("RELEASE SAVEPOINT {name}")),
            format!("ROLLBACK TO SAVEPOINT {name}"),
        ),
        DatabaseKind::SqlServer => (
            format!("SAVE TRANSACTION {name}"),
            None,
            format!("ROLLBACK TRANSACTION {name}"),
        ),
        DatabaseKind::Oracle => (
            format!("SAVEPOINT {name}"),
            None,
            format!("ROLLBACK TO {name}"),
        ),
        DatabaseKind::MongoDb => unreachable!("PDO excludes MongoDB"),
    }
}

fn begin_nested_transaction(object: &Rc<RefCell<PhpObject>>) -> Result<Value, String> {
    let depth = (object
        .borrow()
        .properties
        .get("\0transaction_depth")
        .map(Value::number)
        .unwrap_or(1.0) as usize)
        .checked_add(1)
        .ok_or("PDO nested transaction depth overflow")?;
    let name = format!("php_rust_sp_{depth}");
    let driver = database_registry().get(&database_name(object)?)?;
    let (save, _, _) = savepoint_commands(driver.kind(), &name);
    match execute(
        &database_name(object)?,
        &save,
        Vec::new(),
        Some(pdo_id(object)?),
    ) {
        Ok(_) => {
            object
                .borrow_mut()
                .properties
                .insert("\0transaction_depth".into(), Value::Integer(depth as i64));
            Ok(Value::Bool(true))
        }
        Err(error) => report_error(object, error),
    }
}

fn finish_nested_transaction(
    object: &Rc<RefCell<PhpObject>>,
    depth: usize,
    rollback: bool,
) -> Result<Value, String> {
    let name = format!("php_rust_sp_{depth}");
    let driver = database_registry().get(&database_name(object)?)?;
    let (_, release, rollback_to) = savepoint_commands(driver.kind(), &name);
    let command = if rollback {
        Some(rollback_to)
    } else {
        release.clone()
    };
    if let Some(command) = command {
        match execute(
            &database_name(object)?,
            &command,
            Vec::new(),
            Some(pdo_id(object)?),
        ) {
            Ok(_) => {}
            Err(error) => return report_error(object, error),
        }
        if rollback {
            if let Some(release) = release {
                match execute(
                    &database_name(object)?,
                    &release,
                    Vec::new(),
                    Some(pdo_id(object)?),
                ) {
                    Ok(_) => {}
                    Err(error) => return report_error(object, error),
                }
            }
        }
    }
    object.borrow_mut().properties.insert(
        "\0transaction_depth".into(),
        Value::Integer((depth - 1) as i64),
    );
    Ok(Value::Bool(true))
}

fn execute(
    name: &str,
    sql: &str,
    parameters: Vec<(ArrayKey, DbValue)>,
    session_id: Option<u64>,
) -> Result<DbResult, String> {
    let driver = database_registry().get(name)?;
    let emulate = session_id
        .and_then(|id| PDO_EMULATE_PREPARES.with(|modes| modes.borrow().get(&id).copied()))
        .unwrap_or(false);
    let native_prepared = !emulate
        && matches!(
            driver.kind(),
            DatabaseKind::Mysql
                | DatabaseKind::MariaDb
                | DatabaseKind::SqlServer
                | DatabaseKind::Oracle
        )
        && !parameters.is_empty();
    let style = match driver.kind() {
        DatabaseKind::PostgreSql if emulate => PlaceholderStyle::InlinePostgres,
        DatabaseKind::PostgreSql => PlaceholderStyle::Numbered,
        DatabaseKind::Mysql | DatabaseKind::MariaDb => PlaceholderStyle::QuestionMySql,
        DatabaseKind::SqlServer => PlaceholderStyle::QuestionSqlServer,
        DatabaseKind::Oracle => PlaceholderStyle::QuestionOracle,
        DatabaseKind::MongoDb => unreachable!("MongoDB is not a PDO SQL driver"),
    };
    let (statement, parameters) = rewrite_placeholders(sql, parameters, style)?;
    let command = if native_prepared {
        DbCommand::PreparedSql {
            statement,
            parameters,
        }
    } else {
        DbCommand::Sql {
            statement,
            parameters,
        }
    };
    let implicit_begin = session_id.is_some_and(|id| {
        !PDO_AUTOCOMMIT.with(|modes| modes.borrow().get(&id).copied().unwrap_or(true))
            && !ACTIVE_TRANSACTIONS.with(|transactions| transactions.borrow().contains(&id))
    });
    // A PDO handle owns one checked-out connection for the lifetime of the
    // PHP request. This preserves session state (temporary tables, variables,
    // isolation settings, and lastInsertId) even outside transactions.
    let result = if let Some(id) = session_id {
        PDO_SESSIONS.with(|sessions| {
            let mut sessions = sessions.borrow_mut();
            let timeout =
                PDO_TIMEOUTS.with(|timeouts| timeouts.borrow().get(&id).copied().unwrap_or(0));
            if !sessions.contains_key(&id) {
                sessions.insert(
                    id,
                    driver.start_session_with_timeout(std::time::Duration::from_secs(timeout))?,
                );
            }
            if implicit_begin {
                ACTIVE_TRANSACTIONS.with(|transactions| {
                    transactions.borrow_mut().insert(id);
                });
                if let Some(begin) = begin_sql(driver.kind()) {
                    sessions
                        .get_mut(&id)
                        .ok_or("PDO session is missing")?
                        .execute(&DbCommand::Sql {
                            statement: begin.into(),
                            parameters: Vec::new(),
                        })?;
                }
            }
            sessions
                .get_mut(&id)
                .ok_or("PDO session is missing")?
                .execute(&command)
        })
    } else {
        driver.execute(&command)
    };
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            if let Some(id) = session_id {
                let was_active =
                    ACTIVE_TRANSACTIONS.with(|transactions| transactions.borrow_mut().remove(&id));
                PDO_SESSIONS.with(|sessions| {
                    let mut sessions = sessions.borrow_mut();
                    if let Some(session) = sessions.get_mut(&id) {
                        if was_active {
                            let _ = session.execute(&DbCommand::Sql {
                                statement: "ROLLBACK".into(),
                                parameters: Vec::new(),
                            });
                        }
                    }
                    sessions.remove(&id);
                });
                LAST_ERRORS.with(|errors| {
                    errors.borrow_mut().insert(id, error.clone());
                });
            }
            return Err(error);
        }
    };
    if let Some(id) = session_id {
        LAST_ERRORS.with(|errors| {
            errors.borrow_mut().remove(&id);
        });
    }
    if let Some(id) = session_id
        .filter(|id| ACTIVE_TRANSACTIONS.with(|transactions| transactions.borrow().contains(id)))
    {
        match transaction_command(sql) {
            Some(true) => {
                ACTIVE_TRANSACTIONS.with(|transactions| {
                    transactions.borrow_mut().insert(id);
                });
            }
            Some(false) => {
                ACTIVE_TRANSACTIONS.with(|transactions| {
                    transactions.borrow_mut().remove(&id);
                });
            }
            None => {}
        }
    }
    if let (Some(id), Some(insert_id)) = (session_id, result.last_insert_id.as_ref()) {
        LAST_INSERT_IDS.with(|ids| {
            ids.borrow_mut().insert(id, insert_id.clone());
        });
    }
    Ok(result)
}

fn transaction_command(sql: &str) -> Option<bool> {
    let command = sql.trim_start().to_ascii_lowercase();
    let mut words = command.split_whitespace();
    match (
        words.next()?.trim_end_matches(';'),
        words.next().unwrap_or("").trim_end_matches(';'),
    ) {
        ("begin", _) | ("start", "transaction") => Some(true),
        ("commit", _) => Some(false),
        ("rollback", "to") => None,
        ("rollback", _) => Some(false),
        _ => None,
    }
}

fn report_error(object: &Rc<RefCell<PhpObject>>, error: String) -> Result<Value, String> {
    let id = pdo_id(object)?;
    LAST_ERRORS.with(|errors| {
        errors.borrow_mut().insert(id, error.clone());
    });
    let message = DbErrorInfo::decode(&error).message;
    let mode = ERROR_MODES.with(|modes| modes.borrow().get(&id).copied().unwrap_or(2));
    match mode {
        0 => Ok(Value::Bool(false)),
        1 => {
            eprintln!("PHP Warning: {message}");
            Ok(Value::Bool(false))
        }
        _ => Err(message),
    }
}

fn error_code(id: u64) -> Value {
    LAST_ERRORS.with(|errors| {
        errors
            .borrow()
            .get(&id)
            .map(|error| Value::String(DbErrorInfo::decode(error).sql_state))
            .unwrap_or_else(|| Value::String("00000".into()))
    })
}

fn error_info(id: u64) -> Value {
    let error = LAST_ERRORS.with(|errors| errors.borrow().get(&id).cloned());
    let values = match error {
        Some(error) => {
            let info = DbErrorInfo::decode(&error);
            vec![
                Value::String(info.sql_state),
                info.driver_code.map(Value::Integer).unwrap_or(Value::Null),
                Value::String(info.message),
            ]
        }
        None => vec![Value::String("00000".into()), Value::Null, Value::Null],
    };
    Value::from_array_entries(
        values
            .into_iter()
            .enumerate()
            .map(|(index, value)| (ArrayKey::Integer(index as i64), value))
            .collect(),
    )
}

fn valid_sequence_name(value: &str) -> bool {
    !value.is_empty()
        && value.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'#'))
        })
}

fn statement(
    name: String,
    pdo_id: u64,
    sql: Option<String>,
    result: DbResult,
    case_mode: i64,
    oracle_nulls: i64,
    stringify: bool,
    max_column_len: usize,
    table_names: bool,
    catalog_names: bool,
) -> Value {
    let mut properties = HashMap::new();
    properties.insert("\0database_name".into(), Value::String(name));
    properties.insert("\0pdo_id".into(), Value::Integer(pdo_id as i64));
    properties.insert("\0pdo_case_mode".into(), Value::Integer(case_mode));
    properties.insert("\0pdo_oracle_nulls".into(), Value::Integer(oracle_nulls));
    properties.insert("\0pdo_stringify_fetches".into(), Value::Bool(stringify));
    properties.insert(
        "\0pdo_max_column_len".into(),
        Value::Integer(max_column_len as i64),
    );
    properties.insert("\0pdo_fetch_table_names".into(), Value::Bool(table_names));
    properties.insert(
        "\0pdo_fetch_catalog_names".into(),
        Value::Bool(catalog_names),
    );
    if let Some(sql) = sql {
        properties.insert("\0statement_sql".into(), Value::String(sql));
    }
    let object = Rc::new(RefCell::new(PhpObject {
        class_name: "PDOStatement".into(),
        properties,
        property_order: Vec::new(),
        active_property_hooks: Default::default(),
        exception: None,
    }));
    set_result(&object, result);
    Value::Object(object)
}

fn set_result(object: &Rc<RefCell<PhpObject>>, result: DbResult) {
    let (name, pdo_id, max_column_len, table_names, catalog_names) = {
        let object = object.borrow();
        (
            object
                .properties
                .get("\0database_name")
                .map(Value::show)
                .unwrap_or_default(),
            object
                .properties
                .get("\0pdo_id")
                .map(Value::number)
                .unwrap_or(0.0) as u64,
            object
                .properties
                .get("\0pdo_max_column_len")
                .map(Value::number)
                .unwrap_or(0.0)
                .max(0.0) as usize,
            object
                .properties
                .get("\0pdo_fetch_table_names")
                .is_some_and(Value::truthy),
            object
                .properties
                .get("\0pdo_fetch_catalog_names")
                .is_some_and(Value::truthy),
        )
    };
    let mut result = result;
    let additional = std::mem::take(&mut result.additional_result_sets)
        .into_iter()
        .map(|set| {
            let case_mode = object
                .borrow()
                .properties
                .get("\0pdo_case_mode")
                .map(Value::number)
                .unwrap_or(0.0) as i64;
            let oracle_nulls = object
                .borrow()
                .properties
                .get("\0pdo_oracle_nulls")
                .map(Value::number)
                .unwrap_or(0.0) as i64;
            let stringify = object
                .borrow()
                .properties
                .get("\0pdo_stringify_fetches")
                .is_some_and(Value::truthy);
            statement(
                name.clone(),
                pdo_id,
                None,
                DbResult {
                    columns: set.columns,
                    column_metadata: set.column_metadata,
                    rows: set.rows,
                    affected_rows: set.affected_rows,
                    ..DbResult::default()
                },
                case_mode,
                oracle_nulls,
                stringify,
                max_column_len,
                table_names,
                catalog_names,
            )
        })
        .collect::<Vec<_>>();
    if object
        .borrow()
        .properties
        .get("\0pdo_case_mode")
        .is_some_and(|mode| mode.number() as i64 == 1)
    {
        result
            .columns
            .iter_mut()
            .for_each(|name| *name = name.to_ascii_uppercase());
    } else if object
        .borrow()
        .properties
        .get("\0pdo_case_mode")
        .is_some_and(|mode| mode.number() as i64 == 2)
    {
        result
            .columns
            .iter_mut()
            .for_each(|name| *name = name.to_ascii_lowercase());
    }
    qualify_column_names(
        &mut result.columns,
        &result.column_metadata,
        table_names,
        catalog_names,
    );
    truncate_column_names(&mut result.columns, max_column_len);
    let row_count = result.affected_rows.max(result.rows.len() as u64);
    let column_count = result.columns.len();
    let column_names = Value::from_array_entries(
        result
            .columns
            .iter()
            .enumerate()
            .map(|(index, name)| (ArrayKey::Integer(index as i64), Value::String(name.clone())))
            .collect(),
    );
    let column_metadata = Value::from_array_entries(
        result
            .column_metadata
            .iter()
            .enumerate()
            .map(|(index, metadata)| {
                let mut entries = Vec::new();
                if let Some(native_type) = &metadata.native_type {
                    entries.push((
                        ArrayKey::String("native_type".into()),
                        Value::String(native_type.clone()),
                    ));
                }
                if let Some(length) = metadata.length {
                    entries.push((
                        ArrayKey::String("len".into()),
                        Value::Integer(length as i64),
                    ));
                }
                if let Some(precision) = metadata.precision {
                    entries.push((
                        ArrayKey::String("precision".into()),
                        Value::Integer(precision as i64),
                    ));
                }
                if let Some(nullable) = metadata.nullable {
                    entries.push((ArrayKey::String("nullable".into()), Value::Bool(nullable)));
                }
                if let Some(table_name) = &metadata.table_name {
                    entries.push((
                        ArrayKey::String("table".into()),
                        Value::String(table_name.clone()),
                    ));
                }
                if let Some(catalog_name) = &metadata.catalog_name {
                    entries.push((
                        ArrayKey::String("catalog".into()),
                        Value::String(catalog_name.clone()),
                    ));
                }
                (
                    ArrayKey::Integer(index as i64),
                    Value::from_array_entries(entries),
                )
            })
            .collect(),
    );
    let is_oracle = database_registry()
        .get(&name)
        .is_ok_and(|driver| driver.kind() == DatabaseKind::Oracle);
    let oracle_nulls = object
        .borrow()
        .properties
        .get("\0pdo_oracle_nulls")
        .map(Value::number)
        .unwrap_or(0.0) as i64;
    let stringify = object
        .borrow()
        .properties
        .get("\0pdo_stringify_fetches")
        .is_some_and(Value::truthy);
    let rows = result
        .rows
        .into_iter()
        .map(|row| {
            let mut entries = Vec::with_capacity(row.len() * 2);
            for (index, value) in row.into_iter().enumerate() {
                let mut value = to_php_oracle(value, is_oracle, oracle_nulls);
                if stringify && !matches!(value, Value::Null) {
                    value = Value::String(value.show());
                }
                if let Some(name) = result.columns.get(index) {
                    entries.push((ArrayKey::String(name.clone()), value.clone()));
                }
                entries.push((ArrayKey::Integer(index as i64), value));
            }
            Value::from_array_entries(entries)
        })
        .enumerate()
        .map(|(index, row)| (ArrayKey::Integer(index as i64), row))
        .collect();
    let mut object = object.borrow_mut();
    object
        .properties
        .insert("\0rows".into(), Value::from_array_entries(rows));
    object
        .properties
        .insert("\0cursor".into(), Value::Integer(0));
    object
        .properties
        .insert("\0row_count".into(), Value::Integer(row_count as i64));
    object
        .properties
        .insert("\0column_count".into(), Value::Integer(column_count as i64));
    object
        .properties
        .insert("\0column_names".into(), column_names);
    object
        .properties
        .insert("\0column_metadata".into(), column_metadata);
    object.properties.insert(
        "\0additional_rowsets".into(),
        Value::from_array_entries(
            additional
                .into_iter()
                .enumerate()
                .map(|(index, statement)| (ArrayKey::Integer(index as i64), statement))
                .collect(),
        ),
    );
    object
        .properties
        .insert("\0next_rowset_index".into(), Value::Integer(0));
}

fn to_php_oracle(value: DbValue, is_oracle: bool, oracle_nulls: i64) -> Value {
    if is_oracle {
        match (&value, oracle_nulls) {
            (DbValue::Null, 2) => return Value::String(String::new()),
            (DbValue::String(value), 1) if value.is_empty() => return Value::Null,
            _ => {}
        }
    }
    to_php(value)
}

fn next_rowset(object: &Rc<RefCell<PhpObject>>) -> Result<Value, String> {
    let mut object = object.borrow_mut();
    let index = object
        .properties
        .get("\0next_rowset_index")
        .map(Value::number)
        .unwrap_or(0.0)
        .max(0.0) as i64;
    let next = object
        .properties
        .get("\0additional_rowsets")
        .and_then(|value| match value.dereference() {
            Value::Array(sets) => sets.iter().find_map(|(key, value)| {
                matches!(key, ArrayKey::Integer(candidate) if *candidate == index)
                    .then(|| value.clone())
            }),
            _ => None,
        });
    let Some(Value::Object(next)) = next else {
        return Ok(Value::Bool(false));
    };
    let next_properties = next.borrow().properties.clone();
    for property in [
        "\0rows",
        "\0cursor",
        "\0row_count",
        "\0column_count",
        "\0column_names",
        "\0column_metadata",
    ] {
        if let Some(value) = next_properties.get(property) {
            object.properties.insert(property.into(), value.clone());
        }
    }
    object
        .properties
        .insert("\0next_rowset_index".into(), Value::Integer(index + 1));
    Ok(Value::Bool(true))
}

fn fetch(
    object: &Rc<RefCell<PhpObject>>,
    explicit_mode: Option<&Value>,
    fetch_class: &mut dyn FnMut(&Value, &str, &[Value]) -> Result<Value, String>,
) -> Result<Value, String> {
    let (mode, flags) = {
        let object = object.borrow();
        let mode = explicit_mode
            .map(Value::number)
            .filter(|mode| *mode != 0.0)
            .or_else(|| object.properties.get("\0fetch_mode").map(Value::number))
            .unwrap_or(4.0) as i64;
        (mode & 0xffff, mode & !0xffff)
    };
    let Some(row) = fetch_raw(object)? else {
        return Ok(Value::Bool(false));
    };
    if mode == 6 {
        return fetch_bound(object, row);
    }
    if mode == 9 {
        return fetch_into(object, row);
    }
    if mode == 11 {
        return fetch_named(object, row);
    }
    if mode == 8 {
        let (class_name, constructor_args) = stored_fetch_class(object)?;
        let args = value_array(&constructor_args)?;
        let class_name = if flags & 0x100000 != 0
            || object
                .borrow()
                .properties
                .get("\0fetch_props_late")
                .is_some_and(Value::truthy)
        {
            format!("\0PDO_FETCH_PROPS_LATE:{class_name}")
        } else {
            class_name
        };
        return fetch_class(&row, &class_name, &args);
    }
    if mode == 10 {
        let callback = object
            .borrow()
            .properties
            .get("\0fetch_func")
            .cloned()
            .ok_or("PDO::FETCH_FUNC requires a callback passed to setFetchMode()")?;
        let Value::Array(fields) = row.dereference() else {
            return Err("PDO::FETCH_FUNC row must be an array".into());
        };
        let args = fields
            .iter()
            .filter_map(|(key, value)| matches!(key, ArrayKey::Integer(_)).then(|| value.clone()))
            .collect::<Vec<_>>();
        return fetch_class(&callback, "\0PDO_FETCH_FUNC", &args);
    }
    row_as_fetch_mode(row, mode, 0)
}

fn fetch_named(statement: &Rc<RefCell<PhpObject>>, row: Value) -> Result<Value, String> {
    let Value::Array(row) = row.dereference() else {
        return Ok(row);
    };
    let names = statement.borrow().properties.get("\0column_names").cloned();
    let Some(Value::Array(names)) = names.map(|value| value.dereference()) else {
        return Ok(Value::from_array_entries(
            row.iter()
                .filter(|(key, _)| matches!(key, ArrayKey::String(_)))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ));
    };
    let mut grouped = Vec::<(String, Vec<Value>)>::new();
    for (key, value) in row.iter() {
        let ArrayKey::Integer(index) = key else {
            continue;
        };
        let Some((_, column_name)) = names.get(*index as usize) else {
            continue;
        };
        let name = column_name.show();
        if let Some((_, values)) = grouped.iter_mut().find(|(candidate, _)| candidate == &name) {
            values.push(value.clone());
        } else {
            grouped.push((name, vec![value.clone()]));
        }
    }
    Ok(Value::from_array_entries(
        grouped
            .into_iter()
            .map(|(name, mut values)| {
                let value = if values.len() == 1 {
                    values.pop().expect("single column value")
                } else {
                    Value::from_array_entries(
                        values
                            .into_iter()
                            .enumerate()
                            .map(|(index, value)| (ArrayKey::Integer(index as i64), value))
                            .collect(),
                    )
                };
                (ArrayKey::String(name), value)
            })
            .collect(),
    ))
}

fn fetch_column(object: &Rc<RefCell<PhpObject>>, column: Option<&Value>) -> Result<Value, String> {
    let index = column.map(Value::number).unwrap_or(0.0).max(0.0) as usize;
    fetch_raw(object)?
        .map(|row| row_as_fetch_mode(row, 7, index))
        .transpose()
        .map(|row| row.unwrap_or(Value::Bool(false)))
}

fn fetch_object(
    object: &Rc<RefCell<PhpObject>>,
    arguments: &[Value],
    fetch_class: &mut dyn FnMut(&Value, &str, &[Value]) -> Result<Value, String>,
) -> Result<Value, String> {
    let class_name = arguments
        .first()
        .map(Value::show)
        .unwrap_or_else(|| "stdClass".into());
    let constructor_args = arguments
        .get(1)
        .cloned()
        .unwrap_or_else(|| Value::from_array_entries(Vec::new()));
    let Some(row) = fetch_raw(object)? else {
        return Ok(Value::Bool(false));
    };
    fetch_class(&row, &class_name, &value_array(&constructor_args)?)
}

fn stored_fetch_class(object: &Rc<RefCell<PhpObject>>) -> Result<(String, Value), String> {
    let object = object.borrow();
    let class_name = object
        .properties
        .get("\0fetch_class_name")
        .map(Value::show)
        .ok_or("PDO::FETCH_CLASS requires a class name in setFetchMode() or fetchAll()")?;
    let args = object
        .properties
        .get("\0fetch_class_args")
        .cloned()
        .unwrap_or_else(|| Value::from_array_entries(Vec::new()));
    Ok((class_name, args))
}

fn value_array(value: &Value) -> Result<Vec<Value>, String> {
    match value.dereference() {
        Value::Array(values) => Ok(values.iter().map(|(_, value)| value.clone()).collect()),
        Value::Null => Ok(Vec::new()),
        _ => Err("PDO fetch constructor arguments must be an array".into()),
    }
}

fn fetch_raw(object: &Rc<RefCell<PhpObject>>) -> Result<Option<Value>, String> {
    let mut object = object.borrow_mut();
    let cursor = object
        .properties
        .get("\0cursor")
        .map(Value::number)
        .unwrap_or(0.0)
        .max(0.0) as usize;
    let row = object
        .properties
        .get("\0rows")
        .and_then(|value| match value.dereference() {
            Value::Array(array) => array.get(cursor).map(|(_, value)| value.clone()),
            _ => None,
        });
    object.properties.insert(
        "\0cursor".into(),
        Value::Integer((cursor + usize::from(row.is_some())) as i64),
    );
    Ok(row)
}

fn fetch_all(
    object: &Rc<RefCell<PhpObject>>,
    arguments: &[Value],
    fetch_class: &mut dyn FnMut(&Value, &str, &[Value]) -> Result<Value, String>,
) -> Result<Value, String> {
    let statement = Rc::clone(object);
    let mut object = object.borrow_mut();
    let requested_mode = arguments
        .first()
        .map(Value::number)
        .filter(|mode| *mode != 0.0)
        .or_else(|| object.properties.get("\0fetch_mode").map(Value::number))
        .unwrap_or(4.0) as i64;
    let flags = requested_mode & !0xffff;
    let mode = requested_mode & 0xffff;
    let column = arguments.get(1).map(Value::number).unwrap_or(0.0).max(0.0) as usize;
    let cursor = object
        .properties
        .get("\0cursor")
        .map(Value::number)
        .unwrap_or(0.0)
        .max(0.0) as usize;
    let rows: Vec<Value> = object
        .properties
        .get("\0rows")
        .and_then(|value| match value.dereference() {
            Value::Array(array) => Some(
                array
                    .iter()
                    .skip(cursor)
                    .map(|(_, value)| value.clone())
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    if mode == 8 {
        let class_name = arguments
            .get(1)
            .map(Value::show)
            .or_else(|| object.properties.get("\0fetch_class_name").map(Value::show))
            .ok_or("PDO::FETCH_CLASS requires a class name")?;
        let constructor_args = if arguments.len() > 2 {
            value_array(&arguments[2])?
        } else {
            object
                .properties
                .get("\0fetch_class_args")
                .map(value_array)
                .transpose()?
                .unwrap_or_default()
        };
        drop(object);
        let class_name = if flags & 0x100000 != 0 {
            format!("\0PDO_FETCH_PROPS_LATE:{class_name}")
        } else {
            class_name
        };
        let entries = rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                fetch_class(&row, &class_name, &constructor_args)
                    .map(|value| (ArrayKey::Integer(index as i64), value))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let count = entries.len();
        let mut object = statement.borrow_mut();
        object
            .properties
            .insert("\0cursor".into(), Value::Integer((cursor + count) as i64));
        return Ok(Value::from_array_entries(entries));
    }
    if mode == 10 {
        let callback = arguments
            .get(1)
            .cloned()
            .or_else(|| object.properties.get("\0fetch_func").cloned())
            .ok_or("PDO::FETCH_FUNC requires a callback")?;
        drop(object);
        let entries = rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let Value::Array(fields) = row.dereference() else {
                    return Err("PDO::FETCH_FUNC row must be an array".into());
                };
                let args = fields
                    .iter()
                    .filter_map(|(key, value)| {
                        matches!(key, ArrayKey::Integer(_)).then(|| value.clone())
                    })
                    .collect::<Vec<_>>();
                fetch_class(&callback, "\0PDO_FETCH_FUNC", &args)
                    .map(|value| (ArrayKey::Integer(index as i64), value))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let count = entries.len();
        statement
            .borrow_mut()
            .properties
            .insert("\0cursor".into(), Value::Integer((cursor + count) as i64));
        return Ok(Value::from_array_entries(entries));
    }
    if mode == 12 {
        let mut pairs = crate::PhpArray::new();
        for row in &rows {
            let Value::Array(row) = row.dereference() else {
                continue;
            };
            let key = row.iter().find_map(|(key, value)| {
                matches!(key, ArrayKey::Integer(0)).then(|| value.clone())
            });
            let value = row.iter().find_map(|(key, value)| {
                matches!(key, ArrayKey::Integer(1)).then(|| value.clone())
            });
            let (Some(key), Some(value)) = (key, value) else {
                continue;
            };
            let key = match key.dereference() {
                Value::Integer(index) => ArrayKey::Integer(index),
                Value::Number(index) if index.is_finite() && index.fract() == 0.0 => {
                    ArrayKey::Integer(index as i64)
                }
                Value::String(key) => ArrayKey::String(key),
                _ => ArrayKey::String(key.show()),
            };
            pairs.set(key, value);
        }
        let count = rows.len();
        object
            .properties
            .insert("\0cursor".into(), Value::Integer((cursor + count) as i64));
        return Ok(Value::Array(pairs));
    }
    if mode == 11 {
        drop(object);
        let entries = rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                fetch_named(&statement, row).map(|value| (ArrayKey::Integer(index as i64), value))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let count = entries.len();
        statement
            .borrow_mut()
            .properties
            .insert("\0cursor".into(), Value::Integer((cursor + count) as i64));
        return Ok(Value::from_array_entries(entries));
    }
    if matches!(flags, 0x10000 | 0x30000) {
        let mut result = crate::PhpArray::new();
        for row in &rows {
            let Value::Array(fields) = row.dereference() else {
                continue;
            };
            let Some(key) = fields.iter().find_map(|(key, value)| {
                matches!(key, ArrayKey::Integer(0)).then(|| value.clone())
            }) else {
                continue;
            };
            let key = match key.dereference() {
                Value::Integer(index) => ArrayKey::Integer(index),
                Value::Number(index) if index.is_finite() && index.fract() == 0.0 => {
                    ArrayKey::Integer(index as i64)
                }
                Value::String(key) => ArrayKey::String(key),
                _ => ArrayKey::String(key.show()),
            };
            let fetched = row_as_fetch_mode(row.clone(), mode, column)?;
            if flags == 0x30000 {
                result.set(key, fetched);
            } else {
                let mut group = match result
                    .iter()
                    .find_map(|(candidate, value)| (candidate == &key).then(|| value.dereference()))
                {
                    Some(Value::Array(group)) => group,
                    _ => crate::PhpArray::new(),
                };
                group.append(fetched)?;
                result.set(key, Value::Array(group));
            }
        }
        let count = rows.len();
        object
            .properties
            .insert("\0cursor".into(), Value::Integer((cursor + count) as i64));
        return Ok(Value::Array(result));
    }
    let entries = rows
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            row_as_fetch_mode(row, mode, column)
                .map(|value| (ArrayKey::Integer(index as i64), value))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let count = entries.len();
    object
        .properties
        .insert("\0cursor".into(), Value::Integer((cursor + count) as i64));
    Ok(Value::from_array_entries(entries))
}

fn set_statement_fetch_mode(statement: &Value, mode: Value, mode_arguments: &[Value]) {
    if let Value::Object(object) = statement {
        let mut object = object.borrow_mut();
        let mode_number = mode.number() as i64;
        match mode_number & 0xffff {
            8 => {
                if let Some(class_name) = mode_arguments.first() {
                    object.properties.insert(
                        "\0fetch_class_name".into(),
                        Value::String(class_name.show()),
                    );
                    object.properties.insert(
                        "\0fetch_class_args".into(),
                        mode_arguments
                            .get(1)
                            .cloned()
                            .unwrap_or_else(|| Value::from_array_entries(Vec::new())),
                    );
                }
                object.properties.insert(
                    "\0fetch_props_late".into(),
                    Value::Bool(mode_number & 0x100000 != 0),
                );
            }
            9 => {
                if let Some(Value::Object(target)) = mode_arguments.first() {
                    object
                        .properties
                        .insert("\0fetch_into".into(), Value::Object(target.clone()));
                }
            }
            10 => {
                if let Some(callback) = mode_arguments.first() {
                    object
                        .properties
                        .insert("\0fetch_func".into(), callback.clone());
                }
            }
            _ => {}
        }
        object.properties.insert("\0fetch_mode".into(), mode);
    }
}

fn validate_fetch_setup(mode: &Value, arguments: &[Value], method: &str) -> Result<(), String> {
    let mode_number = mode.number() as i64;
    if !valid_fetch_mode(mode_number) {
        return Err(format!("{method}(): invalid fetch mode {mode_number}"));
    }
    match mode_number & 0xffff {
        8 if !matches!(arguments.first(), Some(Value::String(_))) => {
            Err(format!("{method}(): FETCH_CLASS requires a class name"))
        }
        9 if !matches!(arguments.first(), Some(Value::Object(_))) => {
            Err(format!("{method}(): FETCH_INTO requires an object"))
        }
        10 if arguments.is_empty() => Err(format!("{method}(): FETCH_FUNC requires a callable")),
        _ => Ok(()),
    }
}

fn valid_fetch_mode(mode: i64) -> bool {
    let base = mode & 0xffff;
    let flags = mode & !0xffff;
    matches!(base, 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12)
        && (base != 0 || flags == 0)
        && ((flags & 0x100000) == 0 || base == 8)
        && (flags & !(0x10000 | 0x30000 | 0x100000)) == 0
}

fn fetch_into(statement: &Rc<RefCell<PhpObject>>, row: Value) -> Result<Value, String> {
    let target = statement
        .borrow()
        .properties
        .get("\0fetch_into")
        .cloned()
        .ok_or("PDO::FETCH_INTO requires an object passed to setFetchMode()")?;
    let Value::Object(target) = target else {
        return Err("PDO::FETCH_INTO target is not an object".into());
    };
    let Value::Array(row) = row.dereference() else {
        return Ok(Value::Object(target));
    };
    let mut target = target.borrow_mut();
    for (key, value) in row.iter() {
        if let ArrayKey::String(name) = key {
            if !target.properties.contains_key(name) {
                target.property_order.push(name.clone());
            }
            target.properties.insert(name.clone(), value.clone());
        }
    }
    drop(target);
    let target = statement
        .borrow()
        .properties
        .get("\0fetch_into")
        .cloned()
        .unwrap_or(Value::Null);
    Ok(target)
}

fn fetch_bound(object: &Rc<RefCell<PhpObject>>, row: Value) -> Result<Value, String> {
    let Value::Array(row) = row.dereference() else {
        return Ok(Value::Bool(true));
    };
    let object = object.borrow();
    let Some(Value::Array(bindings)) = object
        .properties
        .get("\0bound_columns")
        .map(Value::dereference)
    else {
        return Err("PDO::FETCH_BOUND requires PDOStatement::bindColumn() first".into());
    };
    let types = object
        .properties
        .get("\0bound_column_types")
        .and_then(|value| match value.dereference() {
            Value::Array(types) => Some(types),
            _ => None,
        });
    let max_lengths = object
        .properties
        .get("\0bound_column_max_lengths")
        .and_then(|value| match value.dereference() {
            Value::Array(lengths) => Some(lengths),
            _ => None,
        });
    for (column, target) in bindings.iter() {
        let Value::Reference(reference) = target else {
            continue;
        };
        let value = match column {
            ArrayKey::Integer(index) => row.iter().find(|(key, _)| matches!(key, ArrayKey::Integer(candidate) if candidate == index)),
            ArrayKey::String(name) => row.iter().find(|(key, _)| matches!(key, ArrayKey::String(candidate) if candidate.eq_ignore_ascii_case(name))),
        }.map(|(_, value)| value.clone());
        if let Some(value) = value {
            let kind = types
                .as_ref()
                .and_then(|types| {
                    types.iter().find_map(|(candidate, value)| {
                        (candidate == column).then(|| value.number() as i64)
                    })
                })
                .unwrap_or(2);
            let max_length = max_lengths
                .as_ref()
                .and_then(|lengths| {
                    lengths.iter().find_map(|(candidate, value)| {
                        (candidate == column).then(|| value.number().max(0.0) as usize)
                    })
                })
                .unwrap_or(0);
            *reference.borrow_mut() = cast_bound_column(value, kind, max_length);
        }
    }
    Ok(Value::Bool(true))
}

fn row_as_fetch_mode(row: Value, mode: i64, column: usize) -> Result<Value, String> {
    let Value::Array(row) = row.dereference() else {
        return Ok(row);
    };
    let fields = row.iter().cloned().collect::<Vec<_>>();
    match mode {
        2 => {
            let mut result = crate::PhpArray::new();
            for (key, value) in fields {
                if matches!(key, ArrayKey::String(_)) {
                    result.set(key, value);
                }
            }
            Ok(Value::Array(result))
        }
        3 => {
            let mut result = crate::PhpArray::new();
            for (key, value) in fields {
                if matches!(key, ArrayKey::Integer(_)) {
                    result.set(key, value);
                }
            }
            Ok(Value::Array(result))
        }
        4 => {
            let mut result = crate::PhpArray::new();
            for (key, value) in fields {
                result.set(key, value);
            }
            Ok(Value::Array(result))
        }
        5 => {
            let properties = fields
                .into_iter()
                .map(|(key, value)| {
                    let name = match key {
                        ArrayKey::Integer(index) => index.to_string(),
                        ArrayKey::String(name) => name,
                    };
                    (name, value)
                })
                .collect();
            Ok(Value::Object(Rc::new(RefCell::new(PhpObject {
                class_name: "stdClass".into(),
                properties,
                property_order: Vec::new(),
                active_property_hooks: Default::default(),
                exception: None,
            }))))
        }
        1 => {
            let properties = fields
                .into_iter()
                .map(|(key, value)| {
                    let name = match key {
                        ArrayKey::Integer(index) => index.to_string(),
                        ArrayKey::String(name) => name,
                    };
                    (name, value)
                })
                .collect();
            Ok(Value::Object(Rc::new(RefCell::new(PhpObject {
                class_name: "PDO::PDORow".into(),
                properties,
                property_order: Vec::new(),
                active_property_hooks: Default::default(),
                exception: None,
            }))))
        }
        11 => {
            let mut grouped = Vec::<(String, Vec<Value>)>::new();
            for (key, value) in fields {
                let ArrayKey::String(name) = key else {
                    continue;
                };
                if let Some((_, values)) = grouped
                    .iter_mut()
                    .find(|(candidate, _)| candidate.as_str() == name.as_str())
                {
                    values.push(value);
                } else {
                    grouped.push((name, vec![value]));
                }
            }
            let entries = grouped
                .into_iter()
                .map(|(name, mut values)| {
                    let value = if values.len() == 1 {
                        values.pop().expect("single value")
                    } else {
                        Value::from_array_entries(
                            values
                                .into_iter()
                                .enumerate()
                                .map(|(index, value)| (ArrayKey::Integer(index as i64), value))
                                .collect(),
                        )
                    };
                    (ArrayKey::String(name), value)
                })
                .collect();
            Ok(Value::from_array_entries(entries))
        }
        7 => fields
            .iter()
            .find_map(|(key, value)| match key {
                ArrayKey::Integer(index) if *index == column as i64 => Some(value.clone()),
                _ => None,
            })
            .ok_or_else(|| format!("PDO fetch column {column} does not exist")),
        6 | 8 | 9 | 10 | 12 => Err(format!("PDO fetch mode {mode} is not valid for fetch()")),
        _ => Err(format!("invalid PDO fetch mode {mode}")),
    }
}

fn statement_context(object: &Rc<RefCell<PhpObject>>) -> Result<(String, String, u64), String> {
    let object = object.borrow();
    let name = object
        .properties
        .get("\0database_name")
        .map(Value::show)
        .ok_or("PDOStatement is detached")?;
    let sql = object
        .properties
        .get("\0statement_sql")
        .map(Value::show)
        .ok_or("PDOStatement has no prepared SQL")?;
    let id = object
        .properties
        .get("\0pdo_id")
        .map(Value::number)
        .ok_or("PDOStatement is detached")? as u64;
    Ok((name, sql, id))
}

fn pdo_id(object: &Rc<RefCell<PhpObject>>) -> Result<u64, String> {
    object
        .borrow()
        .properties
        .get("\0pdo_id")
        .map(Value::number)
        .filter(|id| *id >= 0.0)
        .map(|id| id as u64)
        .ok_or("PDO object is detached".into())
}

fn database_name(object: &Rc<RefCell<PhpObject>>) -> Result<String, String> {
    object
        .borrow()
        .properties
        .get("\0database_name")
        .map(Value::show)
        .ok_or("PDO object is detached".into())
}

fn server_version_query(kind: DatabaseKind) -> Option<&'static str> {
    match kind {
        DatabaseKind::Mysql | DatabaseKind::MariaDb => Some("SELECT VERSION()"),
        DatabaseKind::PostgreSql => Some("SELECT version()"),
        DatabaseKind::SqlServer => Some("SELECT @@VERSION"),
        DatabaseKind::Oracle | DatabaseKind::MongoDb => None,
    }
}

fn database_value_string(value: &DbValue) -> String {
    match value {
        DbValue::Null => String::new(),
        DbValue::Bool(value) => {
            if *value {
                "1".into()
            } else {
                "0".into()
            }
        }
        DbValue::Integer(value) => value.to_string(),
        DbValue::Float(value) => value.to_string(),
        DbValue::String(value) => value.clone(),
        DbValue::Bytes(value) => String::from_utf8_lossy(value).into_owned(),
        DbValue::Array(_) | DbValue::Document(_) => String::new(),
    }
}

fn truncate_column_names(columns: &mut [String], max_length: usize) {
    if max_length == 0 {
        return;
    }
    for name in columns {
        *name = name.chars().take(max_length).collect();
    }
}

fn qualify_column_names(
    columns: &mut [String],
    metadata: &[php_db::DbColumnMeta],
    table_names: bool,
    catalog_names: bool,
) {
    if !table_names && !catalog_names {
        return;
    }
    for (index, name) in columns.iter_mut().enumerate() {
        let Some(metadata) = metadata.get(index) else {
            continue;
        };
        let mut prefix = Vec::new();
        if catalog_names {
            if let Some(catalog) = metadata
                .catalog_name
                .as_ref()
                .filter(|name| !name.is_empty())
            {
                prefix.push(catalog.as_str());
            }
        }
        if table_names {
            if let Some(table) = metadata.table_name.as_ref().filter(|name| !name.is_empty()) {
                prefix.push(table.as_str());
            }
        }
        if !prefix.is_empty() {
            *name = format!("{}.{}", prefix.join("."), name);
        }
    }
}

fn required_string(arguments: &[Value], index: usize, error: &str) -> Result<String, String> {
    arguments
        .get(index)
        .map(Value::show)
        .ok_or_else(|| error.to_owned())
}

fn bind_parameters(value: &Value) -> Result<Vec<(ArrayKey, DbValue)>, String> {
    match value.dereference() {
        Value::Array(array) => array
            .iter()
            .map(|(key, value)| Ok((key.clone(), to_db(value)?)))
            .collect(),
        _ => Err("PDOStatement::execute() parameters must be an array".into()),
    }
}

fn bound_parameters(object: &Rc<RefCell<PhpObject>>) -> Result<Vec<(ArrayKey, DbValue)>, String> {
    let object = object.borrow();
    let Some(Value::Array(bound)) = object
        .properties
        .get("\0bound_params")
        .map(Value::dereference)
    else {
        return Ok(Vec::new());
    };
    let types = match object
        .properties
        .get("\0bound_param_types")
        .map(Value::dereference)
    {
        Some(Value::Array(types)) => Some(types),
        _ => None,
    };
    bound
        .iter()
        .map(|(key, value)| {
            let kind = types
                .as_ref()
                .and_then(|types| {
                    types.iter().find_map(|(candidate, value)| {
                        (candidate == key).then(|| value.number() as i64)
                    })
                })
                .unwrap_or(2);
            Ok((key.clone(), typed_parameter(value, kind)?))
        })
        .collect()
}

fn typed_parameter(value: &Value, kind: i64) -> Result<DbValue, String> {
    let value = value.dereference();
    if matches!(value, Value::Null) {
        return Ok(DbValue::Null);
    }
    let raw_kind = kind as u32;
    if raw_kind & 0x8000_0000 != 0 {
        return Err(
            "PDO::PARAM_INPUT_OUTPUT is not supported by the configured database drivers".into(),
        );
    }
    match raw_kind & 0x1f {
        0 => Ok(DbValue::Null),
        1 => Ok(DbValue::Integer(value.number() as i64)),
        2 if raw_kind & !0x6000_001f == 0 => Ok(DbValue::String(value.show())),
        3 => match value {
            Value::String(value) => Ok(DbValue::Bytes(value.into_bytes())),
            Value::Stream(stream) => stream
                .borrow_mut()
                .read(None)
                .map(DbValue::Bytes)
                .map_err(|error| format!("PDO::PARAM_LOB could not read stream: {error}")),
            _ => Err("PDO::PARAM_LOB requires a string or stream resource".into()),
        },
        5 => Ok(DbValue::Bool(value.truthy())),
        _ => Err(format!("unsupported PDO parameter type {kind}")),
    }
}

fn store_bound_type(object: &mut PhpObject, key: ArrayKey, arguments: &[Value]) {
    let kind = arguments
        .get(2)
        .map(|value| value.number() as i64)
        .unwrap_or(2);
    let mut types = match object
        .properties
        .get("\0bound_param_types")
        .map(Value::dereference)
    {
        Some(Value::Array(array)) => array,
        _ => crate::PhpArray::new(),
    };
    types.set(key, Value::Integer(kind));
    object
        .properties
        .insert("\0bound_param_types".into(), Value::Array(types));
}

fn validate_parameter_type(
    arguments: &[Value],
    method: &str,
    default_string_flags: i64,
) -> Result<i64, String> {
    let kind = arguments
        .get(2)
        .map(|value| value.number() as i64)
        .unwrap_or(2 | default_string_flags);
    let raw_kind = kind as u32;
    let base_kind = raw_kind & 0x1f;
    let string_flags = raw_kind & 0x6000_0000;
    if raw_kind & 0x8000_0000 == 0
        && matches!(base_kind, 0 | 1 | 2 | 3 | 5)
        && (string_flags == 0 || base_kind == 2)
        && raw_kind & !0x6000_001f == 0
    {
        Ok(kind)
    } else {
        Err(format!("{method}(): unsupported PDO parameter type {kind}"))
    }
}

fn statement_default_string_flags(object: &Rc<RefCell<PhpObject>>) -> i64 {
    let id = object
        .borrow()
        .properties
        .get("\0pdo_id")
        .map(Value::number)
        .unwrap_or_default() as u64;
    PDO_DEFAULT_STR_PARAMS.with(|modes| modes.borrow().get(&id).copied().unwrap_or(1 << 29))
}

fn bind_value(object: &Rc<RefCell<PhpObject>>, arguments: &[Value]) -> Result<Value, String> {
    let parameter = arguments
        .first()
        .ok_or("PDOStatement::bindValue() expects a parameter")?;
    let value = arguments
        .get(1)
        .ok_or("PDOStatement::bindValue() expects a value")?;
    let kind = validate_parameter_type(
        arguments,
        "PDOStatement::bindValue",
        statement_default_string_flags(object),
    )?;
    let key =
        match parameter.dereference() {
            Value::Integer(index) if index >= 1 => ArrayKey::Integer(index - 1),
            Value::Number(index) if index >= 1.0 && index.fract() == 0.0 => {
                ArrayKey::Integer(index as i64 - 1)
            }
            Value::String(name) if !name.trim_start_matches(':').is_empty() => {
                ArrayKey::String(name.trim_start_matches(':').to_owned())
            }
            _ => return Err(
                "PDOStatement::bindValue() parameter must be a positive index or named placeholder"
                    .into(),
            ),
        };
    let mut object = object.borrow_mut();
    let mut bound = match object
        .properties
        .get("\0bound_params")
        .map(Value::dereference)
    {
        Some(Value::Array(array)) => array,
        _ => crate::PhpArray::new(),
    };
    bound.set(key.clone(), value.clone());
    object
        .properties
        .insert("\0bound_params".into(), Value::Array(bound));
    let mut type_arguments = arguments.to_vec();
    if type_arguments.get(2).is_none() {
        type_arguments.push(Value::Integer(kind));
    }
    store_bound_type(&mut object, key, &type_arguments);
    Ok(Value::Bool(true))
}

fn bind_param(object: &Rc<RefCell<PhpObject>>, arguments: &[Value]) -> Result<Value, String> {
    let parameter = arguments
        .first()
        .ok_or("PDOStatement::bindParam() expects a parameter")?;
    let value = arguments
        .get(1)
        .ok_or("PDOStatement::bindParam() expects a variable")?;
    let kind = validate_parameter_type(
        arguments,
        "PDOStatement::bindParam",
        statement_default_string_flags(object),
    )?;
    let Value::Reference(reference) = value else {
        return Err(
            "PDOStatement::bindParam(): argument #2 ($var) must be passed by reference".into(),
        );
    };
    let key =
        match parameter.dereference() {
            Value::Integer(index) if index >= 1 => ArrayKey::Integer(index - 1),
            Value::Number(index) if index >= 1.0 && index.fract() == 0.0 => {
                ArrayKey::Integer(index as i64 - 1)
            }
            Value::String(name) if !name.trim_start_matches(':').is_empty() => {
                ArrayKey::String(name.trim_start_matches(':').to_owned())
            }
            _ => return Err(
                "PDOStatement::bindParam() parameter must be a positive index or named placeholder"
                    .into(),
            ),
        };
    let mut object = object.borrow_mut();
    let mut bound = match object
        .properties
        .get("\0bound_params")
        .map(Value::dereference)
    {
        Some(Value::Array(array)) => array,
        _ => crate::PhpArray::new(),
    };
    bound.set_reference(key, reference.clone());
    object
        .properties
        .insert("\0bound_params".into(), Value::Array(bound));
    let key = match parameter.dereference() {
        Value::Integer(index) => ArrayKey::Integer(index - 1),
        Value::Number(index) => ArrayKey::Integer(index as i64 - 1),
        Value::String(name) => ArrayKey::String(name.trim_start_matches(':').to_owned()),
        _ => unreachable!(),
    };
    let mut type_arguments = arguments.to_vec();
    if type_arguments.get(2).is_none() {
        type_arguments.push(Value::Integer(kind));
    }
    store_bound_type(&mut object, key, &type_arguments);
    Ok(Value::Bool(true))
}

fn bind_column(object: &Rc<RefCell<PhpObject>>, arguments: &[Value]) -> Result<Value, String> {
    let column = arguments
        .first()
        .ok_or("PDOStatement::bindColumn() expects a column")?;
    let target = arguments
        .get(1)
        .ok_or("PDOStatement::bindColumn() expects a variable")?;
    let kind = validate_parameter_type(arguments, "PDOStatement::bindColumn", 0)?;
    let max_length = arguments.get(3).map(|value| value.number()).unwrap_or(0.0);
    if !max_length.is_finite()
        || max_length < 0.0
        || max_length.fract() != 0.0
        || max_length > i64::MAX as f64
    {
        return Err("PDOStatement::bindColumn(): max_length must be a non-negative integer".into());
    }
    if arguments
        .get(4)
        .is_some_and(|value| !matches!(value.dereference(), Value::Null))
    {
        return Err("PDOStatement::bindColumn(): driver-specific options are not supported".into());
    }
    let Value::Reference(reference) = target else {
        return Err(
            "PDOStatement::bindColumn(): argument #2 ($var) must be passed by reference".into(),
        );
    };
    let key = match column.dereference() {
        Value::Integer(index) if index >= 1 => ArrayKey::Integer(index - 1),
        Value::Number(index) if index >= 1.0 && index.fract() == 0.0 => {
            ArrayKey::Integer(index as i64 - 1)
        }
        Value::String(name) if !name.is_empty() => ArrayKey::String(name),
        _ => {
            return Err(
                "PDOStatement::bindColumn() column must be a positive index or column name".into(),
            )
        }
    };
    let mut object = object.borrow_mut();
    let mut bindings = match object
        .properties
        .get("\0bound_columns")
        .map(Value::dereference)
    {
        Some(Value::Array(array)) => array,
        _ => crate::PhpArray::new(),
    };
    bindings.set_reference(key.clone(), reference.clone());
    object
        .properties
        .insert("\0bound_columns".into(), Value::Array(bindings));
    let mut types = match object
        .properties
        .get("\0bound_column_types")
        .map(Value::dereference)
    {
        Some(Value::Array(array)) => array,
        _ => crate::PhpArray::new(),
    };
    types.set(key.clone(), Value::Integer(kind));
    object
        .properties
        .insert("\0bound_column_types".into(), Value::Array(types));
    let mut max_lengths = match object
        .properties
        .get("\0bound_column_max_lengths")
        .map(Value::dereference)
    {
        Some(Value::Array(array)) => array,
        _ => crate::PhpArray::new(),
    };
    max_lengths.set(key, Value::Integer(max_length as i64));
    object.properties.insert(
        "\0bound_column_max_lengths".into(),
        Value::Array(max_lengths),
    );
    Ok(Value::Bool(true))
}

fn cast_bound_column(value: Value, kind: i64, max_length: usize) -> Value {
    let value = value.dereference();
    let cast = match kind {
        0 => Value::Null,
        1 => Value::Integer(value.number() as i64),
        2 | 3 => Value::String(value.show()),
        5 => Value::Bool(value.truthy()),
        _ => value.clone(),
    };
    if max_length == 0 {
        return cast;
    }
    match cast {
        Value::String(value) => {
            let end = value
                .char_indices()
                .map(|(index, character)| index + character.len_utf8())
                .take_while(|end| *end <= max_length)
                .last()
                .unwrap_or(0);
            Value::String(value[..end].to_owned())
        }
        value => value,
    }
}

#[derive(Clone, Copy)]
enum PlaceholderStyle {
    Numbered,
    InlinePostgres,
    QuestionMySql,
    QuestionSqlServer,
    QuestionOracle,
}

fn rewrite_placeholders(
    sql: &str,
    parameters: Vec<(ArrayKey, DbValue)>,
    style: PlaceholderStyle,
) -> Result<(String, Vec<DbValue>), String> {
    let mut output = String::with_capacity(sql.len() + 8);
    let mut bound = Vec::new();
    let mut positional = 0usize;
    let mut named = HashMap::<String, usize>::new();
    let mut cursor = 0usize;
    let bytes = sql.as_bytes();
    let mut state = SqlScanState::Normal;
    while cursor < bytes.len() {
        match state.clone() {
            SqlScanState::DollarQuote(delimiter) => {
                if sql[cursor..].starts_with(&delimiter) {
                    output.push_str(&delimiter);
                    cursor += delimiter.len();
                    state = SqlScanState::Normal;
                } else {
                    cursor += push_sql_character(sql, cursor, &mut output);
                }
            }
            SqlScanState::SingleQuote => {
                if matches!(style, PlaceholderStyle::QuestionMySql)
                    && bytes[cursor] == b'\\'
                    && cursor + 1 < bytes.len()
                {
                    output.push('\\');
                    cursor += 1;
                    cursor += push_sql_character(sql, cursor, &mut output);
                } else if bytes[cursor] == b'\'' {
                    output.push('\'');
                    if bytes.get(cursor + 1) == Some(&b'\'') {
                        output.push('\'');
                        cursor += 2;
                        continue;
                    }
                    state = SqlScanState::Normal;
                    cursor += 1;
                } else {
                    cursor += push_sql_character(sql, cursor, &mut output);
                }
            }
            SqlScanState::EscapeSingleQuote => {
                if bytes[cursor] == b'\\' && cursor + 1 < bytes.len() {
                    output.push('\\');
                    cursor += 1;
                    cursor += push_sql_character(sql, cursor, &mut output);
                } else if bytes[cursor] == b'\'' {
                    output.push('\'');
                    if bytes.get(cursor + 1) == Some(&b'\'') {
                        output.push('\'');
                        cursor += 2;
                    } else {
                        cursor += 1;
                        state = SqlScanState::Normal;
                    }
                } else {
                    cursor += push_sql_character(sql, cursor, &mut output);
                }
            }
            SqlScanState::DoubleQuote => {
                if matches!(style, PlaceholderStyle::QuestionMySql)
                    && bytes[cursor] == b'\\'
                    && cursor + 1 < bytes.len()
                {
                    output.push('\\');
                    cursor += 1;
                    cursor += push_sql_character(sql, cursor, &mut output);
                } else if bytes[cursor] == b'"' {
                    output.push('"');
                    if bytes.get(cursor + 1) == Some(&b'"') {
                        output.push('"');
                        cursor += 2;
                        continue;
                    }
                    state = SqlScanState::Normal;
                    cursor += 1;
                } else {
                    cursor += push_sql_character(sql, cursor, &mut output);
                }
            }
            SqlScanState::BacktickQuote => {
                if bytes[cursor] == b'`' {
                    output.push('`');
                    if bytes.get(cursor + 1) == Some(&b'`') {
                        output.push('`');
                        cursor += 2;
                        continue;
                    }
                    state = SqlScanState::Normal;
                    cursor += 1;
                } else {
                    cursor += push_sql_character(sql, cursor, &mut output);
                }
            }
            SqlScanState::BracketQuote => {
                if bytes[cursor] == b']' {
                    output.push(']');
                    if bytes.get(cursor + 1) == Some(&b']') {
                        output.push(']');
                        cursor += 2;
                        continue;
                    }
                    state = SqlScanState::Normal;
                    cursor += 1;
                } else {
                    cursor += push_sql_character(sql, cursor, &mut output);
                }
            }
            SqlScanState::LineComment | SqlScanState::HashComment => {
                let newline = bytes[cursor] == b'\n';
                cursor += push_sql_character(sql, cursor, &mut output);
                if newline {
                    state = SqlScanState::Normal;
                }
            }
            SqlScanState::BlockComment(depth) => {
                if bytes.get(cursor..cursor + 2) == Some(b"/*") {
                    output.push_str("/*");
                    cursor += 2;
                    state = SqlScanState::BlockComment(depth + 1);
                } else if bytes.get(cursor..cursor + 2) == Some(b"*/") {
                    output.push_str("*/");
                    cursor += 2;
                    state = if depth == 1 {
                        SqlScanState::Normal
                    } else {
                        SqlScanState::BlockComment(depth - 1)
                    };
                } else {
                    cursor += push_sql_character(sql, cursor, &mut output);
                }
            }
            SqlScanState::Normal => {
                if bytes.get(cursor..cursor + 2) == Some(b"--")
                    && (!matches!(style, PlaceholderStyle::QuestionMySql)
                        || bytes
                            .get(cursor + 2)
                            .is_none_or(|byte| byte.is_ascii_whitespace() || *byte < 0x20))
                {
                    output.push_str("--");
                    cursor += 2;
                    state = SqlScanState::LineComment;
                } else if bytes[cursor] == b'#' && matches!(style, PlaceholderStyle::QuestionMySql)
                {
                    output.push('#');
                    cursor += 1;
                    state = SqlScanState::HashComment;
                } else if bytes.get(cursor..cursor + 2) == Some(b"/*") {
                    output.push_str("/*");
                    cursor += 2;
                    state = SqlScanState::BlockComment(1);
                } else if bytes[cursor] == b'\'' {
                    output.push('\'');
                    cursor += 1;
                    state = SqlScanState::SingleQuote;
                } else if bytes[cursor] == b'"' {
                    output.push('"');
                    cursor += 1;
                    state = SqlScanState::DoubleQuote;
                } else if bytes[cursor] == b'`' && matches!(style, PlaceholderStyle::QuestionMySql)
                {
                    output.push('`');
                    cursor += 1;
                    state = SqlScanState::BacktickQuote;
                } else if bytes[cursor] == b'['
                    && matches!(style, PlaceholderStyle::QuestionSqlServer)
                {
                    output.push('[');
                    cursor += 1;
                    state = SqlScanState::BracketQuote;
                } else if matches!(bytes[cursor], b'e' | b'E')
                    && bytes.get(cursor + 1) == Some(&b'\'')
                    && (cursor == 0
                        || !(bytes[cursor - 1].is_ascii_alphanumeric()
                            || bytes[cursor - 1] == b'_'))
                {
                    output.push(bytes[cursor] as char);
                    output.push('\'');
                    cursor += 2;
                    state = SqlScanState::EscapeSingleQuote;
                } else if bytes[cursor] == b'$' && bytes.get(cursor + 1) == Some(&b'$') {
                    output.push_str("$$");
                    cursor += 2;
                    state = SqlScanState::DollarQuote("$$".into());
                } else if bytes[cursor] == b'$'
                    && bytes
                        .get(cursor + 1)
                        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
                {
                    let end = (cursor + 2..bytes.len()).find(|index| {
                        bytes[*index] == b'$'
                            && bytes[cursor + 1..*index]
                                .iter()
                                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                    });
                    if let Some(end) = end {
                        let delimiter = &sql[cursor..=end];
                        output.push_str(delimiter);
                        state = SqlScanState::DollarQuote(delimiter.to_owned());
                        cursor = end + 1;
                    } else {
                        cursor += push_sql_character(sql, cursor, &mut output);
                    }
                } else if bytes[cursor] == b'?' {
                    let value = parameters.iter().find(|(key, _)| matches!(key, ArrayKey::Integer(index) if *index == positional as i64))
                        .or_else(|| parameters.iter().filter(|(key, _)| matches!(key, ArrayKey::Integer(_))).nth(positional))
                        .map(|(_, value)| value.clone()).ok_or("PDOStatement::execute(): missing positional parameter")?;
                    positional += 1;
                    match style {
                        PlaceholderStyle::Numbered => {
                            bound.push(value);
                            output.push_str(&format!("${}", bound.len()));
                        }
                        PlaceholderStyle::InlinePostgres => {
                            output.push_str(&postgres_literal(&value)?)
                        }
                        _ => {
                            bound.push(value);
                            output.push('?');
                        }
                    }
                    cursor += 1;
                } else if bytes[cursor] == b':'
                    && bytes
                        .get(cursor + 1)
                        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
                    && bytes.get(cursor.wrapping_sub(1)) != Some(&b':')
                {
                    let start = cursor + 1;
                    let mut end = start + 1;
                    while bytes
                        .get(end)
                        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                    {
                        end += 1;
                    }
                    let key = &sql[start..end];
                    if matches!(style, PlaceholderStyle::InlinePostgres) {
                        let value = parameters.iter().find(|(candidate, _)| matches!(candidate, ArrayKey::String(candidate) if candidate.trim_start_matches(':') == key))
                            .map(|(_, value)| value).ok_or_else(|| format!("PDOStatement::execute(): missing named parameter :{key}"))?;
                        output.push_str(&postgres_literal(value)?);
                        named.insert(key.to_owned(), 0);
                    } else {
                        let index = if matches!(style, PlaceholderStyle::Numbered)
                            && named.contains_key(key)
                        {
                            named[key]
                        } else {
                            let parameter = parameters.iter().find(|(candidate, _)| matches!(candidate, ArrayKey::String(candidate) if candidate.trim_start_matches(':') == key))
                                .map(|(_, value)| value.clone()).ok_or_else(|| format!("PDOStatement::execute(): missing named parameter :{key}"))?;
                            bound.push(parameter);
                            let index = bound.len();
                            named.insert(key.to_owned(), index);
                            index
                        };
                        match style {
                            PlaceholderStyle::Numbered => output.push_str(&format!("${index}")),
                            _ => output.push('?'),
                        }
                    }
                    cursor = end;
                } else {
                    let character = sql[cursor..]
                        .chars()
                        .next()
                        .expect("cursor is a character boundary");
                    output.push(character);
                    cursor += character.len_utf8();
                }
            }
        }
    }
    if parameters
        .iter()
        .filter(|(key, _)| matches!(key, ArrayKey::Integer(_)))
        .count()
        != positional
    {
        return Err(
            "PDOStatement::execute(): parameter count does not match placeholder count".into(),
        );
    }
    if parameters.iter().any(|(key, _)| match key {
        ArrayKey::String(name) => !named.contains_key(name.trim_start_matches(':')),
        ArrayKey::Integer(_) => false,
    }) {
        return Err(
            "PDOStatement::execute(): parameter count does not match placeholder count".into(),
        );
    }
    Ok((output, bound))
}

fn postgres_literal(value: &DbValue) -> Result<String, String> {
    Ok(match value {
        DbValue::Null => "NULL".into(),
        DbValue::Bool(value) => {
            if *value {
                "TRUE".into()
            } else {
                "FALSE".into()
            }
        }
        DbValue::Integer(value) => value.to_string(),
        DbValue::Float(value) if value.is_finite() => value.to_string(),
        DbValue::Float(_) => return Err("non-finite floats cannot be bound to PostgreSQL".into()),
        DbValue::String(value) => {
            if value.contains('\0') {
                return Err("PostgreSQL text values cannot contain NUL bytes".into());
            }
            let mut escaped = String::with_capacity(value.len() + 3);
            for character in value.chars() {
                match character {
                    '\\' => escaped.push_str("\\\\"),
                    '\'' => escaped.push_str("\\'"),
                    '\n' => escaped.push_str("\\n"),
                    '\r' => escaped.push_str("\\r"),
                    '\t' => escaped.push_str("\\t"),
                    character if character.is_control() && (character as u32) <= 0xff => {
                        escaped.push_str(&format!("\\{:03o}", character as u32))
                    }
                    character if character.is_control() && (character as u32) <= 0xffff => {
                        escaped.push_str(&format!("\\u{:04X}", character as u32))
                    }
                    character if character.is_control() => {
                        escaped.push_str(&format!("\\U{:08X}", character as u32))
                    }
                    character => escaped.push(character),
                }
            }
            format!("E'{escaped}'")
        }
        DbValue::Bytes(value) => format!(
            "decode('{}', 'hex')",
            value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
        DbValue::Array(_) | DbValue::Document(_) => {
            return Err("composite parameters must be encoded by the caller".into())
        }
    })
}

#[derive(Clone)]
enum SqlScanState {
    Normal,
    SingleQuote,
    EscapeSingleQuote,
    DoubleQuote,
    BacktickQuote,
    BracketQuote,
    LineComment,
    HashComment,
    BlockComment(usize),
    DollarQuote(String),
}

fn push_sql_character(sql: &str, cursor: usize, output: &mut String) -> usize {
    let character = sql[cursor..]
        .chars()
        .next()
        .expect("cursor is a character boundary");
    output.push(character);
    character.len_utf8()
}

pub(crate) fn to_db(value: &Value) -> Result<DbValue, String> {
    Ok(match value.dereference() {
        Value::Null => DbValue::Null,
        Value::Bool(value) => DbValue::Bool(value),
        Value::Integer(value) => DbValue::Integer(value),
        Value::Number(value) => DbValue::Float(value),
        Value::String(value) => DbValue::String(value),
        Value::Array(array) => {
            let mut list = Vec::new();
            let mut document = Vec::new();
            let mut is_list = true;
            for (key, value) in array.iter() {
                let converted = to_db(value)?;
                match key {
                    ArrayKey::Integer(_) => list.push(converted),
                    ArrayKey::String(key) => {
                        is_list = false;
                        document.push((key.clone(), converted));
                    }
                }
            }
            if is_list {
                DbValue::Array(list)
            } else {
                DbValue::Document(document)
            }
        }
        Value::Object(_) | Value::Closure(_) | Value::Generator(_) | Value::Stream(_) => {
            return Err("PDO parameters must be scalar or array values".into())
        }
        Value::Reference(_) => unreachable!(),
    })
}

pub(crate) fn to_php(value: DbValue) -> Value {
    match value {
        DbValue::Null => Value::Null,
        DbValue::Bool(value) => Value::Bool(value),
        DbValue::Integer(value) => Value::Integer(value),
        DbValue::Float(value) => Value::Number(value),
        DbValue::String(value) => Value::String(value),
        DbValue::Bytes(value) => Value::String(String::from_utf8_lossy(&value).into_owned()),
        DbValue::Array(values) => Value::from_array_entries(
            values
                .into_iter()
                .enumerate()
                .map(|(index, value)| (ArrayKey::Integer(index as i64), to_php(value)))
                .collect(),
        ),
        DbValue::Document(values) => Value::from_array_entries(
            values
                .into_iter()
                .map(|(key, value)| (ArrayKey::String(key), to_php(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
#[path = "../../../tests/crates/php-runtime/runtime/pdo.rs"]
mod tests;
