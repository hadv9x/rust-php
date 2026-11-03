//! PHP function bridge for the MongoDB document driver.

use php_db::{DatabaseKind, DbCommand, DbResult, DbValue, MongoAction};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::{database_registry, pdo::{to_db, to_php}, ArrayKey, PhpObject, Value};

thread_local! { static MONGO_SESSIONS: RefCell<HashMap<String, Box<dyn php_db::DatabaseSession>>> = RefCell::new(HashMap::new()); }

pub(crate) fn clear_sessions() { MONGO_SESSIONS.with(|sessions| sessions.borrow_mut().clear()); }

pub(crate) fn method_exists(class: &str, method: &str) -> bool {
    match (class.to_ascii_lowercase().as_str(), method.to_ascii_lowercase().as_str()) {
        ("mongodb\\client", "selectdatabase" | "selectcollection") => true,
        ("mongodb\\database", "selectcollection") => true,
        ("mongodb\\collection", "find" | "findone" | "aggregate" | "countdocuments" | "estimateddocumentcount" | "insertone" | "insertmany" | "replaceone" | "updateone" | "updatemany" | "deleteone" | "deletemany") => true,
        ("mongodb\\cursor", "toarray") => true,
        ("mongodb\\insertresult", "getinsertedcount") => true,
        ("mongodb\\insertmanyresult", "getinsertedcount") => true,
        ("mongodb\\updateresult", "getmatchedcount" | "getmodifiedcount") => true,
        ("mongodb\\deleteresult", "getdeletedcount") => true,
        _ => false,
    }
}

pub(crate) fn construct(class: &str, arguments: &[Value]) -> Result<Value, String> {
    if class != "mongodb\\client" { return Err(format!("unknown built-in MongoDB class {class}")); }
    let uri = string_arg(arguments, 0, "MongoDB URI")?;
    let (connection, driver) = database_registry().resolve_dsn(&uri, None, None)?;
    if driver.kind() != DatabaseKind::MongoDb { return Err("MongoDB\\Client requires a configured MongoDB DSN".into()); }
    Ok(make_object(class, [("connection", Value::String(connection))]))
}

