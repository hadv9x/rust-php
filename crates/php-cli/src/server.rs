//! Tokio HTTP server for executing PHP files with the existing VM.

use std::{
    env,
    io::{self, BufRead, Cursor, Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use php_compiler::execute_file_with_globals_limits_diagnostics_and_include_path;
use php_db::{ConnectionConfig, DatabaseKind, DatabaseRegistry, PoolOptions, PostgresDriver};
use php_runtime::{ArrayKey, ExecutionLimits, RuntimeDiagnostic, SessionValue, Value};
use tokio::{
    io::{
        AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt,
        BufReader,
    },
    net::{TcpListener, TcpSocket, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore},
    task::JoinSet,
};

const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_HEADER_COUNT: usize = 100;
const MAX_PHP_SOURCE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_STATIC_FILE_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_MAX_IN_FLIGHT: usize = 128;
const ACCEPT_BACKLOG: u32 = 1024;
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 15;
const DEFAULT_MAX_INPUT_VARS: usize = 1000;
const DEFAULT_MAX_INPUT_NESTING_LEVEL: usize = 64;
const DEFAULT_MAX_FILE_UPLOADS: usize = 20;
#[cfg(windows)]
const PATH_SEPARATOR: char = ';';
#[cfg(not(windows))]
const PATH_SEPARATOR: char = ':';
const MAX_KEEPALIVE_REQUESTS: usize = 100;
static LOG_LEVEL: OnceLock<u8> = OnceLock::new();
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
static NEXT_UPLOAD_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

struct UploadTempGuard(Vec<PathBuf>);

impl Drop for UploadTempGuard {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(unix)]
extern "C" fn handle_shutdown_signal(_: libc::c_int) {
    SHUTDOWN_REQUESTED.store(true, Ordering::Relaxed);
}

#[cfg(unix)]
fn install_shutdown_handlers() -> Result<(), String> {
    // The handler only stores an atomic flag; Tokio observes it from the accept loop.
    unsafe {
        for signal in [libc::SIGINT, libc::SIGTERM] {
            if libc::signal(
                signal,
                handle_shutdown_signal as *const () as libc::sighandler_t,
            ) == libc::SIG_ERR
            {
                return Err(format!(
                    "cannot register shutdown handler for signal {signal}"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn install_shutdown_handlers() -> Result<(), String> {
    Ok(())
}

#[derive(Clone)]
struct ServerConfig {
    root: PathBuf,
    bind: SocketAddr,
    max_in_flight: usize,
    request_timeout: Duration,
    max_body_bytes: usize,
    max_upload_bytes: usize,
    file_uploads: bool,
    max_file_uploads: usize,
    max_multipart_body_parts: Option<usize>,
    max_input_vars: usize,
    max_input_nesting_level: usize,
    include_path: Vec<PathBuf>,
    exception_string_param_max_len: usize,
    execution_limits: ExecutionLimits,
    secure_session_cookies: bool,
    display_errors: DisplayErrors,
    default_mimetype: String,
    default_charset: String,
    log_level: String,
    database_connections: Vec<ConnectionConfig>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DisplayErrors {
    On,
    Off,
    Stderr,
}

impl DisplayErrors {
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "1" | "on" | "yes" | "true" => Ok(Self::On),
            "0" | "off" | "no" | "false" | "" => Ok(Self::Off),
            "stderr" => Ok(Self::Stderr),
            _ => Err("display_errors must be On, Off, or stderr".into()),
        }
    }
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    query: String,
    protocol: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    keep_alive: bool,
}

struct WebResponse {
    output: Vec<u8>,
    status: u16,
    headers: Vec<(String, String)>,
}

#[derive(Clone)]
struct SessionEntry {
    data: SessionValue,
    bytes: usize,
    last_access: Instant,
}

type SessionStore = Arc<Mutex<std::collections::HashMap<String, SessionEntry>>>;

const MAX_SESSIONS: usize = 4_096;
const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_SESSION_BYTES: usize = 128 * 1024;
const MAX_SESSION_STORE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
enum RequestError {
    NotFound,
    BadPath,
    InvalidRequest(String),
    PayloadTooLarge,
    Execution(String),
}

fn usage() {
    println!("Usage: php-rust-server [--config FILE] [--root DIR] [--bind ADDRESS] [--max-in-flight N] [--request-timeout-secs N] [--max-body-bytes N] [--max-upload-bytes N] [--max-file-uploads N] [--max-multipart-body-parts N] [--max-input-vars N] [--max-input-nesting-level N] [--max-execution-steps N] [--max-execution-time-secs N] [--max-output-bytes N]");
    println!("Defaults: --root public --bind 127.0.0.1:3000 --max-in-flight 128");
    println!("Config file: php-rust.ini (or --config FILE / PHP_RUST_CONFIG)");
    println!("Config precedence: command line > PHP_RUST_* environment > php-rust.ini > defaults");
}

fn config_value<T: std::str::FromStr>(
    settings: &std::collections::HashMap<String, String>,
    key: &str,
    env_key: &str,
    default: T,
) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    match env::var(key) {
        Ok(value) => value
            .parse()
            .map_err(|error| format!("invalid {env_key}: {error}")),
        Err(env::VarError::NotPresent) => match settings.get(env_key) {
            Some(value) => value
                .parse()
                .map_err(|error| format!("invalid config value {env_key}: {error}")),
            None => Ok(default),
        },
        Err(error) => Err(format!("invalid {key}: {error}")),
    }
}

fn config_bool(
    settings: &std::collections::HashMap<String, String>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    let value = env::var(key).ok().or_else(|| settings.get(key).cloned());
    match value.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None => Ok(default),
        Some("true" | "on" | "yes" | "1") => Ok(true),
        Some("false" | "off" | "no" | "none" | "0") => Ok(false),
        Some(_) => Err(format!("{key} must be true/false, on/off, or 1/0")),
    }
}

fn parse_ini_into(
    path: &Path,
    settings: &mut std::collections::HashMap<String, String>,
) -> Result<(), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read config {}: {error}", path.display()))?;
    if content.len() > 1024 * 1024 {
        return Err("configuration file exceeds 1 MiB".into());
    }
    let mut unsupported = std::collections::HashSet::new();
    let mut section = String::new();
    for (line_index, line) in content.lines().enumerate() {
        let line = strip_ini_comment(line.trim().trim_start_matches('\u{feff}')).trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_ascii_lowercase();
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| {
            format!(
                "{}:{}: expected key = value",
                path.display(),
                line_index + 1
            )
        })?;
        let key = key.trim().to_ascii_lowercase();
        let value = strip_ini_comment(value).trim();
        let value = if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            &value[1..value.len() - 1]
        } else {
            value
        };
        let value = expand_ini_variables(value, settings);
        if let Some(name) = section.strip_prefix("database.") {
            if name.is_empty() {
                return Err(format!(
                    "{}:{}: database section requires a name",
                    path.display(),
                    line_index + 1
                ));
            }
            if !matches!(
                key.as_str(),
                "dsn"
                    | "username"
                    | "password"
                    | "driver"
                    | "pool_min"
                    | "pool_max"
                    | "pool_acquire_timeout_secs"
                    | "pool_idle_timeout_secs"
                    | "max_result_rows"
                    | "max_column_bytes"
            ) {
                return Err(format!(
                    "{}:{}: unsupported database setting '{key}'",
                    path.display(),
                    line_index + 1
                ));
            }
            settings.insert(format!("PHP_RUST_DATABASE:{name}:{key}"), value);
            continue;
        }
        let (target, value) = match key.as_str() {
            "post_max_size" => (
                "PHP_RUST_MAX_BODY_BYTES",
                parse_php_size(&value)
                    .ok_or_else(|| {
                        format!(
                            "{}:{}: invalid post_max_size",
                            path.display(),
                            line_index + 1
                        )
                    })?
                    .to_string(),
            ),
            "upload_max_filesize" => (
                "PHP_RUST_MAX_UPLOAD_FILE_BYTES",
                if value.trim() == "0" {
                    0
                } else {
                    parse_php_size(&value).ok_or_else(|| {
                        format!(
                            "{}:{}: invalid upload_max_filesize",
                            path.display(),
                            line_index + 1
                        )
                    })?
                }
                .to_string(),
            ),
            "file_uploads" => ("PHP_RUST_FILE_UPLOADS", value.clone()),
            "max_file_uploads" => ("PHP_RUST_MAX_FILE_UPLOADS", value.clone()),
            "max_multipart_body_parts" => ("PHP_RUST_MAX_MULTIPART_BODY_PARTS", value.clone()),
            "session.cookie_secure" => ("PHP_RUST_COOKIE_SECURE", value.clone()),
            "display_errors" => ("PHP_RUST_DISPLAY_ERRORS", value.clone()),
            "default_mimetype" => ("PHP_RUST_DEFAULT_MIMETYPE", value.clone()),
            "default_charset" => ("PHP_RUST_DEFAULT_CHARSET", value.clone()),
            "max_input_time" => (
                "PHP_RUST_REQUEST_TIMEOUT_SECS",
                parse_php_timeout(&value)
                    .ok_or_else(|| {
                        format!(
                            "{}:{}: invalid max_input_time",
                            path.display(),
                            line_index + 1
                        )
                    })?
                    .to_string(),
            ),
            "max_input_vars" => ("PHP_RUST_MAX_INPUT_VARS", value.clone()),
            "max_input_nesting_level" => ("PHP_RUST_MAX_INPUT_NESTING_LEVEL", value.clone()),
            "zend.exception_string_param_max_len" => {
                let parsed = value.parse::<usize>().ok().filter(|n| *n <= 1_000_000)
                    .ok_or_else(|| format!("{}:{}: invalid zend.exception_string_param_max_len", path.display(), line_index + 1))?;
                ("PHP_RUST_EXCEPTION_STRING_PARAM_MAX_LEN", parsed.to_string())
            }
            "root" => ("PHP_RUST_ROOT", value.clone()),
            "bind" => ("PHP_RUST_BIND", value.clone()),
            "max_in_flight" => ("PHP_RUST_MAX_IN_FLIGHT", value.clone()),
            "request_timeout_secs" => ("PHP_RUST_REQUEST_TIMEOUT_SECS", value.clone()),
            "max_body_bytes" => (
                "PHP_RUST_MAX_BODY_BYTES",
                parse_ini_size(&value)
                    .ok_or_else(|| {
                        format!(
                            "{}:{}: invalid max_body_bytes",
                            path.display(),
                            line_index + 1
                        )
                    })?
                    .to_string(),
            ),
            "max_upload_file_bytes" => (
                "PHP_RUST_MAX_UPLOAD_FILE_BYTES",
                parse_ini_size(&value)
                    .ok_or_else(|| {
                        format!(
                            "{}:{}: invalid max_upload_file_bytes",
                            path.display(),
                            line_index + 1
                        )
                    })?
                    .to_string(),
            ),
            "max_execution_steps" => ("PHP_RUST_MAX_EXECUTION_STEPS", value.clone()),
            "max_execution_time" | "max_execution_time_secs" => {
                ("PHP_RUST_MAX_EXECUTION_TIME_SECS", value.clone())
            }
            "max_output_bytes" => (
                "PHP_RUST_MAX_OUTPUT_BYTES",
                parse_ini_size(&value)
                    .ok_or_else(|| {
                        format!(
                            "{}:{}: invalid max_output_bytes",
                            path.display(),
                            line_index + 1
                        )
                    })?
                    .to_string(),
            ),
            "cookie_secure" => ("PHP_RUST_COOKIE_SECURE", value.clone()),
            "log" => ("PHP_RUST_LOG", value.clone()),
            _ => {
                unsupported.insert(key.clone());
                settings.insert(format!("PHP_INI_VALUE:{key}"), value.clone());
                continue;
            }
        };
        settings.insert(format!("PHP_INI_VALUE:{key}"), value.clone());
        settings.insert(target.to_string(), value);
    }
    if !unsupported.is_empty() {
        let mut names = unsupported.into_iter().collect::<Vec<_>>();
        names.sort();
        eprintln!(
            "php-rust-server: PHP INI directives preserved but not applied: {}",
            names.join(", ")
        );
    }
    Ok(())
}

fn expand_ini_variables(
    value: &str,
    settings: &std::collections::HashMap<String, String>,
) -> String {
    let mut expanded = String::with_capacity(value.len());
    let mut cursor = 0;
    while let Some(relative_start) = value[cursor..].find("${") {
        let start = cursor + relative_start;
        expanded.push_str(&value[cursor..start]);
        let expression_start = start + 2;
        let Some(relative_end) = value[expression_start..].find('}') else {
            expanded.push_str(&value[start..]);
            return expanded;
        };
        let end = expression_start + relative_end;
        let expression = &value[expression_start..end];
        let (name, fallback) = expression
            .split_once(":-")
            .map_or((expression, None), |(name, fallback)| {
                (name, Some(fallback))
            });
        let setting_key = format!("PHP_INI_VALUE:{}", name.to_ascii_lowercase());
        let replacement = env::var(name)
            .ok()
            .or_else(|| settings.get(&setting_key).cloned())
            .or_else(|| fallback.map(str::to_owned))
            .unwrap_or_default();
        expanded.push_str(&replacement);
        cursor = end + 1;
    }
    expanded.push_str(&value[cursor..]);
    expanded
}

fn load_ini_files(
    primary: Option<&Path>,
    scan_dirs: Option<&str>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let mut settings = std::collections::HashMap::new();
    if let Some(path) = primary {
        let path = if path.is_dir() {
            path.join("php.ini")
        } else {
            path.to_path_buf()
        };
        parse_ini_into(&path, &mut settings)?;
    }
    if let Some(scan_dirs) = scan_dirs {
        for directory in scan_dirs
            .split(':')
            .filter(|directory| !directory.is_empty())
        {
            let path = Path::new(directory);
            let entries = std::fs::read_dir(path).map_err(|error| {
                format!(
                    "cannot read PHP INI scan directory {}: {error}",
                    path.display()
                )
            })?;
            let mut files = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|file| {
                    file.extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("ini"))
                })
                .collect::<Vec<_>>();
            files.sort();
            for file in files {
                parse_ini_into(&file, &mut settings)?;
            }
        }
    }
    Ok(settings)
}

