# Web server

`php-rust-server` hosts the PHP subset implemented by this project. Tokio handles asynchronous networking, while compilation and VM execution run on Tokio's blocking pool. A semaphore bounds active connections. When all permits are occupied, accepting pauses and the OS TCP listen backlog queues connections up to its configured/effective limit instead of immediately returning `503`. The requested backlog is 1,024; operating-system settings may cap it. This is an early web runtime, not a drop-in PHP or PHP-FPM replacement.

## Start

```sh
mkdir -p public
cat > public/index.php <<'PHP'
<?php
echo "Hello ", $_GET['name'] ?? 'web';
PHP
cargo run -p php-cli --bin php-rust-server -- --root public --bind 127.0.0.1:3000
```

Open `http://127.0.0.1:3000/?name=Rust`. The default document root is `public`, and the default bind address is `127.0.0.1:3000`. Use `--bind 0.0.0.0:3000` to listen on external interfaces.

The installed binary starts the same server:

```sh
php-rust-server --root public --bind 127.0.0.1:3000
```

## Configuration

The server reads `php-rust.ini` from the current directory when that file exists. Use `--config FILE`, `PHP_RUST_CONFIG`, or `PHPRC` to select another path; `PHP_INI_DIR` is used as a fallback. `PHP_INI_SCAN_DIR` (or `$PHP_INI_DIR/conf.d`) loads `.ini` files in filename order after the main file. The parser accepts sections, comments, quoted values, and `${NAME}` / `${NAME:-fallback}` substitutions. It preserves unknown directives for variable expansion and reports them, but preservation does not implement their PHP behavior. Only the directives listed here have an effect; this is not a drop-in implementation of every PHP setting. Copy [the example file](../php-rust.ini.example) as a starting point.

Precedence is command-line options, then `PHP_RUST_*` environment variables, then the config file, then built-in defaults.

For a system-wide install, copy and edit the example, then point the service at it:

```sh
sudo install -d -m 0755 /etc/php-rust
sudo install -m 0644 php-rust.ini.example /etc/php-rust/php-rust.ini
sudoedit /etc/php-rust/php-rust.ini
```

The sample systemd unit reads `/etc/php-rust/php-rust.ini`; service-level CPU and memory ceilings remain in the unit because they are OS resource controls, not PHP runtime settings.

- Document root: `--root` / `PHP_RUST_ROOT`, default `public`.
- Listen address: `--bind` / `PHP_RUST_BIND`, default `127.0.0.1:3000`.
- Active connections: `--max-in-flight` / `PHP_RUST_MAX_IN_FLIGHT`, default `128`. Excess TCP connections wait in the OS listen backlog (requested capacity `1,024`, subject to OS limits); after the backlog fills, clients may time out or fail to connect.
- Request read timeout: `--request-timeout-secs` / `PHP_RUST_REQUEST_TIMEOUT_SECS`, default `15` seconds.
- Request body limit: `--max-body-bytes` / `PHP_RUST_MAX_BODY_BYTES`, default `2097152` bytes.
- PHP `post_max_size` maps to the request body limit. PHP `upload_max_filesize` maps to the per-file limit (default 2 MiB); `--max-upload-bytes` / `PHP_RUST_MAX_UPLOAD_FILE_BYTES` overrides it.
- PHP `max_input_vars` limits parsed GET/POST fields (default 1,000; server range 1–10,000).
- PHP `max_input_time` maps to the request read timeout, capped at 300 seconds.
- PHP `default_mimetype` and `default_charset` set the fallback `Content-Type` for PHP responses that do not call `header()`; defaults are `text/html` and `UTF-8`.
- VM instruction limit: `--max-execution-steps` / `PHP_RUST_MAX_EXECUTION_STEPS`, default `10000000`.
- PHP call depth is capped at 32 nested VM frames to prevent recursive scripts from exhausting the Rust thread stack.
- VM wall-clock limit: `--max-execution-time-secs` / `PHP_RUST_MAX_EXECUTION_TIME_SECS`, default `30` seconds; configure `0` to disable.
- Captured PHP output limit: `--max-output-bytes` / `PHP_RUST_MAX_OUTPUT_BYTES`, default `8388608` bytes.
- Log verbosity: `PHP_RUST_LOG` accepts `error`, `warn`, `info`, or `debug`; default `info`.
- Secure session cookies behind a trusted TLS reverse proxy: `PHP_RUST_COOKIE_SECURE=true`; default `false`.
- Config file path: `--config FILE` / `PHP_RUST_CONFIG`; otherwise auto-load `php-rust.ini` from the working directory when present.

