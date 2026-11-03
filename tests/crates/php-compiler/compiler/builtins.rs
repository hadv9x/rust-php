use php_compiler::compile_source;

#[test]
fn scalar_conversion_builtins_support_direct_named_and_callback_calls() {
    let source = r#"<?php
        echo intval(value: 'ff', base: 16), ':', intval('12.8'), ':';
        echo intval('101 Dalmations'), ':', intval('0x2a', base: 0), ':', intval('0b11', 0), ':';
        echo intval('077', 0), ':', intval('1.4e2 words'), ':', intval('100', 2), ':';
        echo floatval(value: '1.25'), ':', strval(value: 42), ':', boolval(value: '0'), ':';
        echo implode(',', array_map('intval', ['4', '5'])), ':';
        echo function_exists('intval'), function_exists('floatval'), function_exists('strval'), function_exists('boolval');
        try { intval('10', base: 1); }
        catch (ValueError $error) { echo '|', get_class($error), ':', $error->getMessage(); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "255:12:101:42:3:63:140:4:1.25:42::4,5:1111|ValueError:intval(): Argument #2 ($base) must be between 2 and 36");
}

#[test]
fn intval_callback_uses_the_base_argument_for_each_value() {
    let source = r#"<?php
        echo implode(',', array_map('intval', ['ff', '101', '0x2a'], [16, 2, 0])), "\n";
        try { array_map('intval', ['12'], [1]); }
        catch (ValueError $error) { echo get_class($error), ':', $error->getMessage(); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "255,5,42\nValueError:intval(): Argument #2 ($base) must be between 2 and 36");
}

#[test]
fn common_math_builtins_support_named_arguments_and_function_checks() {
    let source = r#"<?php
        echo abs(num: -7), ':', ceil(1.2), ':', floor(1.8), ':';
        echo round(num: 1.235, precision: 2), ':', round(1.25, 1, 3), ':';
        echo sqrt(9), ':', fdiv(1, 0), ':', intdiv(-7, 3), ':';
        echo is_finite(1), ':', is_infinite(fdiv(1, 0)), ':', is_nan(sqrt(-1)), ':';
        echo log(num: 8, base: 2), ':', max(value: 4, other: 8), ':', min([3, -2]), ':';
        echo function_exists('round'), ':', function_exists('max'), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "7:2:1:1.24:1.2:3:inf:-2:1:1:1:3:8:-2:1:1\n");
}

#[test]
fn common_string_escape_and_rot13_builtins_dispatch_by_name() {
    let source = r#"<?php
        echo addslashes(string: "a'b"), ':', stripslashes("a\\'b"), ':';
        echo str_rot13(string: 'Php'), ':', function_exists('str_rot13'), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "a\\'b:a'b:Cuc:1\n");
}

#[test]
fn match_expressions_are_strict_lazy_and_throw_the_builtin_error() {
    let source = r#"<?php
        $calls = 0;
        function nextSubject(&$calls) { $calls++; return 2; }
        echo match (nextSubject($calls)) { 1 => 'one', 2, 3 => 'selected', default => 'other' }, ':';
        echo match ('2') { 2 => 'integer', default => 'string' }, ':';
        $expected = 2;
        $closure = fn() => match ($expected) { $expected => 'captured', default => 'miss' };
        echo $closure(), ':', $calls, ':';
        try { match (9) { 1 => 'unreachable' }; }
        catch (UnhandledMatchError $error) { echo get_class($error), ':', $error->getMessage(), "\n"; }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "selected:string:captured:1:UnhandledMatchError:Unhandled match case\n");
}

#[test]
fn file_constants_support_namespaces_arrays_and_constant_references() {
    let source = r#"<?php
        namespace App {
            const LOCAL_VALUE = GLOBAL_VALUE + 2, SETTINGS = ['answer' => LOCAL_VALUE], SORTING = SORT_NUMERIC;
            echo LOCAL_VALUE, ':', SETTINGS['answer'], ':', constant('App\\LOCAL_VALUE'), ':', defined('App\\SETTINGS'), "\n";
        }
        namespace {
            const GLOBAL_VALUE = 40;
            function fromConstant($value = App\LOCAL_VALUE) { return $value; }
            echo GLOBAL_VALUE, ':', constant('App\\SORTING'), ':', fromConstant(), "\n";
        }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "42:42:42:1\n40:1:42\n");
}

#[test]
fn file_constant_cycles_and_duplicate_declarations_are_rejected() {
    let circular = match compile_source("<?php const FIRST = SECOND; const SECOND = FIRST;") { Err(error) => error, Ok(_) => panic!("expected circular constant error") };
    assert!(circular.contains("Circular constant reference"));
    let duplicate = match compile_source("<?php const VALUE = 1; const VALUE = 2;") { Err(error) => error, Ok(_) => panic!("expected duplicate constant error") };
    assert!(duplicate.contains("already declared"));
}

#[test]
fn common_php_predefined_constants_are_available() {
    let source = r#"<?php
        const MAXIMUM = PHP_INT_MAX;
        echo PHP_EOL === "\n", ':', PHP_INT_SIZE, ':', MAXIMUM === PHP_INT_MAX, ':';
        echo PHP_FLOAT_DIG, ':', PHP_ROUND_HALF_EVEN, ':', defined('PHP_FLOAT_EPSILON'), ':', PHP_QUERY_RFC1738, ':', PHP_QUERY_RFC3986, ':', is_infinite(INF), ':', is_nan(NAN), ':', PHP_OS_FAMILY !== '', ':', PHP_OS !== '', "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:8:1:15:3:1:1:2:1:1:1:1\n");
}

