use php_compiler::compile_source;

#[test]
fn json_encode_supports_pretty_and_unescaped_slashes_flags() {
    let source = r#"<?php
        echo json_encode(['url' => 'https://example.test/a'], JSON_UNESCAPED_SLASHES), "\n";
        echo json_encode(['url' => 'https://example.test/a'], JSON_PRETTY_PRINT), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "{\"url\":\"https://example.test/a\"}\n{\n  \"url\": \"https:\\/\\/example.test\\/a\"\n}\n");
}

#[test]
fn json_encode_formats_floats_with_php_exponent_thresholds() {
    let source = r#"<?php
        echo json_encode([1e16, 1e17, 1.25e20, 1e-4, 1e-5, 1.25e-7]), "\n";
        echo json_encode([1e17, 1e16], JSON_PRESERVE_ZERO_FRACTION), "\n";
        echo json_encode([-0.0]), ":", json_encode([-0.0], JSON_PRESERVE_ZERO_FRACTION), "\n";
        echo json_encode(['large' => '1e17', 'small' => '1e-5'], JSON_NUMERIC_CHECK), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "[10000000000000000,1.0e+17,1.25e+20,0.0001,1.0e-5,1.25e-7]\n[1.0e+17,10000000000000000.0]\n[-0]:[-0.0]\n{\"large\":1.0e+17,\"small\":1.0e-5}\n");
}

#[test]
fn json_numeric_check_trims_ascii_whitespace_but_not_unicode_whitespace() {
    let source = "<?php echo json_encode(['ascii' => \"\\t 42 \\n\", 'nbsp' => \" 42 \"], JSON_NUMERIC_CHECK), \"\\n\";";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "{\"ascii\":42,\"nbsp\":\"\\u00a042\\u00a0\"}\n");
}

#[test]
fn json_encode_escapes_unicode_by_default_and_supports_unescaped_flag() {
    let source = r#"<?php
        echo json_encode('café 🚀'), "\n";
        echo json_encode('café 🚀', JSON_UNESCAPED_UNICODE), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), r#""caf\u00e9 \ud83d\ude80"
"café 🚀"
"#);
}

#[test]
fn json_encode_force_object_uses_named_flag() {
    let source = r#"<?php echo json_encode([7, 8], JSON_FORCE_OBJECT), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "{\"0\":7,\"1\":8}\n");
}

#[test]
fn json_encode_invokes_json_serializable_recursively() {
    let source = r#"<?php
        class JsonRecord implements JsonSerializable {
            public $name = 'Ada';
            public function jsonSerialize() { return ['display' => $this->name, 'active' => true]; }
        }
        $record = new JsonRecord();
        echo interface_exists('JsonSerializable'), ':', method_exists('JsonSerializable', 'jsonSerialize'), ':', $record instanceof JsonSerializable, "\n";
        try { echo json_encode(['record' => $record], JSON_THROW_ON_ERROR), "\n"; }
        catch (JsonException $error) { echo 'ERROR:', $error->getMessage(), "\n"; }
        class RecursiveJson implements JsonSerializable {
            public function jsonSerialize() { return $this; }
        }
        echo json_encode(new RecursiveJson(), JSON_PARTIAL_OUTPUT_ON_ERROR), ':', json_last_error(), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:1:1\n{\"record\":{\"display\":\"Ada\",\"active\":true}}\nnull:6\n"
    );
}

#[test]
fn json_encode_supports_unescaped_line_terminators_flag() {
    let source = "<?php echo json_encode(\"\u{2028}\", JSON_UNESCAPED_UNICODE), \"\\n\", json_encode(\"\u{2028}\", JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_LINE_TERMINATORS), \"\\n\";";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "\"\\u2028\"\n\"\u{2028}\"\n");
}

