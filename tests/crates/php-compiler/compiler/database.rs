use php_compiler::compile_source;
use php_db::{ConnectionConfig, DatabaseDriver, DatabaseKind, DatabaseRegistry, DatabaseSession, DbCommand, DbResult, DbValue};
use std::sync::{Arc, Mutex};

static DB_TEST_LOCK: Mutex<()> = Mutex::new(());
static DB_COMMANDS: Mutex<Vec<DbCommand>> = Mutex::new(Vec::new());
static DB_SESSION_COMMANDS: Mutex<Vec<DbCommand>> = Mutex::new(Vec::new());
static MONGO_SESSION_EVENTS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
static MONGO_FIND_OPTIONS: Mutex<Vec<DbValue>> = Mutex::new(Vec::new());
static MONGO_COUNT_OPTIONS: Mutex<Vec<DbValue>> = Mutex::new(Vec::new());
static MONGO_AGGREGATE_OPTIONS: Mutex<Vec<DbValue>> = Mutex::new(Vec::new());
static MONGO_WRITE_OPTIONS: Mutex<Vec<(String, DbValue)>> = Mutex::new(Vec::new());

struct FakePostgres;
struct FakeSession;
impl DatabaseSession for FakeSession {
    fn client_version(&self) -> Option<String> { Some("libpq-test".into()) }
    fn execute(&mut self, command: &DbCommand) -> Result<DbResult, String> {
        DB_SESSION_COMMANDS.lock().unwrap().push(command.clone());
        let last_insert_id = match command { DbCommand::Sql { statement, .. } if statement.starts_with("INSERT") => Some("77".into()), _ => None };
        Ok(DbResult {
            columns: vec!["id".into(), "name".into()],
            rows: vec![vec![DbValue::String("42".into()), DbValue::String("Ada".into())]],
            affected_rows: 1,
            last_insert_id,
            ..DbResult::default()
        })
    }
}
impl DatabaseDriver for FakePostgres {
    fn kind(&self) -> DatabaseKind { DatabaseKind::PostgreSql }
    fn ping(&self) -> Result<(), String> { Ok(()) }
    fn start_session(&self) -> Result<Box<dyn DatabaseSession>, String> { Ok(Box::new(FakeSession)) }
    fn execute(&self, command: &DbCommand) -> Result<DbResult, String> {
        DB_COMMANDS.lock().unwrap().push(command.clone());
        let last_insert_id = match command { DbCommand::Sql { statement, .. } if statement.starts_with("INSERT") => Some("77".into()), _ => None };
        Ok(DbResult {
            columns: vec!["id".into(), "name".into()],
            rows: vec![vec![DbValue::String("42".into()), DbValue::String("Ada".into())]],
            affected_rows: 1,
            last_insert_id,
            ..DbResult::default()
        })
    }
}

