# Database connections

The HTTP server loads named database pools from `php-rust.ini`. PHP code uses the runtime's `PDO` and `PDOStatement` compatibility classes. Pools live for the server process and are shared across request workers. Each PDO instance checks out one pooled session connection when it is constructed and holds it for the PHP request, preserving session state; `beginTransaction()` uses that same connection through `commit()` or `rollBack()`. Open SQL and MongoDB transactions are rolled back when their PHP request ends; rollback is attempted if a database operation fails during a transaction. Waiting for a pool slot occupies the PHP VM's blocking request worker while Tokio continues accepting and serving other connections.

## Configuration

Add one section per named connection. Quote DSNs containing semicolons so the INI parser does not treat them as comments. Credentials support environment expansion; keep passwords out of checked-in files.

```ini
[database.primary]
dsn = "pgsql:host=127.0.0.1;port=5432;dbname=app"
username = ${DB_USER}
password = ${DB_PASSWORD}
pool_min = 0
pool_max = 20
pool_acquire_timeout_secs = 5
pool_idle_timeout_secs = 600
max_result_rows = 1000000
max_column_bytes = 8388608
```

Pool keys are `pool_min`, `pool_max`, `pool_acquire_timeout_secs`, and `pool_idle_timeout_secs` (`0` disables idle expiration). `max_result_rows` limits rows per materialized result set across all backends; `max_column_bytes` limits an ODBC text column while it is read. Both values must be greater than zero. Defaults are one million rows and 8 MiB per ODBC column. DSNs and credentials are redacted from debug output; logs include configured connection names only.

## Driver backends

| PHP DSN | Backend | Runtime dependency |
| --- | --- | --- |
| `pgsql:` / `postgres:` | PostgreSQL | `libpq` runtime library |
| `mysql:` | MySQL | `libmysqlclient` or MariaDB client runtime |
| `mariadb:` | MariaDB | `libmariadb` or compatible MySQL client runtime |
| `sqlsrv:` / `sqlserver:` | SQL Server through ODBC | unixODBC/iODBC and Microsoft SQL Server ODBC driver |
| `oci:` / `oracle:` | Oracle through ODBC | unixODBC/iODBC and Oracle ODBC driver |
| `mongodb://` / `mongodb+srv://` | MongoDB | official `libmongoc` runtime |

For SQL Server and Oracle, set `driver` to the installed ODBC driver's registered name if it differs from the default:

```ini
[database.reporting]
dsn = "sqlsrv:Server=db.internal,1433;Database=reporting;Encrypt=yes"
driver = ODBC Driver 18 for SQL Server
username = ${DB_USER}
password = ${DB_PASSWORD}
pool_max = 12
```

PHP supports file-backed temporary streams through `fopen()`, `fwrite()`, `fread()`, `rewind()`, `fseek()`, `ftell()`, `feof()`, `stream_get_contents()`, and `fclose()` for LOB input. MongoDB uses libmongoc's topology-aware client pool, with the configured pool limits applied to its URI. PHP accesses it through document functions and the documented built-in `MongoDB\Client` subset, not PDO. The built-in subset exposes `find`, `findOne`, `aggregate`, `countDocuments`, `estimatedDocumentCount`, `insertOne`, `insertMany`, `replaceOne`, `updateOne`, `updateMany`, `deleteOne`, and `deleteMany` on collections, plus matching `mongo_*` function helpers for CRUD, count, and transactions. It checks out clients from that shared pool for each operation. libmongoc failures include both the error domain and native code in their messages; MongoDB does not expose PDO `errorCode()` / `errorInfo()`.

## PHP API currently implemented

```php
$pdo = new PDO('pgsql:host=127.0.0.1;port=5432;dbname=app', getenv('DB_USER'), getenv('DB_PASSWORD'));
$statement = $pdo->prepare('SELECT id, name FROM users WHERE id = :id');
$statement->execute(['id' => 42]);
$user = $statement->fetch(PDO::FETCH_ASSOC);
```

MongoDB functions take the configured connection name, database, and collection explicitly:

```php
$users = mongo_find('documents', 'app', 'users', ['active' => true], 100);
$counts = mongo_aggregate('documents', 'app', 'users', [['$group' => ['_id' => '$active', 'count' => ['$sum' => 1]]]]);
mongo_begin_transaction('documents');
mongo_insert_one('documents', 'app', 'users', ['name' => 'Ada']);
mongo_commit('documents');
mongo_update_many('documents', 'app', 'users', ['name' => 'Ada'], ['$set' => ['active' => true]]);
mongo_delete_many('documents', 'app', 'users', ['active' => false]);
```