fn parse_database_connections(
    settings: &std::collections::HashMap<String, String>,
) -> Result<Vec<ConnectionConfig>, String> {
    let mut names = settings
        .keys()
        .filter_map(|key| {
            key.strip_prefix("PHP_RUST_DATABASE:")?
                .split_once(':')
                .map(|(name, _)| name.to_owned())
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut configs = Vec::with_capacity(names.len());
    for name in std::mem::take(&mut names) {
        let value = |key: &str| settings.get(&format!("PHP_RUST_DATABASE:{name}:{key}"));
        let dsn = value("dsn")
            .ok_or_else(|| format!("database '{name}' requires dsn"))?
            .clone();
        let mut config = ConnectionConfig::new(name.clone(), dsn)?;
        config.username = value("username").cloned();
        config.password = value("password").cloned();
        config.driver = value("driver").cloned();
        let parse_number = |key: &str, default: usize| -> Result<usize, String> {
            value(key).map_or(Ok(default), |value| {
                value
                    .parse()
                    .map_err(|_| format!("database '{}': invalid {key}", config.name))
            })
        };
        config.pool = PoolOptions {
            min_connections: parse_number("pool_min", 0)?,
            max_connections: parse_number("pool_max", 10)?,
            acquire_timeout: Duration::from_secs(
                parse_number("pool_acquire_timeout_secs", 5)? as u64
            ),
            idle_timeout: match parse_number("pool_idle_timeout_secs", 600)? {
                0 => None,
                seconds => Some(Duration::from_secs(seconds as u64)),
            },
        };
        config.max_result_rows = parse_number("max_result_rows", 1_000_000)?;
        config.max_column_bytes = parse_number("max_column_bytes", 8 * 1024 * 1024)?;
        config.validate()?;
        configs.push(config);
    }
    Ok(configs)
}

fn build_database_registry(configs: &[ConnectionConfig]) -> Result<DatabaseRegistry, String> {
    let registry = DatabaseRegistry::default();
    for connection in configs {
        let driver: Arc<dyn php_db::DatabaseDriver> = match connection.kind {
            DatabaseKind::PostgreSql => Arc::new(PostgresDriver::new(connection)?),
            DatabaseKind::Mysql | DatabaseKind::MariaDb => {
                Arc::new(php_db::MysqlDriver::new(connection)?)
            }
            DatabaseKind::SqlServer | DatabaseKind::Oracle => {
                Arc::new(php_db::OdbcDriver::new(connection)?)
            }
            DatabaseKind::MongoDb => Arc::new(php_db::MongoDriver::new(connection)?),
        };
        registry.register(connection, driver)?;
    }
    Ok(registry)
}

fn strip_ini_comment(value: &str) -> &str {
    let mut quote = None;
    for (index, character) in value.char_indices() {
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            }
        } else if character == '\'' || character == '"' {
            quote = Some(character);
        } else if character == ';' || character == '#' {
            return &value[..index];
        }
    }
    value
}

fn parse_ini_size(value: &str) -> Option<usize> {
    let value = value.trim();
    let split = value
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(value.len());
    let number = value[..split].parse::<usize>().ok()?;
    let multiplier = match value[split..].trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1usize,
        "k" | "kb" => 1024,
        "m" | "mb" => 1024usize.pow(2),
        "g" | "gb" => 1024usize.pow(3),
        _ => return None,
    };
    number.checked_mul(multiplier)
}

fn parse_php_size(value: &str) -> Option<usize> {
    let size = if value.trim() == "0" {
        64 * 1024 * 1024
    } else {
        parse_ini_size(value)?
    };
    if size > 64 * 1024 * 1024 {
        eprintln!("php-rust-server: configured PHP size limit capped at the server hard maximum of 64 MiB");
    }
    Some(size.min(64 * 1024 * 1024))
}

fn parse_php_timeout(value: &str) -> Option<u64> {
    let seconds = value.trim().parse::<i64>().ok()?;
    Some(if seconds <= 0 {
        300
    } else {
        seconds.min(300) as u64
    })
}

fn parse_multipart_body_limit(value: Option<&str>) -> Result<Option<usize>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value.trim().parse::<i64>() {
        Ok(-1) => Ok(None),
        Ok(limit) if limit >= 0 => Ok(Some(limit as usize)),
        _ => Err("max_multipart_body_parts must be -1 or a non-negative integer".into()),
    }
}