pub(crate) fn call_method(object: &Rc<std::cell::RefCell<PhpObject>>, method: &str, arguments: &[Value]) -> Result<Value, String> {
    let (class, connection, database, collection) = {
        let object = object.borrow();
        let get = |key: &str| object.properties.get(key).map(Value::show).unwrap_or_default();
        (object.class_name.to_ascii_lowercase(), get("connection"), get("database"), get("collection"))
    };
    match (class.as_str(), method.to_ascii_lowercase().as_str()) {
        ("mongodb\\client", "selectdatabase") => Ok(make_object("mongodb\\database", [("connection", Value::String(connection)), ("database", Value::String(string_arg(arguments, 0, "database name")?))])),
        ("mongodb\\client", "selectcollection") => Ok(make_object("mongodb\\collection", [("connection", Value::String(connection)), ("database", Value::String(string_arg(arguments, 0, "database name")?)), ("collection", Value::String(string_arg(arguments, 1, "collection name")?))])),
        ("mongodb\\database", "selectcollection") => Ok(make_object("mongodb\\collection", [("connection", Value::String(connection)), ("database", Value::String(database)), ("collection", Value::String(string_arg(arguments, 0, "collection name")?))])),
        ("mongodb\\collection", "find" | "findone") => {
            let limit = if method.eq_ignore_ascii_case("findone") { Some(1) } else { option_limit(arguments.get(1)) };
            let rows = execute_object_query(&connection, &database, &collection, MongoAction::Find {
                filter: document_arg(arguments.first(), "filter")?,
                limit,
                options: document_arg(arguments.get(1), "options")?,
            })?;
            if method.eq_ignore_ascii_case("findone") {
                let first = match rows.dereference() {
                    Value::Array(entries) => entries.first().map(|(_, value)| value.clone()).unwrap_or(Value::Null),
                    _ => Value::Null,
                };
                Ok(first)
            }
            else { Ok(make_object("mongodb\\cursor", [("documents", rows)])) }
        }
        ("mongodb\\collection", "aggregate") => {
            let pipeline = pipeline_arg(arguments.first())?;
            let rows = execute_object_query(&connection, &database, &collection, MongoAction::Aggregate {
                pipeline, options: document_arg(arguments.get(1), "options")?,
            })?;
            Ok(make_object("mongodb\\cursor", [("documents", rows)]))
        }
        ("mongodb\\collection", "countdocuments") => {
            let result = execute_object_write(&connection, &database, &collection, MongoAction::CountDocuments {
                filter: document_arg(arguments.first(), "filter")?, estimated: false,
                options: document_arg(arguments.get(1), "options")?,
            })?;
            Ok(Value::Integer(result.affected_rows as i64))
        }
        ("mongodb\\collection", "estimateddocumentcount") => {
            let result = execute_object_write(&connection, &database, &collection, MongoAction::CountDocuments {
                filter: DbValue::Document(Vec::new()), estimated: true,
                options: document_arg(arguments.first(), "options")?,
            })?;
            Ok(Value::Integer(result.affected_rows as i64))
        }
        ("mongodb\\collection", "insertone") => {
            let result = execute_object_write(&connection, &database, &collection, MongoAction::InsertOne {
                document: document_arg(arguments.first(), "document")?, options: document_arg(arguments.get(1), "options")?,
            })?;
            Ok(make_object("mongodb\\insertresult", [("count", Value::Integer(result.affected_rows as i64))]))
        }
        ("mongodb\\collection", "insertmany") => {
            let documents = documents_arg(arguments.first())?;
            let result = execute_object_write(&connection, &database, &collection, MongoAction::InsertMany {
                documents, options: document_arg(arguments.get(1), "options")?,
            })?;
            Ok(make_object("mongodb\\insertmanyresult", [("count", Value::Integer(result.affected_rows as i64))]))
        }
        ("mongodb\\collection", "updateone" | "updatemany") => {
            let action = if method.eq_ignore_ascii_case("updateone") { MongoAction::UpdateOne {
                filter: document_arg(arguments.first(), "filter")?, update: document_arg(arguments.get(1), "update")?, options: document_arg(arguments.get(2), "options")?,
            } }
                else { MongoAction::UpdateMany {
                    filter: document_arg(arguments.first(), "filter")?, update: document_arg(arguments.get(1), "update")?, options: document_arg(arguments.get(2), "options")?,
                } };
            let result = execute_object_write(&connection, &database, &collection, action)?;
            Ok(make_object("mongodb\\updateresult", [
                ("count", Value::Integer(result.modified_rows.unwrap_or(result.affected_rows) as i64)),
                ("matched_count", Value::Integer(result.matched_rows.unwrap_or(result.affected_rows) as i64)),
            ]))
        }
        ("mongodb\\collection", "replaceone") => {
            let result = execute_object_write(&connection, &database, &collection, MongoAction::ReplaceOne {
                filter: document_arg(arguments.first(), "filter")?, replacement: document_arg(arguments.get(1), "replacement")?,
                options: document_arg(arguments.get(2), "options")?,
            })?;
            Ok(make_object("mongodb\\updateresult", [
                ("count", Value::Integer(result.modified_rows.unwrap_or(result.affected_rows) as i64)),
                ("matched_count", Value::Integer(result.matched_rows.unwrap_or(result.affected_rows) as i64)),
            ]))
        }
        ("mongodb\\collection", "deleteone" | "deletemany") => {
            let action = if method.eq_ignore_ascii_case("deleteone") { MongoAction::DeleteOne {
                filter: document_arg(arguments.first(), "filter")?, options: document_arg(arguments.get(1), "options")?,
            } }
                else { MongoAction::DeleteMany {
                    filter: document_arg(arguments.first(), "filter")?, options: document_arg(arguments.get(1), "options")?,
                } };
            let result = execute_object_write(&connection, &database, &collection, action)?;
            Ok(make_object("mongodb\\deleteresult", [("count", Value::Integer(result.affected_rows as i64))]))
        }
        ("mongodb\\cursor", "toarray") => Ok(object.borrow().properties.get("documents").cloned().unwrap_or_else(|| Value::from_array_entries(Vec::new()))),
        ("mongodb\\insertresult", "getinsertedcount") => Ok(object.borrow().properties.get("count").cloned().unwrap_or(Value::Integer(0))),
        ("mongodb\\insertmanyresult", "getinsertedcount") => Ok(object.borrow().properties.get("count").cloned().unwrap_or(Value::Integer(0))),
        ("mongodb\\updateresult", "getmatchedcount") => Ok(object.borrow().properties.get("matched_count").cloned().unwrap_or(Value::Integer(0))),
        ("mongodb\\updateresult", "getmodifiedcount") => Ok(object.borrow().properties.get("count").cloned().unwrap_or(Value::Integer(0))),
        ("mongodb\\deleteresult", "getdeletedcount") => Ok(object.borrow().properties.get("count").cloned().unwrap_or(Value::Integer(0))),
        _ => Err(format!("Call to undefined method {}::{method}()", class)),
    }
}