#[test]
fn computed_break_and_continue_levels_select_nested_targets() {
    let source = r#"<?php
        $level = 2;
        for ($outer = 0; $outer < 2; $outer++) {
            for ($inner = 0; $inner < 3; $inner++) {
                echo "b", $outer, $inner, ";";
                if ($inner == 1) { break $level; }
            }
        }
        $outer = 0;
        while ($outer < 2) {
            $outer++;
            for ($inner = 0; $inner < 2; $inner++) {
                echo "c", $outer, $inner, ";";
                continue $level;
            }
            echo "bad;";
        }
        echo "done\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "b00;b01;c10;c20;done\n");
}

#[test]
fn computed_loop_control_levels_count_switches_as_breakable() {
    let source = r#"<?php
        $level = 1;
        for ($i = 0; $i < 3; $i++) {
            switch ($i) {
                case 0:
                    continue $level;
                case 1:
                    $level = 2;
                    break $level;
                default:
                    echo "bad";
            }
            echo "after-switch:", $i, ";";
        }
        echo "done\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "after-switch:0;done\n");
}

#[test]
fn foreach_advances_user_iterator_objects_after_each_body() {
    let source = r#"<?php
        class PairIterator implements Iterator {
            private $position = 0;
            public function rewind() { $this->position = 0; }
            public function valid() { return $this->position < 3; }
            public function current() { return $this->position * 10; }
            public function key() { return "item-" . $this->position; }
            public function next() { $this->position++; }
        }
        foreach (new PairIterator() as $key => $value) {
            echo $key, "=", $value, ";";
        }
        echo interface_exists('Iterator'), ":", interface_exists('Traversable'), ":done\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "item-0=0;item-1=10;item-2=20;1:1:done\n");
}

#[test]
fn foreach_preserves_non_scalar_keys_from_user_iterators() {
    let source = r#"<?php
        class KeyObject {}
        class ArbitraryKeyIterator implements Iterator {
            private $position = 0;
            public function rewind() { $this->position = 0; }
            public function valid() { return $this->position < 2; }
            public function current() { return $this->position; }
            public function key() { return $this->position === 0 ? new KeyObject() : ['key']; }
            public function next() { $this->position++; }
        }
        foreach (new ArbitraryKeyIterator() as $key => $value) {
            echo is_object($key) ? 'object' : 'array', ':', $value, ';';
        }
        echo iterator_count(new ArbitraryKeyIterator()), ':';
        echo count(iterator_to_array(new ArbitraryKeyIterator())), ':';
        $reindexed = iterator_to_array(new ArbitraryKeyIterator(), false);
        echo $reindexed[0], ':', $reindexed[1];
    "#;
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "object:0;array:1;2:0:0:1");
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic.message == "Illegal offset type"));
}

#[test]
fn foreach_supports_iterator_aggregate_returning_a_user_iterator() {
    let source = r#"<?php
        class ValueIterator implements Iterator {
            private $position = 0;
            public function rewind() { $this->position = 0; }
            public function valid() { return $this->position < 2; }
            public function current() { return $this->position + 4; }
            public function key() { return $this->position; }
            public function next() { $this->position++; }
        }
        class ValueCollection implements IteratorAggregate {
            public function getIterator() { return new ValueIterator(); }
        }
        foreach (new ValueCollection() as $key => $value) echo $key, ":", $value, ";";
        echo "done\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "0:4;1:5;done\n");
}

#[test]
fn argument_unpacking_accepts_generators_and_user_traversables() {
    let source = r#"<?php
        function pair_values() { yield 3; yield 4; }
        class PairIteratorForUnpack implements Iterator {
            private $position = 0;
            public function rewind() { $this->position = 0; }
            public function valid() { return $this->position < 2; }
            public function current() { return $this->position + 7; }
            public function key() { return $this->position; }
            public function next() { $this->position++; }
        }
        function add_pair($left, $right) { return $left + $right; }
        echo add_pair(...pair_values()), ":", add_pair(...new PairIteratorForUnpack()), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "7:15\n");
}