fn parse_args() -> Result<ServerConfig, String> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        usage();
        std::process::exit(0);
    }
    let cli_config = arguments
        .windows(2)
        .find(|pair| pair[0] == "--config")
        .map(|pair| pair[1].clone());
    let config_path = cli_config
        .or_else(|| env::var("PHP_RUST_CONFIG").ok())
        .or_else(|| env::var("PHPRC").ok())
        .or_else(|| {
            env::var("PHP_INI_DIR")
                .ok()
                .map(|directory| format!("{directory}/php.ini"))
        })
        .or_else(|| Path::new("php.ini").is_file().then(|| "php.ini".into()))
        .or_else(|| {
            Path::new("php-rust.ini")
                .is_file()
                .then(|| "php-rust.ini".into())
        });
    let scan_dirs = env::var("PHP_INI_SCAN_DIR").ok().or_else(|| {
        env::var("PHP_INI_DIR")
            .ok()
            .map(|directory| format!("{directory}/conf.d"))
    });
    let config_path = config_path.map(PathBuf::from);
    let settings = load_ini_files(config_path.as_deref(), scan_dirs.as_deref())?;
    let max_execution_time_secs = config_value(
        &settings,
        "PHP_RUST_MAX_EXECUTION_TIME_SECS",
        "PHP_RUST_MAX_EXECUTION_TIME_SECS",
        30u64,
    )?;
    let mut config = ServerConfig {
        root: PathBuf::from(config_value(
            &settings,
            "PHP_RUST_ROOT",
            "PHP_RUST_ROOT",
            "public".to_string(),
        )?),
        bind: config_value(
            &settings,
            "PHP_RUST_BIND",
            "PHP_RUST_BIND",
            "127.0.0.1:3000".parse().expect("valid default address"),
        )?,
        max_in_flight: config_value(
            &settings,
            "PHP_RUST_MAX_IN_FLIGHT",
            "PHP_RUST_MAX_IN_FLIGHT",
            DEFAULT_MAX_IN_FLIGHT,
        )?,
        request_timeout: Duration::from_secs(config_value(
            &settings,
            "PHP_RUST_REQUEST_TIMEOUT_SECS",
            "PHP_RUST_REQUEST_TIMEOUT_SECS",
            DEFAULT_REQUEST_TIMEOUT_SECS,
        )?),
        max_body_bytes: config_value(
            &settings,
            "PHP_RUST_MAX_BODY_BYTES",
            "PHP_RUST_MAX_BODY_BYTES",
            MAX_BODY_BYTES,
        )?,
        max_upload_bytes: config_value(
            &settings,
            "PHP_RUST_MAX_UPLOAD_FILE_BYTES",
            "PHP_RUST_MAX_UPLOAD_FILE_BYTES",
            MAX_BODY_BYTES,
        )?,
        file_uploads: config_bool(&settings, "PHP_RUST_FILE_UPLOADS", true)?,
        max_file_uploads: config_value(
            &settings,
            "PHP_RUST_MAX_FILE_UPLOADS",
            "PHP_RUST_MAX_FILE_UPLOADS",
            DEFAULT_MAX_FILE_UPLOADS,
        )?,
        max_multipart_body_parts: parse_multipart_body_limit(
            env::var("PHP_RUST_MAX_MULTIPART_BODY_PARTS")
                .ok()
                .or_else(|| settings.get("PHP_RUST_MAX_MULTIPART_BODY_PARTS").cloned())
                .as_deref(),
        )?,
        max_input_vars: config_value(
            &settings,
            "PHP_RUST_MAX_INPUT_VARS",
            "PHP_RUST_MAX_INPUT_VARS",
            DEFAULT_MAX_INPUT_VARS,
        )?,
        max_input_nesting_level: config_value(
            &settings,
            "PHP_RUST_MAX_INPUT_NESTING_LEVEL",
            "PHP_RUST_MAX_INPUT_NESTING_LEVEL",
            DEFAULT_MAX_INPUT_NESTING_LEVEL,
        )?,
        include_path: settings
            .get("PHP_INI_VALUE:include_path")
            .map(|value| {
                value
                    .split(PATH_SEPARATOR)
                    .map(|part| PathBuf::from(if part.is_empty() { "." } else { part }))
                    .collect()
            })
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    vec![PathBuf::from(".")]
                } else {
                    vec![PathBuf::from("."), PathBuf::from("/usr/share/php")]
                }
            }),
        exception_string_param_max_len: config_value(
            &settings,
            "PHP_RUST_EXCEPTION_STRING_PARAM_MAX_LEN",
            "PHP_RUST_EXCEPTION_STRING_PARAM_MAX_LEN",
            15usize,
        )?,
        execution_limits: ExecutionLimits {
            max_instructions: config_value(
                &settings,
                "PHP_RUST_MAX_EXECUTION_STEPS",
                "PHP_RUST_MAX_EXECUTION_STEPS",
                ExecutionLimits::default().max_instructions,
            )?,
            max_output_bytes: config_value(
                &settings,
                "PHP_RUST_MAX_OUTPUT_BYTES",
                "PHP_RUST_MAX_OUTPUT_BYTES",
                ExecutionLimits::default().max_output_bytes,
            )?,
            max_execution_time: (max_execution_time_secs != 0)
                .then(|| Duration::from_secs(max_execution_time_secs)),
        },
        secure_session_cookies: config_bool(&settings, "PHP_RUST_COOKIE_SECURE", false)?,
        display_errors: DisplayErrors::parse(
            &env::var("PHP_RUST_DISPLAY_ERRORS")
                .ok()
                .or_else(|| settings.get("PHP_RUST_DISPLAY_ERRORS").cloned())
                .unwrap_or_else(|| "On".into()),
        )?,
        default_mimetype: config_value(
            &settings,
            "PHP_RUST_DEFAULT_MIMETYPE",
            "PHP_RUST_DEFAULT_MIMETYPE",
            "text/html".to_string(),
        )?,
        default_charset: config_value(
            &settings,
            "PHP_RUST_DEFAULT_CHARSET",
            "PHP_RUST_DEFAULT_CHARSET",
            "UTF-8".to_string(),
        )?,
        log_level: config_value(
            &settings,
            "PHP_RUST_LOG",
            "PHP_RUST_LOG",
            "info".to_string(),
        )?,
        database_connections: parse_database_connections(&settings)?,
    };
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                usage();
                std::process::exit(0);
            }
            "--root" => config.root = args.next().ok_or("--root requires a directory")?.into(),
            "--config" => {
                let _ = args.next().ok_or("--config requires a file")?;
            }
            "--bind" => {
                config.bind = args
                    .next()
                    .ok_or("--bind requires an address")?
                    .parse()
                    .map_err(|error| format!("invalid bind address: {error}"))?;
            }
            "--max-in-flight" => config.max_in_flight = parse_option(&mut args, "--max-in-flight")?,
            "--request-timeout-secs" => {
                config.request_timeout =
                    Duration::from_secs(parse_option(&mut args, "--request-timeout-secs")?)
            }
            "--max-body-bytes" => {
                config.max_body_bytes = parse_option(&mut args, "--max-body-bytes")?
            }
            "--max-upload-bytes" => {
                config.max_upload_bytes = parse_option(&mut args, "--max-upload-bytes")?
            }
            "--max-file-uploads" => {
                config.max_file_uploads = parse_option(&mut args, "--max-file-uploads")?
            }
            "--max-multipart-body-parts" => {
                config.max_multipart_body_parts =
                    Some(parse_option(&mut args, "--max-multipart-body-parts")?)
            }
            "--max-input-vars" => {
                config.max_input_vars = parse_option(&mut args, "--max-input-vars")?
            }
            "--max-input-nesting-level" => {
                config.max_input_nesting_level =
                    parse_option(&mut args, "--max-input-nesting-level")?
            }
            "--max-execution-steps" => {
                config.execution_limits.max_instructions =
                    parse_option(&mut args, "--max-execution-steps")?
            }
            "--max-execution-time-secs" => {
                config.execution_limits.max_execution_time =
                    match parse_option::<u64>(&mut args, "--max-execution-time-secs")? {
                        0 => None,
                        seconds => Some(Duration::from_secs(seconds)),
                    }
            }
            "--max-output-bytes" => {
                config.execution_limits.max_output_bytes =
                    parse_option(&mut args, "--max-output-bytes")?
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if !(1..=4096).contains(&config.max_in_flight) {
        return Err("max in-flight requests must be between 1 and 4096".into());
    }
    if config.request_timeout.is_zero() || config.request_timeout > Duration::from_secs(300) {
        return Err("request timeout must be between 1 and 300 seconds".into());
    }
    if config.max_body_bytes == 0 || config.max_body_bytes > 64 * 1024 * 1024 {
        return Err("max body size must be between 1 byte and 64 MiB".into());
    }
    if config.max_upload_bytes > 64 * 1024 * 1024 {
        return Err("max upload file size must be at most 64 MiB".into());
    }
    if config.max_file_uploads > 1000 {
        return Err("max file uploads must be between 0 and 1000".into());
    }
    if config
        .max_multipart_body_parts
        .is_some_and(|limit| limit > 100_000)
    {
        return Err("max multipart body parts must be at most 100000".into());
    }
    if config.max_input_vars == 0 || config.max_input_vars > 10_000 {
        return Err("max input variables must be between 1 and 10000".into());
    }
    if config.max_input_nesting_level > 256 {
        return Err("max input nesting level must be between 0 and 256".into());
    }
    if config.exception_string_param_max_len > 1_000_000 {
        return Err("zend.exception_string_param_max_len must be between 0 and 1000000".into());
    }
    if config.default_mimetype.is_empty()
        || config
            .default_mimetype
            .chars()
            .any(|ch| matches!(ch, '\r' | '\n' | ';'))
        || config
            .default_charset
            .chars()
            .any(|ch| matches!(ch, '\r' | '\n' | ';'))
    {
        return Err("default_mimetype/default_charset contain invalid characters".into());
    }
    if config.execution_limits.max_instructions == 0
        || config.execution_limits.max_instructions > 100_000_000
    {
        return Err("execution step limit must be between 1 and 100 million".into());
    }
    if config.execution_limits.max_output_bytes == 0
        || config.execution_limits.max_output_bytes > 64 * 1024 * 1024
    {
        return Err("output limit must be between 1 byte and 64 MiB".into());
    }
    Ok(config)
}

fn parse_option<T: std::str::FromStr>(
    args: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    args.next()
        .ok_or_else(|| format!("{option} requires a value"))?
        .parse()
        .map_err(|error| format!("invalid value for {option}: {error}"))
}

fn init_logging(configured_level: &str) -> Result<(), String> {
    let level = match configured_level.to_ascii_lowercase().as_str() {
        "error" => 1,
        "warn" => 2,
        "info" => 3,
        "debug" => 4,
        other => {
            return Err(format!(
                "log level must be error, warn, info, or debug; got {other}"
            ))
        }
    };
    let _ = LOG_LEVEL.set(level);
    Ok(())
}