For example:

```sh
PHP_RUST_MAX_IN_FLIGHT=64 PHP_RUST_LOG=debug \
  php-rust-server --root public --bind 127.0.0.1:3000 \
  --request-timeout-secs 10 --max-body-bytes 1048576 \
  --max-execution-steps 2000000 --max-output-bytes 1048576
```

The accepted ranges are 1–4096 active connections, 1–300 seconds, 1 byte–64 MiB request bodies, 0–64 MiB per upload, 1–10,000 input variables, 1–100 million VM instructions, and 1 byte–64 MiB PHP output. PHP source files are limited to 4 MiB and static files to 64 MiB.

`max_execution_time` is checked between VM instructions, so it cannot interrupt a single long-running native operation. PHP's `memory_limit` does not constrain the Rust VM heap; set the process/container memory limit in the service manager or container configuration. Directives for PHP extensions and features not implemented here are preserved but have no effect. PHP size suffixes `K`, `M`, and `G` are accepted for body/upload limits and capped at the server's 64 MiB hard maximum.

## HTTP and PHP behavior

- Existing files under the document root are served directly. PHP files are compiled and executed for each request; other files use a small extension-based MIME map.
- `/` runs `index.php`. Unknown paths are sent to the root `index.php` front controller.
- `include`, `include_once`, `require`, and `require_once` are resolved while compiling a file when the path is a string literal or a `__DIR__` / `__FILE__` concatenation. A value-returning include expression is supported only when the included file contains exactly one top-level `return` expression and no declarations. Laravel's `file_exists($path = static_path)` maintenance guard is resolved while compiling. The resulting bytecode is cached per blocking worker and reused until the size or modification time of a PHP source file in the compiled unit changes. Web requests may include files within the application root (the document root parent), allowing a typical `public/` + `vendor/` layout. A `require vendor/autoload.php` reads the app and installed package manifests' `autoload.psr-4`, `autoload.psr-0`, `autoload.classmap`, and `autoload.files`; mapped PHP sources are compiled eagerly and included in the per-worker cache. Composer manifest changes themselves are not currently part of cache invalidation. Composer plugins, dynamically generated loader behavior, dynamic include paths, general include return semantics, and `include_path` are not implemented.
- Each execution gets fresh `$_GET`, form or multipart `$_POST`, `$_COOKIE`, merged `$_REQUEST`, `$_FILES`, and common `$_SERVER` values.
- `header()` and `http_response_code()` control response metadata. `HEAD` returns headers without the response body.
- HTTP/1.1 keep-alive is supported, with a maximum of 100 requests per connection. HTTP/1.0 connections close unless `Connection: keep-alive` is requested.
- `Content-Length` and chunked request bodies are supported within the configured body limit. Conflicting `Content-Length` and `Transfer-Encoding` are rejected. HTTP/1.1 `Expect: 100-continue` receives an interim response after the body limit is checked.
- `file_get_contents('php://input')` returns the request body as a UTF-8 string. Binary bodies are converted lossily because the VM string type is UTF-8. The implemented filesystem functions (`file_get_contents`, `file`, `file_put_contents`, `fopen`, `unlink`, metadata checks, and `tempnam`) are confined to the configured document root when running in the server; temporary in-memory streams remain available. The CLI does not apply this server path restriction and follows the operating system user's permissions.
- `json_encode()` and `json_decode()` support scalar, array, and basic object conversion, nested values, and Unicode escapes. Encoding supports pretty printing, slash/Unicode handling, HTML-safe hex flags, numeric conversion, partial output, throw-on-error, and invalid-UTF-8 flags (the latter are no-ops because runtime strings are valid UTF-8); decoding supports object-as-array, big integers as strings, and throw-on-error. Invalid input returns `null` for decode and `false` for encode unless throw mode is enabled. `json_last_error()` and `json_last_error_msg()` report common error states, while detailed UTF-8 classification and exact PHP numeric edge behavior are not implemented.
- Basic `multipart/form-data` fields and file inputs including `upload[]` and nested names such as `upload[user][avatar]` are exposed through `$_POST` and PHP-shaped `$_FILES` arrays. An empty upload filename creates an entry with `UPLOAD_ERR_NO_FILE` (4); a file over `upload_max_filesize` creates `UPLOAD_ERR_INI_SIZE` (1). Both entries have an empty temporary name/type and size zero. Uploads use temporary files with owner-only permissions. `is_uploaded_file()` validates a temporary path; `move_uploaded_file()` only moves it to a destination whose canonical parent stays inside the document root. Temporary files not moved by the script are removed at the end of that request. Other upload errors and exact PHP upload field normalization remain partial.
- `session_start()`, `session_id()`, `session_status()`, `session_unset()`, and `session_destroy()` are implemented. Sessions use an in-memory store, expire after 24 hours, and are capped at 4,096 entries, 128 KiB per session, and 64 MiB total. They support scalar and array values; objects and closures cannot be persisted. Sessions disappear when the server restarts and are not shared across server processes. The cookie is `HttpOnly` and `SameSite=Lax`. Set `PHP_RUST_COOKIE_SECURE=true` only when a trusted reverse proxy terminates TLS; this binary does not terminate HTTPS or infer the external scheme.