MongoDB provides the listed `mongo_*` functions plus a built-in subset of `MongoDB\\Client`: construct it with the configured URI, call `selectDatabase` / `selectCollection`, then use collection `find`, `findOne`, `aggregate`, `countDocuments`, `estimatedDocumentCount`, `insertOne`, `insertMany`, `replaceOne`, `updateOne`, `updateMany`, `deleteOne`, or `deleteMany`. Named arguments bind against the PHP library parameter names, including `uri`, `databaseName`, `collectionName`, and each operation's `filter`, `options`, `document`, `documents`, `replacement`, `update`, or `pipeline` parameters. `find()` forwards driver options such as `sort`, `skip`, and `projection`, while enforcing the configured result-row limit; `countDocuments()`, `aggregate()`, and collection write methods forward their options documents. Cursors support `toArray`; write results expose their affected-count getters. The full Composer MongoDB library is not included. The function-based `mongo_begin_transaction` / `mongo_commit` / `mongo_rollback` helpers keep one client checked out for the transaction; MongoDB transactions require a replica set or sharded cluster.

Implemented methods are `PDO::connect`, `query`, `exec`, `prepare`, `beginTransaction`, `commit`, `rollBack`, `inTransaction`, `lastInsertId`, `quote`, `errorCode`, `errorInfo`, `setAttribute`, `getAttribute`, `getAvailableDrivers`, and `PDOStatement::execute`, `bindValue`, `bindParam`, `bindColumn`, `fetch`, `fetchAll`, `fetchColumn`, `fetchObject`, `setFetchMode`, `rowCount`, `columnCount`, `getColumnMeta`, `getAttribute`, `setAttribute`, `nextRowset`, `debugDumpParams`, `closeCursor`, `errorCode`, `errorInfo`, and `getIterator`. Statements can be consumed by `foreach`, `iterator_to_array()`, and `iterator_count()`; iteration consumes unread rows from the forward-only cursor and applies the configured fetch mode (`FETCH_ASSOC`, `FETCH_NUM`, `FETCH_BOTH`, `FETCH_OBJ`, `FETCH_COLUMN`, `FETCH_NAMED`, `FETCH_BOUND`, `FETCH_INTO`, `FETCH_CLASS`, or `FETCH_FUNC`). `PDO::query()` accepts fetch-mode arguments for `FETCH_CLASS`, `FETCH_INTO`, and `FETCH_FUNC`. `getAvailableDrivers()` returns configured SQL driver families; `PDO::ATTR_DRIVER_NAME` reports the selected SQL family. `quote()` uses backend string escaping and honors `PARAM_LOB` with a backend-specific binary literal. `bindParam()` retains a PHP variable reference and reads its current value when `execute()` runs; `bindParam()` and `bindColumn()` require a variable passed by reference. `bindValue()` / `bindParam()` support `PARAM_NULL`, `PARAM_INT`, `PARAM_STR`, `PARAM_LOB` (string payloads or `php://temp` / `php://memory` stream resources), and `PARAM_BOOL`. A LOB stream is read from its current position when the statement executes and converted to a byte payload before the backend bind; native drivers do not yet transfer or fetch LOBs incrementally. Fetch supports `FETCH_LAZY`, `FETCH_ASSOC`, `FETCH_NUM`, `FETCH_BOTH`, `FETCH_OBJ`, `FETCH_BOUND`, `FETCH_COLUMN`, `FETCH_INTO` (named fields copied onto the supplied object), `FETCH_NAMED`, `FETCH_KEY_PAIR`, `FETCH_CLASS` (hydrates row properties before running the constructor), `FETCH_PROPS_LATE` (hydrates after the constructor), and `FETCH_FUNC` (calls a PHP callable with row fields); `fetchAll()` also supports `FETCH_GROUP` and `FETCH_UNIQUE`. `ATTR_CASE` supports natural, upper-case, and lower-case result column names. `ATTR_FETCH_TABLE_NAMES` and `ATTR_FETCH_CATALOG_NAMES` qualify names when MySQL/MariaDB provides source metadata; other drivers leave names unchanged when that metadata is unavailable. `ATTR_TIMEOUT` controls how long PDO waits to acquire its pooled connection during construction; zero uses the configured pool timeout. Changing it after that PDO has checked out a session does not replace the active connection. `ATTR_CLIENT_VERSION` reports the MySQL/MariaDB client, libpq library, or active ODBC driver version; MongoDB returns `false`. `ATTR_SERVER_VERSION` and `ATTR_SERVER_INFO` query the server version through the active session for MySQL/MariaDB, PostgreSQL, and SQL Server. `ATTR_STRINGIFY_FETCHES` converts non-null materialized column values to strings. `ATTR_DEFAULT_STR_PARAM` selects national or regular character string binding when no explicit PDO parameter type is provided. Oracle `ATTR_ORACLE_NULLS` supports natural handling, empty-string-to-null conversion, and null-to-empty-string conversion. `ATTR_ERRMODE` supports silent, warning (written to stderr), and exception-style runtime errors; `ATTR_DEFAULT_FETCH_MODE` controls `query()` and prepared-statement fetch defaults. `ATTR_EMULATE_PREPARES` controls parameter execution: false uses native binding through libpq (PostgreSQL), MySQL/MariaDB C prepared statements, or ODBC (SQL Server/Oracle); true uses driver-specific escaped literal interpolation. MySQL/MariaDB default to emulation; PostgreSQL, SQL Server, and Oracle default to native binding. Native MySQL/MariaDB and ODBC statement handles are cached per PDO session and released at request end. `ATTR_AUTOCOMMIT` is emulated with explicit transaction commands: disabling it starts a transaction when a session is already open, or before its first command otherwise; enabling it commits an active transaction. `ATTR_PERSISTENT` is accepted and returned by `getAttribute()`, while actual connection reuse is controlled by the configured process-wide connection pool, shared by PDO instances. These do not expose per-PDO persistent connections. PDO errorCode()/errorInfo() retain MySQL/MariaDB SQLSTATE and native code, PostgreSQL SQLSTATE, and ODBC SQLSTATE/native code where the client reports them; errors without driver diagnostics fall back to `HY000`. `lastInsertId()` returns generated IDs for MySQL/MariaDB and SQL Server; PostgreSQL supports a named sequence or an explicit `INSERT ... RETURNING`, and Oracle supports a named sequence on the same PDO session. `getColumnMeta()` exposes available native type, length, decimal precision, and nullability for MySQL/MariaDB and ODBC, plus native type/length for PostgreSQL where libpq provides them. Multi-result traversal is provided by MySQL/MariaDB and ODBC; PostgreSQL supports it for simple queries and emulated statements; native parameterized PostgreSQL commands return one result set. MySQL/MariaDB connections enable the client multi-statement capability to support `nextRowset()`. PDOStatement supports the forward-only cursor attribute; scrollable cursors and other driver-specific statement attributes are unsupported. Nested `beginTransaction()` calls use backend savepoints; nested commit/rollBack resolve the innermost level, and the outer level controls the actual transaction.