#[test]
fn iterator_helpers_consume_user_iterator_and_aggregate_objects() {
    let source = r#"<?php
        class ThreeItems implements Iterator {
            private $position = 0;
            public function rewind() { $this->position = 0; }
            public function valid() { return $this->position < 3; }
            public function current() { return $this->position + 20; }
            public function key() { return "k" . $this->position; }
            public function next() { $this->position++; }
        }
        class ThreeItemCollection implements IteratorAggregate {
            public function getIterator() { return new ThreeItems(); }
        }
        $array = iterator_to_array(new ThreeItemCollection());
        echo $array["k1"], ":", iterator_count(new ThreeItems()), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "21:3\n");
}

#[test]
fn iterator_interfaces_require_their_protocol_methods() {
    let iterator_error = match compile_source("<?php class InvalidIterator implements Iterator {}") { Err(error) => error, Ok(_) => panic!("missing Iterator methods should fail compilation") };
    assert!(iterator_error.contains("must implement rewind()"));
    let aggregate_error = match compile_source("<?php class InvalidAggregate implements IteratorAggregate {}") { Err(error) => error, Ok(_) => panic!("missing IteratorAggregate methods should fail compilation") };
    assert!(aggregate_error.contains("must implement getIterator()"));
}

#[test]
fn traversable_hierarchy_matches_iterable_types_and_is_iterable() {
    let source = r#"<?php
        class SampleIterator implements Iterator {
            private $position = 0;
            public function rewind() { $this->position = 0; }
            public function valid() { return $this->position < 1; }
            public function current() { return 'item'; }
            public function key() { return 0; }
            public function next() { $this->position++; }
        }
        class SampleAggregate implements IteratorAggregate {
            public function getIterator(): Traversable { return new SampleIterator(); }
        }
        class NotIterable {}
        function acceptsIterable(iterable $value) { return 'accepted'; }
        function sampleGenerator() { yield 'item'; }
        $iterator = new SampleIterator();
        $aggregate = new SampleAggregate();
        $plain = new NotIterable();
        echo interface_exists('Traversable'), ':', interface_exists('Iterator'), ':';
        echo function_exists('is_iterable'), ':';
        echo ($iterator instanceof Traversable), ':', is_a($iterator, 'Traversable'), ':';
        echo is_iterable(value: []), ':', is_iterable($iterator), ':', is_iterable($aggregate), ':', is_iterable($plain), ':', is_iterable(sampleGenerator()), ':';
        echo acceptsIterable($iterator), ':', acceptsIterable($aggregate), ':', acceptsIterable(sampleGenerator()), ':';
        echo implode(',', array_map('is_iterable', [[], $iterator, $aggregate, $plain, sampleGenerator()]));
        try { acceptsIterable($plain); } catch (TypeError $error) { echo ':', get_class($error); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:1:1:1:1:1:1:1::1:accepted:accepted:accepted:1,1,1,,1:TypeError");
}

#[test]
fn json_last_error_tracks_encode_and_decode_results() {
    let source = r#"<?php
        json_decode('{');
        echo json_last_error(), ':', json_last_error_msg(), ':';
        json_decode('{"ok":true}');
        echo json_last_error(), ':', json_last_error_msg(), ':', function_exists('json_last_error'), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "4:Syntax error:0:No error:1\n");
}

#[test]
fn json_decode_and_validate_accept_only_json_whitespace() {
    let source = r#"<?php
        $valid = " \t\r\nnull";
        $formFeed = "\fnull";
        $verticalTab = "\vnull";
        echo json_decode($valid) === null, ':', json_last_error(), ':';
        echo json_decode($formFeed) === null, ':', json_last_error(), ':';
        echo json_validate($verticalTab) ? '1' : '0', ':', json_last_error(), ':';
        echo json_validate("\t true \r") ? '1' : '0', ':', json_last_error(), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:0:1:4:0:4:1:0\n");
}