#[test]
fn mbstring_case_constants_are_available_in_constant_expressions() {
    let source = r#"<?php const CASE_FOLD = MB_CASE_FOLD; echo CASE_FOLD, ':', MB_CASE_FOLD_SIMPLE, "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "3:7\n");
}

#[test]
fn json_hex_flags_are_exposed_to_php() {
    let source = r#"<?php
        echo json_encode("<b>&\"x\"</b>", JSON_HEX_TAG | JSON_HEX_AMP | JSON_HEX_QUOT), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), format!("{}\n", r#""\u003cb\u003e\u0026\u0022x\u0022\u003c\/b\u003e""#));
}

#[test]
fn json_preserve_zero_fraction_flag_is_exposed_to_php() {
    let source = r#"<?php echo json_encode(1.0, JSON_PRESERVE_ZERO_FRACTION), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1.0\n");
}

#[test]
fn json_numeric_check_flag_is_exposed_to_php() {
    let source = r#"<?php echo json_encode(['id' => '42', 'label' => 'item'], JSON_NUMERIC_CHECK), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), format!("{}\n", r#"{"id":42,"label":"item"}"#));
}

#[test]
fn base_convert_supports_named_arguments_and_function_introspection() {
    let source = r#"<?php echo base_convert(num: 'ff', from_base: 16, to_base: 2), ':', function_exists('base_convert'), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "11111111:1\n");
}

#[test]
fn decimal_base_converters_support_named_arguments() {
    let source = r#"<?php echo decbin(num: 13), ':', dechex(num: 255), ':', decoct(num: 8), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1101:ff:10\n");
}

#[test]
fn bin2hex_is_available_through_php_dispatch() {
    let source = r#"<?php echo bin2hex(string: "aé"), ':', function_exists('bin2hex'), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "61c3a9:1\n");
}

#[test]
fn ord_reads_the_first_utf8_byte() {
    assert_eq!(compile_source("<?php echo ord('é'), ':', ord('A'), \"\\n\";").unwrap().execute().unwrap(), "195:65\n");
}

#[test]
fn pos_is_an_alias_for_current_array_value() {
    let source = r#"<?php $items = ['first', 'second']; echo pos(array: $items), ':', next($items), ':', pos($items), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "first:second:second\n");
}

#[test]
fn number_format_binds_named_arguments_and_default_separators() {
    let source = r#"<?php echo number_format(num: 1234.5678, decimals: 2, decimal_separator: ',', thousands_separator: ' '), ':', function_exists('number_format'), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1 234,57:1\n");
}

#[test]
fn stristr_is_case_insensitive_and_supports_named_before_needle() {
    let source = r#"<?php echo stristr(haystack: 'User@Example.test', needle: '@EXAMPLE', before_needle: true), ':', function_exists('stristr'), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "User:1\n");
}

#[test]
fn natural_string_comparison_functions_support_named_arguments() {
    let source = r#"<?php echo strnatcmp(string1: 'file12', string2: 'file3'), ':', strnatcasecmp('File2', 'file10'), ':', function_exists('strnatcmp'), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:-1:1\n");
}

#[test]
fn php83_string_increment_and_decrement_are_exposed_to_php() {
    let source = r#"<?php echo str_increment(string: 'A9'), ':', str_decrement('AA'), ':', function_exists('str_increment'), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "B0:Z:1\n");
}

#[test]
fn string_increment_and_decrement_invalid_values_raise_catchable_value_errors() {
    let source = r#"<?php
        $messages = [];
        foreach ([fn() => str_increment(''), fn() => str_increment('a-b'), fn() => str_decrement('A'), fn() => str_decrement('0')] as $operation) {
            try { $operation(); } catch (ValueError $error) { $messages[] = get_class($error); }
        }
        echo implode(':', $messages);
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ValueError:ValueError:ValueError:ValueError");
}

#[test]
fn span_functions_support_php_named_arguments() {
    let source = r#"<?php echo strspn(string: '42 items', characters: '0123456789'), ':', strcspn(string: '42 items', characters: ' ', offset: 1, length: 2), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2:1\n");
}

#[test]
fn stripos_supports_named_arguments_and_function_introspection() {
    let source = r#"<?php echo stripos(haystack: 'PHP runtime', needle: 'RUNTIME', offset: 1), ':', function_exists('stripos'), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "4:1\n");
}

#[test]
fn strpbrk_supports_named_arguments_and_function_introspection() {
    let source = r#"<?php echo strpbrk(string: 'hello world', characters: 'ow'), ':', function_exists('strpbrk'), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "o world:1\n");
}

#[test]
fn str_word_count_supports_named_arguments_and_introspection() {
    let source = r#"<?php echo json_encode(str_word_count(string: 'PHP-Rust', format: 1, characters: '-')), ':', function_exists('str_word_count'), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "[\"PHP-Rust\"]:1\n");
}

#[test]
fn error_get_last_keeps_filtered_warning_location_and_can_be_cleared() {
    let source = "<?php\nerror_reporting(0);\n$ignored = $missingForLastError;\n$last = error_get_last();\necho $last['type'], ':', $last['message'], ':', $last['file'], ':', $last['line'], ':';\nerror_clear_last();\necho error_get_last() === null, ':', function_exists('error_get_last'), ':', function_exists('error_clear_last');";
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "2:Undefined variable $missingForLastError:Command line code:3:1:1:1");
    assert!(result.diagnostics.is_empty());
}