#[test]
fn pdo_construct_prepare_execute_fetch_and_query_use_registered_driver() {
    let _lock = DB_TEST_LOCK.lock().unwrap();
    DB_COMMANDS.lock().unwrap().clear();
    DB_SESSION_COMMANDS.lock().unwrap().clear();
    let registry = DatabaseRegistry::default();
    let config = ConnectionConfig::new("test", "pgsql:host=test;dbname=test").unwrap();
    registry.register(&config, Arc::new(FakePostgres)).unwrap();
    php_runtime::set_database_registry(registry);
    let source = r#"<?php
        echo class_exists('PDO') ? 'pdo:' : 'missing:';
        $pdo = new PDO('pgsql:host=test;dbname=test');
        $statement = $pdo->prepare('SELECT id, name WHERE id = :id');
        $statement->execute(['id' => 42]);
        $row = $statement->fetch(PDO::FETCH_ASSOC);
        echo $row['id'], ':', $row['name'], ':', $statement->rowCount(), "\n";
        $bound = $pdo->prepare('SELECT id, name WHERE id = :id');
        $bound->bindValue(':id', 42, PDO::PARAM_INT);
        $bound->execute();
        echo $bound->fetchColumn(1), "\n";
        $currentId = 10;
        $byReference = $pdo->prepare('SELECT id, name WHERE id = :id');
        $byReference->bindParam(':id', $currentId, PDO::PARAM_INT);
        $currentId = 99;
        $byReference->execute();
        echo $byReference->fetchColumn(0), "\n";
        $rows = $pdo->query('SELECT id, name')->fetchAll();
        echo $rows[0]['name'], "\n";
        $pdo->exec('INSERT INTO users (name) VALUES (\'Ada\')');
        echo $pdo->lastInsertId(), "\n";
        echo $pdo->inTransaction() ? 'bad' : 'idle', ':';
        $pdo->beginTransaction();
        echo $pdo->inTransaction() ? 'active' : 'bad', ':';
        $pdo->query('SELECT 1');
        $pdo->commit();
        echo $pdo->inTransaction() ? 'bad' : 'idle', "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "pdo:42:Ada:1\nAda\n42\nAda\n77\nidle:active:idle\n");
    assert!(DB_SESSION_COMMANDS.lock().unwrap().iter().any(|command| matches!(command,
        DbCommand::Sql { statement, parameters } if statement == "SELECT id, name WHERE id = $1" && parameters == &[DbValue::Integer(42)]
    )));
    assert!(DB_SESSION_COMMANDS.lock().unwrap().iter().any(|command| matches!(command,
        DbCommand::Sql { statement, parameters } if statement == "SELECT id, name WHERE id = $1" && parameters == &[DbValue::Integer(99)]
    )));
    assert!(DB_SESSION_COMMANDS.lock().unwrap().iter().any(|command| matches!(command, DbCommand::Sql { statement, .. } if statement == "BEGIN")));
    assert!(DB_SESSION_COMMANDS.lock().unwrap().iter().any(|command| matches!(command, DbCommand::Sql { statement, .. } if statement == "COMMIT")));
    assert!(DB_SESSION_COMMANDS.lock().unwrap().iter().any(|command| matches!(command, DbCommand::Sql { statement, .. } if statement == "SELECT 1")));
}

#[test]
fn pdo_fetch_modes_and_common_attributes_are_applied() {
    let _lock = DB_TEST_LOCK.lock().unwrap();
    let registry = DatabaseRegistry::default();
    let config = ConnectionConfig::new("fetch", "pgsql:host=fetch;dbname=test").unwrap();
    registry.register(&config, Arc::new(FakePostgres)).unwrap();
    php_runtime::set_database_registry(registry);
    let source = r#"<?php
        $pdo = new PDO('pgsql:host=fetch;dbname=test');
        echo $pdo->getAttribute(PDO::ATTR_CLIENT_VERSION), ':';
        echo $pdo->getAttribute(PDO::ATTR_DEFAULT_FETCH_MODE), ':';
        $pdo->setAttribute(PDO::ATTR_DEFAULT_FETCH_MODE, PDO::FETCH_ASSOC);
        $statement = $pdo->query('SELECT id, name');
        $row = $statement->fetch();
        echo $row['name'], ':';
        $statement = $pdo->query('SELECT id, name', PDO::FETCH_NUM);
        echo $statement->fetch()[0], ':';
        $statement = $pdo->query('SELECT id, name');
        echo $statement->fetchAll(PDO::FETCH_COLUMN, 1)[0], ':';
        $statement = $pdo->query('SELECT id, name');
        $statement->setFetchMode(PDO::FETCH_OBJ);
        echo $statement->fetch()->name, "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "libpq-test:4:Ada:42:Ada:Ada\n");
}

#[test]
fn file_backed_php_streams_bind_as_pdo_lobs() {
    let _lock = DB_TEST_LOCK.lock().unwrap();
    let registry = DatabaseRegistry::default();
    let config = ConnectionConfig::new("stream", "pgsql:host=stream;dbname=test").unwrap();
    registry.register(&config, Arc::new(FakePostgres)).unwrap();
    php_runtime::set_database_registry(registry);
    let source = r#"<?php
        $stream = fopen('php://temp', 'w+b');
        fwrite($stream, 'large-object-data');
        rewind($stream);
        echo is_resource($stream) ? 'resource:' : 'invalid:';
        echo gettype($stream), ':', stream_get_contents($stream), ':';
        rewind($stream);
        $pdo = new PDO('pgsql:host=stream;dbname=test');
        $statement = $pdo->prepare('INSERT INTO blobs (payload) VALUES (?)');
        $statement->bindValue(1, $stream, PDO::PARAM_LOB);
        $statement->execute();
        echo fclose($stream) ? 'closed' : 'open', ':', is_resource($stream) ? 'resource' : 'closed', "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "resource:resource:large-object-data:closed:closed\n");
    assert!(DB_SESSION_COMMANDS.lock().unwrap().iter().any(|command| matches!(command,
        DbCommand::Sql { statement, parameters } if statement == "INSERT INTO blobs (payload) VALUES ($1)" && parameters == &[DbValue::Bytes(b"large-object-data".to_vec())]
    )));
}