#[test]
fn json_decode_and_validate_count_empty_nested_containers_toward_depth() {
    let source = r#"<?php
        $oneLevel = json_decode('[[]]', depth: 1);
        echo ($oneLevel === null), ':', json_last_error(), ':';
        echo json_decode('[[]]', depth: 2) !== null, ':', json_last_error(), ':';
        echo json_validate('{"nested":{}}', depth: 1) ? 'valid' : 'invalid', ':', json_last_error(), ':';
        echo json_validate('{"nested":{}}', depth: 2) ? 'valid' : 'invalid', ':', json_last_error(), "\n";
        try { json_decode('{"nested":{}}', depth: 1, flags: JSON_THROW_ON_ERROR); }
        catch (JsonException $error) { echo get_class($error), ':', $error->getCode(), ':', $error->getMessage(), "\n"; }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:1:1:0:invalid:1:valid:0\nJsonException:1:Maximum stack depth exceeded\n",
    );
}

#[test]
fn json_mismatched_closing_delimiters_use_state_mismatch_error() {
    let source = r#"<?php
        $inputs = [']', '[}', '{"value":]', '{"value":1]'];
        foreach ($inputs as $input) {
            $value = json_decode($input);
            echo ($value === null ? 'null' : 'value'), ':', json_last_error(), ':', json_last_error_msg(), ':';
            echo json_validate($input) ? 'valid' : 'invalid', ':', json_last_error(), "\n";
        }
        try { json_decode(']', flags: JSON_THROW_ON_ERROR); }
        catch (JsonException $error) { echo get_class($error), ':', $error->getCode(), ':', $error->getMessage(), "\n"; }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "null:2:State mismatch (invalid or malformed JSON):invalid:2\nnull:2:State mismatch (invalid or malformed JSON):invalid:2\nnull:2:State mismatch (invalid or malformed JSON):invalid:2\nnull:2:State mismatch (invalid or malformed JSON):invalid:2\nJsonException:2:State mismatch (invalid or malformed JSON)\n",
    );
}

#[test]
fn json_trailing_commas_remain_syntax_errors_not_state_mismatch() {
    let source = r#"<?php
        json_decode('[1,]');
        echo json_last_error(), ':', json_last_error_msg(), ':';
        json_decode('{"value":1,}');
        echo json_last_error(), ':', json_last_error_msg(), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "4:Syntax error:4:Syntax error\n",
    );
}

#[test]
fn json_last_error_distinguishes_depth_control_and_unicode_failures() {
    let source = r#"<?php
        json_decode('[[1]]', true, 1);
        echo json_last_error(), ':';
        json_decode('"a
b"');
        echo json_last_error(), ':';
        json_decode('"\\uD800"');
        echo json_last_error(), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:3:10\n");
}

#[test]
fn json_decode_rejects_nul_prefixed_object_properties_but_allows_array_keys() {
    let source = r#"<?php
        $input = '{"\u0000private":1}';
        $object = json_decode($input);
        echo is_null($object), ':', json_last_error(), ':', json_last_error_msg(), ':';
        $array = json_decode($input, true);
        echo count($array), ':', json_last_error(), ':';
        try { json_decode($input, flags: JSON_THROW_ON_ERROR); }
        catch (JsonException $error) { echo get_class($error), ':', $error->getCode(), ':', $error->getMessage(), ':', json_last_error(); }
        echo "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:9:The decoded property name is invalid:1:0:JsonException:9:The decoded property name is invalid:0\n");
}

#[test]
fn json_decode_associative_objects_apply_php_array_key_conversion() {
    let source = r#"<?php
        $list = json_decode('{"0":"No","1":"Yes"}', true);
        $strings = json_decode('{"08":"zero-eight","+1":"plus","-2":"negative"}', true);
        $duplicate = json_decode('{"1":"first","1":"last"}', true);
        echo json_encode($list), ':', $list[0], ':', $list[1], ':';
        echo json_encode($strings), ':', count($duplicate), ':', $duplicate[1], "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "[\"No\",\"Yes\"]:No:Yes:{\"08\":\"zero-eight\",\"+1\":\"plus\",\"-2\":\"negative\"}:1:last\n");
}

#[test]
fn json_decode_object_duplicate_members_keep_one_property_at_first_position() {
    let source = r#"<?php
        $object = json_decode('{"first":1,"second":2,"first":3}');
        echo json_encode($object), ':', $object->first, ':', $object->second, "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "{\"first\":3,\"second\":2}:3:2\n");
}

