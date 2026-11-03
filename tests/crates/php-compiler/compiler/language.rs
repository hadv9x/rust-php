use php_compiler::{compile_file, compile_source, execute_file_with_globals_and_limits};

#[test]
fn class_property_defaults_accept_nested_constant_arrays() {
    let source = "<?php class Defaults { public array $options = ['mode' => 'safe', 'items' => [2, 4], 5 => true, 'next']; public SortDirection $direction = SortDirection::Ascending; public int $size = 20 * 1024 * 1024; } $defaults = new Defaults(); echo $defaults->options['mode'], ':', $defaults->options['items'][1], ':', $defaults->options[6] ? 'yes' : 'no', ':', $defaults->direction->name, ':', $defaults->size;";
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "safe:4:yes:Ascending:20971520");
}

#[test]
fn laravel_request_property_defaults_accept_inherited_symfony_header_constants() {
    let source = r#"<?php namespace Illuminate\Http; class Request { public int $forwarded = Request::HEADER_X_FORWARDED_FOR; } $request = new Request(); echo $request->forwarded;"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2");
}

#[test]
fn named_class_name_constant_resolves_to_the_qualified_name() {
    assert_eq!(compile_source(r"<?php namespace Example; echo \DateTime::class;").unwrap().execute().unwrap(), "DateTime");
}

#[test]
fn compile_file_resolves_magic_file_constants_in_class_defaults() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!("rust_php_magic_defaults_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&directory).unwrap();
    let entry = directory.join("entry.php");
    std::fs::write(&entry, "<?php if (true) { class FileDefaults { public string $directory = __DIR__; public string $file = __FILE__; } } $value = new FileDefaults(); echo $value->directory, ':', $value->file;").unwrap();
    let output = compile_file(&entry).unwrap().execute().unwrap();
    std::fs::remove_dir_all(directory.clone()).unwrap();
    assert_eq!(output, format!("{}:{}", directory.display(), entry.display()));
}