Named arguments are supported for implemented PDO/PDOStatement method signatures, including PHP parameter names such as `query`, `fetchMode`, `attribute`, `value`, `params`, `mode`, `column`, and `constructorArgs`. Remaining internal class methods and complete signature/default parity still need a broader audit.

PostgreSQL uses libpq parameter binding by default. SQL Server and Oracle use ODBC parameter binding, and MySQL/MariaDB use the client prepared-statement API when emulation is disabled. MySQL/MariaDB and ODBC handles are cached by SQL string for the PDO request session; PostgreSQL sends parameterized commands through libpq for each execution. With emulation enabled, the drivers interpolate values using their backend-specific escaping rules. PostgreSQL, MySQL/MariaDB, ODBC, and MongoDB enforce the configured `max_result_rows` before converting or accumulating rows in the Rust result. PostgreSQL and the MySQL client APIs still receive buffered native result objects from their C libraries. ODBC also enforces `max_column_bytes` while collecting each text column. Query smaller pages or columns for larger data; avoid building SQL by concatenating untrusted input.

## Live driver checks

For a local Docker test stack, run `./scripts/live-db-docker.sh up`, inspect readiness with `./scripts/live-db-docker.sh ps`, then run `./scripts/live-db-docker.sh test`. The default test covers PostgreSQL, MySQL, MariaDB, MongoDB (single-node replica set), and SQL Server; all five passed the latest live run, including MongoDB single-document and bulk CRUD, option forwarding, and transaction rollback. The Docker test container requires all five DSNs and fails if one is missing, so an `ok` result cannot hide a skipped backend. Oracle Free is available through the opt-in `./scripts/live-db-docker.sh test-oracle` command, but its live connection check remains unresolved and is excluded from the default run. The test command waits for health checks, builds a separate test container with native client drivers, supplies DSNs, and runs the live integration harness; it does not require host database client libraries. The runner uses host networking and targets Linux Docker hosts. See [docker/database/README.md](../docker/database/README.md) for credentials, ports, and setup details. The SQL Server image requires accepting Microsoft's container license; Oracle Free uses a community image.

`PDOStatement::bindColumn()` accepts the PDO scalar type argument and casts each fetched value into the referenced variable using that type. Its optional maximum length truncates string values at a valid UTF-8 boundary; non-null driver-specific bind-column options are rejected as unsupported.

Unit tests use fake backends for PHP API behavior. To verify native client loading and connectivity against real servers, set a DSN for each backend available in the environment and run:

```sh
PHP_RUST_TEST_POSTGRESQL_DSN='pgsql:host=127.0.0.1;port=5432;dbname=app' \
PHP_RUST_TEST_POSTGRESQL_USERNAME=app \
PHP_RUST_TEST_POSTGRESQL_PASSWORD=secret \
./scripts/database-integration-test.sh
```

The same test runner accepts `PHP_RUST_TEST_MYSQL_DSN`, `PHP_RUST_TEST_MARIADB_DSN`, `PHP_RUST_TEST_MONGODB_DSN`, `PHP_RUST_TEST_SQLSERVER_DSN`, and `PHP_RUST_TEST_ORACLE_DSN`. SQL backends also accept matching `*_USERNAME` and `*_PASSWORD` variables; ODBC backends accept `*_ODBC_DRIVER`. SQL checks cover prepared parameters, result metadata, native error diagnostics, transactions where explicitly supported, and multi-result queries where supported. MongoDB checks ping and transaction rollback against a replica set. A test without its DSN is reported as skipped; the script requires at least one configured DSN.