fn log_event(level: u8, name: &str, fields: &str) {
    if level > *LOG_LEVEL.get_or_init(|| 3) {
        return;
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let label = match level {
        1 => "ERROR",
        2 => "WARN",
        3 => "INFO",
        _ => "DEBUG",
    };
    eprintln!("timestamp={timestamp} level={label} event={name} {fields}");
}

fn php_diagnostic_log_fields(diagnostic: &RuntimeDiagnostic) -> String {
    format!(
        "file={:?} line={:?} message={:?}",
        diagnostic.source_file, diagnostic.source_line, diagnostic.message
    )
}

fn render_diagnostic(diagnostic: &RuntimeDiagnostic) -> String {
    let severity = match diagnostic.level {
        php_runtime::DiagnosticLevel::Warning => "Warning",
        php_runtime::DiagnosticLevel::Deprecated => "Deprecated",
        php_runtime::DiagnosticLevel::Notice | php_runtime::DiagnosticLevel::UserNotice => "Notice",
        php_runtime::DiagnosticLevel::UserWarning => "Warning",
        php_runtime::DiagnosticLevel::UserDeprecated => "Deprecated",
        php_runtime::DiagnosticLevel::Fatal => "Fatal error",
    };
    match (&diagnostic.source_file, diagnostic.source_line) {
        (Some(file), Some(line)) => format!(
            "{severity}: {} in {file} on line {line}\n",
            diagnostic.message
        ),
        (Some(file), None) => format!("{severity}: {} in {file}\n", diagnostic.message),
        _ => format!("{severity}: {}\n", diagnostic.message),
    }
}

fn render_diagnostics_inline(output: &str, diagnostics: &[RuntimeDiagnostic]) -> String {
    let mut ordered = diagnostics
        .iter()
        .enumerate()
        .map(|(order, diagnostic)| {
            let mut offset = diagnostic.output_offset.min(output.len());
            while !output.is_char_boundary(offset) {
                offset -= 1;
            }
            (offset, order, render_diagnostic(diagnostic))
        })
        .collect::<Vec<_>>();
    ordered.sort_by_key(|(offset, order, _)| (*offset, *order));
    let mut rendered = String::with_capacity(
        output.len() + ordered.iter().map(|(_, _, text)| text.len()).sum::<usize>(),
    );
    let mut cursor = 0;
    for (offset, _, diagnostic) in ordered {
        let offset = offset.max(cursor);
        rendered.push_str(&output[cursor..offset]);
        rendered.push_str(&diagnostic);
        cursor = offset;
    }
    rendered.push_str(&output[cursor..]);
    rendered
}

async fn handle_connection(
    stream: TcpStream,
    config: Arc<ServerConfig>,
    sessions: SessionStore,
    permit: OwnedSemaphorePermit,
) {
    let _permit = permit;
    let mut reader = BufReader::new(stream);
    let mut requests_served = 0usize;
    loop {
        let request = match tokio::time::timeout(
            config.request_timeout,
            read_request_async(&mut reader, config.max_body_bytes),
        )
        .await
        {
            Ok(Ok(request)) => request,
            Ok(Err((status, message))) => {
                if requests_served > 0 || !message.is_empty() {
                    send_response(
                        reader.get_mut(),
                        status,
                        &[],
                        message.as_bytes(),
                        false,
                        false,
                    )
                    .await;
                }
                return;
            }
            Err(_) => {
                if requests_served == 0 {
                    send_response(reader.get_mut(), 408, &[], b"Request timeout", false, false)
                        .await;
                }
                return;
            }
        };
        requests_served += 1;
        let keep_alive = request.keep_alive && requests_served < MAX_KEEPALIVE_REQUESTS;
        log_event(
            4,
            "request",
            &format!("method={} path={}", request.method, request.path),
        );
        let is_head = request.method == "HEAD";
        let host = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("host"))
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        let content_type = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.as_str())
            .unwrap_or("");

        let root = config.root.clone();
        let limits = config.execution_limits;
        let secure_session_cookies = config.secure_session_cookies;
        let display_errors = config.display_errors;
        let max_upload_bytes = config.max_upload_bytes;
        let file_uploads = config.file_uploads;
        let max_file_uploads = config.max_file_uploads;
        let max_multipart_body_parts = config.max_multipart_body_parts.unwrap_or_else(|| {
            config
                .max_input_vars
                .saturating_add(config.max_file_uploads)
        });
        let max_input_vars = config.max_input_vars;
        let max_input_nesting_level = config.max_input_nesting_level;
        let include_path = config.include_path.clone();
        let exception_string_param_max_len = config.exception_string_param_max_len;
        let default_content_type = format!(
            "{}; charset={}",
            config.default_mimetype, config.default_charset
        );
        let path = request.path;
        let query = request.query;
        let method = request.method;
        let protocol = request.protocol;
        let content_type = content_type.to_owned();
        let host = host.to_owned();
        let request_headers = request.headers;
        let body = request.body;
        let sessions = Arc::clone(&sessions);
        let response = tokio::task::spawn_blocking(move || {
            execute_request_with_session_and_limits_and_include_path(
                &root,
                &path,
                &query,
                &method,
                &protocol,
                &content_type,
                &host,
                &request_headers,
                &body,
                limits,
                sessions,
                secure_session_cookies,
                display_errors,
                max_upload_bytes,
                file_uploads,
                max_file_uploads,
                max_multipart_body_parts,
                max_input_vars,
                max_input_nesting_level,
                &include_path,
                exception_string_param_max_len,
            )
        })
        .await;
        match response {
            Ok(Ok(mut response)) => {
                if !response
                    .headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                {
                    response
                        .headers
                        .push(("Content-Type".into(), default_content_type));
                }
                let body_allowed = !(100..200).contains(&response.status)
                    && response.status != 204
                    && response.status != 304;
                let body = if body_allowed {
                    response.output.as_slice()
                } else {
                    &[]
                };
                send_response(
                    reader.get_mut(),
                    response.status,
                    &response.headers,
                    body,
                    is_head,
                    keep_alive,
                )
                .await;
            }
            Ok(Err(RequestError::NotFound)) => {
                send_response(
                    reader.get_mut(),
                    404,
                    &[],
                    b"PHP script not found",
                    is_head,
                    keep_alive,
                )
                .await
            }
            Ok(Err(RequestError::BadPath)) => {
                send_response(
                    reader.get_mut(),
                    400,
                    &[],
                    b"Invalid request path",
                    is_head,
                    keep_alive,
                )
                .await
            }
            Ok(Err(RequestError::PayloadTooLarge)) => {
                send_response(
                    reader.get_mut(),
                    413,
                    &[],
                    b"File exceeds configured limit",
                    is_head,
                    false,
                )
                .await;
                return;
            }
            Ok(Err(RequestError::InvalidRequest(error))) => {
                log_event(2, "invalid_form_data", &format!("error={error:?}"));
                send_response(
                    reader.get_mut(),
                    400,
                    &[],
                    b"Invalid form data",
                    is_head,
                    false,
                )
                .await;
                return;
            }
            Ok(Err(RequestError::Execution(error))) => {
                log_event(2, "php_execution_error", &format!("error={error:?}"));
                send_response(
                    reader.get_mut(),
                    500,
                    &[],
                    b"PHP execution failed",
                    is_head,
                    false,
                )
                .await;
                return;
            }
            Err(error) => {
                log_event(1, "blocking_task_failed", &format!("error={error:?}"));
                send_response(
                    reader.get_mut(),
                    500,
                    &[],
                    b"Request failed",
                    is_head,
                    false,
                )
                .await;
                return;
            }
        }
        if !keep_alive {
            return;
        }
    }
}

async fn send_response(
    stream: &mut TcpStream,
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
    head_only: bool,
    keep_alive: bool,
) {
    let mut bytes = Vec::new();
    write_response(&mut bytes, status, headers, body, head_only, keep_alive);
    let _ = tokio::time::timeout(Duration::from_secs(15), stream.write_all(&bytes)).await;
}

async fn read_request_async<R: AsyncBufRead + AsyncRead + AsyncWrite + Unpin>(
    reader: &mut R,
    max_body_bytes: usize,
) -> Result<HttpRequest, (u16, &'static str)> {
    let request_line = read_async_bounded_line(reader, 8192)
        .await
        .map_err(|_| (400, "Invalid HTTP request"))?;
    let mut total = request_line.len() + 2;
    let mut header_count = 0;
    let mut content_length = None;
    let mut transfer_encoding = None;
    let mut expect_continue = false;
    let mut headers = Vec::new();
    loop {
        let line = read_async_bounded_line(reader, 8192)
            .await
            .map_err(|_| (400, "Invalid HTTP headers"))?;
        total = total.saturating_add(line.len() + 2);
        if total > MAX_HEADER_BYTES {
            return Err((431, "Request headers too large"));
        }
        if line.is_empty() {
            break;
        }
        header_count += 1;
        if header_count > MAX_HEADER_COUNT {
            return Err((431, "Too many request headers"));
        }
        let (name, value) = line.split_once(':').ok_or((400, "Invalid HTTP header"))?;
        if name.is_empty() || !name.bytes().all(is_http_token_byte) {
            return Err((400, "Invalid HTTP header name"));
        }
        let value = value.trim().to_string();
        if value.contains('\r') || value.contains('\n') {
            return Err((400, "Invalid HTTP header value"));
        }
        if name.eq_ignore_ascii_case("content-length") {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| (400, "Invalid Content-Length"))?;
            if content_length.is_some_and(|previous| previous != parsed) {
                return Err((400, "Conflicting Content-Length headers"));
            }
            content_length = Some(parsed);
        }
        if name.eq_ignore_ascii_case("expect") {
            if expect_continue || !value.eq_ignore_ascii_case("100-continue") {
                return Err((417, "Unsupported expectation"));
            }
            expect_continue = true;
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            if transfer_encoding.is_some() {
                return Err((400, "Conflicting Transfer-Encoding headers"));
            }
            transfer_encoding = Some(value.clone());
        }
        headers.push((name.to_string(), value));
    }
    let chunked = match transfer_encoding.as_deref() {
        None => false,
        Some(value) if value.eq_ignore_ascii_case("identity") => false,
        Some(value) if value.eq_ignore_ascii_case("chunked") => true,
        Some(_) => return Err((501, "Transfer-Encoding is not supported")),
    };
    if chunked && content_length.is_some() {
        return Err((400, "Content-Length with Transfer-Encoding is invalid"));
    }
    if expect_continue && request_line.split_whitespace().nth(2) != Some("HTTP/1.1") {
        return Err((417, "100-continue requires HTTP/1.1"));
    }
    if content_length.is_some_and(|length| length > max_body_bytes) {
        return Err((413, "Request body exceeds configured limit"));
    }
    if expect_continue && (chunked || content_length.unwrap_or(0) > 0) {
        reader
            .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .await
            .map_err(|_| (400, "Cannot send 100 Continue"))?;
    }
    let body = if chunked {
        read_chunked_body(reader, max_body_bytes).await?
    } else {
        let length = content_length.unwrap_or(0);
        if length > max_body_bytes {
            return Err((413, "Request body exceeds configured limit"));
        }
        let mut body = vec![0; length];
        reader
            .read_exact(&mut body)
            .await
            .map_err(|_| (400, "Incomplete request body"))?;
        body
    };
    let mut raw = Vec::with_capacity(request_line.len() + total + body.len() + 32);
    raw.extend_from_slice(request_line.as_bytes());
    raw.extend_from_slice(b"\r\n");
    for (name, value) in headers.iter().filter(|(name, _)| {
        !name.eq_ignore_ascii_case("content-length")
            && !name.eq_ignore_ascii_case("transfer-encoding")
    }) {
        raw.extend_from_slice(name.as_bytes());
        raw.extend_from_slice(b": ");
        raw.extend_from_slice(value.as_bytes());
        raw.extend_from_slice(b"\r\n");
    }
    raw.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
    raw.extend_from_slice(&body);
    parse_request_with_limit(&mut Cursor::new(raw), max_body_bytes)
}

async fn read_chunked_body<R: AsyncBufRead + AsyncRead + Unpin>(
    reader: &mut R,
    max_body_bytes: usize,
) -> Result<Vec<u8>, (u16, &'static str)> {
    let mut body = Vec::new();
    loop {
        let line = read_async_bounded_line(reader, 8192)
            .await
            .map_err(|_| (400, "Invalid chunk size"))?;
        let size = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size, 16).map_err(|_| (400, "Invalid chunk size"))?;
        if size == 0 {
            let mut trailer_bytes = 0usize;
            loop {
                let trailer = read_async_bounded_line(reader, 8192)
                    .await
                    .map_err(|_| (400, "Invalid chunk trailer"))?;
                trailer_bytes = trailer_bytes.saturating_add(trailer.len() + 2);
                if trailer_bytes > MAX_HEADER_BYTES {
                    return Err((431, "Chunk trailers too large"));
                }
                if trailer.is_empty() {
                    return Ok(body);
                }
                if !trailer.contains(':') {
                    return Err((400, "Invalid chunk trailer"));
                }
            }
        }
        if body.len().saturating_add(size) > max_body_bytes {
            return Err((413, "Request body exceeds configured limit"));
        }
        let old_len = body.len();
        body.resize(old_len + size, 0);
        reader
            .read_exact(&mut body[old_len..])
            .await
            .map_err(|_| (400, "Incomplete chunk"))?;
        let mut crlf = [0u8; 2];
        reader
            .read_exact(&mut crlf)
            .await
            .map_err(|_| (400, "Incomplete chunk terminator"))?;
        if crlf != *b"\r\n" {
            return Err((400, "Invalid chunk terminator"));
        }
    }
}

async fn read_async_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max_bytes: usize,
) -> io::Result<String> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        let count = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if bytes.len().saturating_add(count) > max_bytes {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        let has_newline = available[count - 1] == b'\n';
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count);
        if has_newline {
            break;
        }
    }
    bytes.pop();
    if bytes.ends_with(b"\r") {
        bytes.pop();
    }
    String::from_utf8(bytes).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
}