fn make_object<const N: usize>(class: &str, properties: [(&str, Value); N]) -> Value {
    Value::Object(Rc::new(std::cell::RefCell::new(PhpObject { class_name: class.into(), property_order: properties.iter().map(|(key, _)| (*key).into()).collect(), properties: properties.into_iter().map(|(key, value)| (key.into(), value)).collect(), active_property_hooks: Default::default(), exception: None })))
}

fn option_limit(value: Option<&Value>) -> Option<u64> {
    let Some(Value::Array(options)) = value.map(Value::dereference) else { return None; };
    options.iter().find(|(key, _)| matches!(key, ArrayKey::String(name) if name == "limit")).map(|(_, value)| value.number().max(0.0) as u64)
}

fn execute_object_query(connection: &str, database: &str, collection: &str, action: MongoAction) -> Result<Value, String> {
    let result = execute_object_command(connection, database, collection, action)?;
    Ok(rows_to_php(result))
}

fn execute_object_write(connection: &str, database: &str, collection: &str, action: MongoAction) -> Result<DbResult, String> {
    execute_object_command(connection, database, collection, action)
}

fn execute_object_command(connection: &str, database: &str, collection: &str, action: MongoAction) -> Result<DbResult, String> {
    let driver = database_registry().get(connection)?;
    MONGO_SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let command = DbCommand::Mongo { database: database.into(), collection: collection.into(), action };
        if let Some(session) = sessions.get_mut(connection) { session.execute(&command) } else { driver.execute(&command) }
    })
}

pub(crate) fn call(name: &str, arguments: &[Value]) -> Result<Value, String> {
    let connection = string_arg(arguments, 0, "connection name")?;
    if name == "mongo_begin_transaction" { return begin_transaction(&connection); }
    if name == "mongo_commit" { return finish_transaction(&connection, true); }
    if name == "mongo_rollback" { return finish_transaction(&connection, false); }
    let database = string_arg(arguments, 1, "database name")?;
    let collection = string_arg(arguments, 2, "collection name")?;
    let action = match name {
        "mongo_find" => {
            let filter = document_arg(arguments.get(3), "filter")?;
            let limit = arguments.get(4).map(Value::number).filter(|value| *value > 0.0).map(|value| value as u64);
            MongoAction::Find { filter, limit, options: DbValue::Document(Vec::new()) }
        }
        "mongo_aggregate" => MongoAction::Aggregate { pipeline: pipeline_arg(arguments.get(3))?, options: DbValue::Document(Vec::new()) },
        "mongo_count_documents" => MongoAction::CountDocuments { filter: document_arg(arguments.get(3), "filter")?, estimated: false, options: DbValue::Document(Vec::new()) },
        "mongo_estimated_document_count" => MongoAction::CountDocuments { filter: DbValue::Document(Vec::new()), estimated: true, options: DbValue::Document(Vec::new()) },
        "mongo_insert_one" => MongoAction::InsertOne { document: document_arg(arguments.get(3), "document")?, options: DbValue::Document(Vec::new()) },
        "mongo_insert_many" => MongoAction::InsertMany { documents: documents_arg(arguments.get(3))?, options: DbValue::Document(Vec::new()) },
        "mongo_replace_one" => MongoAction::ReplaceOne { filter: document_arg(arguments.get(3), "filter")?, replacement: document_arg(arguments.get(4), "replacement")?, options: DbValue::Document(Vec::new()) },
        "mongo_update_one" => MongoAction::UpdateOne { filter: document_arg(arguments.get(3), "filter")?, update: document_arg(arguments.get(4), "update")?, options: DbValue::Document(Vec::new()) },
        "mongo_update_many" => MongoAction::UpdateMany { filter: document_arg(arguments.get(3), "filter")?, update: document_arg(arguments.get(4), "update")?, options: DbValue::Document(Vec::new()) },
        "mongo_delete_one" => MongoAction::DeleteOne { filter: document_arg(arguments.get(3), "filter")?, options: DbValue::Document(Vec::new()) },
        "mongo_delete_many" => MongoAction::DeleteMany { filter: document_arg(arguments.get(3), "filter")?, options: DbValue::Document(Vec::new()) },
        _ => return Err(format!("undefined MongoDB function {name}()")),
    };
    let driver = database_registry().get(&connection)?;
    if driver.kind() != DatabaseKind::MongoDb { return Err(format!("database connection '{connection}' is not MongoDB")); }
    let command = DbCommand::Mongo { database, collection, action };
    let result = MONGO_SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        if let Some(session) = sessions.get_mut(&connection) { session.execute(&command) }
        else { driver.execute(&command) }
    })?;
    match name {
        "mongo_find" | "mongo_aggregate" => Ok(rows_to_php(result)),
        "mongo_insert_one" => Ok(Value::Bool(result.affected_rows > 0)),
        "mongo_insert_many" => Ok(Value::Integer(result.affected_rows as i64)),
        "mongo_count_documents" | "mongo_estimated_document_count" | "mongo_replace_one" | "mongo_update_one" | "mongo_update_many" | "mongo_delete_one" | "mongo_delete_many" => Ok(Value::Integer(result.affected_rows as i64)),
        _ => unreachable!(),
    }
}

