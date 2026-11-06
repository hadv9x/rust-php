use php_compiler::compile_source;

#[test]
fn php_sapi_name_reports_cli_and_is_discoverable() {
    let source = "<?php echo php_sapi_name(), ':', function_exists('php_sapi_name');";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "cli:1");
}

#[test]
fn recursive_php_calls_stop_at_a_safe_vm_depth() {
    let error = std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
        compile_source("<?php function recurse_forever() { recurse_forever(); } recurse_forever();")
            .unwrap().execute().unwrap_err()
    })
        .unwrap().join().unwrap();
    assert!(error.contains("Maximum call depth exceeded"), "{error}");
}

#[test]
fn imported_interface_constants_resolve_in_parameter_defaults() {
    let source = r#"<?php
        namespace Framework\Contract;
        interface DataPolicy { public const REPLACE = 'replace'; }
        namespace Framework\Consumer;
        use Framework\Contract\DataPolicy as Policy;
        function configure($mode = Policy::REPLACE) { return $mode; }
        echo configure();
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "replace");
}

#[test]
fn imported_interface_constants_resolve_in_class_constants() {
    let source = r#"<?php
        namespace Framework\Contract;
        interface PasswordBroker { const RESET_LINK_SENT = 'passwords.sent'; }
        namespace Framework\Facades;
        use Framework\Contract\PasswordBroker;
        class Password { const RESET_LINK_SENT = PasswordBroker::RESET_LINK_SENT; }
        echo Password::RESET_LINK_SENT;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "passwords.sent");
}

#[test]
fn inherited_interface_constants_resolve_in_parameter_defaults() {
    let source = r#"<?php
        interface RootPolicy { const VALUE = 'root'; }
        interface ChildPolicy extends RootPolicy {}
        class PolicyConsumer implements ChildPolicy { public function value($mode = self::VALUE) { return $mode; } }
        echo (new PolicyConsumer())->value();
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "root");
}

#[test]
fn user_objects_can_read_write_compound_unset_and_detect_dynamic_properties() {
    let source = r#"<?php
        class DynamicRecord { public $declaredValue = 1; public function keptMethod() { return $this->declaredValue; } }
        $record = new DynamicRecord();
        $record->extraValue = 2;
        $record->extraValue += 3;
        $record->extravalue = 8;
        echo $record->extraValue, ':', $record->extravalue, ':', property_exists($record, 'extraValue'), ':', property_exists($record, 'ExtraValue'), ':', property_exists($record, 'declaredvalue'), ':';
        unset($record->extraValue);
        echo property_exists($record, 'extraValue'), ':', isset($record->extraValue), ':', $record->declaredValue, ':', $record->keptMethod(), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "5:8:1:::::1:1\n");
}

#[test]
fn traits_can_declare_static_properties_used_by_classes_and_trait_methods() {
    let source = r#"<?php
        trait HasSequence {
            public static int $sequence = 3;
            public static function next() { self::$sequence += 1; return self::$sequence; }
        }
        class SequenceA { use HasSequence; }
        class SequenceB { use HasSequence; }
        echo SequenceA::$sequence, ':', SequenceA::next(), ':', SequenceA::$sequence, ':', SequenceB::$sequence, "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "3:4:4:3\n");
}

#[test]
fn trait_methods_override_same_named_methods_from_nested_traits() {
    let source = r#"<?php
        trait NestedBase { public function value() { return 'nested'; } }
        trait NestedOverride { use NestedBase; public function value() { return 'override'; } }
        class NestedTraitHost { use NestedOverride; }
        echo (new NestedTraitHost())->value();
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "override");
}

#[test]
fn class_trait_alias_can_target_a_method_from_a_nested_trait() {
    let source = r#"<?php
        trait AliasNestedBase { public function value() { return 'nested'; } }
        trait AliasNested { use AliasNestedBase; }
        class AliasNestedHost { use AliasNested { value as renamed; } }
        $host = new AliasNestedHost();
        echo $host->value(), ':', $host->renamed();
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "nested:nested");
}

#[test]
fn nested_trait_insteadof_resolves_conflicts_and_keeps_the_selected_method() {
    let source = r#"<?php
        trait Preferred { public function value() { return 'preferred'; } }
        trait Rejected { public function value() { return 'rejected'; } }
        trait Combined { use Preferred, Rejected { Preferred::value insteadof Rejected; } }
        class NestedConflictHost { use Combined; }
        echo (new NestedConflictHost())->value();
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "preferred");
}

#[test]
fn new_object_parameter_defaults_create_a_fresh_instance_per_call() {
    let source = r#"<?php
        class DefaultObject { public $value; }
        function makeDefault($object = new DefaultObject()) { return $object; }
        $first = makeDefault(); $second = makeDefault();
        $first->value = 9;
        echo $first->value, ':', $second->value, ':', ($first === $second ? 'same' : 'fresh');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "9::fresh");
}

#[test]
fn closures_created_in_methods_implicitly_capture_this_and_keep_class_scope() {
    let source = r#"<?php
        class ClosureScopeHost {
            private $secret = 'hidden';
            private function reveal() { return $this->secret; }
            public function explicitClosure() { return function () { return $this->reveal() . ':' . get_class(); }; }
            public function arrowClosure() { return fn () => $this->reveal() . ':' . get_class(); }
        }
        $host = new ClosureScopeHost();
        $regular = $host->explicitClosure();
        $arrow = $host->arrowClosure();
        echo $regular(), ':', $arrow(), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "hidden:ClosureScopeHost:hidden:ClosureScopeHost\n");
}

#[test]
fn common_type_predicates_classify_values_and_numeric_strings() {
    let source = r#"<?php
        echo is_string('12'), ':', is_string(12), ':', is_array([]), ':', is_bool(false), ':', is_null(null), ':';
        echo is_object(function () {}), ':', is_scalar('x'), ':', is_scalar([]), ':', is_resource(null), ':';
        echo is_numeric(' -1.25e2 '), ':', is_numeric('12cats'), ':', is_numeric(value: 12), ':', function_exists('is_numeric'), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "1::1:1:1:1:1:::1::1:1\n");
}

#[test]
fn array_column_reads_public_fields_from_object_rows() {
    let source = r#"<?php
        class ColumnRow {
            public $id;
            public $name;
            private $secret = 'hidden';
            public function __construct($id, $name) { $this->id = $id; $this->name = $name; }
        }
        $rows = [new ColumnRow(4, 'Ada'), new ColumnRow(7, 'Lin')];
        $names = array_column($rows, 'name', 'id');
        $wholeRows = array_column($rows, null);
        echo $names[4], ':', $names[7], ':', get_class($wholeRows[0]), ':', array_key_exists('secret', array_column($rows, 'secret')), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "Ada:Lin:ColumnRow:\n");
}

#[test]
fn range_handles_php_string_integer_float_and_descending_semantics() {
    let source = r#"<?php
        $letters = range('ab', 'de');
        $numeric_strings = range('10', '13');
        $mixed = range('9', 'A');
        $descending = range(5, 1, -2);
        $integral_float_step = range(1, 3, 1.0);
        $fractional_step = range(1, 3, 1.5);
        echo implode(',', $letters), ':', gettype($numeric_strings[0]), ':', $numeric_strings[3], ':',
            gettype($mixed[0]), ':', $mixed[8], ':', implode(',', $descending), ':', gettype($integral_float_step[0]), ':',
            gettype($fractional_step[1]), ':', $fractional_step[1], "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "a,b,c,d:integer:13:string:A:5,3,1:integer:double:2.5\n");
    let invalid_step = compile_source("<?php range(1, 5, -1);").unwrap().execute().unwrap_err();
    assert!(invalid_step.contains("ValueError"));
    let zero_step = compile_source("<?php range(1, 5, 0);").unwrap().execute().unwrap_err();
    assert!(zero_step.contains("ValueError"));
    let invalid_endpoint = compile_source("<?php try { range([], 5); } catch (TypeError $error) { echo get_class($error); }").unwrap().execute().unwrap();
    assert_eq!(invalid_endpoint, "TypeError");
}