fn parse_request_with_limit(
    reader: &mut impl BufRead,
    max_body_bytes: usize,
) -> Result<HttpRequest, (u16, &'static str)> {
    let mut total_header_bytes = 0usize;
    let request_line =
        read_bounded_line(reader, 8192).map_err(|_| (400, "Invalid HTTP request"))?;
    total_header_bytes += request_line.len();
    let mut fields = request_line.split_ascii_whitespace();
    let method = fields
        .next()
        .ok_or((400, "Invalid HTTP request"))?
        .to_string();
    let target = fields.next().ok_or((400, "Invalid HTTP request"))?;
    let protocol = fields
        .next()
        .ok_or((400, "Invalid HTTP request"))?
        .to_string();
    if fields.next().is_some() || !method.bytes().all(is_http_token_byte) {
        return Err((400, "Invalid HTTP request"));
    }
    if protocol != "HTTP/1.0" && protocol != "HTTP/1.1" {
        return Err((505, "HTTP version not supported"));
    }
    let origin_target = if let Some(authority_path) = target.strip_prefix("http://") {
        authority_path
            .find('/')
            .map(|index| &authority_path[index..])
            .unwrap_or("/")
    } else if target.starts_with('/') {
        target
    } else {
        return Err((400, "Invalid request target"));
    };
    let (path, query) = origin_target.split_once('?').unwrap_or((origin_target, ""));
    let mut headers = Vec::new();
    let mut content_length = None;
    loop {
        let line = read_bounded_line(reader, 8192).map_err(|_| (400, "Invalid HTTP headers"))?;
        total_header_bytes = total_header_bytes.saturating_add(line.len());
        if total_header_bytes > MAX_HEADER_BYTES {
            return Err((431, "Request headers too large"));
        }
        if line.is_empty() {
            break;
        }
        if headers.len() >= MAX_HEADER_COUNT {
            return Err((431, "Too many request headers"));
        }
        let (name, value) = line.split_once(':').ok_or((400, "Invalid HTTP header"))?;
        if name.is_empty() || !name.bytes().all(is_http_token_byte) {
            return Err((400, "Invalid HTTP header name"));
        }
        let value = value.trim().to_string();
        if value.contains('\r') || value.contains('\n') {
            return Err((400, "Invalid HTTP header value"));
        }
        if name.eq_ignore_ascii_case("content-length") {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| (400, "Invalid Content-Length"))?;
            if content_length.is_some_and(|previous| previous != parsed) {
                return Err((400, "Conflicting Content-Length headers"));
            }
            content_length = Some(parsed);
        }
        if name.eq_ignore_ascii_case("transfer-encoding") && !value.eq_ignore_ascii_case("identity")
        {
            return Err((501, "Transfer-Encoding is not supported"));
        }
        headers.push((name.to_string(), value));
    }
    if protocol == "HTTP/1.1"
        && !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("host"))
    {
        return Err((400, "Host header is required for HTTP/1.1"));
    }
    let body_length = content_length.unwrap_or(0);
    if body_length > max_body_bytes {
        return Err((413, "Request body exceeds configured limit"));
    }
    let mut body = vec![0; body_length];
    reader
        .read_exact(&mut body)
        .map_err(|_| (400, "Incomplete request body"))?;
    let has_connection_token = |token: &str| {
        headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("connection"))
            .flat_map(|(_, value)| value.split(','))
            .any(|value| value.trim().eq_ignore_ascii_case(token))
    };
    let keep_alive = !has_connection_token("close")
        && (protocol == "HTTP/1.1" || has_connection_token("keep-alive"));
    Ok(HttpRequest {
        method,
        path: path.to_string(),
        query: query.to_string(),
        protocol,
        headers,
        body,
        keep_alive,
    })
}

fn read_bounded_line(reader: &mut impl BufRead, max_bytes: usize) -> io::Result<String> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if bytes.is_empty() {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
            break;
        }
        let count = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if bytes.len().saturating_add(count) > max_bytes {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        let has_newline = available[count - 1] == b'\n';
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count);
        if has_newline {
            break;
        }
    }
    if !bytes.ends_with(b"\n") {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
    }
    bytes.pop();
    if bytes.ends_with(b"\r") {
        bytes.pop();
    }
    String::from_utf8(bytes).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
}

fn write_response(
    stream: &mut impl Write,
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
    head_only: bool,
    keep_alive: bool,
) {
    let reason = reason_phrase(status);
    let mut response = format!("HTTP/1.1 {status} {reason}\r\n");
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("connection")
            || name.eq_ignore_ascii_case("transfer-encoding")
        {
            continue;
        }
        if name.bytes().all(is_http_token_byte) && !value.contains('\r') && !value.contains('\n') {
            response.push_str(name);
            response.push_str(": ");
            response.push_str(value);
            response.push_str("\r\n");
        }
    }
    response.push_str(&format!(
        "Content-Length: {}\r\nConnection: {}\r\n\r\n",
        body.len(),
        if keep_alive { "keep-alive" } else { "close" }
    ));
    if stream.write_all(response.as_bytes()).is_ok() && !head_only {
        let _ = stream.write_all(body);
    }
    let _ = stream.flush();
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        417 => "Expectation Failed",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        _ => "Response",
    }
}

fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