struct FakeMongo;
struct FakeMongoSession;
impl DatabaseSession for FakeMongoSession {
    fn execute(&mut self, command: &DbCommand) -> Result<DbResult, String> {
        DB_SESSION_COMMANDS.lock().unwrap().push(command.clone());
        Ok(DbResult { columns: vec!["document".into()], rows: vec![vec![DbValue::Document(vec![("name".into(), DbValue::String("Txn".into()))])]], affected_rows: 1, ..DbResult::default() })
    }
    fn begin_transaction(&mut self) -> Result<(), String> { MONGO_SESSION_EVENTS.lock().unwrap().push("begin"); Ok(()) }
    fn commit_transaction(&mut self) -> Result<(), String> { MONGO_SESSION_EVENTS.lock().unwrap().push("commit"); Ok(()) }
    fn rollback_transaction(&mut self) -> Result<(), String> { MONGO_SESSION_EVENTS.lock().unwrap().push("rollback"); Ok(()) }
}
impl DatabaseDriver for FakeMongo {
    fn kind(&self) -> DatabaseKind { DatabaseKind::MongoDb }
    fn ping(&self) -> Result<(), String> { Ok(()) }
    fn start_session(&self) -> Result<Box<dyn DatabaseSession>, String> { Ok(Box::new(FakeMongoSession)) }
    fn execute(&self, command: &DbCommand) -> Result<DbResult, String> {
        if let DbCommand::Mongo { action: php_db::MongoAction::Find { options, .. }, .. } = command {
            MONGO_FIND_OPTIONS.lock().unwrap().push(options.clone());
        }
        if let DbCommand::Mongo { action: php_db::MongoAction::CountDocuments { options, estimated: false, .. }, .. } = command {
            MONGO_COUNT_OPTIONS.lock().unwrap().push(options.clone());
        }
        if let DbCommand::Mongo { action: php_db::MongoAction::Aggregate { options, .. }, .. } = command {
            MONGO_AGGREGATE_OPTIONS.lock().unwrap().push(options.clone());
        }
        let write_options = match command {
            DbCommand::Mongo { action: php_db::MongoAction::InsertOne { options, .. }, .. } => Some(("insertOne", options)),
            DbCommand::Mongo { action: php_db::MongoAction::InsertMany { options, .. }, .. } => Some(("insertMany", options)),
            DbCommand::Mongo { action: php_db::MongoAction::ReplaceOne { options, .. }, .. } => Some(("replaceOne", options)),
            DbCommand::Mongo { action: php_db::MongoAction::UpdateOne { options, .. }, .. } => Some(("updateOne", options)),
            DbCommand::Mongo { action: php_db::MongoAction::UpdateMany { options, .. }, .. } => Some(("updateMany", options)),
            DbCommand::Mongo { action: php_db::MongoAction::DeleteOne { options, .. }, .. } => Some(("deleteOne", options)),
            DbCommand::Mongo { action: php_db::MongoAction::DeleteMany { options, .. }, .. } => Some(("deleteMany", options)),
            _ => None,
        };
        if let Some((method, options)) = write_options { MONGO_WRITE_OPTIONS.lock().unwrap().push((method.into(), options.clone())); }
        Ok(match command {
            DbCommand::Mongo { action: php_db::MongoAction::Find { .. }, .. } => DbResult {
                columns: vec!["document".into()], rows: vec![vec![DbValue::Document(vec![("name".into(), DbValue::String("Ada".into()))])]], affected_rows: 1, ..DbResult::default()
            },
            DbCommand::Mongo { action: php_db::MongoAction::Aggregate { pipeline, .. }, .. } if pipeline.len() == 1 => DbResult {
                columns: vec!["document".into()], rows: vec![vec![DbValue::Document(vec![("count".into(), DbValue::Integer(1))])]], affected_rows: 1, ..DbResult::default()
            },
            DbCommand::Mongo { action: php_db::MongoAction::CountDocuments { estimated: true, .. }, .. } => DbResult { affected_rows: 9, ..DbResult::default() },
            DbCommand::Mongo { action: php_db::MongoAction::CountDocuments { estimated: false, .. }, .. } => DbResult { affected_rows: 7, ..DbResult::default() },
            DbCommand::Mongo { action: php_db::MongoAction::InsertOne { .. }, .. } => DbResult { affected_rows: 1, ..DbResult::default() },
            DbCommand::Mongo { action: php_db::MongoAction::InsertMany { .. }, .. } => DbResult { affected_rows: 2, ..DbResult::default() },
            DbCommand::Mongo { action: php_db::MongoAction::ReplaceOne { .. }, .. } => DbResult { affected_rows: 1, matched_rows: Some(1), modified_rows: Some(1), ..DbResult::default() },
            DbCommand::Mongo { action: php_db::MongoAction::UpdateOne { .. }, .. } => DbResult { affected_rows: 1, matched_rows: Some(1), modified_rows: Some(1), ..DbResult::default() },
            DbCommand::Mongo { action: php_db::MongoAction::UpdateMany { .. }, .. } => DbResult { affected_rows: 1, matched_rows: Some(2), modified_rows: Some(1), ..DbResult::default() },
            DbCommand::Mongo { action: php_db::MongoAction::DeleteOne { .. }, .. } => DbResult { affected_rows: 1, ..DbResult::default() },
            DbCommand::Mongo { action: php_db::MongoAction::DeleteMany { .. }, .. } => DbResult { affected_rows: 3, ..DbResult::default() },
            _ => return Err("unexpected Mongo test command".into()),
        })
    }
}