#[test]
fn included_files_contribute_namespaced_constants() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!("rust_php_constants_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&directory).unwrap();
    let included = directory.join("definitions.php");
    let entry = directory.join("entry.php");
    std::fs::write(&included, "<?php namespace Shared; const ANSWER = 42;").unwrap();
    std::fs::write(&entry, r#"<?php include 'definitions.php'; echo Shared\ANSWER, "\n";"#).unwrap();
    let output = compile_file(&entry).unwrap().execute().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert_eq!(output, "42\n");
}

#[test]
fn web_program_cache_reuses_bytecode_and_invalidates_when_an_included_source_changes() {
    use php_runtime::ExecutionLimits;
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!("rust_php_program_cache_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&directory).unwrap();
    let included = directory.join("included.php");
    let entry = directory.join("entry.php");
    std::fs::write(&included, "<?php echo 'first';").unwrap();
    std::fs::write(&entry, "<?php include 'included.php';").unwrap();
    let execute = || execute_file_with_globals_and_limits(&entry, &directory, Default::default(), ExecutionLimits::default()).unwrap().output;
    assert_eq!(execute(), "first");
    std::fs::write(&included, "<?php echo 'changed';").unwrap();
    assert_eq!(execute(), "changed");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn compiled_file_exceptions_report_canonical_source_file_and_creation_line() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!("rust_php_exception_file_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&directory).unwrap();
    let entry = directory.join("entry.php");
    std::fs::write(&entry, "<?php\nfunction makeLocated() {\n    return new RuntimeException('located');\n}\ntry {\n    $error = makeLocated();\n    throw $error;\n} catch (RuntimeException $caught) {\n    $frame = $caught->getTrace()[0];\n    echo $caught->getFile(), ':', $caught->getLine(), ':', $frame['file'], ':', $frame['line'], ':', $frame['function'];\n}\n").unwrap();
    let expected_file = entry.canonicalize().unwrap().to_string_lossy().into_owned();
    assert_eq!(compile_file(&entry).unwrap().execute().unwrap(), format!("{expected_file}:3:{expected_file}:3:makeLocated"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn exceptions_created_in_included_files_report_the_included_source_path() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!("rust_php_exception_include_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&directory).unwrap();
    let included = directory.join("included.php");
    let entry = directory.join("entry.php");
    std::fs::write(&included, "<?php\n\n$error = new RuntimeException('from include');\n").unwrap();
    std::fs::write(&entry, "<?php\ninclude 'included.php';\necho $error->getFile(), ':', $error->getLine();\n").unwrap();
    let expected_file = included.canonicalize().unwrap().to_string_lossy().into_owned();
    assert_eq!(compile_file(&entry).unwrap().execute().unwrap(), format!("{expected_file}:3"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn exceptions_created_in_functions_from_included_files_report_their_source_path() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!("rust_php_exception_include_function_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&directory).unwrap();
    let included = directory.join("included.php");
    let entry = directory.join("entry.php");
    std::fs::write(&included, "<?php\nfunction includeLocated() {\n    return new RuntimeException('from include');\n}\n").unwrap();
    std::fs::write(&entry, "<?php\ninclude 'included.php';\n$error = includeLocated();\n$frame = $error->getTrace()[0];\necho $error->getFile(), ':', $error->getLine(), ':', $frame['file'], ':', $frame['line'];\n").unwrap();
    let expected_file = included.canonicalize().unwrap().to_string_lossy().into_owned();
    assert_eq!(compile_file(&entry).unwrap().execute().unwrap(), format!("{expected_file}:3:{expected_file}:3"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn exceptions_created_in_methods_from_included_files_report_their_source_path() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!("rust_php_exception_include_method_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&directory).unwrap();
    let included = directory.join("included.php");
    let entry = directory.join("entry.php");
    std::fs::write(&included, "<?php\nclass IncludedError {\n    public static function create() {\n        return new RuntimeException('from include');\n    }\n}\n").unwrap();
    std::fs::write(&entry, "<?php\ninclude 'included.php';\n$error = IncludedError::create();\n$frame = $error->getTrace()[0];\necho $error->getFile(), ':', $error->getLine(), ':', $frame['file'], ':', $frame['line'];\n").unwrap();
    let expected_file = included.canonicalize().unwrap().to_string_lossy().into_owned();
    assert_eq!(compile_file(&entry).unwrap().execute().unwrap(), format!("{expected_file}:4:{expected_file}:4"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn exceptions_created_in_closures_from_included_files_report_their_source_path() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!("rust_php_exception_include_closure_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&directory).unwrap();
    let included = directory.join("included.php");
    let entry = directory.join("entry.php");
    std::fs::write(&included, "<?php\n$makeIncludedError = function() {\n    return new RuntimeException('from include');\n};\n").unwrap();
    std::fs::write(&entry, "<?php\ninclude 'included.php';\n$error = $makeIncludedError();\n$frame = $error->getTrace()[0];\necho $error->getFile(), ':', $error->getLine(), ':', $frame['file'], ':', $frame['line'];\n").unwrap();
    let expected_file = included.canonicalize().unwrap().to_string_lossy().into_owned();
    assert_eq!(compile_file(&entry).unwrap().execute().unwrap(), format!("{expected_file}:3:{expected_file}:3"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn runtime_diagnostics_report_canonical_file_and_statement_line() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!("rust_php_diagnostic_source_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&directory).unwrap();
    let entry = directory.join("entry.php");
    std::fs::write(&entry, "<?php\n$value = true; $value++;\necho $value;\n").unwrap();
    let expected_file = entry.canonicalize().unwrap().to_string_lossy().into_owned();
    let result = compile_file(&entry).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].source_file.as_deref(), Some(expected_file.as_str()));
    assert_eq!(result.diagnostics[0].source_line, Some(2));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn undefined_variable_reads_warn_but_coalescing_and_isset_are_silent() {
    let source = r#"<?php
        echo $missing;
        $dynamicName = 'dynamicMissing';
        echo $$dynamicName;
        echo ($coalesced ?? 'fallback'), ':';
        echo ($missingArray['key'] ?? 'nested'), ':';
        echo ($$dynamicName ?? 'dynamic'), ':';
        echo empty($emptyMissing) ? 'empty' : 'not-empty', ':';
        echo empty($missingArray['key']) ? 'nested-empty' : 'nested-not-empty', ':';
        $assigned ??= 'assigned';
        $checkedName = 'checked';
        $arrayName = 'presentArray';
        $presentArray = ['key' => 1];
        echo $assigned, ':', isset($checked) ? 'yes' : 'no', ':';
        echo isset($$checkedName) ? 'yes' : 'no', ':', isset($$arrayName['key']) ? 'yes' : 'no', ':';
        error_reporting(0);
        echo $suppressed;
    "#;
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "fallback:nested:dynamic:empty:nested-empty:assigned:no:no:yes:");
    assert_eq!(result.diagnostics.len(), 2, "{:?}", result.diagnostics);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(result.diagnostics[0].message, "Undefined variable $missing");
    assert_eq!(result.diagnostics[0].source_line, Some(2));
    assert_eq!(result.diagnostics[1].message, "Undefined variable $dynamicMissing");
    assert_eq!(result.diagnostics[1].source_line, Some(4));
}

#[test]
fn exceptions_expose_the_creation_frame_through_trace_apis() {
    let source = r#"<?php
        function createLocatedException() {
            return new RuntimeException('located');
        }
        $error = createLocatedException();
        $trace = $error->getTrace();
        echo count($trace), ':', $trace[0]['function'], ':', $trace[0]['line'], "\n";
        echo $error->getTraceAsString();
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert!(output.starts_with("1:createLocatedException:3\n#0 "), "{output}");
    assert!(output.ends_with("#1 {main}"), "{output}");

    let top_level = compile_source("<?php $error = new Exception('top'); echo count($error->getTrace()), ':', $error->getTraceAsString();")
        .unwrap().execute().unwrap();
    assert_eq!(top_level, "0:#0 {main}");

    let method_trace = compile_source(r#"<?php
        class TraceFactory { public function create() { return new Exception('method'); } }
        $error = (new TraceFactory())->create();
        $trace = $error->getTrace();
        echo $trace[0]['class'], ':', $trace[0]['type'], ':', $trace[0]['function'], '|', $error->getTraceAsString();
    "#).unwrap().execute().unwrap();
    assert!(method_trace.starts_with("TraceFactory:->:create|#0 "), "{method_trace}");
    assert!(method_trace.contains("TraceFactory->create()"), "{method_trace}");
}

#[test]
fn exception_trace_records_nested_callers_with_source_lines() {
    let source = r#"<?php
        function innerFailure() {
            return new RuntimeException('nested');
        }
        function middleFailure() {
            return innerFailure();
        }
        $error = middleFailure();
        $trace = $error->getTrace();
        echo count($trace), ':', $trace[0]['function'], ':', $trace[0]['line'], ':';
        echo $trace[1]['function'], ':', $trace[1]['line'], "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "2:innerFailure:3:middleFailure:6\n"
    );
}

#[test]
fn builtin_pdo_constructor_and_static_methods_validate_named_arguments() {
    let constructor = compile_source("<?php new PDO(username: 'user');").unwrap().execute().unwrap_err();
    assert!(constructor.contains("argument '$dsn'"), "{constructor}");
    let static_call = compile_source("<?php PDO::getAvailableDrivers(unknown: true);").unwrap().execute().unwrap_err();
    assert!(static_call.contains("unknown named parameter '$unknown'"), "{static_call}");

    let pdo = php_runtime::Value::Object(std::rc::Rc::new(std::cell::RefCell::new(php_runtime::PhpObject {
        class_name: "PDO".into(),
        properties: std::collections::HashMap::new(),
        property_order: Vec::new(),
        active_property_hooks: Default::default(),
        exception: None,
    })));
    let program = compile_source("<?php $pdo->setAttribute(value: 4, attribute: 19); echo $pdo->getAttribute(attribute: 19);").unwrap();
    let result = program.execute_with_globals(std::collections::HashMap::from([("pdo".into(), pdo)])).unwrap();
    assert_eq!(result.output, "4");
}

#[test]
fn returning_require_once_expression_can_initialize_a_variable() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!("rust_php_return_include_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&directory).unwrap();
    let included = directory.join("value.php");
    let entry = directory.join("entry.php");
    std::fs::write(&included, "<?php return 'booted';").unwrap();
    std::fs::write(&entry, r#"<?php $app = require_once __DIR__ . '/value.php'; echo $app, "\n";"#).unwrap();
    let output = compile_file(&entry).unwrap().execute().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert_eq!(output, "booted\n");
}

#[test]
fn bare_return_in_included_file_exits_only_that_file_scope() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!("rust_php_bare_return_include_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&directory).unwrap();
    let included = directory.join("autoload.php");
    let entry = directory.join("entry.php");
    std::fs::write(&included, "<?php $GLOBALS['included'] = 'yes'; if (true) { return; } $GLOBALS['after'] = 'no';").unwrap();
    std::fs::write(&entry, "<?php require __DIR__ . '/autoload.php'; echo $GLOBALS['included'], isset($GLOBALS['after']) ? 'bad' : 'ok';").unwrap();
    let output = compile_file(&entry).unwrap().execute().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert_eq!(output, "yesok");
}

#[test]
fn file_exists_assigned_paths_resolve_optional_include_branches() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!("rust_php_optional_include_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(directory.join("storage/framework")).unwrap();
    let entry = directory.join("public.php");
    let maintenance = directory.join("storage/framework/maintenance.php");
    std::fs::write(&entry, r#"<?php if (file_exists($maintenance = __DIR__ . '/storage/framework/maintenance.php')) { require $maintenance; } echo 'ready';"#).unwrap();
    assert_eq!(compile_file(&entry).unwrap().execute().unwrap(), "ready");
    std::fs::write(&maintenance, "<?php echo 'maintenance';").unwrap();
    assert_eq!(compile_file(&entry).unwrap().execute().unwrap(), "maintenanceready");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn composer_autoload_supports_classmap_and_psr0_directories() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!("rust_php_composer_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(directory.join("vendor")).unwrap();
    std::fs::create_dir_all(directory.join("legacy/Legacy")).unwrap();
    std::fs::create_dir_all(directory.join("mapped")).unwrap();
    std::fs::write(directory.join("composer.json"), r#"{"autoload":{"psr-0":{"Legacy\\":"legacy/"},"classmap":["mapped"]}}"#).unwrap();
    std::fs::write(directory.join("vendor/autoload.php"), "<?php // Composer entry point").unwrap();
    std::fs::write(directory.join("legacy/Legacy/Thing.php"), "<?php namespace Legacy; class Thing { function value() { return 'psr0'; } }").unwrap();
    std::fs::write(directory.join("mapped/ClassMapped.php"), "<?php class ClassMapped { function value() { return 'classmap'; } }").unwrap();
    let entry = directory.join("entry.php");
    std::fs::write(&entry, "<?php require __DIR__ . '/vendor/autoload.php'; echo (new \\Legacy\\Thing())->value(), ':', (new ClassMapped())->value();").unwrap();
    let output = compile_file(&entry).unwrap().execute().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert_eq!(output, "psr0:classmap");
}

#[test]
fn strict_types_declaration_is_parsed_as_a_file_directive() {
    assert_eq!(compile_source("<?php declare(strict_types=1); echo 'booted';").unwrap().execute().unwrap(), "booted");
}

#[test]
fn strict_types_rejects_scalar_coercion_at_function_method_closure_and_constructor_calls() {
    let source = r#"<?php
        declare(strict_types=1);
        function acceptsInteger(int $value): int { return $value; }
        function acceptsFloat(float $value): string { return gettype($value); }
        class StrictArgumentTarget {
            public function __construct(int $value) {}
            public function accept(int $value): int { return $value; }
            public static function acceptStatic(int $value): int { return $value; }
        }
        echo acceptsInteger(3), ':', acceptsFloat(4), ':';
        $object = new StrictArgumentTarget(5);
        echo $object->accept(6), ':';
        $closure = function (int $value): int { return $value; };
        echo $closure(7), ':';
        try { acceptsInteger('8'); } catch (TypeError $error) { echo get_class($error), ':'; }
        try { acceptsInteger(value: '8a'); } catch (TypeError $error) { echo get_class($error), ':'; }
        try { $object->accept('9'); } catch (TypeError $error) { echo get_class($error), ':'; }
        try { StrictArgumentTarget::acceptStatic('9a'); } catch (TypeError $error) { echo get_class($error), ':'; }
        try { $closure('10'); } catch (TypeError $error) { echo get_class($error), ':'; }
        try { new StrictArgumentTarget('11'); } catch (TypeError $error) { echo get_class($error); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "3:double:6:7:TypeError:TypeError:TypeError:TypeError:TypeError:TypeError");
}

#[test]
fn weak_scalar_parameter_mode_still_coerces_values() {
    let source = r#"<?php
        declare(strict_types=0);
        function acceptsInteger(int $value): int { return $value; }
        echo acceptsInteger('12');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "12");
}

#[test]
fn strict_scalar_return_types_follow_the_defining_file_mode() {
    let strict = r#"<?php
        declare(strict_types=1);
        function strictIntegerReturn(): int { return '12'; }
        function strictFloatReturn(): float { return 12; }
        class StrictReturnTarget { public function value(): int { return '13'; } }
        try { strictIntegerReturn(); } catch (TypeError $error) { echo get_class($error), ':'; }
        try { (new StrictReturnTarget())->value(); } catch (TypeError $error) { echo get_class($error), ':'; }
        $closure = function (): int { return '14'; };
        try { $closure(); } catch (TypeError $error) { echo get_class($error), ':'; }
        echo gettype(strictFloatReturn());
    "#;
    assert_eq!(compile_source(strict).unwrap().execute().unwrap(), "TypeError:TypeError:TypeError:double");

    let weak = r#"<?php
        declare(strict_types=0);
        function weakIntegerReturn(): int { return '12'; }
        echo weakIntegerReturn();
    "#;
    assert_eq!(compile_source(weak).unwrap().execute().unwrap(), "12");
}

#[test]
fn included_files_keep_their_own_strict_call_and_return_modes() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!("rust_php_strict_include_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&directory).unwrap();
    let weak = directory.join("weak.php");
    let strict = directory.join("strict.php");
    let entry = directory.join("entry.php");
    std::fs::write(&weak, "<?php declare(strict_types=0); function weakTarget(int $value): int { return $value; } function weakCaller(): int { return strictTarget('4'); } function weakReturn(): int { return '6'; } class WeakIncluded { public function caller(): int { return strictTarget('8'); } }").unwrap();
    std::fs::write(&strict, "<?php declare(strict_types=1); function strictTarget(int $value): int { return $value; } function strictCaller(): int { return weakTarget('5'); } function strictReturn(): int { return '7'; } class StrictIncluded { public function caller(): int { return weakTarget('9'); } }").unwrap();
    std::fs::write(&entry, r#"<?php declare(strict_types=1);
        require __DIR__ . '/weak.php';
        require __DIR__ . '/strict.php';
        try { weakTarget('2'); } catch (TypeError $error) { echo 'entry-strict:'; }
        try { strictCaller(); } catch (TypeError $error) { echo 'function-strict:'; }
        try { (new StrictIncluded())->caller(); } catch (TypeError $error) { echo 'method-strict:'; }
        echo weakCaller(), ':', weakReturn(), ':';
        try { strictReturn(); } catch (TypeError $error) { echo 'definition-strict'; }
    "#).unwrap();
    let output = compile_file(&entry).unwrap().execute().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert_eq!(output, "entry-strict:function-strict:method-strict:4:6:definition-strict");
}

#[test]
fn php_integer_bases_and_digit_separators_parse() {
    let source = "<?php echo 0x7FF, ':', 0b1010_0101, ':', 0o755, ':', 1_000_000, \"\\n\";";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2047:165:493:1000000\n");
}

#[test]
fn elseif_chains_select_one_branch() {
    let source = "<?php if (false) { echo 'first'; } elseif (true) { echo 'second'; } else { echo 'last'; }";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "second");
}

#[test]
fn throw_expression_can_appear_in_match_arms() {
    let source = "<?php try { $value = match (1) { 1 => throw new Exception('bad'), default => 'ok' }; } catch (Exception $error) { echo 'caught'; }";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "caught");
}

#[test]
fn spaceship_operator_returns_php_comparison_integer() {
    let source = "<?php echo (1 <=> 2), ':', (2 <=> 2), ':', ('z' <=> 'a');";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "-1:0:1");
}

#[test]
fn goto_can_jump_back_to_a_function_local_label() {
    let source = "<?php $attempt = 0; beginning: $attempt++; if ($attempt < 3) { goto beginning; } echo $attempt;";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "3");
}

#[test]
fn goto_rejects_an_undefined_label() {
    let error = match compile_source("<?php goto missing;") { Ok(_) => panic!("undefined goto label should fail compilation"), Err(error) => error };
    assert!(error.contains("undefined label 'missing'"));
}

#[test]
fn dynamic_static_calls_and_property_references_work() {
    let source = "<?php class RefBox { public $data; } class Factory { public static function make($x) { return $x . '!'; } } $class = 'Factory'; echo $class::make('ok'); $box = new RefBox(); $box->data = []; $value =& $box->data; $value['key'] = 'shared'; echo ':', $box->data['key'];";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ok!:shared");
}

#[test]
fn static_property_array_elements_can_share_references() {
    let source = "<?php class Cache { public static $items; } Cache::$items = []; Cache::$items['left'] = 1; Cache::$items['right'] = 2; Cache::$items['left'] =& Cache::$items['right']; Cache::$items['right'] = 9; echo Cache::$items['left'], ':', Cache::$items['right'];";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "9:9");
}

#[test]
fn prefix_and_postfix_increment_support_dynamic_property_array_paths() {
    let source = "<?php class DynamicCounter { public $items; public $count; } $box = new DynamicCounter(); $name = 'items'; $box->{$name} = ['count' => 2]; echo ++$box->{$name}['count'], ':', $box->{$name}['count']++; echo ':', $box->{$name}['count']; $name = 'count'; $box->{$name} = 5; echo ':', ++$box->{$name}, ':', $box->{$name}++; echo ':', $box->{$name};";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "3:3:4:6:6:7");
}

#[test]
fn prefix_and_postfix_increment_support_dynamic_static_property_paths() {
    let source = "<?php class DynamicStaticCounter { public static $count; public static $items; } DynamicStaticCounter::$count = 5; DynamicStaticCounter::$items = ['n' => 2]; $class = 'DynamicStaticCounter'; $count = 'count'; $items = 'items'; echo ++$class::$count, ':', $class::$count++, ':', ++$class::$items['n'], ':', $class::$items['n']++, ':', $class::$items['n'];";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "6:6:3:3:4");
}

#[test]
fn coalesce_assignment_on_property_array_path_uses_short_circuit() {
    let source = "<?php class CacheBox { public $cache; public function get($key) { return $this->cache[$key] ?? $this->cache[$key] = 'cached'; } } $box = new CacheBox(); echo $box->get('a'), ':', $box->get('a');";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "cached:cached");
}

#[test]
fn property_array_append_assignment_updates_the_property() {
    let source = "<?php class ListBox { public $items; public function add($value) { $this->items[] = $value; } } $box = new ListBox(); $box->add('x'); echo $box->items[0];";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "x");
}

#[test]
fn dynamic_property_array_append_assignment_updates_the_selected_property() {
    let source = "<?php class DynamicListBox { public $items; } $box = new DynamicListBox(); $box->items = []; $name = 'items'; $box->{$name}[] = 'x'; echo $box->items[0];";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "x");
}

#[test]
fn local_variable_can_bind_to_an_object_property_reference() {
    let source = "<?php class ReferencePropertyBox { public $data; public function update() { $value =& $this->data; $value = 'changed'; return $this->data; } } $box = new ReferencePropertyBox(); echo $box->update();";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "changed");
}

#[test]
fn callable_array_invocation_preserves_reference_return_cells() {
    let source = "<?php class CallableReferenceBox { public $value; public function &getValue() { return $this->value; } } $box = new CallableReferenceBox(); $box->value = 'before'; $callable = [$box, 'getValue']; $alias =& $callable(); $alias = 'after'; $direct =& [$box, 'getValue'](); $direct = 'direct'; echo $box->value;";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "direct");
}

#[test]
fn static_callable_array_invocation_preserves_reference_return_cells() {
    let source = "<?php class StaticCallableReferenceBox { public static $value; public static function &getValue() { return self::$value; } } StaticCallableReferenceBox::$value = 'before'; $callable = ['StaticCallableReferenceBox', 'getValue']; $alias =& $callable(); $alias = 'after'; $direct =& ['StaticCallableReferenceBox', 'getValue'](); $direct = 'direct'; echo StaticCallableReferenceBox::$value;";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "direct");
}

#[test]
fn callable_array_methods_forward_by_reference_arguments() {
    let source = "<?php class CallableArgumentBox { public function change(&$value) { $value = $value . '!'; } } $box = new CallableArgumentBox(); $callback = [$box, 'change']; $value = 'first'; $callback($value); $literal = 'second'; [$box, 'change']($literal); echo $value, ':', $literal;";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "first!:second!");
}

#[test]
fn static_property_array_assignment_updates_the_property() {
    let source = "<?php class StaticBag { public static $items; } StaticBag::$items = []; StaticBag::$items['x'] = 7; echo StaticBag::$items['x'];";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "7");
}

#[test]
fn static_property_array_append_assignment_updates_the_property() {
    let source = "<?php class StaticAppendBag { public static $items; } StaticAppendBag::$items = []; StaticAppendBag::$items[] = 'x'; StaticAppendBag::$items['group'][] = 'y'; echo StaticAppendBag::$items[0], StaticAppendBag::$items['group'][0];";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "xy");
}

#[test]
fn unset_list_accepts_object_property_array_paths_after_variables() {
    let source = "<?php class UnsetListBox { public $items; public $other; public function clear($values) { $this->items = ['x' => 1]; $this->other = ['y' => 2]; unset($values[0], $this->items['x'], $this->other['y']); return isset($this->items['x']) || isset($this->other['y']); } } $box = new UnsetListBox(); echo $box->clear([1]) ? 'present' : 'cleared';";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "cleared");
}

#[test]
fn php84_property_get_and_set_hooks_use_the_backing_property() {
    let source = "<?php class HookedValue { public int $value { get { return $this->value; } set { $this->value = $value + 1; } } } $item = new HookedValue(); $item->value = 4; echo $item->value;";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "5");
}

#[test]
fn object_property_array_slots_can_reference_method_parameters() {
    let source = "<?php class PropertyArrayReferenceBox { public $data; public function bind(array &$array) { $this->data['key'] = &$array; $array[0] = 'shared'; } } $box = new PropertyArrayReferenceBox(); $value = ['before']; $box->bind($value); echo $box->data['key'][0];";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "shared");
}

#[test]
fn chained_reference_assignment_expression_binds_a_reference_return() {
    let source = "<?php class RefReturnBox { public $value; public function &getValue() { return $this->value; } } $box = new RefReturnBox(); $copy = $alias = &$box->getValue(); $alias = 'changed'; echo $box->value;";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "changed");
}