#[cfg(test)]
fn execute_request_with_session_and_limits(
    root: &Path,
    url_path: &str,
    query: &str,
    method: &str,
    protocol: &str,
    content_type: &str,
    host: &str,
    request_headers: &[(String, String)],
    body: &[u8],
    limits: ExecutionLimits,
    store: SessionStore,
    secure_session_cookie: bool,
    display_errors: DisplayErrors,
    max_upload_bytes: usize,
    file_uploads: bool,
    max_file_uploads: usize,
    max_multipart_body_parts: usize,
    max_input_vars: usize,
    max_input_nesting_level: usize,
) -> Result<WebResponse, RequestError> {
    execute_request_with_session_and_limits_and_include_path(
        root,
        url_path,
        query,
        method,
        protocol,
        content_type,
        host,
        request_headers,
        body,
        limits,
        store,
        secure_session_cookie,
        display_errors,
        max_upload_bytes,
        file_uploads,
        max_file_uploads,
        max_multipart_body_parts,
        max_input_vars,
        max_input_nesting_level,
        &[],
        15,
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_request_with_session_and_limits_and_include_path(
    root: &Path,
    url_path: &str,
    query: &str,
    method: &str,
    protocol: &str,
    content_type: &str,
    host: &str,
    request_headers: &[(String, String)],
    body: &[u8],
    limits: ExecutionLimits,
    store: SessionStore,
    secure_session_cookie: bool,
    display_errors: DisplayErrors,
    max_upload_bytes: usize,
    file_uploads: bool,
    max_file_uploads: usize,
    max_multipart_body_parts: usize,
    max_input_vars: usize,
    max_input_nesting_level: usize,
    include_path: &[PathBuf],
    exception_string_param_max_len: usize,
) -> Result<WebResponse, RequestError> {
    execute_request_with_context_and_include_path(
        root,
        url_path,
        query,
        method,
        protocol,
        content_type,
        host,
        request_headers,
        body,
        limits,
        Some(store),
        secure_session_cookie,
        display_errors,
        max_upload_bytes,
        file_uploads,
        max_file_uploads,
        max_multipart_body_parts,
        max_input_vars,
        max_input_nesting_level,
        include_path,
        exception_string_param_max_len,
    )
}

#[cfg(test)]
fn execute_request_with_context(
    root: &Path,
    url_path: &str,
    query: &str,
    method: &str,
    protocol: &str,
    content_type: &str,
    host: &str,
    request_headers: &[(String, String)],
    body: &[u8],
    limits: ExecutionLimits,
    session_store: Option<SessionStore>,
    secure_session_cookie: bool,
    display_errors: DisplayErrors,
    max_upload_bytes: usize,
    file_uploads: bool,
    max_file_uploads: usize,
    max_multipart_body_parts: usize,
    max_input_vars: usize,
    max_input_nesting_level: usize,
) -> Result<WebResponse, RequestError> {
    execute_request_with_context_and_include_path(
        root,
        url_path,
        query,
        method,
        protocol,
        content_type,
        host,
        request_headers,
        body,
        limits,
        session_store,
        secure_session_cookie,
        display_errors,
        max_upload_bytes,
        file_uploads,
        max_file_uploads,
        max_multipart_body_parts,
        max_input_vars,
        max_input_nesting_level,
        &[],
        15,
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_request_with_context_and_include_path(
    root: &Path,
    url_path: &str,
    query: &str,
    method: &str,
    protocol: &str,
    content_type: &str,
    host: &str,
    request_headers: &[(String, String)],
    body: &[u8],
    limits: ExecutionLimits,
    session_store: Option<SessionStore>,
    secure_session_cookie: bool,
    display_errors: DisplayErrors,
    max_upload_bytes: usize,
    file_uploads: bool,
    max_file_uploads: usize,
    max_multipart_body_parts: usize,
    max_input_vars: usize,
    max_input_nesting_level: usize,
    include_path: &[PathBuf],
    exception_string_param_max_len: usize,
) -> Result<WebResponse, RequestError> {
    let decoded_path = decode_url_path(url_path).ok_or(RequestError::BadPath)?;
    let trimmed = decoded_path
        .strip_prefix('/')
        .ok_or(RequestError::BadPath)?;
    if trimmed.contains('\\')
        || trimmed
            .split('/')
            .any(|segment| segment == ".." || segment == ".")
    {
        return Err(RequestError::BadPath);
    }
    let requested = if trimmed.is_empty() {
        PathBuf::from("index.php")
    } else {
        PathBuf::from(trimmed)
    };
    let candidate = root.join(&requested);
    let actual = if candidate.is_dir() {
        let php_index = candidate.join("index.php");
        if php_index.is_file() {
            php_index
        } else {
            candidate.join("index.html")
        }
    } else if candidate.is_file() {
        candidate.clone()
    } else {
        root.join("index.php")
    };
    let file = actual.canonicalize().map_err(|_| RequestError::NotFound)?;
    if !file.starts_with(root) || !file.is_file() {
        return Err(RequestError::NotFound);
    }

    if file.extension().and_then(|ext| ext.to_str()) != Some("php") {
        if file
            .metadata()
            .map_err(|error| RequestError::Execution(error.to_string()))?
            .len()
            > MAX_STATIC_FILE_BYTES
        {
            return Err(RequestError::PayloadTooLarge);
        }
        let data =
            std::fs::read(&file).map_err(|error| RequestError::Execution(error.to_string()))?;
        return Ok(WebResponse {
            output: data,
            status: 200,
            headers: vec![("content-type".into(), mime_type(&file))],
        });
    }

    if file
        .metadata()
        .map_err(|error| RequestError::Execution(error.to_string()))?
        .len()
        > MAX_PHP_SOURCE_BYTES
    {
        return Err(RequestError::PayloadTooLarge);
    }
    let session = session_store
        .map(|store| load_session_context(request_headers, &store).map(|session| (store, session)))
        .transpose()?;
    let mut input_diagnostics = Vec::new();
    let get = parse_urlencoded(
        query,
        max_input_vars,
        max_input_nesting_level,
        &mut input_diagnostics,
    );
    let (post, files, upload_paths) = parse_post_data(
        content_type,
        body,
        max_upload_bytes,
        file_uploads,
        max_file_uploads,
        max_multipart_body_parts,
        max_input_vars,
        max_input_nesting_level,
        &mut input_diagnostics,
    )?;
    let cookies = request_headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        .map(|(_, value)| parse_cookies(value))
        .unwrap_or_default();
    let mut request = get.clone();
    for (key, value) in &post {
        set_entry(&mut request, key.clone(), value.clone());
    }
    for (key, value) in &cookies {
        set_entry(&mut request, key.clone(), value.clone());
    }
    let script_name = format!(
        "/{}",
        file.strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/")
    );
    let path_info = if actual != candidate && !trimmed.is_empty() {
        format!("/{trimmed}")
    } else {
        String::new()
    };
    let mut globals = std::collections::HashMap::new();
    globals.insert("_GET".into(), make_array(get));
    globals.insert("_POST".into(), make_array(post));
    globals.insert("_COOKIE".into(), make_array(cookies));
    globals.insert("_REQUEST".into(), make_array(request));
    globals.insert("_FILES".into(), make_array(files));
    globals.insert("_ENV".into(), make_array(Vec::new()));
    let session_id = session
        .as_ref()
        .map(|(_, session)| session.id.as_str())
        .unwrap_or("");
    let initial_session = session
        .as_ref()
        .and_then(|(_, session)| session.data.as_ref())
        .map(session_to_value)
        .unwrap_or_else(|| make_array(Vec::new()));
    globals.insert("_SESSION".into(), initial_session);
    globals.insert(
        "\0php_input".into(),
        Value::String(String::from_utf8_lossy(body).into_owned()),
    );
    globals.insert(
        "\0document_root".into(),
        Value::String(root.display().to_string()),
    );
    globals.insert(
        "\0upload_paths".into(),
        Value::String(
            upload_paths
                .iter()
                .map(|path| path.to_string_lossy())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    );
    globals.insert("\0session_id".into(), Value::String(session_id.into()));
    globals.insert(
        "_SERVER".into(),
        make_array(vec![
            ("REQUEST_METHOD".into(), Value::String(method.into())),
            (
                "REQUEST_URI".into(),
                Value::String(if query.is_empty() {
                    url_path.into()
                } else {
                    format!("{url_path}?{query}")
                }),
            ),
            ("QUERY_STRING".into(), Value::String(query.into())),
            ("SCRIPT_NAME".into(), Value::String(script_name.clone())),
            (
                "SCRIPT_FILENAME".into(),
                Value::String(file.display().to_string()),
            ),
            (
                "PHP_SELF".into(),
                Value::String(format!("{script_name}{path_info}")),
            ),
            ("PATH_INFO".into(), Value::String(path_info)),
            (
                "DOCUMENT_ROOT".into(),
                Value::String(root.display().to_string()),
            ),
            ("HTTP_HOST".into(), Value::String(host.into())),
            (
                "SERVER_NAME".into(),
                Value::String(host.split(':').next().unwrap_or(host).into()),
            ),
            ("REQUEST_SCHEME".into(), Value::String("http".into())),
            ("HTTPS".into(), Value::String(String::new())),
            ("CONTENT_TYPE".into(), Value::String(content_type.into())),
            (
                "CONTENT_LENGTH".into(),
                Value::String(body.len().to_string()),
            ),
            ("SERVER_PROTOCOL".into(), Value::String(protocol.into())),
        ]),
    );
    for (name, value) in request_headers {
        if name.eq_ignore_ascii_case("content-type") || name.eq_ignore_ascii_case("content-length")
        {
            continue;
        }
        let key = format!("HTTP_{}", name.replace('-', "_").to_ascii_uppercase());
        let server = globals.get_mut("_SERVER").expect("server globals exist");
        if let Value::Array(entries) = server {
            entries.set(ArrayKey::String(key), Value::String(value.clone()));
        }
    }
    let include_root = root.parent().unwrap_or(root);
    globals.insert(
        "\0filesystem_root".into(),
        Value::String(root.display().to_string()),
    );
    globals.insert("\0sapi_name".into(), Value::String("cli-server".into()));
    globals.insert(
        "\0zend.exception_string_param_max_len".into(),
        Value::Integer(exception_string_param_max_len as i64),
    );
    for diagnostic in &mut input_diagnostics {
        diagnostic.source_file = Some(file.display().to_string());
    }
    for diagnostic in &input_diagnostics {
        log_event(
            2,
            "php_input_warning",
            &php_diagnostic_log_fields(diagnostic),
        );
    }
    let execution = execute_file_with_globals_limits_diagnostics_and_include_path(
        &file,
        include_root,
        globals,
        limits,
        input_diagnostics,
        include_path,
    );
    for path in upload_paths {
        let _ = std::fs::remove_file(path);
    }
    let mut execution = execution.map_err(RequestError::Execution)?;
    if display_errors != DisplayErrors::Off {
        let rendered = execution
            .diagnostics
            .iter()
            .map(render_diagnostic)
            .collect::<String>();
        if display_errors == DisplayErrors::On {
            execution.output = render_diagnostics_inline(&execution.output, &execution.diagnostics);
        } else if !rendered.is_empty() {
            eprint!("{rendered}");
        }
    }
    if execution.session_started {
        if let Some((store, session)) = session {
            let session_id = execution.session_id.clone();
            let destroyed = execution.session_destroyed;
            let session_data = execution.session_data.clone();
            commit_session(
                &store,
                &session,
                &session_id,
                destroyed,
                session_data,
                &mut execution.headers,
                secure_session_cookie,
            )?;
        }
    }
    Ok(WebResponse {
        output: execution.output.into_bytes(),
        status: execution.status_code.unwrap_or(200),
        headers: execution.headers,
    })
}

fn make_array(entries: Vec<(String, Value)>) -> Value {
    Value::from_array_entries(
        entries
            .into_iter()
            .map(|(key, value)| (ArrayKey::String(key), value))
            .collect(),
    )
}

fn set_entry(entries: &mut Vec<(String, Value)>, key: String, value: Value) {
    if let Some((_, old)) = entries.iter_mut().find(|(existing, _)| *existing == key) {
        *old = value;
    } else {
        entries.push((key, value));
    }
}

fn insert_file_param(
    files: &mut Vec<(String, Value)>,
    name: &str,
    file: Value,
    max_nesting_level: usize,
) -> bool {
    let (base, rest) = name.split_once('[').unwrap_or((name, ""));
    if base.is_empty() {
        return false;
    }
    if rest.is_empty() {
        set_entry(files, base.into(), file);
        return false;
    }
    let mut keys = Vec::new();
    let mut remaining = rest;
    while let Some(end) = remaining.find(']') {
        let part = &remaining[..end];
        keys.push(if part.is_empty() {
            None
        } else {
            Some(part.to_string())
        });
        remaining = &remaining[end + 1..];
        if let Some(next) = remaining.strip_prefix('[') {
            remaining = next;
        } else {
            break;
        }
    }
    if !remaining.is_empty() || keys.is_empty() {
        return false;
    }
    if keys.len() > max_nesting_level {
        return true;
    }
    let current = files
        .iter()
        .find(|(key, _)| key == base)
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| Value::from_array_entries(Vec::new()));
    let mut group = match current {
        Value::Array(array) => array.to_vec(),
        _ => Vec::new(),
    };
    let Value::Array(file_fields) = file else {
        return false;
    };
    for (attribute, value) in file_fields.iter().cloned() {
        let ArrayKey::String(attribute) = attribute else {
            continue;
        };
        let key = ArrayKey::String(attribute);
        let previous = group
            .iter()
            .find(|(candidate, _)| candidate == &key)
            .map(|(_, value)| value.clone())
            .unwrap_or(Value::Null);
        let updated = assign_nested(previous, &keys, value);
        if let Some((_, value)) = group.iter_mut().find(|(candidate, _)| candidate == &key) {
            *value = updated;
        } else {
            group.push((key, updated));
        }
    }
    set_entry(files, base.into(), Value::from_array_entries(group));
    false
}

struct SessionContext {
    id: String,
    data: Option<SessionValue>,
    cookie_id: Option<String>,
}

fn session_to_value(value: &SessionValue) -> Value {
    match value {
        SessionValue::Null => Value::Null,
        SessionValue::Integer(value) => Value::Integer(*value),
        SessionValue::Number(value) => Value::Number(*value),
        SessionValue::String(value) => Value::String(value.clone()),
        SessionValue::Bool(value) => Value::Bool(*value),
        SessionValue::Array(entries) => Value::from_array_entries(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), session_to_value(value)))
                .collect(),
        ),
    }
}

fn measure_session_value(value: &SessionValue, depth: usize, total: &mut usize) -> bool {
    if depth > 32 {
        return false;
    }
    let amount = match value {
        SessionValue::Null
        | SessionValue::Bool(_)
        | SessionValue::Integer(_)
        | SessionValue::Number(_) => 16,
        SessionValue::String(value) => value.len().saturating_add(16),
        SessionValue::Array(entries) => {
            for (key, value) in entries {
                let key_bytes = match key {
                    ArrayKey::Integer(_) => 8,
                    ArrayKey::String(key) => key.len(),
                };
                *total = total.saturating_add(key_bytes + 16);
                if *total > MAX_SESSION_BYTES || !measure_session_value(value, depth + 1, total) {
                    return false;
                }
            }
            0
        }
    };
    *total = total.saturating_add(amount);
    *total <= MAX_SESSION_BYTES
}

fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}

fn generate_session_id() -> Result<String, RequestError> {
    let mut random = [0u8; 16];
    #[cfg(unix)]
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut random))
        .map_err(|error| RequestError::Execution(format!("cannot generate session id: {error}")))?;
    #[cfg(not(unix))]
    return Err(RequestError::Execution(
        "secure session IDs are unavailable on this platform".into(),
    ));
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn load_session_context(
    headers: &[(String, String)],
    store: &SessionStore,
) -> Result<SessionContext, RequestError> {
    let cookie_id = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        .map(|(_, value)| parse_cookies(value))
        .and_then(|cookies| cookies.into_iter().find(|(key, _)| key == "PHPSESSID"))
        .map(|(_, value)| match value {
            Value::String(value) => value,
            _ => String::new(),
        })
        .filter(|id| valid_session_id(id));
    let now = Instant::now();
    let mut sessions = store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    sessions.retain(|_, entry| now.duration_since(entry.last_access) < SESSION_TTL);
    let data = cookie_id.as_ref().and_then(|id| {
        sessions.get_mut(id).map(|entry| {
            entry.last_access = now;
            entry.data.clone()
        })
    });
    let id = if data.is_some() {
        cookie_id.clone().unwrap()
    } else {
        generate_session_id()?
    };
    Ok(SessionContext {
        id,
        data,
        cookie_id,
    })
}