#[test]
fn json_validate_matches_php_flags_depth_errors_and_error_state() {
    let source = r#"<?php
        echo function_exists('json_validate'), ':', json_validate(json: '{"ok":true}', depth: 2, flags: 0), ':';
        echo json_validate('{"\\u0000private":1}'), ':', json_validate('{bad') ? '1' : '0', ':', json_last_error(), ':', json_last_error_msg(), ':';
        echo json_validate('true', flags: JSON_INVALID_UTF8_IGNORE), ':', json_last_error(), ':';
        try { json_validate('true', flags: JSON_INVALID_UTF8_SUBSTITUTE); } catch (ValueError $error) { echo get_class($error), ':'; }
        try { json_validate('true', depth: 0); } catch (ValueError $error) { echo get_class($error), ':'; }
        try { json_validate([]); } catch (TypeError $error) { echo get_class($error); }
        echo "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:1:1:0:4:Syntax error:1:0:ValueError:ValueError:TypeError\n");
}

#[test]
fn json_depth_argument_range_errors_are_value_errors() {
    let source = r#"<?php
        json_decode('{');
        $previousError = json_last_error();
        try { json_decode('{}', depth: 0); } catch (ValueError $error) { echo get_class($error), ':'; }
        try { json_decode('{}', depth: 2147483648); } catch (ValueError $error) { echo get_class($error), ':'; }
        try { json_encode([], depth: 2147483648); } catch (ValueError $error) { echo get_class($error), ':'; }
        try { json_decode('{}', depth: 0, flags: JSON_THROW_ON_ERROR); } catch (ValueError $error) { echo get_class($error), ':'; }
        echo $previousError, ':', json_last_error(), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ValueError:ValueError:ValueError:ValueError:4:4\n");
}

#[test]
fn json_encode_and_decode_honor_depths_above_512() {
    let nesting = 513;
    let json = format!("{}0{}", "[".repeat(nesting), "]".repeat(nesting));
    let source = format!("<?php $value = json_decode('{json}', depth: 514); $encoded = json_encode($value, depth: 514); echo json_last_error(), ':', strlen($encoded), \"\\n\";");
    assert_eq!(compile_source(&source).unwrap().execute().unwrap(), "0:1027\n");
}

#[test]
fn json_encode_rejects_nonpositive_depth_with_value_error() {
    let source = r#"<?php
        foreach ([0, -1] as $depth) {
            try { json_encode([], depth: $depth); }
            catch (ValueError $error) { echo get_class($error), ':', $error->getMessage(), "\n"; }
        }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ValueError:json_encode(): Argument #3 ($depth) must be greater than 0\nValueError:json_encode(): Argument #3 ($depth) must be greater than 0\n");
}

#[test]
fn json_encode_depth_range_error_reports_php_positive_range() {
    let source = r#"<?php
        foreach ([2147483648, -2147483649] as $depth) {
            try { json_encode([], depth: $depth); }
            catch (ValueError $error) { echo $error->getMessage(), "\n"; }
        }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "json_encode(): Argument #3 ($depth) must be between 1 and 2147483647\njson_encode(): Argument #3 ($depth) must be between 1 and 2147483647\n");
}

#[test]
fn json_partial_output_substitutes_nan_unsupported_values_and_recursion() {
    let source = r#"<?php
        echo json_encode(fdiv(1, 0), JSON_PARTIAL_OUTPUT_ON_ERROR), ':', json_last_error(), ':';
        echo json_encode(function () { return 1; }, flags: JSON_PARTIAL_OUTPUT_ON_ERROR), ':', json_last_error(), ':';
        class JsonCycle { public $self; }
        $cycle = new JsonCycle(); $cycle->self = $cycle;
        echo json_encode($cycle, JSON_PARTIAL_OUTPUT_ON_ERROR), ':', json_last_error(), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "0:7:null:8:{\"self\":null}:6\n");
}