#[test]
fn mongo_php_functions_dispatch_document_operations_to_registered_driver() {
    let _lock = DB_TEST_LOCK.lock().unwrap();
    MONGO_FIND_OPTIONS.lock().unwrap().clear();
    MONGO_COUNT_OPTIONS.lock().unwrap().clear();
    MONGO_AGGREGATE_OPTIONS.lock().unwrap().clear();
    MONGO_WRITE_OPTIONS.lock().unwrap().clear();
    let registry = DatabaseRegistry::default();
    let config = ConnectionConfig::new("docs", "mongodb://localhost:27017/app").unwrap();
    registry.register(&config, Arc::new(FakeMongo)).unwrap();
    php_runtime::set_database_registry(registry);
    let source = r#"<?php
        $docs = mongo_find('docs', 'app', 'users', ['active' => true], 10);
        echo function_exists('mongo_find') ? 'mongo:' : 'missing:';
        $aggregate = mongo_aggregate('docs', 'app', 'users', [['$count' => 'count']]);
        echo $aggregate[0]['count'], ':';
        echo mongo_count_documents('docs', 'app', 'users', ['active' => true]), ':';
        echo mongo_estimated_document_count('docs', 'app', 'users'), ':';
        echo mongo_insert_many('docs', 'app', 'users', [['name' => 'M1'], ['name' => 'M2']]), ':';
        echo mongo_replace_one('docs', 'app', 'users', [], ['name' => 'Ada']), ':';
        $client = new MongoDB\Client(uri: 'mongodb://localhost:27017/app');
        $collection = $client->selectDatabase(databaseName: 'app')->selectCollection(collectionName: 'users');
        $client->selectCollection(collectionName: 'users', databaseName: 'app', options: [])->estimatedDocumentCount();
        echo $collection->find(options: ['limit' => 1, 'skip' => 2, 'sort' => ['name' => 1], 'projection' => ['name' => 1]], filter: ['active' => true])->toArray()[0]['name'], ':';
        $collection->aggregate(options: ['allowDiskUse' => true, 'maxTimeMS' => 50], pipeline: [['$count' => 'count']])->toArray();
        echo $collection->countDocuments(options: ['hint' => 'active_idx', 'maxTimeMS' => 25], filter: ['active' => true]), ':';
        echo $collection->estimatedDocumentCount(options: []), ':';
        echo $collection->insertMany(options: ['comment' => 'insertMany'], documents: [['name' => 'N1'], ['name' => 'N2']])->getInsertedCount(), ':';
        echo $collection->replaceOne(options: ['comment' => 'replaceOne'], replacement: ['name' => 'Ada'], filter: ['name' => 'Ada'])->getModifiedCount(), ':';
        echo $collection->insertOne(options: ['comment' => 'insertOne'], document: ['name' => 'Kim'])->getInsertedCount(), ':';
        $update = $collection->updateMany(options: ['comment' => 'updateMany'], update: ['$set' => ['active' => true]], filter: []);
        echo $update->getMatchedCount(), ':', $update->getModifiedCount(), ':';
        echo $collection->updateOne(options: ['comment' => 'updateOne'], update: ['$set' => ['active' => true]], filter: [])->getModifiedCount(), ':';
        echo $collection->deleteOne(options: ['comment' => 'deleteOne'], filter: [])->getDeletedCount(), ':';
        $collection->deleteMany(options: ['comment' => 'deleteMany'], filter: []);
        echo $docs[0]['name'], ':', mongo_insert_one('docs', 'app', 'users', ['name' => 'Lin']) ? 'inserted' : 'failed', ':';
        echo mongo_update_many('docs', 'app', 'users', [], ['$set' => ['active' => true]]), ':';
        echo mongo_update_one('docs', 'app', 'users', [], ['$set' => ['active' => true]]), ':';
        echo mongo_delete_one('docs', 'app', 'users', []), ':';
        echo mongo_delete_many('docs', 'app', 'users', []), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "mongo:1:7:9:2:1:Ada:7:9:2:1:1:2:1:1:1:Ada:inserted:1:1:1:3\n");
    assert!(MONGO_FIND_OPTIONS.lock().unwrap().iter().any(|options| matches!(options,
        DbValue::Document(entries) if entries.iter().any(|(key, value)| key == "skip" && value == &DbValue::Integer(2))
            && entries.iter().any(|(key, value)| key == "sort" && matches!(value, DbValue::Document(_)))
            && entries.iter().any(|(key, value)| key == "projection" && matches!(value, DbValue::Document(_)))
    )));
    assert!(MONGO_COUNT_OPTIONS.lock().unwrap().iter().any(|options| matches!(options,
        DbValue::Document(entries) if entries.iter().any(|(key, value)| key == "hint" && value == &DbValue::String("active_idx".into()))
            && entries.iter().any(|(key, value)| key == "maxTimeMS" && value == &DbValue::Integer(25))
    )));
    assert!(MONGO_AGGREGATE_OPTIONS.lock().unwrap().iter().any(|options| matches!(options,
        DbValue::Document(entries) if entries.iter().any(|(key, value)| key == "allowDiskUse" && value == &DbValue::Bool(true))
            && entries.iter().any(|(key, value)| key == "maxTimeMS" && value == &DbValue::Integer(50))
    )));
    for method in ["insertOne", "insertMany", "replaceOne", "updateOne", "updateMany", "deleteOne", "deleteMany"] {
        assert!(MONGO_WRITE_OPTIONS.lock().unwrap().iter().any(|(seen, options)| seen == method && matches!(options,
            DbValue::Document(entries) if entries.iter().any(|(key, value)| key == "comment" && value == &DbValue::String(method.into()))
        )), "MongoDB {method}() discarded its options document");
    }
}