fn commit_session(
    store: &SessionStore,
    context: &SessionContext,
    session_id: &str,
    destroyed: bool,
    data: Option<SessionValue>,
    headers: &mut Vec<(String, String)>,
    secure_cookie: bool,
) -> Result<(), RequestError> {
    let mut sessions = store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let old_id = context.cookie_id.as_deref().unwrap_or(&context.id);
    if destroyed {
        sessions.remove(old_id);
        sessions.remove(&context.id);
        sessions.remove(session_id);
        let secure = if secure_cookie { "; Secure" } else { "" };
        headers.push((
            "Set-Cookie".into(),
            format!("PHPSESSID=deleted; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}"),
        ));
        return Ok(());
    }
    let session_id = if session_id.is_empty() {
        &context.id
    } else {
        session_id
    };
    if !valid_session_id(session_id) {
        return Err(RequestError::Execution(
            "session_id(): invalid session identifier".into(),
        ));
    }
    let data = data.ok_or_else(|| {
        RequestError::Execution(
            "session contains unsupported values or exceeds the session data limit".into(),
        )
    })?;
    let mut session_bytes = 0usize;
    if !measure_session_value(&data, 0, &mut session_bytes) {
        return Err(RequestError::Execution(
            "session data exceeds the 128 KiB per-session limit".into(),
        ));
    }
    let now = Instant::now();
    sessions.retain(|_, entry| now.duration_since(entry.last_access) < SESSION_TTL);
    let total_bytes = sessions.values().map(|entry| entry.bytes).sum::<usize>();
    let replace_bytes = if old_id == session_id {
        sessions
            .get(session_id)
            .map(|entry| entry.bytes)
            .unwrap_or(0)
    } else {
        sessions
            .get(old_id)
            .map(|entry| entry.bytes)
            .unwrap_or(0)
            .saturating_add(
                sessions
                    .get(session_id)
                    .map(|entry| entry.bytes)
                    .unwrap_or(0),
            )
    };
    if total_bytes
        .saturating_sub(replace_bytes)
        .saturating_add(session_bytes)
        > MAX_SESSION_STORE_BYTES
    {
        return Err(RequestError::Execution(
            "session store reached its 64 MiB memory limit".into(),
        ));
    }
    if old_id != session_id {
        sessions.remove(old_id);
    }
    if !sessions.contains_key(session_id) && sessions.len() >= MAX_SESSIONS {
        return Err(RequestError::Execution("session store is full".into()));
    }
    sessions.insert(
        session_id.into(),
        SessionEntry {
            data,
            bytes: session_bytes,
            last_access: now,
        },
    );
    let secure = if secure_cookie { "; Secure" } else { "" };
    headers.push((
        "Set-Cookie".into(),
        format!("PHPSESSID={session_id}; Path=/; HttpOnly; SameSite=Lax{secure}"),
    ));
    Ok(())
}

fn parse_post_data(
    content_type: &str,
    body: &[u8],
    max_upload_bytes: usize,
    file_uploads: bool,
    max_file_uploads: usize,
    max_multipart_body_parts: usize,
    max_input_vars: usize,
    max_nesting_level: usize,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) -> Result<(Vec<(String, Value)>, Vec<(String, Value)>, Vec<PathBuf>), RequestError> {
    if content_type
        .to_ascii_lowercase()
        .starts_with("application/x-www-form-urlencoded")
    {
        return Ok((
            parse_urlencoded(
                &String::from_utf8_lossy(body),
                max_input_vars,
                max_nesting_level,
                diagnostics,
            ),
            Vec::new(),
            Vec::new(),
        ));
    }
    if !content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    }
    let boundary = content_type
        .split(';')
        .skip(1)
        .find_map(|parameter| {
            let (name, value) = parameter.trim().split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case("boundary")
                .then(|| value.trim().trim_matches('"').to_string())
        })
        .ok_or_else(|| RequestError::InvalidRequest("multipart boundary is missing".into()))?;
    if boundary.is_empty()
        || boundary.len() > 70
        || !boundary
            .bytes()
            .all(|byte| (0x21..=0x7e).contains(&byte) && byte != b'"')
    {
        return Err(RequestError::InvalidRequest(
            "invalid multipart boundary".into(),
        ));
    }
    let marker = format!("--{boundary}");
    if !body.starts_with(marker.as_bytes()) {
        return Err(RequestError::InvalidRequest(
            "multipart body does not start with its boundary".into(),
        ));
    }
    let delimiter = format!("\r\n{marker}");
    let mut cursor = marker.len();
    let mut post = Vec::new();
    let mut files = Vec::new();
    let mut upload_guard = UploadTempGuard(Vec::new());
    let mut max_form_upload_size = 0i64;
    let mut accepted_uploads = 0usize;
    let mut upload_limit_reported = false;
    if body.get(cursor..cursor + 2) == Some(b"--") {
        return Ok((post, files, Vec::new()));
    }
    if body.get(cursor..cursor + 2) != Some(b"\r\n") {
        return Err(RequestError::InvalidRequest(
            "invalid multipart boundary terminator".into(),
        ));
    }
    cursor += 2;
    let mut input_variables = 0usize;
    let mut input_limit_reported = false;
    let mut multipart_parts = 0usize;
    loop {
        multipart_parts += 1;
        if multipart_parts > max_multipart_body_parts {
            diagnostics.push(RuntimeDiagnostic {
                level: php_runtime::DiagnosticLevel::Warning,
                message: format!("Multipart body parts limit exceeded {max_multipart_body_parts}. To increase the limit change max_multipart_body_parts in php.ini."),
                source_file: None,
                source_line: None,
                output_offset: 0,
            });
            break;
        }
        let header_end = find_bytes(&body[cursor..], b"\r\n\r\n")
            .map(|index| cursor + index)
            .ok_or_else(|| {
                RequestError::InvalidRequest("multipart field headers are incomplete".into())
            })?;
        let header_text = std::str::from_utf8(&body[cursor..header_end]).map_err(|_| {
            RequestError::InvalidRequest("multipart headers are not valid UTF-8".into())
        })?;
        let data_start = header_end + 4;
        let next_boundary = find_bytes(&body[data_start..], delimiter.as_bytes())
            .map(|index| data_start + index)
            .ok_or_else(|| {
                RequestError::InvalidRequest("multipart closing boundary is missing".into())
            })?;
        let part_data = &body[data_start..next_boundary];
        let disposition = header_text
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-disposition")
                    .then_some(value.trim())
            })
            .ok_or_else(|| {
                RequestError::InvalidRequest("multipart Content-Disposition is missing".into())
            })?;
        let (field_name, filename) = parse_disposition(disposition)?;
        let filename = filename.map(|filename| uploaded_filename(&filename).to_string());
        let part_content_type = header_text
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-type")
                    .then(|| value.trim().to_string())
            })
            .unwrap_or_else(|| "text/plain".into());
        if let Some(filename) = filename {
            if !file_uploads {
                cursor = next_boundary + delimiter.len();
                if body.get(cursor..cursor + 2) == Some(b"--") {
                    break;
                }
                if body.get(cursor..cursor + 2) != Some(b"\r\n") {
                    return Err(RequestError::InvalidRequest(
                        "invalid multipart boundary separator".into(),
                    ));
                }
                cursor += 2;
                continue;
            }
            if !filename.is_empty() {
                accepted_uploads += 1;
                if accepted_uploads > max_file_uploads {
                    if !upload_limit_reported {
                        diagnostics.push(RuntimeDiagnostic {
                            level: php_runtime::DiagnosticLevel::Warning,
                            message: "Maximum number of allowable file uploads has been exceeded"
                                .into(),
                            source_file: None,
                            source_line: None,
                            output_offset: 0,
                        });
                        upload_limit_reported = true;
                    }
                    cursor = next_boundary + delimiter.len();
                    if body.get(cursor..cursor + 2) == Some(b"--") {
                        break;
                    }
                    if body.get(cursor..cursor + 2) != Some(b"\r\n") {
                        return Err(RequestError::InvalidRequest(
                            "invalid multipart boundary separator".into(),
                        ));
                    }
                    cursor += 2;
                    continue;
                }
            }
            let (temp_name, upload_type, error, size) = if filename.is_empty() {
                (String::new(), String::new(), 4.0, 0.0) // UPLOAD_ERR_NO_FILE
            } else if part_data.len() > max_upload_bytes {
                (String::new(), String::new(), 1.0, 0.0) // UPLOAD_ERR_INI_SIZE
            } else if max_form_upload_size != 0
                && part_data.len() as i128 > max_form_upload_size as i128
            {
                (String::new(), String::new(), 2.0, 0.0) // UPLOAD_ERR_FORM_SIZE
            } else {
                let path = write_upload_temp(part_data)?;
                upload_guard.0.push(path.clone());
                (
                    path.to_string_lossy().into_owned(),
                    part_content_type,
                    0.0,
                    part_data.len() as f64,
                )
            };
            let entry = Value::from_array_entries(vec![
                (ArrayKey::String("name".into()), Value::String(filename)),
                (ArrayKey::String("type".into()), Value::String(upload_type)),
                (
                    ArrayKey::String("tmp_name".into()),
                    Value::String(temp_name),
                ),
                (ArrayKey::String("error".into()), Value::Number(error)),
                (ArrayKey::String("size".into()), Value::Number(size)),
            ]);
            if insert_file_param(
                &mut files,
                &normalize_param_name(&field_name),
                entry,
                max_nesting_level,
            ) {
                diagnostics.push(input_nesting_diagnostic(max_nesting_level));
            }
        } else {
            let value = String::from_utf8_lossy(part_data).into_owned();
            input_variables += 1;
            if input_variables <= max_input_vars {
                if field_name.eq_ignore_ascii_case("MAX_FILE_SIZE") {
                    max_form_upload_size = parse_form_upload_size(&value);
                }
                if insert_param(
                    &mut post,
                    &normalize_param_name(&field_name),
                    Value::String(value),
                    max_nesting_level,
                ) {
                    diagnostics.push(input_nesting_diagnostic(max_nesting_level));
                }
            } else if !input_limit_reported {
                diagnostics.push(input_vars_diagnostic(max_input_vars));
                input_limit_reported = true;
            }
        }
        cursor = next_boundary + delimiter.len();
        if body.get(cursor..cursor + 2) == Some(b"--") {
            break;
        }
        if body.get(cursor..cursor + 2) != Some(b"\r\n") {
            return Err(RequestError::InvalidRequest(
                "invalid multipart boundary separator".into(),
            ));
        }
        cursor += 2;
    }
    Ok((post, files, std::mem::take(&mut upload_guard.0)))
}