#[test]
fn json_decode_bigint_flag_preserves_large_integer_spelling() {
    let source = r#"<?php
        echo is_float(json_decode('9223372036854775808')), ':';
        echo json_decode('9223372036854775808', flags: JSON_BIGINT_AS_STRING), ':';
        echo is_string(json_decode('9223372036854775808', flags: JSON_BIGINT_AS_STRING)), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:9223372036854775808:1\n");
}

#[test]
fn json_decode_preserves_signed_integer_boundaries() {
    let source = r#"<?php
        $min = json_decode('-9223372036854775808');
        $below = json_decode('-9223372036854775809');
        $belowString = json_decode('-9223372036854775809', flags: JSON_BIGINT_AS_STRING);
        $max = json_decode('9223372036854775807');
        $above = json_decode('9223372036854775808', flags: JSON_BIGINT_AS_STRING);
        echo gettype($min), ':', gettype($below), ':', $belowString, ':', gettype($max), ':', $above, ':', is_string($above), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "integer:double:-9223372036854775809:integer:9223372036854775808:1\n");
}

#[test]
fn json_decode_classifies_unpaired_utf16_surrogates_and_throw_mode() {
    let source = r#"<?php
        $high = json_decode('"\uD800"');
        echo $high === null, ':', json_last_error(), ':', JSON_ERROR_UTF16, ':', json_last_error_msg(), "\n";
        $low = json_decode('"\uDC00"');
        echo $low === null, ':', json_last_error(), "\n";
        $badPair = json_decode('"\uD800\u0041"');
        echo $badPair === null, ':', json_last_error(), "\n";
        echo json_decode('"\uD83D\uDE80"'), ':', json_last_error(), "\n";
        try { json_decode('"\uD800"', flags: JSON_THROW_ON_ERROR); }
        catch (JsonException $error) { echo $error->getCode(), ':', $error->getMessage(), "\n"; }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:10:10:Single unpaired UTF-16 surrogate in unicode escape\n1:10\n1:10\n🚀:0\n10:Single unpaired UTF-16 surrogate in unicode escape\n"
    );
}

#[test]
fn json_decode_reports_overflowing_exponent_as_infinity_error() {
    let source = r#"<?php
        $finite = json_decode('1.7976931348623157e308');
        $overflow = json_decode('1.7976931348623159e308');
        echo is_float($finite), ':', $overflow === null, ':', json_last_error(), ':', JSON_ERROR_INF_OR_NAN, ':', json_last_error_msg(), "\n";
        try { json_decode('1e400', flags: JSON_THROW_ON_ERROR); }
        catch (JsonException $error) { echo get_class($error), ':', $error->getCode(), ':', $error->getMessage(), "\n"; }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:1:7:7:Inf and NaN cannot be JSON encoded\nJsonException:7:Inf and NaN cannot be JSON encoded\n");
}

#[test]
fn json_numeric_check_reports_overflow_instead_of_quoting_the_number() {
    let source = r#"<?php
        echo json_encode('1e400', JSON_NUMERIC_CHECK), ':', json_last_error(), ':';
        echo json_encode('1e400', JSON_NUMERIC_CHECK | JSON_PARTIAL_OUTPUT_ON_ERROR), ':', json_last_error(), ':';
        try { json_encode('1e400', JSON_NUMERIC_CHECK | JSON_THROW_ON_ERROR); }
        catch (JsonException $error) { echo get_class($error), ':', $error->getCode(), ':', $error->getMessage(); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), ":7:0:7:JsonException:7:Inf and NaN cannot be JSON encoded");
}

#[test]
fn json_encode_serializes_backed_enums_and_reports_pure_enum_errors() {
    let source = r#"<?php
        enum BackedStatus: string { case Ready = 'ready'; }
        enum PureStatus { case Ready; }
        echo json_encode(BackedStatus::Ready), ':';
        echo json_encode(PureStatus::Ready), ':', json_last_error(), ':', JSON_ERROR_NON_BACKED_ENUM, ':';
        echo json_encode(PureStatus::Ready, JSON_PARTIAL_OUTPUT_ON_ERROR), ':', json_last_error(), ':';
        try { json_encode(PureStatus::Ready, JSON_THROW_ON_ERROR); }
        catch (JsonException $error) { echo get_class($error), ':', $error->getCode(); }
        echo "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "\"ready\"::11:11:null:11:JsonException:11\n");
}