fn begin_transaction(connection: &str) -> Result<Value, String> {
    let driver = database_registry().get(connection)?;
    if driver.kind() != DatabaseKind::MongoDb { return Err(format!("database connection '{connection}' is not MongoDB")); }
    MONGO_SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        if sessions.contains_key(connection) { return Err(format!("MongoDB transaction already active on '{connection}'")); }
        let mut session = driver.start_session()?;
        session.begin_transaction()?;
        sessions.insert(connection.to_owned(), session);
        Ok(Value::Bool(true))
    })
}

fn finish_transaction(connection: &str, commit: bool) -> Result<Value, String> {
    MONGO_SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let Some(session) = sessions.get_mut(connection) else { return Ok(Value::Bool(false)); };
        if commit { session.commit_transaction()?; } else { session.rollback_transaction()?; }
        sessions.remove(connection);
        Ok(Value::Bool(true))
    })
}

fn pipeline_arg(value: Option<&Value>) -> Result<Vec<DbValue>, String> {
    let Some(value) = value else { return Err("MongoDB aggregate requires a pipeline array".into()); };
    let DbValue::Array(stages) = to_db(value)? else { return Err("MongoDB aggregate pipeline must be a list of documents".into()); };
    if stages.iter().any(|stage| !matches!(stage, DbValue::Document(_))) {
        return Err("MongoDB aggregate pipeline stages must be documents".into());
    }
    Ok(stages)
}

fn string_arg(arguments: &[Value], index: usize, label: &str) -> Result<String, String> {
    arguments.get(index).map(Value::show).ok_or_else(|| format!("MongoDB operation requires {label}"))
}

fn document_arg(value: Option<&Value>, label: &str) -> Result<DbValue, String> {
    let Some(value) = value else { return Ok(DbValue::Document(Vec::new())); };
    if matches!(value.dereference(), Value::Null) { return Ok(DbValue::Document(Vec::new())); }
    let converted = to_db(value)?;
    match converted {
        DbValue::Document(_) => Ok(converted),
        DbValue::Array(values) if values.is_empty() => Ok(DbValue::Document(Vec::new())),
        _ => Err(format!("MongoDB {label} must be an associative array")),
    }
}

fn documents_arg(value: Option<&Value>) -> Result<Vec<DbValue>, String> {
    let Some(Value::Array(values)) = value.map(Value::dereference) else { return Err("MongoDB insertMany requires a list of documents".into()); };
    let documents = values.iter().map(|(_, value)| document_arg(Some(value), "document")).collect::<Result<Vec<_>, _>>()?;
    if documents.is_empty() { return Err("MongoDB insertMany requires at least one document".into()); }
    Ok(documents)
}

fn rows_to_php(result: DbResult) -> Value {
    let rows = result.rows.into_iter().enumerate().filter_map(|(index, row)| {
        row.into_iter().next().map(|document| (ArrayKey::Integer(index as i64), to_php(document)))
    }).collect::<Vec<_>>();
    Value::from_array_entries(rows)
}