#[test]
fn asymmetric_property_set_visibility_allows_owner_and_rejects_external_write() {
    let source = "<?php class AsymmetricBox { public private(set) int $value; public function setValue(int $value) { $this->value = $value; } public function value() { return $this->value; } } $box = new AsymmetricBox(); $box->setValue(9); echo $box->value();";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "9");
    let external_write = "<?php class AsymmetricBox { public private(set) int $value; } $box = new AsymmetricBox(); $box->value = 9;";
    assert!(compile_source(external_write).unwrap().execute().unwrap_err().contains("Private member"));
}

#[test]
fn reference_before_variadic_parameter_parses() {
    let source = "<?php function collect_refs(mixed &...$values): int { return count($values); } $a = 1; $b = 2; echo collect_refs($a, $b);";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2");
}

#[test]
fn foreach_can_assign_keys_and_values_to_object_properties() {
    let source = "<?php class ForeachTargetBox { public $key; public $value; } $box = new ForeachTargetBox(); foreach ([3 => 'x', 5 => 'y'] as $box->key => $box->value) {} echo $box->key, $box->value;";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "5y");
}

#[test]
fn static_property_array_coalesce_assignment_short_circuits() {
    let source = "<?php class StaticCoalesceBox { public static $items; } StaticCoalesceBox::$items = ['x' => 1]; StaticCoalesceBox::$items['x'] ??= 2; StaticCoalesceBox::$items['y'] ??= 3; echo StaticCoalesceBox::$items['x'], ':', StaticCoalesceBox::$items['y'];";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:3");
}