fn parse_form_upload_size(value: &str) -> i64 {
    let bytes = value.as_bytes();
    let mut index = 0;
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }
    let negative = match bytes.get(index) {
        Some(b'-') => {
            index += 1;
            true
        }
        Some(b'+') => {
            index += 1;
            false
        }
        _ => false,
    };
    let limit = if negative {
        i64::MAX as u128 + 1
    } else {
        i64::MAX as u128
    };
    let mut magnitude = 0u128;
    let mut has_digits = false;
    while let Some(byte @ b'0'..=b'9') = bytes.get(index).copied() {
        has_digits = true;
        magnitude = magnitude
            .saturating_mul(10)
            .saturating_add((byte - b'0') as u128)
            .min(limit);
        index += 1;
    }
    if !has_digits {
        return 0;
    }
    if negative {
        if magnitude == i64::MAX as u128 + 1 {
            i64::MIN
        } else {
            -(magnitude as i64)
        }
    } else {
        magnitude as i64
    }
}

fn parse_disposition(value: &str) -> Result<(String, Option<String>), RequestError> {
    let mut parts = value.split(';');
    if !parts
        .next()
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("form-data"))
    {
        return Err(RequestError::InvalidRequest(
            "unsupported multipart disposition".into(),
        ));
    }
    let mut name = None;
    let mut filename = None;
    for part in parts {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').replace("\\\"", "\"");
        if key.trim().eq_ignore_ascii_case("name") {
            name = Some(value);
        } else if key.trim().eq_ignore_ascii_case("filename") {
            filename = Some(value);
        }
    }
    let name = name
        .filter(|name| !name.is_empty())
        .ok_or_else(|| RequestError::InvalidRequest("multipart field name is missing".into()))?;
    Ok((name, filename))
}

fn uploaded_filename(filename: &str) -> &str {
    filename.rsplit(['/', '\\']).next().unwrap_or(filename)
}

fn write_upload_temp(data: &[u8]) -> Result<PathBuf, RequestError> {
    for _ in 0..8 {
        let id = NEXT_UPLOAD_ID.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!("php-rust-upload-{}-{id}", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
                }
                if let Err(error) = file.write_all(data) {
                    let _ = std::fs::remove_file(&path);
                    return Err(RequestError::Execution(format!(
                        "cannot write upload temporary file: {error}"
                    )));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(RequestError::Execution(format!(
                    "cannot create upload temporary file: {error}"
                )))
            }
        }
    }
    Err(RequestError::Execution(
        "cannot allocate a unique upload temporary file".into(),
    ))
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn parse_urlencoded(
    input: &str,
    max_variables: usize,
    max_nesting_level: usize,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) -> Vec<(String, Value)> {
    let parts = input
        .split('&')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() > max_variables {
        diagnostics.push(input_vars_diagnostic(max_variables));
    }
    parts
        .into_iter()
        .take(max_variables)
        .filter_map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            Some((url_decode(key)?, Value::String(url_decode(value)?)))
        })
        .fold(Vec::new(), |mut entries, (key, value)| {
            if insert_param(
                &mut entries,
                &normalize_param_name(&key),
                value,
                max_nesting_level,
            ) {
                diagnostics.push(input_nesting_diagnostic(max_nesting_level));
            }
            entries
        })
}

fn input_vars_diagnostic(max_variables: usize) -> RuntimeDiagnostic {
    RuntimeDiagnostic {
        level: php_runtime::DiagnosticLevel::Warning,
        message: format!("Input variables exceeded {max_variables}. To increase the limit change max_input_vars in php.ini."),
        source_file: None,
        source_line: None,
        output_offset: 0,
    }
}

fn input_nesting_diagnostic(max_nesting_level: usize) -> RuntimeDiagnostic {
    RuntimeDiagnostic {
        level: php_runtime::DiagnosticLevel::Warning,
        message: format!("Input variable nesting level exceeded {max_nesting_level}. To increase the limit change max_input_nesting_level in php.ini."),
        source_file: None,
        source_line: None,
        output_offset: 0,
    }
}

fn normalize_param_name(name: &str) -> String {
    let name = name.trim_start_matches(' ');
    if let Some(open) = name.find('[') {
        if !name[open + 1..].contains(']') {
            return name
                .chars()
                .map(|character| match character {
                    '.' | ' ' | '[' => '_',
                    _ => character,
                })
                .collect();
        }
    }
    let base_end = name.find('[').unwrap_or(name.len());
    let base = name[..base_end].replace('.', "_").replace(' ', "_");
    format!("{base}{}", &name[base_end..])
}

fn insert_param(
    entries: &mut Vec<(String, Value)>,
    name: &str,
    value: Value,
    max_nesting_level: usize,
) -> bool {
    if name.len() > 1024 || name.is_empty() {
        return false;
    }
    let (base, rest) = name.split_once('[').unwrap_or((name, ""));
    if base.is_empty() {
        return false;
    }
    if rest.is_empty() {
        set_entry(entries, base.into(), value);
        return false;
    }
    let mut keys = vec![Some(base.to_string())];
    let mut remaining = rest;
    while let Some(end) = remaining.find(']') {
        let key = &remaining[..end];
        keys.push(if key.is_empty() {
            None
        } else {
            Some(key.to_string())
        });
        if keys.len() > max_nesting_level + 1 {
            return true;
        }
        remaining = &remaining[end + 1..];
        if let Some(next) = remaining.strip_prefix('[') {
            remaining = next;
        } else {
            break;
        }
    }
    if !remaining.is_empty() || keys.len() < 2 {
        return false;
    }
    let top = keys.remove(0).unwrap();
    let current = entries
        .iter()
        .find(|(key, _)| key == &top)
        .map(|(_, value)| value.clone())
        .unwrap_or(Value::Null);
    let updated = assign_nested(current, &keys, value);
    set_entry(entries, top, updated);
    false
}

fn assign_nested(current: Value, keys: &[Option<String>], value: Value) -> Value {
    let Some((key, tail)) = keys.split_first() else {
        return value;
    };
    let mut array = match current {
        Value::Array(array) => array,
        _ => php_runtime::PhpArray::from_entries(Vec::new()),
    };
    if key.is_none() && tail.is_empty() {
        let _ = array.append(value);
    } else {
        let array_key = key
            .as_deref()
            .map(to_array_key)
            .unwrap_or_else(|| ArrayKey::Integer(array.len() as i64));
        let previous = array
            .iter()
            .find(|(candidate, _)| candidate == &array_key)
            .map(|(_, value)| value.clone())
            .unwrap_or(Value::Null);
        array.set(array_key, assign_nested(previous, tail, value));
    }
    Value::Array(array)
}

fn to_array_key(key: &str) -> ArrayKey {
    match key.parse::<i64>() {
        Ok(index) if index.to_string() == key => ArrayKey::Integer(index),
        _ => ArrayKey::String(key.into()),
    }
}

fn url_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                output.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                output.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'%' => return None,
            byte => {
                output.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(output).ok()
}

fn decode_url_path(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            let byte = u8::from_str_radix(hex, 16).ok()?;
            if byte == 0 {
                return None;
            }
            output.push(byte);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).ok()
}

fn parse_cookies(header: &str) -> Vec<(String, Value)> {
    header
        .split(';')
        .filter_map(|cookie| {
            let (key, value) = cookie.trim().split_once('=')?;
            Some((
                key.to_string(),
                Value::String(url_decode(value).unwrap_or_else(|| value.into())),
            ))
        })
        .collect()
}

fn mime_type(path: &Path) -> String {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
    .into()
}

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .unwrap_or_else(|error| {
            eprintln!("php-rust-server: cannot start runtime: {error}");
            std::process::exit(2);
        });
    runtime.block_on(run_server());
}

fn bind_listener(address: SocketAddr) -> io::Result<TcpListener> {
    let socket = if address.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    socket.set_reuseaddr(true)?;
    socket.bind(address)?;
    socket.listen(ACCEPT_BACKLOG)
}

async fn run_server() {
    let mut config = parse_args().unwrap_or_else(|error| {
        eprintln!("php-rust-server: {error}");
        usage();
        std::process::exit(2);
    });
    let database_registry =
        build_database_registry(&config.database_connections).unwrap_or_else(|error| {
            eprintln!("php-rust-server: {error}");
            std::process::exit(2);
        });
    php_runtime::set_database_registry(database_registry.clone());
    init_logging(&config.log_level).unwrap_or_else(|error| {
        eprintln!("php-rust-server: {error}");
        std::process::exit(2);
    });
    config.root = config.root.canonicalize().unwrap_or_else(|error| {
        eprintln!(
            "php-rust-server: cannot open document root {}: {error}",
            config.root.display()
        );
        std::process::exit(2);
    });
    let listener = bind_listener(config.bind).unwrap_or_else(|error| {
        eprintln!("php-rust-server: cannot bind to {}: {error}", config.bind);
        std::process::exit(1);
    });
    install_shutdown_handlers().unwrap_or_else(|error| {
        eprintln!("php-rust-server: {error}");
        std::process::exit(1);
    });
    let bind = config.bind;
    let capacity = Arc::new(Semaphore::new(config.max_in_flight));
    let sessions: SessionStore = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let config = Arc::new(config);
    log_event(3, "server_started", &format!(
        "bind={} root={:?} max_in_flight={} request_timeout_secs={} max_body_bytes={} max_upload_bytes={} file_uploads={} max_file_uploads={} max_input_vars={} max_input_nesting_level={} max_execution_steps={} max_execution_time_secs={:?} max_output_bytes={} secure_session_cookies={} databases={:?}",
        bind, config.root, config.max_in_flight, config.request_timeout.as_secs(), config.max_body_bytes, config.max_upload_bytes, config.file_uploads, config.max_file_uploads, config.max_input_vars, config.max_input_nesting_level,
        config.execution_limits.max_instructions, config.execution_limits.max_execution_time.map(|duration| duration.as_secs()), config.execution_limits.max_output_bytes, config.secure_session_cookies, database_registry.names(),
    ));
    let mut tasks = JoinSet::new();
    loop {
        if SHUTDOWN_REQUESTED.load(Ordering::Relaxed) {
            log_event(3, "shutdown_signal", "signal=SIGINT_or_SIGTERM");
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                if let Err(error) = result { log_event(2, "connection_task_failed", &format!("error={error:?}")); }
            }
            accepted = listener.accept(), if capacity.available_permits() > 0 => {
                match accepted {
                    Ok((stream, _peer)) => {
                        let _ = stream.set_nodelay(true);
                        let permit = Arc::clone(&capacity).try_acquire_owned()
                            .expect("accept branch only runs when an in-flight permit is available");
                        tasks.spawn(handle_connection(stream, Arc::clone(&config), Arc::clone(&sessions), permit));
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => log_event(2, "accept_failed", &format!("error={error:?}")),
                }
            }
        }
    }
    let drain = async {
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                log_event(2, "connection_task_failed", &format!("error={error:?}"));
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(30), drain)
        .await
        .is_err()
    {
        log_event(2, "shutdown_timeout", "action=abort_async_connection_tasks");
        tasks.abort_all();
    }
    log_event(3, "server_stopped", "");
}

#[cfg(test)]
#[path = "../../../tests/crates/php-cli/cli/server.rs"]
mod tests;
