    use super::*;
    use std::io::Cursor;

    #[test]
    fn input_warning_log_fields_escape_newlines_and_include_source() {
        let diagnostic = RuntimeDiagnostic {
            level: php_runtime::DiagnosticLevel::Warning,
            message: "bad input\nforged=entry".into(),
            source_file: Some("/app/index.php".into()),
            source_line: None,
            output_offset: 0,
        };
        let fields = php_diagnostic_log_fields(&diagnostic);
        assert!(fields.contains("file=Some(\"/app/index.php\")"));
        assert!(fields.contains("message=\"bad input\\nforged=entry\""));
        assert_eq!(render_diagnostic(&diagnostic), "Warning: bad input\nforged=entry in /app/index.php\n");
    }

    fn parse_ini(path: &Path) -> Result<std::collections::HashMap<String, String>, String> {
        let mut settings = std::collections::HashMap::new();
        parse_ini_into(path, &mut settings)?;
        Ok(settings)
    }

    fn parse_request(reader: &mut impl BufRead) -> Result<HttpRequest, (u16, &'static str)> {
        parse_request_with_limit(reader, MAX_BODY_BYTES)
    }

    fn execute_request(
        root: &Path,
        url_path: &str,
        query: &str,
        method: &str,
        protocol: &str,
        content_type: &str,
        host: &str,
        request_headers: &[(String, String)],
        body: &[u8],
    ) -> Result<WebResponse, RequestError> {
        execute_request_with_limits(root, url_path, query, method, protocol, content_type, host, request_headers, body, ExecutionLimits::default())
    }

    fn execute_request_with_limits(
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
    ) -> Result<WebResponse, RequestError> {
        execute_request_with_context(root, url_path, query, method, protocol, content_type, host, request_headers, body, limits, None, false, DisplayErrors::Off, MAX_BODY_BYTES, DEFAULT_MAX_INPUT_VARS, DEFAULT_MAX_INPUT_NESTING_LEVEL)
    }

    fn execute_request_with_session(
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
    ) -> Result<WebResponse, RequestError> {
        execute_request_with_session_and_limits(root, url_path, query, method, protocol, content_type, host, request_headers, body, limits, store, secure_session_cookie, DisplayErrors::Off, MAX_BODY_BYTES, DEFAULT_MAX_INPUT_VARS, DEFAULT_MAX_INPUT_NESTING_LEVEL)
    }

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let root = env::temp_dir().join(format!("php-rust-web-test-{}-{nonce}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn path(&self) -> &Path { &self.0 }
    }

    impl Drop for Fixture {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn parses_form_request_and_preserves_nested_query_values() {
        let request = b"POST /request.php?user[name]=Ada HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 8\r\n\r\nname=Lin";
        let parsed = parse_request(&mut Cursor::new(request)).unwrap();
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.path, "/request.php");
        assert_eq!(parsed.query, "user[name]=Ada");
        assert_eq!(parsed.body, b"name=Lin");
        assert_eq!(parsed.protocol, "HTTP/1.1");
    }

    #[test]
    fn derives_http_keep_alive_from_version_and_connection_header() {
        let close = parse_request(&mut Cursor::new(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive, close\r\n\r\n")).unwrap();
        assert!(!close.keep_alive);
        let persistent = parse_request(&mut Cursor::new(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")).unwrap();
        assert!(persistent.keep_alive);
        let legacy = parse_request(&mut Cursor::new(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")).unwrap();
        assert!(!legacy.keep_alive);
    }

    #[test]
    fn parses_php_ini_style_server_settings_and_comments() {
        let fixture = Fixture::new();
        let path = fixture.path().join("php-rust.ini");
        std::fs::write(&path, "; comment\nroot = \"site public\"\ninclude_path = \"shared:vendor/php\"\nbind = 127.0.0.1:4321\n[server]\nmax_in_flight = 32\ncookie_secure = On\nlog = debug\n[PHP]\nmax_input_nesting_level = 12\ndisplay_errors = stderr\n").unwrap();
        let settings = parse_ini(&path).unwrap();
        assert_eq!(settings.get("PHP_RUST_ROOT").map(String::as_str), Some("site public"));
        assert_eq!(settings.get("PHP_RUST_BIND").map(String::as_str), Some("127.0.0.1:4321"));
        assert_eq!(settings.get("PHP_INI_VALUE:include_path").map(String::as_str), Some("shared:vendor/php"));
        assert_eq!(settings.get("PHP_RUST_MAX_IN_FLIGHT").map(String::as_str), Some("32"));
        assert_eq!(settings.get("PHP_RUST_MAX_INPUT_NESTING_LEVEL").map(String::as_str), Some("12"));
        assert_eq!(settings.get("PHP_RUST_DISPLAY_ERRORS").map(String::as_str), Some("stderr"));
        assert_eq!(DisplayErrors::parse(settings.get("PHP_RUST_DISPLAY_ERRORS").unwrap()).unwrap(), DisplayErrors::Stderr);
        assert_eq!(settings.get("PHP_RUST_COOKIE_SECURE").map(String::as_str), Some("On"));
        assert!(config_bool(&settings, "PHP_RUST_COOKIE_SECURE", false).unwrap());
    }

    #[test]
    fn display_errors_routes_runtime_warnings_to_response_or_stderr_mode() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("index.php"), "<?php echo 'body'; echo $missing; echo 'tail';").unwrap();
        let execute = |display_errors| execute_request_with_context(
            fixture.path(), "/", "", "GET", "HTTP/1.1", "", "localhost", &[], &[],
            ExecutionLimits::default(), None, false, display_errors, MAX_BODY_BYTES,
            DEFAULT_MAX_INPUT_VARS, DEFAULT_MAX_INPUT_NESTING_LEVEL,
        ).unwrap();
        let displayed = execute(DisplayErrors::On);
        let output = String::from_utf8(displayed.output).unwrap();
        assert!(output.starts_with("bodyWarning: Undefined variable $missing in "), "{output}");
        assert!(output.contains(" on line 1\n"), "{output}");
        assert!(output.ends_with("tail"), "{output}");
        assert!(output.find("Undefined variable").unwrap() < output.find("tail").unwrap(), "{output}");
        assert_eq!(execute(DisplayErrors::Off).output, b"bodytail");
    }

    #[test]
    fn server_ini_include_path_resolves_static_includes_inside_document_root() {
        let fixture = Fixture::new();
        let include_dir = fixture.path().join("shared");
        std::fs::create_dir_all(&include_dir).unwrap();
        std::fs::write(include_dir.join("bootstrap.php"), "<?php echo 'bootstrapped';").unwrap();
        std::fs::write(fixture.path().join("index.php"), "<?php include 'bootstrap.php'; echo ':', get_include_path();").unwrap();
        let response = execute_request_with_context_and_include_path(
            fixture.path(), "/", "", "GET", "HTTP/1.1", "", "localhost", &[], &[],
            ExecutionLimits::default(), None, false, DisplayErrors::Off, MAX_BODY_BYTES,
            DEFAULT_MAX_INPUT_VARS, DEFAULT_MAX_INPUT_NESTING_LEVEL, &[include_dir.clone()],
        ).unwrap();
        assert_eq!(response.output, format!("bootstrapped:{}", include_dir.display()).as_bytes());
    }

    #[test]
    fn reads_common_php_docker_ini_limits_and_ignores_unmapped_directives() {
        let fixture = Fixture::new();
        let path = fixture.path().join("php-rust.ini");
        std::fs::write(&path, "[PHP] ; inline section comment\npost_max_size = 8M ; request body cap\nupload_max_filesize = 2M\nmax_input_vars = 250\nmax_execution_time = 30\ndefault_mimetype = application/json\ndefault_charset = UTF-8\ndate.timezone = UTC\napp.path = ${APP_PATH:-/srv/app}\napp.memory = ${APP_MEMORY:-128M}\n[Session]\nsession.cookie_secure = On\n").unwrap();
        let settings = parse_ini(&path).unwrap();
        assert_eq!(settings.get("PHP_RUST_MAX_BODY_BYTES").map(String::as_str), Some("8388608"));
        assert_eq!(settings.get("PHP_RUST_MAX_UPLOAD_FILE_BYTES").map(String::as_str), Some("2097152"));
        assert_eq!(settings.get("PHP_RUST_MAX_INPUT_VARS").map(String::as_str), Some("250"));
        assert_eq!(settings.get("PHP_RUST_COOKIE_SECURE").map(String::as_str), Some("On"));
        assert_eq!(settings.get("PHP_RUST_MAX_EXECUTION_TIME_SECS").map(String::as_str), Some("30"));
        assert_eq!(settings.get("PHP_RUST_DEFAULT_MIMETYPE").map(String::as_str), Some("application/json"));
        assert_eq!(settings.get("PHP_RUST_DEFAULT_CHARSET").map(String::as_str), Some("UTF-8"));
        assert_eq!(settings.get("PHP_INI_VALUE:date.timezone").map(String::as_str), Some("UTC"));
        assert_eq!(settings.get("PHP_INI_VALUE:app.path").map(String::as_str), Some("/srv/app"));
        assert_eq!(settings.get("PHP_INI_VALUE:app.memory").map(String::as_str), Some("128M"));
        std::fs::write(&path, "this is not an ini directive\n").unwrap();
        assert!(parse_ini(&path).unwrap_err().contains("expected key = value"));
    }

    #[test]
    fn parses_named_database_pool_sections_without_logging_credentials() {
        let fixture = Fixture::new();
        let path = fixture.path().join("php-rust.ini");
        std::fs::write(&path, "[database.primary]\ndsn = \"pgsql:host=db;port=5432;dbname=app\"\nusername = app_user\npassword = ${DB_PASSWORD:-secret}\npool_min = 1\npool_max = 12\npool_acquire_timeout_secs = 3\npool_idle_timeout_secs = 0\nmax_result_rows = 1200\nmax_column_bytes = 4096\n").unwrap();
        let settings = parse_ini(&path).unwrap();
        let configs = parse_database_connections(&settings).unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "primary");
        assert_eq!(configs[0].dsn, "pgsql:host=db;port=5432;dbname=app");
        assert_eq!(configs[0].pool.min_connections, 1);
        assert_eq!(configs[0].pool.max_connections, 12);
        assert_eq!(configs[0].pool.acquire_timeout, Duration::from_secs(3));
        assert_eq!(configs[0].pool.idle_timeout, None);
        assert_eq!(configs[0].max_result_rows, 1200);
        assert_eq!(configs[0].max_column_bytes, 4096);
        assert!(!format!("{:?}", configs[0]).contains("secret"));
    }

    #[test]
    fn loads_php_ini_scan_directory_in_filename_order() {
        let fixture = Fixture::new();
        let primary = fixture.path().join("php.ini");
        let scan = fixture.path().join("conf.d");
        std::fs::create_dir_all(&scan).unwrap();
        std::fs::write(&primary, "post_max_size = 2M\n").unwrap();
        std::fs::write(scan.join("10-base.ini"), "post_max_size = 4M\n").unwrap();
        std::fs::write(scan.join("20-app.ini"), "post_max_size = 8M\n").unwrap();
        std::fs::write(scan.join("ignored.txt"), "post_max_size = 16M\n").unwrap();
        let settings = load_ini_files(Some(&primary), Some(scan.to_str().unwrap())).unwrap();
        assert_eq!(settings.get("PHP_RUST_MAX_BODY_BYTES").map(String::as_str), Some("8388608"));
    }

    #[tokio::test]
    async fn sends_interim_continue_before_reading_expected_body() {
        let (mut client, server) = tokio::io::duplex(1024);
        client.write_all(b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 4\r\nExpect: 100-continue\r\n\r\n").await.unwrap();
        let server_task = tokio::spawn(async move {
            let mut reader = BufReader::new(server);
            read_request_async(&mut reader, 1024).await.unwrap()
        });
        let mut interim = [0; 25];
        client.read_exact(&mut interim).await.unwrap();
        assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
        client.write_all(b"data").await.unwrap();
        let request = server_task.await.unwrap();
        assert_eq!(request.body, b"data");
    }

    #[tokio::test]
    async fn reads_chunked_body_and_keeps_buffered_followup_request() {
        let (mut client, server) = tokio::io::duplex(4096);
        client.write_all(b"POST /upload HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\nGET /next HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.unwrap();
        let mut reader = BufReader::new(server);
        let first = read_request_async(&mut reader, 1024).await.unwrap();
        assert_eq!(first.body, b"Wikipedia");
        assert!(first.keep_alive);
        let second = read_request_async(&mut reader, 1024).await.unwrap();
        assert_eq!(second.path, "/next");
        assert!(!second.keep_alive);
    }

    #[test]
    fn executes_front_controller_with_request_globals_and_response_metadata() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("index.php"), "<?php function show_request() { echo $_GET['name'] ?? 'none'; echo '|'; echo $_POST['name'] ?? 'none'; } show_request(); http_response_code(201); header('X-Web-Test: yes');").unwrap();
        let response = execute_request(
            fixture.path(), "/missing/route", "name=FromGet", "POST", "HTTP/1.1",
            "application/x-www-form-urlencoded", "localhost", &[], b"name=FromPost",
        ).unwrap();
        assert_eq!(response.output, b"FromGet|FromPost");
        assert_eq!(response.status, 201);
        assert_eq!(response.headers, vec![("X-Web-Test".into(), "yes".into())]);
    }

    #[test]
    fn supports_json_encode_and_decode_in_php_request_scripts() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("index.php"), r#"<?php $decoded = json_decode('{"items":[true,null,3]}', true); echo json_encode($decoded);"#).unwrap();
        let response = execute_request(fixture.path(), "/", "", "GET", "HTTP/1.1", "", "localhost", &[], &[]).unwrap();
        assert_eq!(response.output, br#"{"items":[true,null,3]}"#);
    }

    #[test]
    fn enforces_php_instruction_and_response_output_limits() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("index.php"), "<?php echo 'output too large';").unwrap();
        let result = execute_request_with_limits(
            fixture.path(), "/", "", "GET", "HTTP/1.1", "", "localhost", &[], &[],
            ExecutionLimits { max_instructions: 100, max_output_bytes: 4, max_execution_time: None },
        );
        assert!(matches!(result, Err(RequestError::Execution(message)) if message.contains("output limit")));

        let result = execute_request_with_limits(
            fixture.path(), "/", "", "GET", "HTTP/1.1", "", "localhost", &[], &[],
            ExecutionLimits { max_instructions: 1, max_output_bytes: 1024, max_execution_time: None },
        );
        assert!(matches!(result, Err(RequestError::Execution(message)) if message.contains("instruction limit")));

        let result = execute_request_with_limits(
            fixture.path(), "/", "", "GET", "HTTP/1.1", "", "localhost", &[], &[],
            ExecutionLimits { max_instructions: 100, max_output_bytes: 1024, max_execution_time: Some(Duration::ZERO) },
        );
        assert!(matches!(result, Err(RequestError::Execution(message)) if message.contains("Maximum execution time exceeded")));
    }

    #[test]
    fn builds_nested_query_form_cookie_and_server_superglobals() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("request.php"), "<?php function inspect_request() { echo $_GET['user']['name']; echo '|'; echo $_POST['user']['name']; echo '|'; echo $_COOKIE['session']; echo '|'; echo $_SERVER['HTTP_USER_AGENT']; } inspect_request();").unwrap();
        let headers = vec![
            ("Cookie".into(), "session=abc".into()),
            ("User-Agent".into(), "php-rust-check".into()),
        ];
        let response = execute_request(
            fixture.path(), "/request.php", "user[name]=FromGet", "POST", "HTTP/1.1",
            "application/x-www-form-urlencoded", "localhost", &headers, b"user[name]=FromPost",
        ).unwrap();
        assert_eq!(response.output, b"FromGet|FromPost|abc|php-rust-check");
    }

    #[test]
    fn exposes_raw_request_body_through_php_input_stream() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("input.php"), "<?php echo file_get_contents('php://input'); echo file_get_contents('php://input', offset: 4, length: 4); echo file_get_contents('php://input', offset: -4); echo file_get_contents('php://input', offset: 100) === false;").unwrap();
        let response = execute_request(
            fixture.path(), "/input.php", "", "POST", "HTTP/1.1", "application/octet-stream", "localhost", &[], b"raw-body-data",
        ).unwrap();
        assert_eq!(response.output, b"raw-body-databodydata1");
    }

    #[test]
    fn parses_multipart_fields_and_moves_uploaded_files_inside_document_root() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("index.php"), "<?php echo $_POST['title'] ?? ''; echo '|'; echo $_FILES['upload']['name'] ?? ''; echo '|'; echo is_uploaded_file($_FILES['upload']['tmp_name'] ?? ''); echo '|'; echo move_uploaded_file($_FILES['upload']['tmp_name'] ?? '', 'saved.txt');").unwrap();
        let body = b"--Sm0ke\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nhello\r\n--Sm0ke\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"note.txt\"\r\nContent-Type: text/plain\r\n\r\nfile-data\r\n--Sm0ke--\r\n";
        let response = execute_request(
            fixture.path(), "/", "", "POST", "HTTP/1.1", "multipart/form-data; boundary=Sm0ke", "localhost", &[], body,
        ).unwrap();
        assert_eq!(response.output, b"hello|note.txt|1|1");
        assert_eq!(std::fs::read(fixture.path().join("saved.txt")).unwrap(), b"file-data");
    }

    #[test]
    fn parses_multiple_multipart_files_into_php_file_arrays() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("index.php"), "<?php echo $_FILES['upload']['name'][0]; echo '|'; echo $_FILES['upload']['name'][1];").unwrap();
        let body = b"--Batch\r\nContent-Disposition: form-data; name=\"upload[]\"; filename=\"one.txt\"\r\nContent-Type: text/plain\r\n\r\none\r\n--Batch\r\nContent-Disposition: form-data; name=\"upload[]\"; filename=\"two.txt\"\r\nContent-Type: text/plain\r\n\r\ntwo\r\n--Batch--\r\n";
        let headers = vec![("Content-Type".into(), "multipart/form-data; boundary=Batch".into())];
        let response = execute_request(fixture.path(), "/", "", "POST", "HTTP/1.1", "multipart/form-data; boundary=Batch", "localhost", &headers, body).unwrap();
        assert_eq!(response.output, b"one.txt|two.txt");
    }

    #[test]
    fn empty_multipart_filename_produces_php_no_file_upload_entry() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("index.php"), "<?php echo $_FILES['upload']['error'], ':', $_FILES['upload']['name'], ':', $_FILES['upload']['size'], ':', is_uploaded_file($_FILES['upload']['tmp_name']), ':', UPLOAD_ERR_NO_FILE; ").unwrap();
        let body = b"--Empty\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"\"\r\nContent-Type: application/octet-stream\r\n\r\n\r\n--Empty--\r\n";
        let response = execute_request(fixture.path(), "/", "", "POST", "HTTP/1.1", "multipart/form-data; boundary=Empty", "localhost", &[], body).unwrap();
        assert_eq!(response.output, b"4::0::4");
    }

    #[test]
    fn parses_nested_associative_multipart_file_names() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("index.php"), "<?php echo $_FILES['upload']['name']['user']['avatar'], ':', $_FILES['upload']['size']['user']['avatar'], ':', $_FILES['upload']['error']['user']['avatar'];").unwrap();
        let body = b"--Nested\r\nContent-Disposition: form-data; name=\"upload[user][avatar]\"; filename=\"avatar.png\"\r\nContent-Type: image/png\r\n\r\npng-data\r\n--Nested--\r\n";
        let response = execute_request(fixture.path(), "/", "", "POST", "HTTP/1.1", "multipart/form-data; boundary=Nested", "localhost", &[], body).unwrap();
        assert_eq!(response.output, b"avatar.png:8:0");
    }

    #[test]
    fn multipart_names_follow_php_leading_space_and_dot_normalization() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("index.php"), "<?php echo $_POST['profile_name_'] ?? 'missing', ':', $_POST['legacy_name'] ?? 'missing', ':', array_key_exists('', $_POST), ':', $_FILES['upload_file_']['name'] ?? 'missing';").unwrap();
        let body = b"--Names\r\nContent-Disposition: form-data; name=\"  profile.name \"\r\n\r\nAda\r\n--Names\r\nContent-Disposition: form-data; name=\"legacy[name\"\r\n\r\nflattened\r\n--Names\r\nContent-Disposition: form-data; name=\"   \"\r\n\r\nignored\r\n--Names\r\nContent-Disposition: form-data; name=\"  upload.file \"; filename=\"avatar.png\"\r\nContent-Type: image/png\r\n\r\npng\r\n--Names--\r\n";
        let response = execute_request(fixture.path(), "/", "", "POST", "HTTP/1.1", "multipart/form-data; boundary=Names", "localhost", &[], body).unwrap();
        assert_eq!(response.output, b"Ada:flattened::avatar.png");
    }

    #[test]
    fn enforces_php_upload_max_filesize_from_mapped_config() {
        let body = b"--Cap\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"file.txt\"\r\nContent-Type: text/plain\r\n\r\nlarge\r\n--Cap--\r\n";
        let (post, files, temp_files) = parse_post_data("multipart/form-data; boundary=Cap", body, 4, DEFAULT_MAX_INPUT_VARS, DEFAULT_MAX_INPUT_NESTING_LEVEL, &mut Vec::new()).unwrap();
        assert!(post.is_empty());
        assert!(temp_files.is_empty());
        let php_runtime::Value::Array(fields) = &files[0].1 else { panic!("expected PHP upload fields") };
        let error = fields.iter().find_map(|(key, value)| (key == &php_runtime::ArrayKey::String("error".into())).then_some(value));
        assert!(matches!(error, Some(php_runtime::Value::Number(code)) if *code == 1.0));
        let size = fields.iter().find_map(|(key, value)| (key == &php_runtime::ArrayKey::String("size".into())).then_some(value));
        assert!(matches!(size, Some(php_runtime::Value::Number(size)) if *size == 0.0));
    }

    #[test]
    fn max_file_size_form_field_applies_only_to_later_uploads() {
        let body = b"--Limit\r\nContent-Disposition: form-data; name=\"early\"; filename=\"early.txt\"\r\nContent-Type: text/plain\r\n\r\none!\r\n--Limit\r\nContent-Disposition: form-data; name=\"MAX_FILE_SIZE\"\r\n\r\n2\r\n--Limit\r\nContent-Disposition: form-data; name=\"late\"; filename=\"late.txt\"\r\nContent-Type: text/plain\r\n\r\ntwo!\r\n--Limit--\r\n";
        let (post, files, temp_files) = parse_post_data("multipart/form-data; boundary=Limit", body, 100, DEFAULT_MAX_INPUT_VARS, DEFAULT_MAX_INPUT_NESTING_LEVEL, &mut Vec::new()).unwrap();
        let form_limit = post.iter().find_map(|(name, value)| (name == "MAX_FILE_SIZE").then_some(value));
        assert!(matches!(form_limit, Some(php_runtime::Value::String(value)) if value == "2"));
        let upload = |name: &str| files.iter().find_map(|(field, value)| {
            if field != name { return None; }
            let php_runtime::Value::Array(fields) = value else { return None; };
            Some(fields)
        }).expect("upload field should exist");
        fn upload_error(fields: &php_runtime::PhpArray) -> Option<&php_runtime::Value> {
            fields.iter().find_map(|(key, value)| {
            (key == &php_runtime::ArrayKey::String("error".into())).then_some(value)
            })
        }
        assert!(matches!(upload_error(upload("early")), Some(php_runtime::Value::Number(code)) if *code == 0.0));
        assert!(matches!(upload_error(upload("late")), Some(php_runtime::Value::Number(code)) if *code == 2.0));
        assert_eq!(temp_files.len(), 1);
        for path in temp_files { let _ = std::fs::remove_file(path); }
    }

    #[test]
    fn upload_ini_limit_takes_precedence_over_max_file_size_form_limit() {
        let body = b"--Limit\r\nContent-Disposition: form-data; name=\"MAX_FILE_SIZE\"\r\n\r\n10\r\n--Limit\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"large.txt\"\r\nContent-Type: text/plain\r\n\r\nlarger\r\n--Limit--\r\n";
        let (_, files, temp_files) = parse_post_data("multipart/form-data; boundary=Limit", body, 3, DEFAULT_MAX_INPUT_VARS, DEFAULT_MAX_INPUT_NESTING_LEVEL, &mut Vec::new()).unwrap();
        assert!(temp_files.is_empty());
        let php_runtime::Value::Array(fields) = &files[0].1 else { panic!("expected PHP upload fields") };
        let error = fields.iter().find_map(|(key, value)| (key == &php_runtime::ArrayKey::String("error".into())).then_some(value));
        assert!(matches!(error, Some(php_runtime::Value::Number(code)) if *code == 1.0));
    }

    #[test]
    fn caps_urlencoded_input_variables_at_configured_limit() {
        assert_eq!(parse_urlencoded("a=1&b=2&c=3", 2, DEFAULT_MAX_INPUT_NESTING_LEVEL, &mut Vec::new()).len(), 2);
    }

    #[test]
    fn input_names_accept_php_default_nesting_depth() {
        fn depth(value: &php_runtime::Value) -> usize {
            match value {
                php_runtime::Value::Array(array) => 1 + array.iter().next().map_or(0, |(_, value)| depth(value)),
                _ => 0,
            }
        }
        let accepted = format!("root{}=leaf", "[x]".repeat(DEFAULT_MAX_INPUT_NESTING_LEVEL));
        let parsed = parse_urlencoded(&accepted, DEFAULT_MAX_INPUT_VARS, DEFAULT_MAX_INPUT_NESTING_LEVEL, &mut Vec::new());
        assert_eq!(parsed.len(), 1);
        assert_eq!(depth(&parsed[0].1), DEFAULT_MAX_INPUT_NESTING_LEVEL);

        let too_deep = format!("root{}=leaf", "[x]".repeat(DEFAULT_MAX_INPUT_NESTING_LEVEL + 1));
        assert!(parse_urlencoded(&too_deep, DEFAULT_MAX_INPUT_VARS, DEFAULT_MAX_INPUT_NESTING_LEVEL, &mut Vec::new()).is_empty());

        let configured = parse_urlencoded("root[a][b]=leaf", DEFAULT_MAX_INPUT_VARS, 1, &mut Vec::new());
        assert_eq!(parse_urlencoded("root[a]=leaf", DEFAULT_MAX_INPUT_VARS, 1, &mut Vec::new()).len(), 1);
        assert!(configured.is_empty());
    }

    #[test]
    fn request_uses_configured_input_nesting_limit() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("index.php"), "<?php $last = error_get_last(); echo $_GET['root']['a'] ?? 'missing', ':', $last['message'] ?? 'no-warning';").unwrap();
        let request = |query: &str| execute_request_with_context(
            fixture.path(), "/", query, "GET", "HTTP/1.1", "", "localhost", &[], &[],
            ExecutionLimits::default(), None, false, DisplayErrors::Off, MAX_BODY_BYTES, DEFAULT_MAX_INPUT_VARS, 1,
        ).unwrap();
        assert_eq!(request("root[a]=leaf").output, b"leaf:no-warning");
        assert_eq!(request("root[a][b]=leaf").output, b"missing:Input variable nesting level exceeded 1. To increase the limit change max_input_nesting_level in php.ini.");
    }

    #[test]
    fn persists_php_session_values_and_expires_the_cookie_on_destroy() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("index.php"), "<?php session_start(); $_SESSION['count'] = ($_SESSION['count'] ?? 0) + 1; echo $_SESSION['count'];").unwrap();
        std::fs::write(fixture.path().join("destroy.php"), "<?php session_start(); session_destroy(); echo 'destroyed';").unwrap();
        let store: SessionStore = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let first = execute_request_with_session(fixture.path(), "/", "", "GET", "HTTP/1.1", "", "localhost", &[], &[], ExecutionLimits::default(), Arc::clone(&store), true).unwrap();
        assert_eq!(first.output, b"1");
        let cookie = first.headers.iter().find(|(name, _)| name.eq_ignore_ascii_case("set-cookie")).unwrap().1.clone();
        assert!(cookie.contains("Secure"));
        let session_id = cookie.strip_prefix("PHPSESSID=").unwrap().split(';').next().unwrap();

        let headers = vec![("Cookie".into(), format!("PHPSESSID={session_id}"))];
        let second = execute_request_with_session(fixture.path(), "/", "", "GET", "HTTP/1.1", "", "localhost", &headers, &[], ExecutionLimits::default(), Arc::clone(&store), true).unwrap();
        assert_eq!(second.output, b"2");

        let destroyed = execute_request_with_session(fixture.path(), "/destroy.php", "", "GET", "HTTP/1.1", "", "localhost", &headers, &[], ExecutionLimits::default(), Arc::clone(&store), true).unwrap();
        assert_eq!(destroyed.output, b"destroyed");
        assert!(destroyed.headers.iter().any(|(name, value)| name.eq_ignore_ascii_case("set-cookie") && value.contains("Max-Age=0")));
        assert!(store.lock().unwrap().is_empty());
    }

    #[test]
    fn serves_static_files_from_document_root() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("asset.txt"), b"static bytes").unwrap();
        std::fs::write(fixture.path().join("asset with space.txt"), b"encoded path").unwrap();
        let response = execute_request(
            fixture.path(), "/asset.txt", "", "GET", "HTTP/1.1", "", "localhost", &[], &[],
        ).unwrap();
        assert_eq!(response.output, b"static bytes");
        assert_eq!(response.headers, vec![("content-type".into(), "text/plain; charset=utf-8".into())]);
        let encoded = execute_request(
            fixture.path(), "/asset%20with%20space.txt", "", "GET", "HTTP/1.1", "", "localhost", &[], &[],
        ).unwrap();
        assert_eq!(encoded.output, b"encoded path");
    }

    #[test]
    fn rejects_parent_directory_traversal() {
        let fixture = Fixture::new();
        let result = execute_request(
            fixture.path(), "/../secret.php", "", "GET", "HTTP/1.1", "", "localhost", &[], &[],
        );
        assert!(matches!(result, Err(RequestError::BadPath)));
        let encoded = execute_request(
            fixture.path(), "/%2e%2e/secret.php", "", "GET", "HTTP/1.1", "", "localhost", &[], &[],
        );
        assert!(matches!(encoded, Err(RequestError::BadPath)));
    }

    #[test]
    fn rejects_oversized_and_chunked_request_bodies_before_reading_body() {
        let too_large = b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2097153\r\n\r\n";
        assert_eq!(parse_request(&mut Cursor::new(too_large)).unwrap_err().0, 413);
        let chunked = b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert_eq!(parse_request(&mut Cursor::new(chunked)).unwrap_err().0, 501);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_that_escape_document_root() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        std::fs::write(outside.path().join("secret.php"), "<?php echo 'secret';").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret.php"), fixture.path().join("escape.php")).unwrap();
        let result = execute_request(
            fixture.path(), "/escape.php", "", "GET", "HTTP/1.1", "", "localhost", &[], &[],
        );
        assert!(matches!(result, Err(RequestError::NotFound)));
    }

    #[test]
    fn php_filesystem_builtins_are_confined_to_document_root() {
        let fixture = Fixture::new();
        let outside = fixture.path().parent().unwrap().join(format!("outside-{}", std::process::id()));
        std::fs::write(&outside, "keep-me").unwrap();
        let script = format!(
            "<?php $p = '{}'; echo file_get_contents($p) === false, ':'; echo file_exists($p), ':'; echo file_put_contents($p, 'overwrite') === false, ':'; echo fopen($p, 'r') === false, ':'; echo unlink($p) === false, ':';",
            outside.display()
        );
        std::fs::write(fixture.path().join("index.php"), script).unwrap();
        let response = execute_request(
            fixture.path(), "/", "", "GET", "HTTP/1.1", "", "localhost", &[], &[],
        ).unwrap();
        assert_eq!(String::from_utf8(response.output).unwrap(), "1::1:1:1:");
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep-me");
        std::fs::remove_file(outside).unwrap();
    }

    #[test]
    fn php_sapi_name_reports_cli_server_in_requests() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("index.php"), "<?php echo php_sapi_name();").unwrap();
        let response = execute_request(
            fixture.path(), "/", "", "GET", "HTTP/1.1", "", "localhost", &[], &[],
        ).unwrap();
        assert_eq!(String::from_utf8(response.output).unwrap(), "cli-server");
    }

    #[test]
    fn serializes_http_status_headers_and_head_without_body() {
        let mut output = Vec::new();
        write_response(&mut output, 201, &[("X-Test".into(), "yes".into())], b"content", true, false);
        let response = String::from_utf8(output).unwrap();
        assert!(response.starts_with("HTTP/1.1 201 Created\r\n"));
        assert!(response.contains("X-Test: yes\r\n"));
        assert!(response.contains("Content-Length: 7\r\n"));
        assert!(response.ends_with("\r\n\r\n"));
    }

    #[test]
    fn processes_http_form_exchange_through_to_response_bytes() {
        let fixture = Fixture::new();
        std::fs::write(fixture.path().join("index.php"), "<?php echo $_GET['name'] ?? 'none'; echo '|'; echo $_POST['name'] ?? 'none'; http_response_code(201); header('X-Web-Test: yes');").unwrap();
        let input = b"POST /submit?name=from-get HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 14\r\n\r\nname=from-post";
        let request = parse_request(&mut Cursor::new(input)).unwrap();
        let host = request.headers.iter().find(|(name, _)| name.eq_ignore_ascii_case("host")).unwrap().1.clone();
        let content_type = request.headers.iter().find(|(name, _)| name.eq_ignore_ascii_case("content-type")).unwrap().1.clone();
        let response = execute_request(fixture.path(), &request.path, &request.query, &request.method, &request.protocol,
            &content_type, &host, &request.headers, &request.body).unwrap();
        let mut output = Vec::new();
        write_response(&mut output, response.status, &response.headers, &response.output, false, false);
        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("HTTP/1.1 201 Created\r\n"));
        assert!(output.contains("X-Web-Test: yes\r\n"));
        assert!(output.ends_with("\r\n\r\nfrom-get|from-post"));
    }