#[test]
fn mongo_aggregation_and_transactions_use_the_same_session() {
    let _lock = DB_TEST_LOCK.lock().unwrap();
    DB_SESSION_COMMANDS.lock().unwrap().clear();
    MONGO_SESSION_EVENTS.lock().unwrap().clear();
    let registry = DatabaseRegistry::default();
    let config = ConnectionConfig::new("docs_tx", "mongodb://localhost:27017/app").unwrap();
    registry.register(&config, Arc::new(FakeMongo)).unwrap();
    php_runtime::set_database_registry(registry);
    let source = r#"<?php
        mongo_begin_transaction('docs_tx');
        $rows = mongo_aggregate('docs_tx', 'app', 'users', [['$count' => 'count']]);
        echo $rows[0]['name'], ':';
        mongo_commit('docs_tx');
        mongo_begin_transaction('docs_tx');
        mongo_rollback('docs_tx');
        echo "ok\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "Txn:ok\n");
    assert_eq!(*MONGO_SESSION_EVENTS.lock().unwrap(), vec!["begin", "commit", "begin", "rollback"]);
    assert!(matches!(DB_SESSION_COMMANDS.lock().unwrap().as_slice(), [DbCommand::Mongo { action: php_db::MongoAction::Aggregate { .. }, .. }]));
}