#[test]
fn static_property_array_offsets_can_be_unset() {
    let source = "<?php class StaticUnsetBox { public static $items; } StaticUnsetBox::$items = ['x' => 1, 'y' => 2]; unset(StaticUnsetBox::$items['x']); echo isset(StaticUnsetBox::$items['x']) ? 'present' : StaticUnsetBox::$items['y'];";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2");
}

#[test]
fn assignment_can_be_used_as_a_comparison_operand() {
    let source = "<?php if (false !== $position = strpos('abc', 'b')) { echo $position; }";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1");
}

#[test]
fn unit_and_backed_enum_cases_parse_as_class_constants() {
    let source = "<?php enum Mode { case Fast; } enum Flag: int { case On = 1; }";
    let program = php_syntax::parse(source).unwrap();
    assert_eq!(program.classes.len(), 2);
    assert!(program.classes[0].is_enum);
    assert_eq!(program.classes[0].enum_backing_type, None);
    assert_eq!(program.classes[1].enum_backing_type.as_deref(), Some("int"));
    assert_eq!(program.classes[0].constants[0].name, "Fast");
    assert!(program.classes[0].constants[0].is_enum_case);
    assert_eq!(program.classes[1].constants[0].name, "On");
    assert!(program.classes[1].constants[0].is_enum_case);
}