## Resource limits and security boundary

The server caps active connections, request bodies, PHP source files, static files, VM instructions, and captured output. The VM instruction budget bounds PHP loops and recursive calls. Filesystem routing canonicalizes requested files under the document root and included files under the application root, rejects traversal and symlinks that escape those roots, and constrains upload moves to the document root.

The runtime itself does not enforce a hard per-process heap limit; intermediate PHP arrays and strings can use more memory than the request or output size. The sample systemd unit applies a cgroup memory ceiling (`MemoryMax=512M`) and starts reclaim/throttling pressure at `MemoryHigh=384M`; adjust these to the host and workload. A cgroup OOM stops the service so systemd can restart it. Keep the server under a dedicated unprivileged OS user and keep the code/document root read-only where possible. The implemented filesystem builtins are restricted to the document root in server requests; process execution and outbound network builtins are not exposed. Database access is limited to the PDO subset documented in [database connections](database.md).

For HTTPS, terminate TLS in a trusted reverse proxy and bind this server to loopback. [An Nginx example](../deploy/nginx-php-rust.conf.example) shows TLS termination and proxy headers; configure the matching server bind and enable `PHP_RUST_COOKIE_SECURE=true`.

`SIGINT` and `SIGTERM` stop accepting new connections and give active tasks up to 30 seconds to finish. A PHP VM task uses a bounded instruction budget, but Rust cannot forcibly stop an already-running `spawn_blocking` closure.

## Current gaps

The Composer integration is intentionally limited: it reads PSR-4, PSR-0, classmap, and `autoload.files` from the root and installed package manifests, compiles mapped PHP files eagerly, and ignores generated `vendor/autoload.php` implementation code. Composer plugins, optimized generated classmap dumps, and exact Composer `autoload.files` ordering are not implemented. Every eagerly scanned PHP file must use syntax supported by this runtime.

The implementation remains a partial PHP runtime: it does not provide PHP-FPM/FastCGI, direct HTTPS termination, the broad PHP standard library, full PHP type/exception semantics, file-backed/distributed sessions, the complete multipart upload contract, or complete compatibility with frameworks such as Laravel or WordPress. Database driver and PDO coverage is partial; see [database connections](database.md). Put a TLS reverse proxy in front of the HTTP listener for HTTPS deployments and set `PHP_RUST_COOKIE_SECURE=true` when the proxy is trusted. See [supported subset and gaps](../README.md#supported-subset). For build, install, HTTP smoke and load checks, see [testing](testing.md).