#[test]
fn json_encode_preserves_declared_and_dynamic_object_property_order() {
    let source = r#"<?php
        class OrderedJsonBase { public $zeta = 1; private $secret = 9; public $alpha = 2; }
        class OrderedJsonChild extends OrderedJsonBase { protected $hidden = 8; public $middle = 3; }
        $value = new OrderedJsonChild();
        $value->later = 4;
        $value->alpha = 5;
        echo json_encode($value), "\n";
        echo json_encode(json_decode('{"z":1,"a":2}')), ':';
        echo json_encode((object)['z' => 1, 'a' => 2]), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "{\"zeta\":1,\"alpha\":5,\"middle\":3,\"later\":4}\n{\"z\":1,\"a\":2}:{\"z\":1,\"a\":2}\n");
}

#[test]
fn json_encode_preserves_order_of_properties_created_by_reference_arguments() {
    let source = r#"<?php
        function initialize(&$property, $value) { $property = $value; }
        class EmptyReferenceJsonObject {}
        $object = new EmptyReferenceJsonObject();
        initialize($object->zeta, 1);
        initialize($object->alpha, 2);
        initialize($object->z_nested['key'], 3);
        initialize($object->a_nested['key'], 4);
        echo json_encode($object), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "{\"zeta\":1,\"alpha\":2,\"z_nested\":{\"key\":3},\"a_nested\":{\"key\":4}}\n");
}

#[test]
fn json_keeps_public_child_slot_when_it_reuses_a_private_parent_property_name() {
    let source = r#"<?php
        class PrivateShadowBase { private $value = 'hidden'; }
        class PublicShadowChild extends PrivateShadowBase { public $value = 'visible'; }
        echo json_encode(new PublicShadowChild()), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "{\"value\":\"visible\"}\n");
}

#[test]
fn json_invalid_utf8_flags_are_available_for_valid_runtime_strings() {
    let source = r#"<?php
        $text = 'valid UTF-8 🚀';
        echo JSON_INVALID_UTF8_IGNORE, ':', JSON_INVALID_UTF8_SUBSTITUTE, ':';
        echo json_encode($text, flags: JSON_INVALID_UTF8_IGNORE), ':';
        echo json_encode($text, flags: JSON_INVALID_UTF8_SUBSTITUTE), "\n";
        echo json_decode('"valid"', flags: JSON_INVALID_UTF8_IGNORE), ':';
        echo json_decode('"valid"', flags: JSON_INVALID_UTF8_SUBSTITUTE), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1048576:2097152:\"valid UTF-8 \\ud83d\\ude80\":\"valid UTF-8 \\ud83d\\ude80\"\nvalid:valid\n");
}

#[test]
fn json_decode_object_as_array_flag_converts_objects_recursively() {
    let source = r#"<?php
        $value = json_decode('{"outer":{"item":7}}', flags: JSON_OBJECT_AS_ARRAY);
        echo $value['outer']['item'], ':', is_array($value), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "7:1\n");
}

#[test]
fn json_decode_null_associative_argument_defers_to_object_as_array_flag() {
    let source = r#"<?php
        $defaulted = json_decode(json: '{"value":7}', flags: JSON_OBJECT_AS_ARRAY);
        $null = json_decode('{"value":8}', null, 512, JSON_OBJECT_AS_ARRAY);
        $explicitFalse = json_decode('{"value":9}', false, 512, JSON_OBJECT_AS_ARRAY);
        echo is_array($defaulted), ':', $defaulted['value'], ':';
        echo is_array($null), ':', $null['value'], ':';
        echo is_object($explicitFalse), ':', $explicitFalse->value, "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:7:1:8:1:9\n");
}