#[test]
fn enum_cases_are_objects_with_names_values_and_ordered_cases() {
    assert_eq!(compile_source("<?php enum Check: string { case One = 'one'; case Two = 'live'; } echo Check::Two->value;")
        .unwrap().execute().unwrap(), "live");
    let source = r#"<?php
        enum Direction { case North; case South; }
        enum State: string { case Draft = 'draft'; case Published = 'published'; }
        $cases = Direction::cases();
        echo count($cases), ':', $cases[0]->name, ':', $cases[1]->name, ':';
        echo get_class(Direction::North), ':', (Direction::North === Direction::North), ':';
        echo State::Draft->name, ':', State::Draft->value, ':', count(State::cases()), ':';
        echo (State::from(value: 'published') === State::Published), ':', (State::tryFrom(value: 'missing') === null), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "2:North:South:Direction:1:Draft:draft:2:1:1\n");
    let unknown_name = compile_source("<?php enum State: string { case Draft = 'draft'; } State::from(input: 'draft');")
        .unwrap().execute().unwrap_err();
    assert!(unknown_name.contains("unknown named parameter '$input'"));
}

#[test]
fn runtime_value_type_and_arithmetic_errors_are_catchable() {
    let source = r#"<?php
        enum State: string { case Draft = 'draft'; }
        try { State::from('missing'); } catch (ValueError $error) { echo 'value:', $error->getMessage(), "\n"; }
        function needsInteger(int $value) { return $value; }
        try { needsInteger('not an integer'); } catch (TypeError $error) { echo 'type:', $error->getMessage(), "\n"; }
        try { needsInteger(); } catch (ArgumentCountError $error) { echo 'count:', $error->getMessage(), "\n"; }
        try { intdiv(7, 0); } catch (DivisionByZeroError $error) { echo 'division:', $error->getMessage(), "\n"; }
        try { 7 % 0; } catch (DivisionByZeroError $error) { echo 'modulo:', $error->getMessage(), "\n"; }
        try { 1 << -1; } catch (ArithmeticError $error) { echo 'shift:', $error->getMessage(), "\n"; }
        try { intdiv(PHP_INT_MIN, -1); } catch (ArithmeticError $error) { echo 'overflow:', $error->getMessage(), "\n"; }
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "value:\"missing\" is not a valid backing value for enum State\ntype:Argument 1 ($value) passed to function::needsinteger() must be of type int\ncount:needsinteger() expects at least 1 arguments, 0 given\ndivision:Division by zero\nmodulo:Modulo by zero\nshift:Bit shift by negative number\noverflow:Division of PHP_INT_MIN by -1 is not an integer\n");
}