#[test]
fn filesystem_metadata_helpers_support_named_arguments_and_introspection() {
    let source = r#"<?php
        echo file_get_contents(filename: '../../Cargo.toml', offset: 0, length: 10), ':', file_get_contents(filename: 'missing-compatibility-test-file') === false, ':';
        $stream = fopen(filename: '../../Cargo.toml', mode: 'rb');
        echo stream_get_contents($stream, 10), ':'; fclose($stream);
        echo is_resource($stream), ':', file_exists('../../Cargo.toml'), ':', fopen('missing-compatibility-test-file', 'rb') === false, ':';
        echo file_exists(filename: '../../Cargo.toml'), ':', is_file('../../Cargo.toml'), ':', is_dir(filename: '../../crates'), ':', is_link(filename: '../../Cargo.toml'), ':', is_readable(filename: '../../Cargo.toml'), ':';
        $metadataPath = getcwd() . '/Cargo.toml';
        echo 'ctime=', filectime($metadataPath) > 0, ';inode=', fileinode(filename: $metadataPath) > 0, ';owner=', fileowner($metadataPath) >= 0, ';group=', filegroup($metadataPath) >= 0, ';perms=', fileperms($metadataPath) > 0, ';type=', filetype($metadataPath), ';callback=', array_map('filetype', [$metadataPath])[0], ';';
        echo 'size=', filesize(filename: '../../Cargo.toml') > 0, ';mtime=', filemtime(filename: $metadataPath) > 0, ';atime=', fileatime(filename: $metadataPath) > 0, ';callback=', array_map('filemtime', [$metadataPath])[0] > 0, ';realpath=', realpath('../../Cargo.toml') !== false, ';';
        echo file_exists('missing-compatibility-test-file'), ':', is_writable(filename: 'missing-compatibility-test-file'), ':', function_exists('is_executable'), ':', function_exists('filemtime'), ':', function_exists('stat'), ':', function_exists('filetype'), ':', strlen(getcwd()) > 0, "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "[workspace:1:[workspace::1:1:1:1:1::1:ctime=1;inode=1;owner=1;group=1;perms=1;type=file;callback=file;size=1;mtime=1;atime=1;callback=1;realpath=1;::1:1:1:1:1\n");
}

#[test]
fn filesystem_timestamp_failures_return_false_and_emit_warnings_directly_and_as_callbacks() {
    let source = r#"<?php
        error_reporting(E_WARNING);
        echo filemtime(filename: 'missing-timestamp-file') === false, ':';
        echo array_map('fileatime', ['missing-timestamp-file'])[0] === false;
    "#;
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "1:1");
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
    assert_eq!(result.diagnostics[0].message, "filemtime(): stat failed for missing-timestamp-file");
    assert_eq!(result.diagnostics[1].message, "fileatime(): stat failed for missing-timestamp-file");
    assert_eq!(result.diagnostics.iter().map(|diagnostic| diagnostic.source_line).collect::<Vec<_>>(), vec![Some(3), Some(4)]);
}

#[test]
fn md5_file_hashes_raw_file_bytes_and_supports_named_callback_and_warning_paths() {
    let root = std::env::temp_dir().join(format!("php-rust-md5-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("binary.dat"), [b'a', b'b', b'c', 0xff, 0]).unwrap();
    let path = root.join("binary.dat").to_string_lossy().into_owned();
    let missing = root.join("missing.dat").to_string_lossy().into_owned();
    let script = format!(r#"<?php
        error_reporting(E_WARNING);
        echo md5_file(filename: '{path}'), ':';
        echo (md5_file(filename: '{path}', binary: true) !== md5_file('{path}')), ':';
        echo array_map('md5_file', ['{path}'])[0], ':';
        echo md5_file('{missing}') === false, ':', function_exists('md5_file');
    "#);
    let mut globals = std::collections::HashMap::new();
    globals.insert("\0filesystem_root".into(), php_runtime::Value::String(root.to_string_lossy().into_owned()));
    let result = compile_source(&script).unwrap().execute_with_globals(globals).unwrap();
    assert_eq!(result.output, "e01e7b08be3d4d8a1b74b61943856ac2:1:e01e7b08be3d4d8a1b74b61943856ac2:1:1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(result.diagnostics[0].source_line, Some(6));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sha1_file_hashes_raw_file_bytes_and_supports_named_callback_and_warning_paths() {
    let root = std::env::temp_dir().join(format!("php-rust-sha1-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("binary.dat"), [b'a', b'b', b'c', 0xff, 0]).unwrap();
    let path = root.join("binary.dat").to_string_lossy().into_owned();
    let missing = root.join("missing.dat").to_string_lossy().into_owned();
    let script = format!(r#"<?php
        error_reporting(E_WARNING);
        echo sha1_file(filename: '{path}'), ':';
        echo (sha1_file(filename: '{path}', binary: true) !== sha1_file('{path}')), ':';
        echo array_map('sha1_file', ['{path}'])[0], ':';
        echo sha1_file('{missing}') === false, ':', function_exists('sha1_file');
    "#);
    let mut globals = std::collections::HashMap::new();
    globals.insert("\0filesystem_root".into(), php_runtime::Value::String(root.to_string_lossy().into_owned()));
    let result = compile_source(&script).unwrap().execute_with_globals(globals).unwrap();
    assert_eq!(result.output, "35a6dd9eec07ed9764c95cf36940b278b374f7b9:1:35a6dd9eec07ed9764c95cf36940b278b374f7b9:1:1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(result.diagnostics[0].source_line, Some(6));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn hash_hmac_file_hashes_sandboxed_file_bytes_and_supports_callbacks() {
    let root = std::env::temp_dir().join(format!("php-rust-hmac-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("binary.dat"), [b'a', b'b', b'c', 0xff, 0]).unwrap();
    let path = root.join("binary.dat").to_string_lossy().into_owned();
    let missing = root.join("missing.dat").to_string_lossy().into_owned();
    let script = format!(r#"<?php
        error_reporting(E_WARNING);
        echo hash_hmac_file(algo: 'md5', filename: '{path}', key: 'Jefe'), '|';
        echo hash_hmac_file('sha1', '{path}', 'Jefe'), '|';
        echo hash_hmac_file('sha256', '{path}', 'Jefe'), '|';
        echo hash_hmac_file('sha512', '{path}', 'Jefe'), '|';
        echo array_map('hash_hmac_file', ['md5'], ['{path}'], ['Jefe'])[0], '|';
        echo hash_hmac_file('md5', '{missing}', 'Jefe') === false, '|', function_exists('hash_hmac_file');
    "#);
    let mut globals = std::collections::HashMap::new();
    globals.insert("\0filesystem_root".into(), php_runtime::Value::String(root.to_string_lossy().into_owned()));
    let result = compile_source(&script).unwrap().execute_with_globals(globals).unwrap();
    assert_eq!(result.output, "c819ae47a2b770908a0cfdfc97391bbe|6ecf193ad361be423fd50be4a4e5410d2915ba28|8b516fd2265edd5b1c38d222d926abd48ee507d505e6f5e45f7dae2e6f20eead|60437d7f9ecca948390f509a919ba4cc2de606fa1be29c828e9824f5bce1436bd908397bb41355607adba623723f3b882ddad6a9bdaa599906e9e3f906baeefa|c819ae47a2b770908a0cfdfc97391bbe|1|1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(result.diagnostics[0].source_line, Some(8));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn hash_file_hashes_sandboxed_bytes_and_supports_named_algorithms_and_callbacks() {
    let root = std::env::temp_dir().join(format!("php-rust-hash-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("binary.dat"), [b'a', b'b', b'c', 0xff, 0]).unwrap();
    let path = root.join("binary.dat").to_string_lossy().into_owned();
    let missing = root.join("missing.dat").to_string_lossy().into_owned();
    let script = format!(r#"<?php
        error_reporting(E_WARNING);
        echo hash_file(algo: 'md5', filename: '{path}'), '|';
        echo hash_file('sha1', '{path}'), '|', hash_file('sha256', '{path}'), '|', hash_file('sha512', '{path}'), '|', hash_file('crc32b', '{path}'), '|';
        echo array_map('hash_file', ['md5'], ['{path}'])[0], '|';
        echo hash_file('md5', '{missing}') === false, '|', function_exists('hash_file');
    "#);
    let mut globals = std::collections::HashMap::new();
    globals.insert("\0filesystem_root".into(), php_runtime::Value::String(root.to_string_lossy().into_owned()));
    let result = compile_source(&script).unwrap().execute_with_globals(globals).unwrap();
    assert_eq!(result.output, "e01e7b08be3d4d8a1b74b61943856ac2|35a6dd9eec07ed9764c95cf36940b278b374f7b9|aaf2c38a3b8617bdff3f793d98c22458a3714a26faf5d06f900009885d42bdd9|6728812a326d117173726d543ac8fce73782f68335c7efacaed2767b40a01efb2286d2229d3fef49bf68bc984e123a2829abd570cee5cf9f098972f6aff932eb|2aea1e63|e01e7b08be3d4d8a1b74b61943856ac2|1|1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(result.diagnostics[0].source_line, Some(6));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn stat_and_lstat_return_php_numeric_and_named_fields() {
    let source = r#"<?php
        $status = stat(filename: '../../Cargo.toml');
        $linkStatus = lstat('../../Cargo.toml');
        $callbackStatus = array_map('stat', ['../../Cargo.toml'])[0];
        echo count($status), ':', $status[7] === $status['size'], ':', $status[9] === $status['mtime'], ':';
        echo $linkStatus[7] === $linkStatus['size'], ':', $callbackStatus['size'] > 0, ':';
        echo function_exists('stat'), ':', function_exists('lstat');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "26:1:1:1:1:1:1");
}

#[test]
fn stat_failures_return_false_and_emit_source_located_warnings() {
    let source = "<?php\nerror_reporting(E_WARNING);\nvar_dump(stat(filename: 'missing-stat-file'));\n";
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert!(result.output.contains("bool(false)"));
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(result.diagnostics[0].message, "stat(): stat failed for missing-stat-file");
    assert_eq!(result.diagnostics[0].source_line, Some(3));
}

#[test]
fn scandir_supports_php_sort_orders_named_arguments_and_callback_dispatch() {
    let source = r#"<?php
        $ascending = scandir(directory: '../../examples', sorting_order: SCANDIR_SORT_ASCENDING);
        $descending = scandir('../../examples', SCANDIR_SORT_DESCENDING);
        $unsorted = scandir('../../examples', SCANDIR_SORT_NONE);
        $callback = array_map('scandir', ['../../examples'])[0];
        echo $ascending[0], ':', $ascending[1], ':', $ascending[2], ':';
        echo $descending[0], ':', $descending[count($descending) - 1], ':';
        echo $unsorted[0], ':', $unsorted[1], ':', file_exists('../../examples/' . $callback[2]), ':', function_exists('scandir');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), ".:..:ExampleAutoloaded.php:word-logical-operators.php:.:.:..:1:1");
}

#[test]
fn scandir_failure_returns_false_and_emits_source_located_warning() {
    let source = "<?php\nerror_reporting(E_WARNING);\n$entries = scandir(directory: 'missing-scan-directory');\necho $entries === false;";
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(result.diagnostics[0].message, "scandir(): failed to open directory missing-scan-directory");
    assert_eq!(result.diagnostics[0].source_line, Some(3));
}

#[test]
fn opendir_readdir_closedir_share_directory_resource_state() {
    let source = r#"<?php
        $handle = opendir(directory: '../../examples');
        $first = array_map('readdir', [$handle])[0];
        $second = readdir();
        $closed = array_map('closedir', [$handle])[0];
        $implicitHandle = array_map('opendir', ['../../examples'])[0];
        $implicitClosed = closedir();
        echo is_resource($handle), ':', $first, ':', $second, ':', $closed === null, ':';
        echo $implicitClosed === null, ':', is_resource($implicitHandle), ':', function_exists('opendir'), ':', function_exists('readdir'), ':', function_exists('closedir');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), ":.:..:1:1::1:1:1");
}

#[test]
fn opendir_failure_returns_false_and_emits_source_located_warning() {
    let source = "<?php\nerror_reporting(E_WARNING);\n$handle = opendir('missing-open-directory');\necho $handle === false;";
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(result.diagnostics[0].source_line, Some(3));
}

#[test]
fn rewinddir_resets_the_directory_handle_and_is_discoverable() {
    let source = r#"<?php
        $handle = opendir('../../examples');
        $first = readdir($handle);
        readdir($handle);
        $rewound = rewinddir($handle);
        $again = readdir($handle);
        $seen = 1;
        while (readdir($handle) !== false) { $seen++; }
        echo $first, ':', $again, ':', $rewound === null, ':', $seen === count(scandir('../../examples')), ':', readdir($handle) === false, ':', function_exists('rewinddir');
        closedir($handle);
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), ".:.:1:1:1:1");
}

#[test]
fn explicit_null_directory_handles_emit_php_85_deprecations() {
    let source = "<?php\n$handle = opendir('../../examples');\nreaddir(null);\nrewinddir(null);\nclosedir(null);";
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.diagnostics.len(), 3);
    for (diagnostic, function) in result.diagnostics.iter().zip(["readdir", "rewinddir", "closedir"]) {
        assert_eq!(diagnostic.level, php_runtime::DiagnosticLevel::Deprecated);
        assert_eq!(diagnostic.message, format!("Passing null to {function}() is deprecated, instead the last opened directory stream should be provided"));
    }
}

#[test]
fn touch_creates_files_sets_both_timestamps_and_dispatches_as_callback() {
    let root = std::env::temp_dir().join(format!("php-rust-touch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("created.txt").to_string_lossy().into_owned();
    let script = format!(r#"<?php
        $created = touch(filename: '{path}', mtime: 123456789, atime: 234567890);
        $mtime = filemtime('{path}');
        $atime = fileatime('{path}');
        $callback = array_map('touch', ['{path}'])[0];
        echo $created, ':', $mtime, ':', $atime, ':', $callback, ':', function_exists('touch');
    "#);
    let mut globals = std::collections::HashMap::new();
    globals.insert("\0filesystem_root".into(), php_runtime::Value::String(root.to_string_lossy().into_owned()));
    assert_eq!(compile_source(&script).unwrap().execute_with_globals(globals).unwrap().output, "1:123456789:234567890:1:1");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn touch_defaults_atime_to_mtime_and_warns_when_sandbox_rejects_path() {
    let root = std::env::temp_dir().join(format!("php-rust-touch-failure-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("created.txt").to_string_lossy().into_owned();
    let missing = root.join("missing").join("file.txt").to_string_lossy().into_owned();
    let script = format!("<?php\ntouch('{path}', 123456789);\n$ok = touch('{missing}');\necho fileatime('{path}'), ':', $ok === false;");
    let mut globals = std::collections::HashMap::new();
    globals.insert("\0filesystem_root".into(), php_runtime::Value::String(root.to_string_lossy().into_owned()));
    let result = compile_source(&script).unwrap().execute_with_globals(globals).unwrap();
    assert_eq!(result.output, "123456789:1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(result.diagnostics[0].source_line, Some(3));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn chmod_updates_sandboxed_file_modes_and_dispatches_as_callback() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("php-rust-chmod-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let file_path = root.join("mode.txt");
    let path = file_path.to_string_lossy().into_owned();
    std::fs::write(&file_path, "x").unwrap();
    let script = format!(r#"<?php
        $first = chmod(filename: '{path}', permissions: 416);
        $firstMode = fileperms('{path}') & 511;
        $callback = array_map('chmod', ['{path}'], [420])[0];
        echo $first, ':', $firstMode, ':', $callback, ':', (fileperms('{path}') & 511), ':', function_exists('chmod');
    "#);
    let mut globals = std::collections::HashMap::new();
    globals.insert("\0filesystem_root".into(), php_runtime::Value::String(root.to_string_lossy().into_owned()));
    assert_eq!(compile_source(&script).unwrap().execute_with_globals(globals).unwrap().output, "1:416:1:420:1");
    assert_eq!(std::fs::metadata(&file_path).unwrap().permissions().mode() & 0o777, 0o644);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn chmod_failure_returns_false_and_emits_source_located_warning() {
    let root = std::env::temp_dir().join(format!("php-rust-chmod-failure-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let missing = root.join("missing.txt").to_string_lossy().into_owned();
    let script = format!("<?php\nerror_reporting(E_WARNING);\n$result = chmod('{missing}', 420);\necho $result === false;");
    let mut globals = std::collections::HashMap::new();
    globals.insert("\0filesystem_root".into(), php_runtime::Value::String(root.to_string_lossy().into_owned()));
    let result = compile_source(&script).unwrap().execute_with_globals(globals).unwrap();
    assert_eq!(result.output, "1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(result.diagnostics[0].source_line, Some(3));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn disk_space_functions_return_bytes_support_aliases_callbacks_and_sandbox_warnings() {
    let root = std::env::temp_dir().join(format!("php-rust-disk-space-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let directory = root.to_string_lossy().into_owned();
    let missing = root.join("missing").to_string_lossy().into_owned();
    let script = format!(r#"<?php
        error_reporting(E_WARNING);
        $free = disk_free_space(directory: '{directory}');
        $total = disk_total_space('{directory}');
        $alias = diskfreespace('{directory}');
        $callback = array_map('disk_total_space', ['{directory}'])[0];
        $failed = disk_free_space('{missing}');
        echo ($free > 0), ':', ($total >= $free), ':', ($alias > 0), ':', ($callback > 0), ':', ($failed === false), ':', function_exists('diskfreespace');
    "#);
    let mut globals = std::collections::HashMap::new();
    globals.insert("\0filesystem_root".into(), php_runtime::Value::String(directory));
    let result = compile_source(&script).unwrap().execute_with_globals(globals).unwrap();
    assert_eq!(result.output, "1:1:1:1:1:1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(result.diagnostics[0].source_line, Some(7));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn lstat_keeps_the_final_symlink_component_inside_the_sandbox() {
    use std::os::unix::fs::symlink;
    let root = std::env::temp_dir().join(format!("php-rust-lstat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("target.txt"), "payload").unwrap();
    symlink("target.txt", root.join("link.txt")).unwrap();
    let script = format!("<?php $path = '{}'; echo stat($path)['size'], ':', lstat($path)['size'];", root.join("link.txt").display());
    let mut globals = std::collections::HashMap::new();
    globals.insert("\0filesystem_root".into(), php_runtime::Value::String(root.to_string_lossy().into_owned()));
    assert_eq!(compile_source(&script).unwrap().execute_with_globals(globals).unwrap().output, "7:10");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn include_path_is_request_local_and_honored_by_file_read_apis() {
    static NEXT_FILE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    let directory = std::env::temp_dir().join(format!("php-rust-include-path-{}-{}", std::process::id(), NEXT_FILE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("payload.txt"), "alpha\nbeta\n").unwrap();
    let directory = directory.to_string_lossy().into_owned();
    let source = format!(r#"<?php
        $old = set_include_path(include_path: '{directory}');
        echo get_include_path(), ':', $old, ':', PATH_SEPARATOR, ':';
        echo file_get_contents(filename: 'payload.txt', use_include_path: true), ':';
        echo implode('|', file(filename: 'payload.txt', flags: FILE_USE_INCLUDE_PATH | FILE_IGNORE_NEW_LINES)), ':';
        $stream = fopen(filename: 'payload.txt', mode: 'rb', use_include_path: true);
        echo stream_get_contents($stream), ':', fclose($stream), ':', function_exists('set_include_path'), ':', function_exists('get_include_path'), "\n";
    "#);
    let output = compile_source(&source).unwrap().execute().unwrap();
    assert!(output.starts_with(&format!("{directory}:.{}:", if cfg!(windows) { ";" } else { ":/usr/share/php" })), "{output}");
    assert!(output.contains("alpha\nbeta\n:alpha|beta:"), "{output}");
    assert!(output.ends_with("alpha\nbeta\n:1:1:1\n"), "{output}");
    let fresh_request = compile_source("<?php echo get_include_path();").unwrap().execute().unwrap();
    assert_eq!(fresh_request, if cfg!(windows) { ".;" } else { ".:/usr/share/php" });
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn fopen_local_files_supports_write_append_read_and_persistent_close() {
    static NEXT_FILE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    let path = std::env::temp_dir().join(format!("php-rust-fopen-{}-{}.tmp", std::process::id(), NEXT_FILE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    assert!(!path.exists());
    let path = path.to_string_lossy().into_owned();
    let source = format!(r#"<?php
        $file = fopen(filename: '{path}', mode: 'w+b');
        fwrite($file, 'first'); rewind($file);
        echo stream_get_contents($file), ':'; fclose($file);
        $file = fopen('{path}', 'ab'); fwrite($file, '-second'); fclose($file);
        echo file_get_contents('{path}'), "\n";
    "#);
    assert_eq!(compile_source(&source).unwrap().execute().unwrap(), "first:first-second\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"first-second");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn file_put_contents_writes_arrays_and_honors_append_and_lock_flags() {
    static NEXT_FILE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    let path = std::env::temp_dir().join(format!("php-rust-put-{}-{}.tmp", std::process::id(), NEXT_FILE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    assert!(!path.exists());
    let path = path.to_string_lossy().into_owned();
    let source = format!(r#"<?php
        echo file_put_contents(filename: '{path}', data: ['hello', '-world']), ':';
        echo file_put_contents(filename: '{path}', data: '-again', flags: FILE_APPEND | LOCK_EX), ':';
        echo file_get_contents('{path}'), ':';
        $stream = fopen('php://temp', 'w+b'); fwrite($stream, 'streamed'); rewind($stream);
        echo file_put_contents(filename: '{path}', data: $stream), ':', file_get_contents('{path}'), ':';
        fclose($stream); echo function_exists('file_put_contents'), "\n";
    "#);
    assert_eq!(compile_source(&source).unwrap().execute().unwrap(), "11:6:hello-world-again:8:streamed:1\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"streamed");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn copy_supports_named_arguments_callback_dispatch_and_failure_diagnostics() {
    static NEXT_COPY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    let nonce = NEXT_COPY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let source_path = std::env::temp_dir().join(format!("php-rust-copy-{}-{nonce}-source.tmp", std::process::id()));
    let target_path = std::env::temp_dir().join(format!("php-rust-copy-{}-{nonce}-target.tmp", std::process::id()));
    let callback_target = std::env::temp_dir().join(format!("php-rust-copy-{}-{nonce}-callback.tmp", std::process::id()));
    std::fs::write(&source_path, b"copy payload").unwrap();
    let source_path = source_path.to_string_lossy().into_owned();
    let target_path = target_path.to_string_lossy().into_owned();
    let callback_target = callback_target.to_string_lossy().into_owned();
    let source = format!(r#"<?php
        echo copy(from: '{source_path}', to: '{target_path}'), ':';
        $results = array_map('copy', ['{source_path}'], ['{callback_target}']);
        echo $results[0], ':', file_get_contents('{target_path}'), ':', file_get_contents('{callback_target}'), ':';
        echo function_exists('copy');
    "#);
    assert_eq!(compile_source(&source).unwrap().execute().unwrap(), "1:1:copy payload:copy payload:1");
    assert_eq!(std::fs::read(&target_path).unwrap(), b"copy payload");
    assert_eq!(std::fs::read(&callback_target).unwrap(), b"copy payload");

    let failure = compile_source("<?php $result = copy('/missing/php-rust-source', '/tmp/php-rust-copy-target'); echo $result;")
        .unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(failure.output, "");
    assert_eq!(failure.diagnostics.len(), 1);
    assert_eq!(failure.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert!(failure.diagnostics[0].message.starts_with("copy(): Failed to copy "));

    std::fs::remove_file(target_path).unwrap();
    std::fs::remove_file(callback_target).unwrap();
    std::fs::remove_file(source_path).unwrap();
}

#[test]
fn rename_supports_named_arguments_callback_dispatch_and_failure_diagnostics() {
    static NEXT_RENAME: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    let nonce = NEXT_RENAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let direct_source = std::env::temp_dir().join(format!("php-rust-rename-{}-{nonce}-source.tmp", std::process::id()));
    let direct_target = std::env::temp_dir().join(format!("php-rust-rename-{}-{nonce}-target.tmp", std::process::id()));
    let callback_source = std::env::temp_dir().join(format!("php-rust-rename-{}-{nonce}-callback-source.tmp", std::process::id()));
    let callback_target = std::env::temp_dir().join(format!("php-rust-rename-{}-{nonce}-callback-target.tmp", std::process::id()));
    std::fs::write(&direct_source, b"direct").unwrap();
    std::fs::write(&callback_source, b"callback").unwrap();
    let direct_source = direct_source.to_string_lossy().into_owned();
    let direct_target = direct_target.to_string_lossy().into_owned();
    let callback_source = callback_source.to_string_lossy().into_owned();
    let callback_target = callback_target.to_string_lossy().into_owned();
    let source = format!(r#"<?php
        echo rename(from: '{direct_source}', to: '{direct_target}'), ':', file_get_contents('{direct_target}'), ':';
        $results = array_map('rename', ['{callback_source}'], ['{callback_target}']);
        echo $results[0], ':', file_get_contents('{callback_target}'), ':', function_exists('rename');
    "#);
    assert_eq!(compile_source(&source).unwrap().execute().unwrap(), "1:direct:1:callback:1");

    let failure = compile_source("<?php $result = rename('/missing/php-rust-source', '/tmp/php-rust-rename-target'); echo $result;")
        .unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(failure.output, "");
    assert_eq!(failure.diagnostics.len(), 1);
    assert_eq!(failure.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert!(failure.diagnostics[0].message.starts_with("rename(): Failed to rename "));

    std::fs::remove_file(direct_target).unwrap();
    std::fs::remove_file(callback_target).unwrap();
}

#[test]
fn mkdir_and_rmdir_support_recursive_sandboxed_and_callback_operations() {
    static NEXT_DIRECTORY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    let nonce = NEXT_DIRECTORY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("php-rust-dir-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let nested = root.join("a/b").to_string_lossy().into_owned();
    let first = root.join("a").to_string_lossy().into_owned();
    let callback_dir = root.join("callback").to_string_lossy().into_owned();
    let root_path = root.to_string_lossy().into_owned();
    let source = format!(r#"<?php
        echo mkdir(directory: '{nested}', recursive: true), ':', is_dir('{nested}'), ':', mkdir(directory: '{nested}', recursive: true), ':';
        $created = array_map('mkdir', ['{callback_dir}']);
        echo $created[0], ':', is_dir('{callback_dir}'), ':';
        $removed = array_map('rmdir', ['{nested}', '{first}', '{callback_dir}']);
        echo $removed[0], $removed[1], $removed[2], ':';
        echo function_exists('mkdir'), function_exists('rmdir');
    "#);
    let mut globals = std::collections::HashMap::new();
    globals.insert("\0filesystem_root".to_string(), php_runtime::Value::String(root_path.clone()));
    let result = compile_source(&source).unwrap().execute_with_globals(globals).unwrap();
    assert_eq!(result.output, "1:1::1:1:111:11");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn temporary_files_can_be_written_read_and_removed() {
    let source = r#"<?php
        $path = tempnam(directory: sys_get_temp_dir(), prefix: 'php-rust-fs');
        echo file_put_contents(filename: $path, data: 'hey'), ':', file_get_contents($path), ':';
        echo unlink(filename: $path), ':', file_exists($path), ':', function_exists('tempnam'), ':', function_exists('unlink'), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "3:hey:1::1:1\n");
}

#[test]
fn file_reads_lines_and_applies_newline_and_empty_line_flags() {
    let source = r#"<?php
        $path = tempnam(sys_get_temp_dir(), 'php-rust-lines');
        echo count(file($path)), ':';
        file_put_contents($path, "alpha\r\n\r\nbeta\n");
        echo implode('|', file(filename: $path)), ':';
        echo implode('|', file($path, FILE_IGNORE_NEW_LINES)), ':';
        echo implode('|', file(filename: $path, flags: FILE_IGNORE_NEW_LINES | FILE_SKIP_EMPTY_LINES)), "\n";
        unlink($path);
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "0:alpha\r\n|\r\n|beta\n:alpha||beta:alpha|beta\n");
}

#[test]
fn core_time_helpers_support_named_arguments_and_expected_shapes() {
    let source = r#"<?php
        $high = hrtime();
        echo gettype(time()), ':', count($high), ':', gettype($high[0]), ':', gettype($high[1]), ':';
        echo gettype(hrtime(as_number: true)), ':', is_float(microtime(as_float: true)), ':';
        echo str_starts_with(microtime(), '0.'), ':', function_exists('time'), ':', function_exists('microtime'), ':', function_exists('hrtime'), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "integer:2:integer:integer:integer:1:1:1:1:1\n");
}

#[test]
fn date_and_gmdate_format_common_tokens_and_escaped_literals() {
    let source = r#"<?php
        echo gmdate(format: 'Y-m-d H:i:s e p P O U', timestamp: 0), ':';
        echo gmdate('l, F jS \\a\\t g:i A', 0), ':';
        echo function_exists('date'), ':', strlen(date('c')) > 10, "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "1970-01-01 00:00:00 UTC Z +00:00 +0000 0:Thursday, January 1st at 12:00 AM:1:1\n");
}

#[test]
fn getdate_idate_and_checkdate_cover_calendar_edges() {
    let source = r#"<?php
        $parts = getdate(timestamp: 0);
        echo $parts[0], ':', $parts['year'], ':', $parts['month'], ':', $parts['weekday'], ':';
        echo idate(format: 'Y', timestamp: 0), ':', idate('W', 0), ':', idate('S', 0) === false, ':';
        echo checkdate(month: 2, day: 29, year: 2024), ':', checkdate(2, 29, 2023), ':', checkdate(13, 1, 2024), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "0:1970:January:Thursday:1970:1:1:1::\n");
}

#[test]
fn mktime_and_gmmktime_accept_named_components_and_normalize_dates() {
    let source = r#"<?php
        echo gmmktime(year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0), ':';
        echo gmdate('Y-m-d H:i:s', gmmktime(hour: 0, minute: 0, second: 0, month: 2, day: 0, year: 1970)), ':';
        echo function_exists('mktime'), ':', function_exists('gmmktime'), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "0:1970-01-31 00:00:00:1:1\n");
}

#[test]
fn localtime_supports_numeric_and_associative_fields() {
    let source = r#"<?php
        $numeric = localtime(0);
        $named = localtime(timestamp: 0, associative: true);
        echo count($numeric), ':', count($named), ':', is_int($numeric[0]), ':', is_int($named['tm_year']), ':', function_exists('localtime'), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "9:9:1:1:1\n");
}

#[test]
fn strtotime_parses_iso_timestamps_and_common_relative_words() {
    let source = r#"<?php
        $absolute = strtotime(datetime: '1970-01-02T03:04:05Z');
        $nextDay = strtotime(datetime: 'tomorrow', baseTimestamp: 0);
        echo gmdate('Y-m-d H:i:s', $absolute), ':', $nextDay > 0, ':', strtotime('not a date') === false, "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1970-01-02 03:04:05:1:1\n");
}

#[test]
fn random_int_uses_named_bounds_and_throws_for_invalid_bounds() {
    let source = r#"<?php
        $value = random_int(max: 11, min: -7);
        echo ($value >= -7 && $value <= 11), ':', function_exists('random_int'), ':';
        try { random_int(min: 4, max: 3); } catch (ValueError $error) { echo get_class($error), ':'; }
        try { random_int([], 3); } catch (TypeError $error) { echo get_class($error); }
        echo "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:1:ValueError:TypeError\n");
}

#[test]
fn mt_rand_and_rand_support_defaults_named_bounds_and_getrandmax() {
    let source = r#"<?php
        $one = mt_rand(max: 3);
        $two = rand(8, 10);
        $default = mt_rand();
        echo ($one >= 0 && $one <= 3), ':', ($two >= 8 && $two <= 10), ':';
        echo ($default >= 0 && $default <= mt_getrandmax()), ':', getrandmax() === mt_getrandmax(), ':';
        echo function_exists('rand'), ':', function_exists('mt_rand'), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:1:1:1:1:1\n");
}

#[test]
fn integer_and_float_literals_keep_distinct_runtime_types_through_arithmetic() {
    let source = r#"<?php
        $integer = 1;
        $integer++;
        echo gettype(1), ':', gettype(1.0), ':', gettype(1e0), ':', get_debug_type(-1), ':';
        echo is_int(1), ':', is_int(1.0), ':', is_float(1.0), ':';
        echo gettype(1 + 2), ':', gettype(1 + 2.0), ':', gettype(5 % 2), ':', gettype($integer), "\n";
        echo implode(':', array_map('gettype', [1, 1.0, 1e0])), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "integer:double:double:int:1::1:integer:double:integer:integer\ninteger:double:double\n");
}

#[test]
fn strict_array_membership_keeps_integer_and_float_types_distinct() {
    let source = r#"<?php
        echo in_array(1, [1.0], true), ':', in_array(1.0, [1.0], true), ':', in_array(1, [1.0]), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, ":1:1\n");
}

#[test]
fn weak_numeric_parameter_types_coerce_values_before_function_execution() {
    let source = r#"<?php
        function inspectNumbers(int $whole, float $decimal) {
            echo gettype($whole), ':', $whole, ':', gettype($decimal), ':', $decimal, "\n";
        }
        inspectNumbers(whole: '7.8', decimal: 4);
        function inspectDecimals(float ...$values) { echo gettype($values[0]), ':', gettype($values[1]), "\n"; }
        inspectDecimals(2, '3.5');
        class NumericMethod { public function show(int $value) { return gettype($value) . ':' . $value; } }
        $numericClosure = function (float $value) { return gettype($value) . ':' . $value; };
        echo (new NumericMethod())->show('9.4'), ':', $numericClosure(2), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "integer:7:double:4\ndouble:double\ninteger:9:double:2\n");
}

#[test]
fn union_types_work_for_functions_methods_returns_closures_and_properties() {
    let source = r#"<?php
        function keep(int|string $value): int|string { return $value; }
        function number(int|float $value): int|float { return $value; }
        function numericResult(): int|float { return '6.5'; }
        class UnionRecord {
            public int|string $value;
            public function __construct(int|string $value) { $this->value = $value; }
            public function read(): int|string { return $this->value; }
        }
        $closure = function (int|string $value): int|string { return $value; };
        $record = new UnionRecord('text');
        echo keep(7), ':', keep('word'), ':', number('4.5'), ':', numericResult(), ':', $closure(8), ':', $record->read(), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "7:word:4.5:6.5:8:text\n");
}

#[test]
fn union_types_support_contract_variance_nullable_and_reject_invalid_values() {
    let source = r#"<?php
        interface NumberContract { public function value(int $input): int|float; }
        class NumberImplementation implements NumberContract {
            public function value(int|float $input): int { return 5; }
        }
        function nullable(?int $value): int|null { return $value; }
        echo (new NumberImplementation())->value(2), ':', nullable(null) === null, ':', nullable(3), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "5:1:3\n");
    let invalid = compile_source("<?php function onlyNumbers(int|float $value) {} onlyNumbers('word');").unwrap().execute().unwrap_err();
    assert!(invalid.contains("must be of type int|float"), "unexpected error: {invalid}");
    assert!(compile_source("<?php function invalid(?int|string $value) {} ").is_err());
}

#[test]
fn intersection_and_dnf_types_validate_objects_and_properties() {
    let source = r#"<?php
        interface FirstContract {}
        interface SecondContract {}
        class BothContracts implements FirstContract, SecondContract {}
        class FirstOnly implements FirstContract {}
        class Holder { public FirstContract&SecondContract $value; }
        function accept(FirstContract&SecondContract $value): (FirstContract&SecondContract)|FirstOnly { return $value; }
        $holder = new Holder();
        $holder->value = new BothContracts();
        $value = $holder->value;
        echo get_class(accept($value)), ':', get_class($value), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "BothContracts:BothContracts\n");
    let invalid = r#"<?php interface FirstContract {} interface SecondContract {} class FirstOnly implements FirstContract {} function accept(FirstContract&SecondContract $value) {} accept(new FirstOnly());"#;
    assert!(compile_source(invalid).unwrap().execute().unwrap_err().contains("must be of type firstcontract&secondcontract"));
    assert!(compile_source("<?php interface A {} interface B {} function invalid(A|B&C $x) {} ").is_err());
    assert!(compile_source("<?php interface A {} interface B {} function invalid(?A&B $x) {} ").is_err());
    let variance = r#"<?php
        interface LeftContract {}
        interface RightContract {}
        interface VarianceContract { public function convert(LeftContract&RightContract $value): LeftContract; }
        class VarianceImplementation implements VarianceContract {
            public function convert(LeftContract $value): LeftContract&RightContract { return $value; }
        }
    "#;
    assert!(compile_source(variance).is_ok());
}

#[test]
fn array_parameter_defaults_are_constant_and_fresh_per_call() {
    let source = r#"<?php
        function defaults($items = ['first', 'label' => [2, 'deep' => true]]) { return $items; }
        $first = defaults();
        $first[0] = 'changed';
        $first['label'][0] = 99;
        $second = defaults();
        echo $second[0], ':', $second['label'][0], ':', $second['label']['deep'], "\n";
        $closure = function ($items = ['closure']) { return $items[0]; };
        class Defaulted { public function value($items = ['method']) { return $items[0]; } }
        echo $closure(), ':', (new Defaulted())->value(), "\n";
        function keyed($items = ['1' => 'integer key', '01' => 'string key', 3 => 'explicit', 'tail']) { return $items; }
        $keys = keyed();
        echo $keys[1], ':', $keys['01'], ':', $keys[4], "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "first:2:1\nclosure:method\ninteger key:string key:tail\n");
}

#[test]
fn parameter_default_arrays_reject_runtime_expressions_and_reference_entries() {
    for source in [
        "<?php function invalid($value = [$runtime]);",
        "<?php function invalid($value = [&$other]);",
        "<?php function invalid($value = [...[1]]);",
    ] {
        assert!(compile_source(source).is_err(), "unexpectedly accepted {source}");
    }
}

#[test]
fn parameter_defaults_evaluate_constant_arithmetic_and_concatenation() {
    let source = r#"<?php
        function computed($number = 2 + 3 * 4, $text = 'php' . '-' . 'runtime', $items = [1 + 1 => -3], $flags = (1 << 2) | 1) {
            echo $number, ':', $text, ':', $items[2], ':', $flags, "\n";
        }
        computed();
        computed(8, 'custom', [2 => 9]);
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "14:php-runtime:-3:5\n8:custom:9:5\n");

    for source in [
        "<?php function invalid($value = 1 / 0);",
        "<?php function invalid($value = 1 + $dynamic);",
        "<?php function invalid($value = strlen('constant'));",
    ] {
        assert!(compile_source(source).is_err(), "unexpectedly accepted {source}");
    }
}

#[test]
fn parameter_defaults_resolve_public_self_and_parent_class_constants() {
    let source = r#"<?php
        function useLaterClass($value = LaterDefault::ANSWER) { return $value; }
        function useSplDefault($value = CachingIterator::CALL_TOSTRING | RecursiveIteratorIterator::SELF_FIRST) { return $value; }
        function usePdoDefault($value = PDO::FETCH_OBJ) { return $value; }
        function useSortDefault($value = SortDirection::Descending) { return $value->name; }
        class ChildDefault extends ParentDefault {
            public function fromSelf($value = self::ANSWER) { return $value; }
            public function fromParent($value = parent::BASE) { return $value; }
            public function closureDefault() {
                $closure = function ($value = self::ANSWER) { return $value; };
                return $closure();
            }
        }
        interface InterfaceDefault { public const INTERFACE_VALUE = 12; }
        class InterfaceConstantDefault implements InterfaceDefault { public function value($value = self::INTERFACE_VALUE) { return $value; } }
        class LaterDefault { public const ANSWER = 42; }
        class ParentDefault { protected const BASE = 7; public const ANSWER = 9; }
        echo useLaterClass(), ':', (new ChildDefault())->fromSelf(), ':', (new ChildDefault())->fromParent(), ':', (new ChildDefault())->closureDefault(), ':', useSplDefault(), ':', (new InterfaceConstantDefault())->value(), ':', usePdoDefault(), ':', useSortDefault(), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "42:9:7:9:1:12:5:Descending\n");

    let inaccessible = "<?php class HiddenDefault { private const VALUE = 1; } function invalid($value = HiddenDefault::VALUE) {}";
    assert!(compile_source(inaccessible).is_err());
    let late_static = "<?php class LateDefault { const VALUE = 1; public function invalid($value = static::VALUE) {} }";
    assert!(compile_source(late_static).is_err());
}

#[test]
fn weak_numeric_return_types_coerce_function_method_and_closure_results() {
    let source = r#"<?php
        function whole(): int { return 7.8; }
        function decimal(): float { return 4; }
        class NumericReturn {
            public function whole(): int { return '9.6'; }
            public function decimal(): float { return true; }
        }
        $wholeClosure = function (): int { return 2.9; };
        $decimalClosure = fn (): float => '3.5';
        $object = new NumericReturn();
        echo gettype(whole()), ':', whole(), ':', gettype(decimal()), ':', decimal(), "\n";
        echo gettype($object->whole()), ':', $object->whole(), ':', gettype($object->decimal()), ':', $object->decimal(), "\n";
        echo gettype($wholeClosure()), ':', $wholeClosure(), ':', gettype($decimalClosure()), ':', $decimalClosure(), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "integer:7:double:4\ninteger:9:double:1\ninteger:2:double:3.5\n");
}

#[test]
fn cli_superglobals_exist_as_shared_arrays_in_function_scopes() {
    let source = r#"<?php
        echo isset($_GET), ':', count($_GET), ':';
        function updateQuery() {
            echo isset($_GET), ':';
            $_GET['name'] = 'Ada';
        }
        updateQuery();
        echo $_GET['name'], "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "1:0:1:Ada\n");
}

#[test]
fn globals_array_reads_writes_nested_values_and_unsets_global_slots() {
    let source = r#"<?php
        $count = 2;
        $GLOBALS['count'] += 3;
        $GLOBALS['nested']['value'] = 'updated';
        $GLOBALS['created'] = 9;
        function updateGlobals() {
            $GLOBALS['count']++;
            unset($GLOBALS['created']);
        }
        updateGlobals();
        $copy = $GLOBALS;
        $copy['count'] = 100;
        echo $count, ':', $GLOBALS['count'], ':', $nested['value'], ':', isset($GLOBALS['created']), ':', count($copy), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "6:6:updated::10\n");
}

#[test]
fn globals_array_cannot_be_reassigned() {
    let error = compile_source("<?php $GLOBALS = []; ").err().unwrap();
    assert!(error.contains("Cannot re-assign $GLOBALS"));
}

#[test]
fn globals_follow_php81_readonly_snapshot_and_no_self_entry_rules() {
    let source = r#"<?php
        $value = 1;
        $snapshot = $GLOBALS;
        $snapshot['value'] = 2;
        $GLOBALS['value'] = 3;
        echo $value, ':', $snapshot['value'], ':', array_key_exists('GLOBALS', $GLOBALS), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "3:2:\n");

    let error = compile_source("<?php $GLOBALS = ['value' => 2];").err().unwrap();
    assert!(error.contains("Cannot re-assign $GLOBALS"));
    let by_reference = compile_source("<?php $alias =& $GLOBALS;").err().unwrap();
    assert!(by_reference.contains("Cannot create a reference to $GLOBALS itself"));
    let target_reference = compile_source("<?php $GLOBALS =& $alias;").err().unwrap();
    assert!(target_reference.contains("Cannot create a reference to $GLOBALS itself"));
    let mutator = compile_source("<?php array_pop($GLOBALS);").err().unwrap();
    assert!(mutator.contains("Cannot re-assign $GLOBALS"));
    let unset = compile_source("<?php unset($GLOBALS);").err().unwrap();
    assert!(unset.contains("Cannot unset $GLOBALS"));
}

#[test]
fn globals_write_and_unset_diagnostics_happen_during_compilation() {
    for (source, expected) in [
        ("<?php $GLOBALS = [];", "Cannot re-assign $GLOBALS"),
        ("<?php $GLOBALS += [];", "Cannot re-assign $GLOBALS"),
        ("<?php $GLOBALS =& $alias;", "Cannot create a reference to $GLOBALS itself"),
        ("<?php $alias =& $GLOBALS;", "Cannot create a reference to $GLOBALS itself"),
        ("<?php unset($GLOBALS);", "Cannot unset $GLOBALS"),
        ("<?php array_pop($GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php extract($GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php extract(array: $GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php next($GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php reset(array: $GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php end($GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php array_push(array: $GLOBALS, value: 1);", "Cannot re-assign $GLOBALS"),
        ("<?php function mutateGlobals(&$table) {} mutateGlobals($GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php function mutateNamed($prefix, &$table) {} mutateNamed(table: $GLOBALS, prefix: 1);", "Cannot re-assign $GLOBALS"),
        ("<?php class GlobalMutator { public function mutate(&$table) {} } (new GlobalMutator())->mutate($GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php class GlobalMutator { public function mutate(&$table) {} } (new GlobalMutator())->mutate(table: $GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php class GlobalMutator { public static function mutate(&$table) {} } GlobalMutator::mutate($GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php function bindGlobal(PDOStatement $statement) { $statement->bindParam(':value', $GLOBALS); }", "Cannot re-assign $GLOBALS"),
        ("<?php function bindGlobal(PDOStatement $statement) { $statement->bindColumn(column: 1, var: $GLOBALS); }", "Cannot re-assign $GLOBALS"),
        ("<?php ('array_pop')($GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php $callback = 'array_pop'; $callback($GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php $callback = 'array_pop'; $alias = $callback; $alias($GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php function mutateCallback(&$table) {} $callback = 'mutateCallback'; $callback($GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php array_pop(... )($GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php class GlobalMutator { public function mutate(&$table) {} } [new GlobalMutator(), 'mutate']($GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php class GlobalMutator { public static function mutate(&$table) {} } ['GlobalMutator', 'mutate']($GLOBALS);", "Cannot re-assign $GLOBALS"),
        ("<?php class GlobalMutatorParent { public static function mutate(&$table) {} } class GlobalMutatorChild extends GlobalMutatorParent { public static function call() { parent::mutate(... )($GLOBALS); } }", "Cannot re-assign $GLOBALS"),
        ("<?php trait GlobalMutatorTrait { public function mutate(&$table) {} } class GlobalMutator { use GlobalMutatorTrait; } (new GlobalMutator())->mutate($GLOBALS);", "Cannot re-assign $GLOBALS"),
    ] {
        let error = compile_source(source).err().unwrap_or_else(|| panic!("expected compile error for {source}"));
        assert!(error.contains(expected), "{source}: {error}");
    }
    assert!(compile_source("<?php $callback = 'array_pop'; $callback = 'strlen'; $callback($GLOBALS);").is_ok());
    assert!(compile_source("<?php $callback = 'array_pop'; unset($callback); $callback($GLOBALS);").is_ok());
    assert!(compile_source("<?php $callback = 'strlen'; if ($condition) { $callback = 'array_pop'; } else { $callback($GLOBALS); }").is_ok());
    assert!(compile_source("<?php class LocalStatement { public function bindParam($key, $value) {} } function safe(PDOStatement $statement) { $statement = new LocalStatement(); $statement->bindParam(':value', $GLOBALS); }").is_ok());
}

#[test]
fn globals_entries_can_bind_direct_and_nested_references() {
    let source = r#"<?php
        $left = 1;
        $right = 2;
        $GLOBALS['left'] =& $right;
        $right = 5;
        $GLOBALS['nested']['value'] =& $right;
        $right = 7;
        $alias =& $GLOBALS['right'];
        $alias = 8;
        echo $left, ':', $GLOBALS['left'], ':', $nested['value'], "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "8:8:8\n");
}

#[test]
fn by_reference_array_mutators_update_globals_entries() {
    let source = r#"<?php
        $GLOBALS['items'] = ['first', 'last'];
        $removed = array_pop($GLOBALS['items']);
        array_push($GLOBALS['items'], 'next');
        $GLOBALS['appended'][] = 'one';
        $GLOBALS['appended'][] = 'two';
        $removedAppended = array_pop($GLOBALS['appended']);
        echo $removed, ':', count($items), ':', $items[1], ':', $removedAppended, ':', count($GLOBALS['appended']), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "last:2:next:two:1\n");
}


#[test]
fn bitwise_operators_follow_php_precedence_and_compound_assignment() {
    let source = r#"<?php
        echo (6 & 3), ':', (6 ^ 3), ':', (6 | 3), ':', (1 + 2 & 1), ':';
        echo (1 | 2 & 3), ':', ((1 | 2 == 2) ? 'true' : 'false'), "\n";
        $value = 6;
        $values = [6];
        class BitwiseBox { public $value = 6; }
        $box = new BitwiseBox();
        echo ($value &= 3), ':', ($values[0] ^= 3), ':', ($box->value |= 1), ':', $value, ':', $values[0], ':', $box->value, "\n";
        echo ('a' & 'b'), ':', ('A' ^ 'a'), ':', ('a' | 'b'), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "2:5:7:1:3:true\n2:5:7:2:5:7\n`: :c\n");
}

#[test]
fn bit_shifts_and_complement_follow_php_precedence_and_errors() {
    let source = r#"<?php
        echo (3 << 2), ':', (16 >> 2), ':', (1 + 2 << 1), ':', (8 >> 1 + 1), ':';
        echo (1 << 2 . 'x'), ':', (~0), ':', (~1), ':', (-8 >> 2), "\n";
        $value = 1;
        $value <<= 3;
        $value >>= 1;
        echo $value, "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "12:4:6:2:4x:-1:-2:-2\n4\n");

    let negative_shift = compile_source("<?php echo 1 << -1;").unwrap().execute().unwrap_err();
    assert!(negative_shift.contains("Bit shift by negative number"));
}

#[test]
fn strict_equality_compares_runtime_types_order_and_object_identity() {
    let source = r#"<?php
        function flag($value) { return $value ? '1' : '0'; }
        echo flag(1 === 1), flag(1 === 1.0), flag('1' !== 1), flag(false === 0), "\n";
        echo flag([1] === [1]), flag([1] === [1.0]), flag(['a' => 1, 'b' => 2] === ['b' => 2, 'a' => 1]), "\n";
        class IdentityValue {}
        $first = new IdentityValue();
        $same = $first;
        $copy = clone $first;
        echo flag($first === $same), flag($first !== $copy), "\n";
        $original = 4;
        $alias =& $original;
        echo flag($original === $alias), flag(null === null), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "1010\n100\n11\n11\n");
}

#[test]
fn loose_equality_uses_php8_scalar_array_and_object_rules() {
    let source = r#"<?php
        function flag($value) { return $value ? '1' : '0'; }
        echo flag(0 == 'foo'), flag(0 == '0'), flag('01' == 1), flag('1e2' == 100), flag(true == 'foo'), flag(null == ''), flag(null == 0), "\n";
        echo flag([1] == [1.0]), flag(['x' => 0] == ['x' => 'foo']), flag(['a' => 1, 'b' => 2] == ['b' => 2, 'a' => 1]), "\n";
        class LooseEqualA { public $value = 3; }
        class LooseEqualB { public $value = 3; }
        $first = new LooseEqualA();
        $second = new LooseEqualA();
        $otherClass = new LooseEqualB();
        echo flag($first == $second), flag($first == $otherClass), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "0111111\n101\n10\n");
}

#[test]
fn scalar_casts_parse_common_aliases_and_numeric_prefixes() {
    let source = r#"<?php
        echo gettype((int) '12.9tail'), ':', (int) '-3.8', ':', (int) '1e2', "\n";
        echo gettype((float) 10), ':', (float) '2.5 meters', ':', (string) true, ':', (string) false, ':', (string) null, "\n";
        echo (int) 'not numeric', ':', (bool) '0', ':', (bool) '0.0', ':', (binary) 42, ':', (real) 2, "\n";
        echo (string) (1 + 2), ':', (int) (2.9 ** 2), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "integer:-3:100\ndouble:2.5:1::\n0::1:42:2\n3:8\n");
}

#[test]
fn string_cast_dispatches_and_validates_magic_to_string() {
    let source = r#"<?php
        class CastStringable {
            public function __toString(): string { return 'cast value'; }
        }
        $value = new CastStringable();
        echo (string) $value, ':', gettype((string) $value), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "cast value:string\n");

    let missing = compile_source("<?php class NotStringable {} echo (string) new NotStringable();")
        .unwrap().execute().unwrap_err();
    assert!(missing.contains("could not be converted to string"));

    let weak_scalar = compile_source("<?php class ScalarStringable { public function __toString() { return 1; } } echo (string) new ScalarStringable();")
        .unwrap().execute().unwrap();
    assert_eq!(weak_scalar, "1");
}

#[test]
fn string_builtins_coerce_stringable_objects_at_string_parameters() {
    let source = r#"<?php
        class BuiltinStringable {
            public function __toString(): string { return 'hello'; }
        }
        $value = new BuiltinStringable();
        echo strlen($value), ':', strtoupper($value), ':', strpos('say hello', $value), ':';
        echo str_replace($value, 'bye', 'hello!'), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "5:HELLO:4:bye!\n");

    let plain_object = compile_source("<?php class PlainValue {} echo strlen(new PlainValue());")
        .unwrap().execute().unwrap_err();
    assert!(plain_object.contains("could not be converted to string"));
}

#[test]
fn array_and_object_casts_convert_scalars_and_preserve_objects() {
    let source = r#"<?php
        $scalarArray = (array) 7;
        $nullArray = (array) null;
        echo count($scalarArray), ':', $scalarArray[0], ':', count($nullArray), "\n";
        $object = (object) ['name' => 'Ada', 'role' => 'admin'];
        $arrayAgain = (array) $object;
        echo get_class($object), ':', $object->name, ':', $arrayAgain['role'], "\n";
        $scalarObject = (object) false;
        $nullObject = (object) null;
        echo get_class($scalarObject), ':', $scalarObject->scalar, ':', get_class($nullObject), "\n";
        class KeepIdentity {}
        $instance = new KeepIdentity();
        $castInstance = (object) $instance;
        echo get_class($castInstance), ':', ($instance === $castInstance), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "1:7:0\nstdClass:Ada:admin\nstdClass::stdClass\nKeepIdentity:1\n");
}