#[test]
fn json_throw_on_error_uses_json_exception_and_partial_output_takes_precedence() {
    let source = r#"<?php
        json_decode('{invalid');
        try {
            json_encode(fdiv(1, 0), JSON_THROW_ON_ERROR);
        } catch (JsonException $error) {
            echo get_class($error), ':', $error->getCode(), ':', $error->getMessage(), ':', json_last_error(), "\n";
        }
        echo json_encode([fdiv(1, 0)], JSON_PARTIAL_OUTPUT_ON_ERROR | JSON_THROW_ON_ERROR), ':', json_last_error(), "\n";
        try {
            json_decode('{invalid', flags: JSON_THROW_ON_ERROR);
        } catch (JsonException $error) {
            echo $error->getCode(), ':', $error->getMessage(), ':', json_last_error(), "\n";
        }
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "JsonException:7:Inf and NaN cannot be JSON encoded:4\n[0]:7\n4:Syntax error:7\n");
}

#[test]
fn enums_cannot_be_instantiated_or_modified() {
    let construct = compile_source("<?php enum Mode { case Fast; } new Mode();").unwrap().execute();
    let construct_error = construct.unwrap_err();
    assert!(construct_error.contains("Cannot instantiate enum"), "{construct_error}");
    let modify = compile_source("<?php enum Mode { case Fast; } Mode::Fast->name = 'Slow';").unwrap().execute();
    assert!(modify.unwrap_err().contains("Cannot modify enum property Mode::$name"));
    let clone = compile_source("<?php enum Mode { case Fast; } clone Mode::Fast;").unwrap().execute();
    assert!(clone.unwrap_err().contains("uncloneable object of class Mode"));
}

#[test]
fn enum_declarations_reject_invalid_cases_and_members() {
    for source in [
        "<?php enum Mode: string { case Fast; }",
        "<?php enum Mode { case Fast = 'fast'; }",
        "<?php enum Mode: int { case First = 1; case Second = 1; }",
        "<?php enum Mode { public string $name; case Fast; }",
        "<?php enum Mode { case Fast; public static function cases() {} }",
    ] {
        assert!(compile_source(source).is_err(), "accepted invalid enum: {source}");
    }
}
