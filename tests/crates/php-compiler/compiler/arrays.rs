use php_compiler::compile_source;

#[test]
fn by_reference_array_mutators_detach_copied_slots_and_preserve_embedded_references() {
    let source = r#"<?php
        $original = [1, 2];
        $pushed = $original;
        array_push($pushed, 3);
        $popped = $original;
        $removed = array_pop($popped);
        $unshifted = $original;
        array_unshift($unshifted, 0);
        echo implode(',', $original), '|', implode(',', $pushed), '|';
        echo implode(',', $popped), ':', $removed, '|', implode(',', $unshifted), '|';
        $shared = 4;
        $with_reference = [&$shared];
        $reference_copy = $with_reference;
        array_push($reference_copy, 5);
        $reference_copy[0] = 9;
        echo $shared, ':', $with_reference[0], ':', $reference_copy[0];
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1,2|1,2,3|1:2|0,1,2|9:9:9"
    );
}

#[test]
fn array_shift_and_splice_detach_copied_arrays_and_preserve_retained_references() {
    let source = r#"<?php
        $shared = 4;
        $original = ['first', 'linked' => &$shared, 'last'];
        $shifted = $original;
        $removed = array_shift($shifted);
        $shifted['linked'] = 8;
        $spliced = $original;
        $removedBySplice = array_splice($spliced, 0, 1, ['replacement']);
        $spliced['linked'] = 9;
        echo $removed, ':', $removedBySplice[0], '|';
        echo implode(',', $original), '|', implode(',', $shifted), '|', implode(',', $spliced), '|', $shared;
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "first:first|first,9,last|9,last|replacement,9,last|9"
    );
}

#[test]
fn fractional_float_array_keys_emit_precision_deprecation_and_truncate() {
    let source = r#"<?php
        $array = [1.5 => 'literal'];
        $array[2.75] = 'write';
        echo $array[1.25], ':', $array[2];
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "literal:write");
    assert_eq!(result.diagnostics.len(), 3);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Deprecated));
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic
        .message
        .starts_with("Implicit conversion from float ")
        && diagnostic.message.ends_with(" to int loses precision")));
}

#[test]
fn array_and_object_offsets_raise_catchable_illegal_offset_type_errors() {
    let source = r#"<?php
        $array = [];
        $object = new stdClass();
        foreach ([[], $object] as $invalidKey) {
            try { $array[$invalidKey] = 1; }
            catch (TypeError $error) { echo $error->getMessage(), ';'; }
            try { $value = $array[$invalidKey]; }
            catch (TypeError $error) { echo $error->getMessage(), ';'; }
        }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "Illegal offset type;Illegal offset type;Illegal offset type;Illegal offset type;"
    );
}

#[test]
fn isset_and_empty_reject_array_and_object_offset_keys_with_php_message() {
    let source = r#"<?php
        $array = ['key' => 1];
        $object = new stdClass();
        foreach ([[], $object] as $invalidKey) {
            try { isset($array[$invalidKey]); }
            catch (TypeError $error) { echo $error->getMessage(), ';'; }
            try { empty($array[$invalidKey]); }
            catch (TypeError $error) { echo $error->getMessage(), ';'; }
        }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(),
        "Cannot access offset of type array in isset or empty;Cannot access offset of type array in isset or empty;Cannot access offset of type stdClass in isset or empty;Cannot access offset of type stdClass in isset or empty;");
}

#[test]
fn unset_rejects_invalid_array_offset_keys_with_php_85_error_message() {
    let source = r#"<?php
        class InvalidUnsetKeyHolder { public $items = []; public static $staticItems = []; }
        $array = [];
        $holder = new InvalidUnsetKeyHolder();
        foreach ([[], new stdClass()] as $invalidKey) {
            try { unset($array[$invalidKey]); }
            catch (Error $error) { echo get_class($error), ':', $error->getMessage(), ';'; }
            try { unset($holder->items[$invalidKey]); }
            catch (Error $error) { echo get_class($error), ':', $error->getMessage(), ';'; }
            try { unset(InvalidUnsetKeyHolder::$staticItems[$invalidKey]); }
            catch (Error $error) { echo get_class($error), ':', $error->getMessage(), ';'; }
        }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(),
        "Error:Cannot unset offset of type array on array;Error:Cannot unset offset of type array on array;Error:Cannot unset offset of type array on array;Error:Cannot unset offset of type stdClass on array;Error:Cannot unset offset of type stdClass on array;Error:Cannot unset offset of type stdClass on array;");
}

#[test]
fn isset_and_empty_on_string_invalid_keys_warn_and_keep_php_boolean_results() {
    let source = r#"<?php
        $text = 'abc';
        $object = new stdClass();
        echo isset($text[[]]), ':', empty($text[$object]);
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, ":1");
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic
        .message
        .starts_with("Cannot access offset of type ")
        && diagnostic.message.ends_with(" on string")));
}

#[test]
fn fractional_float_keys_in_array_helpers_emit_precision_deprecations() {
    let source = r#"<?php
        $filled = array_fill_keys([1.5], 'filled');
        $combined = array_combine([2.5], ['combined']);
        $column = array_column([['id' => 3.5, 'value' => 'column']], 'value', 'id');
        $exists = array_key_exists(1.5, [1 => 'present']);
        echo $filled[1], ':', $combined[2], ':', $column[3], ':', $exists ? 'yes' : 'no';
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "filled:combined:column:yes");
    assert_eq!(result.diagnostics.len(), 4);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Deprecated));
}

#[test]
fn fractional_float_keys_in_destructuring_targets_emit_deprecations() {
    let source = r#"<?php
        $target = [];
        [$target[2.5]] = ['value'];
        echo $target[2];
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "value");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(
        result.diagnostics[0].level,
        php_runtime::DiagnosticLevel::Deprecated
    );
    assert_eq!(
        result.diagnostics[0].message,
        "Implicit conversion from float 2.5 to int loses precision"
    );
}

#[test]
fn non_representable_float_array_keys_emit_php_85_warnings() {
    let source = r#"<?php
        $array = [];
        $array[sqrt(-1)] = 'nan';
        $array[1e100] = 'large';
        echo count($array);
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "2");
    assert_eq!(result.diagnostics.len(), 2);
    assert_eq!(
        result.diagnostics[0].level,
        php_runtime::DiagnosticLevel::Warning
    );
    assert_eq!(
        result.diagnostics[0].message,
        "unexpected NAN value was coerced to int"
    );
    assert_eq!(
        result.diagnostics[1].level,
        php_runtime::DiagnosticLevel::Warning
    );
    assert_eq!(
        result.diagnostics[1].message,
        "The float 1.0E+100 is not representable as an int, cast occurred"
    );
}

#[test]
fn non_representable_float_keys_in_array_helpers_emit_php_85_warnings() {
    let source = r#"<?php
        $array = array_fill_keys([sqrt(-1), 1e100], 'value');
        echo count($array);
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "2");
    assert_eq!(result.diagnostics.len(), 2);
    assert_eq!(
        result.diagnostics[0].message,
        "unexpected NAN value was coerced to int"
    );
    assert_eq!(
        result.diagnostics[1].message,
        "The float 1.0E+100 is not representable as an int, cast occurred"
    );
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
}

#[test]
fn php85_destructuring_scalars_warns_and_assigns_null_but_null_stays_silent() {
    let source = r#"<?php
        [$integer] = 42;
        [$float] = 1.5;
        [$string] = 'x';
        [$boolean] = true;
        [$resource] = fopen('php://memory', 'r+');
        [$nullValue] = null;
        echo gettype($integer), ':', gettype($float), ':', gettype($string), ':', gettype($boolean), ':', gettype($resource), ':', gettype($nullValue);
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "NULL:NULL:NULL:NULL:NULL:NULL");
    assert_eq!(result.diagnostics.len(), 5);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
    assert_eq!(
        result
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>(),
        [
            "Cannot use int as array",
            "Cannot use float as array",
            "Cannot use string as array",
            "Cannot use bool as array",
            "Cannot use resource as array",
        ]
    );
}

#[test]
fn php85_destructuring_objects_still_raise_error() {
    let source = r#"<?php
        class DestructureObject {}
        try { [$value] = new DestructureObject(); }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(); }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "Error:Cannot use object of type DestructureObject as array"
    );
}

#[test]
fn destructuring_by_reference_binds_source_slots_and_nested_slots() {
    let source = r#"<?php
        $source = [10, ['value' => 20]];
        [$first, ['value' => &$nested]] = $source;
        $first = 11;
        $nested = 22;
        [$temporary, &$temporaryAlias] = [30, 40];
        $temporaryAlias = 41;
   $holder = ['slot' => 0];
   [&$holder['slot']] = $source;
   $holder['slot'] = 12;
   echo $source[0], ':', $source[1]['value'], ':', $temporaryAlias;
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "12:22:41"
    );
}

#[test]
fn destructuring_by_reference_binds_direct_and_array_offset_object_properties() {
    let source = r#"<?php
        $source = [10, 20];
        $holder = new class { public $direct = 0; public $items = [0]; };
        [&$holder->direct] = $source;
        [&$holder->items[0]] = $source;
        $holder->direct = 11;
        $holder->items[0] = 12;
        echo $source[0], ':', $holder->direct, ':', $holder->items[0];
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "12:12:12"
    );
}

#[test]
fn destructuring_by_reference_rebinds_a_local_that_already_has_an_alias() {
    let source = r#"<?php
        $old = 1;
        $existing =& $old;
        $new = 2;
        [&$existing] = [&$new];
        $new = 3;
        echo $existing, ':', $old, ':', $new;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "3:1:3");
}

#[test]
fn destructuring_missing_array_keys_emit_undefined_key_warnings() {
    let source = r#"<?php
        [$first, $second] = [42];
        ['known' => $known, 'missing' => $missing] = ['known' => 7];
        [, $omittedTarget] = [];
        echo $first, ':', gettype($second), ':', $known, ':', gettype($missing), ':', gettype($omittedTarget);
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "42:NULL:7:NULL:NULL");
    assert_eq!(
        result
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>(),
        [
            "Undefined array key 1",
            "Undefined array key \"missing\"",
            "Undefined array key 1",
        ]
    );
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
}

#[test]
fn destructuring_by_reference_rejects_typed_object_properties() {
    let source = r#"<?php
        class TypedReferenceDestructureTarget { public int $value = 0; }
        $object = new TypedReferenceDestructureTarget();
        try { [&$object->value] = [5]; }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(); }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "Error:Typed property TypedReferenceDestructureTarget::$value must not be referenced"
    );
}

#[test]
fn array_combine_throws_value_error_for_mismatched_lengths() {
    let source = r#"<?php
        try { array_combine(['a', 'b'], [1]); }
        catch (ValueError $error) { echo get_class($error), ':', $error->getMessage(); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ValueError:array_combine(): Argument #1 ($keys) and argument #2 ($values) must have the same number of elements");
}

#[test]
fn array_flip_warns_and_skips_values_that_cannot_be_array_keys() {
    let source = r#"<?php
        $referenced = 'shared';
        $flipped = array_flip(['first' => &$referenced, 'float' => 1.5, 'array' => [], 'last' => 'shared']);
        echo $flipped['shared'], ':', count($flipped);
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "last:1");
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.message
            == "array_flip(): Can only flip STRING and INTEGER values!"));
}

#[test]
fn array_count_values_warns_and_skips_non_integer_and_non_string_values() {
    let source = r#"<?php
        $referenced = 7;
        $counts = array_count_values(['int' => &$referenced, 'string' => 'x', 'float' => 2.5, 'bool' => true, 'array' => []]);
        echo $counts[7], ':', $counts['x'], ':', count($counts);
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "1:1:2");
    assert_eq!(result.diagnostics.len(), 3);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.message
            == "array_count_values(): Can only count string and integer values, entry skipped"));
}

#[test]
fn array_fill_throws_value_error_for_out_of_range_counts() {
    let source = r#"<?php
        try { array_fill(0, -1, 'x'); } catch (ValueError $error) { echo $error->getMessage(), '|'; }
        try { array_fill(0, 2147483648, 'x'); } catch (ValueError $error) { echo $error->getMessage(); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "array_fill(): Argument #2 ($count) must be greater than or equal to 0|array_fill(): Argument #2 ($count) is too large");
}

#[test]
fn array_fill_rejects_integer_key_overflow_with_error() {
    let source = r#"<?php
        try { array_fill(9223372036854775807, 2, 'x'); }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(); }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "Error:Cannot add element to the array as the next element is already occupied",
    );
}

#[test]
fn array_change_key_case_folds_ascii_without_changing_unicode_bytes() {
    let source = r#"<?php
        $input = ['ÄBC' => 1, 'AbC' => 2, 'äbc' => 3];
        $lower = array_change_key_case($input, CASE_LOWER);
        $upper = array_change_key_case($input, CASE_UPPER);
        echo implode(',', array_keys($lower)), '|', implode(',', array_keys($upper));
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "Äbc,abc,äbc|ÄBC,ABC,äBC"
    );
}

#[test]
fn array_chunk_throws_value_error_for_non_positive_lengths() {
    let source = r#"<?php
        try { array_chunk([1], 0); } catch (ValueError $error) { echo $error->getMessage(), '|'; }
        try { array_chunk([1], -1); } catch (ValueError $error) { echo $error->getMessage(); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "array_chunk(): Argument #2 ($length) must be greater than 0|array_chunk(): Argument #2 ($length) must be greater than 0");
}

#[test]
fn assignment_appends_at_multiple_levels_of_an_array_path() {
    let source = r#"<?php
        $body = [];
        $body['personalizations'][]['to'][]['email'] = 'ada@example.test';
        echo $body['personalizations'][0]['to'][0]['email'];
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "ada@example.test"
    );
}

#[test]
fn foreach_iterates_visible_object_properties_and_warns_for_scalar_inputs() {
    let source = r#"<?php
        class ForeachPropertyBox {
            public $first = 2;
            private $hidden = 9;
            public $second = 3;
        }
        $box = new ForeachPropertyBox();
        foreach ($box as $key => $value) { echo $key, ':', $value, '|'; }
        foreach ($box as &$value) { $value *= 2; }
        unset($value);
        echo $box->first, ':', $box->second, '|';
        foreach (42 as $value) {}
        $last = error_get_last();
        echo $last['type'], ':', $last['message'], '|';
        error_reporting(0);
        foreach (null as $value) {}
        echo error_get_last()['message'];
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "first:2|second:3|4:6|2:foreach() argument must be of type array|object, int given|foreach() argument must be of type array|object, null given",
    );
}

#[test]
fn list_assignment_condition_preserves_assigned_value_truthiness() {
    let source = r#"<?php
        $parts = [['ready'], []];
        while ([$part] = array_shift($parts)) { echo $part; }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ready");
}

#[test]
fn compound_assignment_updates_a_dynamically_named_property() {
    let source = r#"<?php
        class Counter { public int $count = 2; public function add($unit, $value) { $this->$unit += $value; return $this->count; } }
        echo (new Counter())->add('count', 3);
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "5");
}

#[test]
fn array_filter_modes_pass_keys_and_values_with_php_constants() {
    let source = r#"<?php
        function filterPair($value, $key) { return $value >= 3 && $key !== 'skip'; }
        $input = ['keep' => 3, 'skip' => 4, 'small' => 1];
        $byKey = array_filter($input, function ($key) { return $key === 'skip'; }, ARRAY_FILTER_USE_KEY);
        $byBoth = array_filter($input, 'filterPair', ARRAY_FILTER_USE_BOTH);
        echo ARRAY_FILTER_USE_BOTH, ':', ARRAY_FILTER_USE_KEY, ':', count($byKey), ':', $byKey['skip'], ':', count($byBoth), ':', $byBoth['keep'], "\n";
        $withoutCallback = array_filter([0, 1], null, 12345);
        echo count($withoutCallback), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "1:2:1:4:1:3\n1\n");

    let invalid = compile_source("<?php array_filter([1], 'is_int', 3);")
        .unwrap()
        .execute()
        .unwrap_err();
    assert!(invalid.contains("ARRAY_FILTER_USE_BOTH"));
}

#[test]
fn random_array_helpers_return_valid_keys_and_shuffle_values() {
    let source = r#"<?php
        $source = ['a' => 10, 'b' => 20, 'c' => 30, 'd' => 40];
        $one = array_rand(array: $source);
        $many = array_rand(array: $source, num: 3);
        $keysValid = array_key_exists($one, $source) && count($many) === 3;
        foreach ($many as $key) { if (!array_key_exists($key, $source)) { $keysValid = false; } }
        $copy = $source;
        $shuffled = shuffle($copy);
        sort($copy);
        echo $keysValid, ':', $shuffled, ':', implode(',', $copy), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "1:1:10,20,30,40\n");
}

#[test]
fn descending_array_sorts_preserve_the_order_of_equal_values() {
    let source = r#"<?php
        $values = ['first' => 3, 'second' => 3, 'third' => 1, 'fourth' => 3];
        arsort($values);
        echo implode(',', array_keys($values)), ':', implode(',', $values), "\n";
        $values = ['first' => 3, 'second' => 3, 'third' => 1, 'fourth' => 3];
        rsort($values);
        echo implode(',', $values), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "first,second,fourth,third:3,3,3,1\n3,3,3,1\n"
    );
}

#[test]
fn regular_array_sort_compares_numeric_strings_numerically_and_unique_honors_flags() {
    let source = r#"<?php
        $values = [2, '10', '3'];
        sort($values, SORT_REGULAR);
        echo implode(',', $values), "\n";
        $values = [1.5, '1.50', '1.5'];
        echo count(array_unique($values, SORT_STRING)), ':', count(array_unique($values, SORT_NUMERIC)), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "2,3,10\n2:1\n"
    );
}

#[test]
fn regular_array_comparison_matches_values_by_key_and_handles_missing_keys() {
    let source = r#"<?php
        $left = ['first' => 1, 'second' => 2];
        $reordered = ['second' => 2, 'first' => 1];
        $other = ['third' => 1, 'fourth' => 2];
        echo ($left <=> $reordered), ':', ($left <=> $other), ':', ($other <=> $left);
        $values = [$left, $reordered];
        sort($values, SORT_REGULAR);
        echo ':', $values[0]['first'], ':', $values[1]['first'], ':', ([] <=> 1), ':';
        class SortComparisonObject {}
        $object = new SortComparisonObject();
        echo ($object <=> []);
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "0:1:1:1:1:1:1"
    );
}

#[test]
fn regular_array_sort_does_not_treat_unicode_whitespace_as_numeric_string_padding() {
    let source = r#"<?php
        $values = [" 2", '10', '2'];
        sort($values, SORT_REGULAR);
        echo implode('|', $values), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "2|10|\u{00a0}2\n"
    );
}

#[test]
fn regular_array_sort_compares_large_integer_strings_without_float_rounding() {
    let source = r#"<?php
        $values = ['9007199254740993', '9007199254740992'];
        sort($values, SORT_REGULAR);
        echo implode(',', $values), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "9007199254740992,9007199254740993\n"
    );
}

#[test]
fn case_insensitive_string_and_natural_sorts_fold_ascii_only() {
    let source = r#"<?php
        $strings = ['İ', 'z', 'A', 'a'];
        sort($strings, SORT_STRING | SORT_FLAG_CASE);
        echo implode(',', $strings), "\n";
        $natural = ['İ2', 'z10', 'Z1', 'z2'];
        sort($natural, SORT_NATURAL | SORT_FLAG_CASE);
        echo implode(',', $natural), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "A,a,z,İ\nZ1,z2,z10,İ2\n"
    );
}

#[test]
fn regular_array_sort_compares_objects_by_class_and_properties() {
    let source = r#"<?php
        class SortableRecord { public $rank; public $label; function __construct($rank, $label) { $this->rank = $rank; $this->label = $label; } }
        $records = [new SortableRecord(2, 'b'), new SortableRecord(1, 'a'), new SortableRecord(2, 'a')];
        sort($records, SORT_REGULAR);
        echo $records[0]->rank, ':', $records[0]->label, ',', $records[1]->rank, ':', $records[1]->label, ',', $records[2]->rank, ':', $records[2]->label, "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:a,2:a,2:b\n"
    );
}

#[test]
fn min_max_compare_same_class_objects_recursively_like_regular_sorts() {
    let source = r#"<?php
        class ExtremumRecord { public $rank; public $meta; function __construct($rank, $score) { $this->rank = $rank; $this->meta = ['score' => $score]; } }
        $low = new ExtremumRecord(2, 4);
        $high = new ExtremumRecord(3, 1);
        echo min($high, $low)->rank, ':', max($low, $high)->rank, "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2:3\n");
}

#[test]
fn min_max_on_empty_array_throws_catchable_value_error() {
    let source = r#"<?php
        try { max([]); } catch (ValueError $error) { echo get_class($error), ':', $error->getMessage(), "\n"; }
        try { min([]); } catch (ValueError $error) { echo get_class($error), ':', $error->getMessage(), "\n"; }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "ValueError:max(): Argument #1 ($value) must contain at least one element\nValueError:min(): Argument #1 ($value) must contain at least one element\n"
    );
}

#[test]
fn regular_array_sort_compares_nested_arrays_by_keys_and_values() {
    let source = r#"<?php
        $rows = [['rank' => 2, 'tie' => ['score' => 5]], ['rank' => 1, 'tie' => ['score' => 9]], ['rank' => 2, 'tie' => ['score' => 1]]];
        sort($rows, SORT_REGULAR);
        echo $rows[0]['rank'], ':', $rows[1]['rank'], ':', $rows[1]['tie']['score'], ':', $rows[2]['tie']['score'], "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:2:1:5\n"
    );
}

#[test]
fn recursive_array_comparison_throws_php_84_error_during_sorting() {
    let source = r#"<?php
        $left = [];
        $left['self'] =& $left;
        $right = [];
        $right['self'] =& $right;
        try {
            sort([$left, $right], SORT_REGULAR);
        } catch (Error $error) {
            echo get_class($error), ':', $error->getMessage(), "\n";
        }
        try { $comparison = $left <=> $right; }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(), "\n"; }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "Error:Nesting level too deep - recursive dependency?\nError:Nesting level too deep - recursive dependency?\n");
}

#[test]
fn array_multisort_reports_catchable_value_and_type_errors() {
    let source = r#"<?php
        try { array_multisort([1, 2], [3]); }
        catch (ValueError $error) { echo get_class($error), ':'; }
        try { array_multisort('not an array'); }
        catch (TypeError $error) { echo get_class($error); }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "ValueError:TypeError"
    );
}

#[test]
fn array_multisort_rejects_unknown_and_repeated_flag_kinds() {
    let source = r#"<?php
        foreach ([
            fn() => array_multisort([2, 1], 1234),
            fn() => array_multisort([2, 1], SORT_ASC, SORT_DESC),
            fn() => array_multisort([2, 1], SORT_STRING, SORT_NATURAL),
            fn() => array_multisort([2, 1], SORT_REGULAR | SORT_FLAG_CASE),
        ] as $call) {
            try { $call(); }
            catch (ValueError $error) { echo 'ValueError:'; }
            catch (TypeError $error) { echo 'TypeError:'; }
        }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "ValueError:TypeError:TypeError:ValueError:"
    );
}

#[test]
fn array_copies_separate_nested_values_but_keep_embedded_reference_cells() {
    let source = r#"<?php
        $shared = 1;
        $original = ['reference' => &$shared, 'nested' => ['value' => 10]];
        $copy = $original;
        $copy['nested']['value'] = 20;
        $copy['reference'] = 2;
        $third = $copy;
        $third['nested']['value'] = 30;
        echo $shared, ':', $original['nested']['value'], ':', $copy['nested']['value'], ':', $third['nested']['value'], ':', $original['reference'], "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "2:10:20:30:2\n"
    );
}

#[test]
fn array_transformations_preserve_embedded_reference_cells() {
    let source = r#"<?php
        $shared = 1;
        $source = [&$shared];
        $merged = array_merge($source);
        $sliced = array_slice($source, 0);
        $reversed = array_reverse($source);
        $replaced = array_replace([], $source);
        $chunks = array_chunk($source, 1);
        $padded = array_pad($source, 2, null);
        $unique = array_unique($source);
        $recursiveSource = ['nested' => ['value' => &$shared]];
        $recursiveMerged = array_merge_recursive($recursiveSource, ['nested' => ['value' => 7]]);
        $recursiveReplaced = array_replace_recursive([], $recursiveSource);
        $merged[0] = 2;
        $sliced[0] = 3;
        $reversed[0] = 4;
        $replaced[0] = 5;
        $chunks[0][0] = 6;
        $padded[0] = 8;
        $unique[0] = 9;
        $recursiveMerged['nested']['value'][0] = 10;
        $recursiveReplaced['nested']['value'] = 11;
        echo $shared;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "11");
}

#[test]
fn array_values_and_filter_preserve_embedded_reference_cells() {
    let source = r#"<?php
        $shared = 1;
        $source = ['keep' => &$shared, 'drop' => 0];
        $values = array_values($source);
        $filtered = array_filter($source);
        $values[0] = 2;
        echo $shared, ':';
        $filtered['keep'] = 3;
        echo $shared;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2:3");
}

#[test]
fn array_column_preserves_reference_cells_in_selected_values() {
    let source = r#"<?php
        $shared = 1;
        $rows = [['value' => &$shared]];
        $column = array_column($rows, 'value');
        $column[0] = 7;
        echo $shared;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "7");
}

#[test]
fn recursive_count_warns_once_and_returns_partial_entry_count() {
    let source = r#"<?php
        $self = [];
        $self['loop'] =& $self;
        echo count($self), ':', count($self, COUNT_RECURSIVE);
        $outer = [];
        $inner = [];
        $outer['inner'] =& $inner;
        $inner['outer'] =& $outer;
        echo ':', count($outer, COUNT_RECURSIVE);
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "1:1:2");
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.message == "count(): Recursion detected"));
}

#[test]
fn array_pop_and_shift_return_values_without_leaking_removed_references() {
    let source = r#"<?php
        $shared = 1;
        $tail = [&$shared];
        $popped = array_pop($tail);
        $popped = 2;
        $remaining = 4;
        $head = ['first' => 0, 8 => &$shared, 20 => &$remaining];
        $shifted = array_shift($head);
        $shifted = 3;
        $head[0] = 5;
        $head[1] = 6;
        echo $shared, ':', $remaining, ':', count($tail), ':', count($head), ':', array_key_first($head), ':', array_key_last($head);
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "5:6:0:2:0:1"
    );
}

#[test]
fn array_splice_casts_object_replacements_to_property_values() {
    let source = r#"<?php
        class SpliceReplacementObject {
            public $first = 'one';
            public $second = 'two';
        }
        $array = ['left', 'removed', 'right'];
        $removed = array_splice($array, 1, 1, new SpliceReplacementObject());
        echo implode(',', $removed), ':', implode(',', $array), ':', count($array);
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "removed:left,one,two,right:4"
    );
}

#[test]
fn copy_on_write_detaches_array_aliases_without_losing_element_references() {
    let source = r#"<?php
        $shared = 1;
        $array = ['value' => 10, 'alias' => &$shared];
        $alias =& $array;
        $snapshot = $array;
        $alias['value'] = 20;
        $alias['alias'] = 2;
        echo $array['value'], ':', $snapshot['value'], ':', $shared, ':', $snapshot['alias'];
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "20:10:2:2"
    );
}

#[test]
fn by_reference_array_walk_detaches_a_copied_array_but_keeps_shared_slots() {
    let source = r#"<?php
        $sourceValue = 1;
        $source = ['plain' => 2, 'shared' => &$sourceValue];
        $copy = $source;
        array_walk($copy, function (&$value) { $value += 10; });
        echo $source['plain'], ':', $copy['plain'], ':', $sourceValue, ':', $source['shared'];
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "2:12:11:11"
    );
}

#[test]
fn by_reference_foreach_detaches_a_copy_and_keeps_the_loop_variable_alias() {
    let source = r#"<?php
        $source = [1, 2];
        $copy = $source;
        foreach ($copy as &$value) { $value *= 10; }
        $value = 9;
        echo implode(',', $source), ':', implode(',', $copy);
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1,2:10,9"
    );
}

#[test]
fn regular_object_sort_compares_property_keys_without_insertion_order() {
    let source = r#"<?php
        class DynamicShape {}
        $first = new DynamicShape();
        $first->alpha = 1;
        $first->beta = 2;
        $second = new DynamicShape();
        $second->beta = 2;
        $second->alpha = 1;
        $items = [$second, $first];
        sort($items, SORT_REGULAR);
        echo ($items[0] === $second), ':', ($items[1] === $first), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:1\n");
}

#[test]
fn regular_sort_and_extrema_keep_order_for_incomparable_object_classes() {
    let source = r#"<?php
        class AlphaSortable {}
        class ZuluSortable {}
        $zulu = new ZuluSortable();
        $alpha = new AlphaSortable();
        $items = [$zulu, $alpha];
        sort($items, SORT_REGULAR);
        echo ($items[0] === $zulu), ':', ($items[1] === $alpha), ':';
        echo (max($zulu, $alpha) === $zulu), ':', (min($zulu, $alpha) === $zulu);
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:1:1:1"
    );
}

#[test]
fn sorting_a_copied_array_separates_keys_but_keeps_element_references() {
    let source = r#"<?php
        $shared = 2;
        $original = ['named' => &$shared, 'other' => 1];
        $copy = $original;
        sort($copy, SORT_NUMERIC);
        $copy[1] = 9;
        echo array_key_exists('named', $original), ':', $original['other'], ':', $original['named'], ':', $shared, ':', implode(',', $copy), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:1:9:9:1,9\n"
    );
}

#[test]
fn shuffle_detaches_copied_array_keys_and_preserves_embedded_reference_cells() {
    let source = r#"<?php
        $shared = 7;
        $source = ['linked' => &$shared, 'plain' => 2];
        $copy = $source;
        shuffle($copy);
        if ($copy[0] === 7) { $copy[0] = 9; } else { $copy[1] = 9; }
        echo implode(',', array_keys($source)), ':', $source['plain'], ':', $source['linked'], ':', $shared, ':', array_key_first($copy);
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "linked,plain:2:9:9:0"
    );
}

#[test]
fn array_mutators_reject_literal_array_arguments_with_php_diagnostics() {
    for (source, expected) in [
        (
            "<?php array_push([], 1);",
            "array_push(): Argument #1 ($array) cannot be passed by reference",
        ),
        (
            "<?php array_pop(array: []);",
            "array_pop(): Argument #1 ($array) cannot be passed by reference",
        ),
        (
            "<?php sort(array: [2, 1]);",
            "sort(): Argument #1 ($array) cannot be passed by reference",
        ),
    ] {
        let error = compile_source(source).unwrap().execute().unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
    let caught = compile_source(
        r#"<?php
        try { array_push([], 1); }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(); }
    "#,
    )
    .unwrap()
    .execute()
    .unwrap();
    assert_eq!(
        caught,
        "Error:array_push(): Argument #1 ($array) cannot be passed by reference"
    );
}

#[test]
fn array_walk_ignores_callback_return_and_writes_only_by_reference_changes() {
    let source = r#"<?php
        $returnedOnly = ['first' => 'lower'];
        array_walk($returnedOnly, function ($value, $key) { return strtoupper($value); });
        $values = ['first' => 'lower', 'second' => 'also lower'];
        function uppercaseValue(&$value) { $value = strtoupper($value); return 'ignored'; }
        array_walk($values, 'uppercaseValue');
        echo implode(',', $returnedOnly), '|', implode(',', $values), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "lower|LOWER,ALSO LOWER\n"
    );
}

#[test]
fn array_walk_recursive_traverses_nested_arrays_held_by_reference() {
    let source = r#"<?php
        $nested = ['value' => 4];
        $items = ['branch' => &$nested];
        array_walk_recursive($items, function (&$value) { $value *= 3; });
        echo $nested['value'], ':', $items['branch']['value'];
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "12:12");
}

#[test]
fn array_walk_recursive_warns_once_and_skips_a_recursive_reference_edge() {
    let source = r#"<?php
        $items = ['leaf' => 2];
        $items['self'] =& $items;
        $visited = 0;
        array_walk_recursive($items, function (&$value) use (&$visited) { $visited++; });
        $left = ['left' => 1];
        $right = ['right' => 2];
        $left['right'] =& $right;
        $right['left'] =& $left;
        $cycleVisits = 0;
        array_walk_recursive($left, function (&$value) use (&$cycleVisits) { $cycleVisits++; });
        echo $visited, ':', $cycleVisits;
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "2:3");
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.message == "array_walk_recursive(): Recursion detected"));
}

#[test]
fn count_accepts_countable_objects_and_validates_modes_and_input_types() {
    let source = r#"<?php
        class CountedValues implements Countable {
            public function count(): int { return 7; }
        }
        class OptionalCountedValues implements Countable {
            public function count(int $ignored = 0): int { return 4; }
        }
        class ChildCountedValues extends CountedValues {}
        class PlainValues {}
        echo interface_exists('Countable'), ':', is_countable(value: []), ':', is_countable(new ChildCountedValues()), ':', is_countable(new PlainValues()), ':';
        echo implode(',', array_map('is_countable', [[], new CountedValues(), new PlainValues()])), ':', call_user_func('is_countable', new OptionalCountedValues()), ':';
        echo function_exists('is_countable'), ':', count(value: new CountedValues(), mode: COUNT_NORMAL), ':', count(new ChildCountedValues()), ':', count(new OptionalCountedValues()), ':';
        echo count([[1, 2], 3], COUNT_RECURSIVE), ':', count([1, 2]);
        try { count(new PlainValues()); } catch (TypeError $error) { echo ':', get_class($error); }
        try { count('not an array'); } catch (TypeError $error) { echo ':', get_class($error); }
        try { count(null); } catch (TypeError $error) { echo ':', get_class($error); }
        try { count([], 2); } catch (ValueError $error) { echo ':', get_class($error); }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:1:1::1,1,:1:1:7:7:4:4:2:TypeError:TypeError:TypeError:ValueError"
    );
}

#[test]
fn countable_implementations_validate_the_builtin_method_contract() {
    for (source, expected) in [
        ("<?php class MissingCount implements Countable {}", "must implement count() from interface countable"),
        ("<?php class HiddenCount implements Countable { protected function count(): int { return 0; } }", "public instance method"),
        ("<?php class StaticCount implements Countable { public static function count(): int { return 0; } }", "public instance method"),
        ("<?php class ArgumentCount implements Countable { public function count(int $extra): int { return 0; } }", "without required parameters"),
        ("<?php class WrongCount implements Countable { public function count(): string { return '0'; } }", "Countable::count(): int"),
    ] {
        let error = compile_source(source).err().unwrap();
        assert!(error.contains(expected), "{source}: {error}");
    }
}

#[test]
fn array_callbacks_dispatch_registered_string_and_math_builtins() {
    let source = r#"<?php
        echo implode(',', array_map('number_format', [1234.5, 12.4])), '|';
        echo implode(',', array_map('sqrt', [4, 9])), '|';
        echo implode(',', array_map('strnatcmp', ['item2', 'item10'], ['item10', 'item2'])), '|';
        echo implode(',', array_map('str_replace', ['cat', 'dog'], ['a cat', 'a dog'], ['fox', 'fox']));
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1,235,12|2,3|-1,1|fox,fox"
    );
}

#[test]
fn array_callback_is_resource_tracks_stream_lifecycle() {
    let source = r#"<?php
        $stream = fopen('php://memory', 'r+');
        $before = array_map('is_resource', [$stream]);
        fclose($stream);
        $after = array_map('is_resource', [$stream]);
        echo ($before[0] ? 'open' : 'closed'), ':', ($after[0] ? 'open' : 'closed');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "open:closed"
    );
}

#[test]
fn callback_array_functions_warn_when_callbacks_require_references() {
    let source = r#"<?php
        function byReference(&$value) { return true; }
        function mutateReference(&$value) { $value = 99; return $value; }
        class ReferenceCallback { public function inspect(&$value, &$key, &$extra) { $value = 8; $key = 'changed'; $extra = 'changed'; return true; } }
        $callback = new ReferenceCallback();
        $empty = [];
        array_map('byReference', []);
        array_filter([], 'byReference');
        array_reduce([], 'byReference');
        usort($empty, 'byReference');
        array_map([$callback, 'inspect'], []);
        $walkValues = ['original' => 1];
        $extra = 'source';
        array_walk($walkValues, [$callback, 'inspect'], $extra);
        array_udiff([], [], 'byReference');
        array_udiff_uassoc([], [], 'byReference', 'byReference');
        $source = [1];
        $mapped = array_map('mutateReference', $source);
        array_all([1], 'byReference');
        array_any([1], 'byReference');
        array_find([1], 'byReference');
        array_find_key([1], 'byReference');
        echo $mapped[0], ':', $source[0], '|', $walkValues['original'], ':', array_key_first($walkValues), ':', $extra;
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.diagnostics.len(), 6);
    assert_eq!(result.output, "99:1|8:original:source");
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
    assert!(result.diagnostics[0].message.contains("array_walk"));
    assert!(result.diagnostics[1].message.contains("array_map"));
    assert!(result.diagnostics[2].message.contains("array_all"));
    assert!(result.diagnostics[3].message.contains("array_any"));
    assert!(result.diagnostics[4].message.contains("array_find"));
    assert!(result.diagnostics[5].message.contains("array_find_key"));
}

#[test]
fn user_array_diff_callbacks_match_value_and_key_variants() {
    let source = r#"<?php
        function compareAlwaysEqual($left, $right) { return 0; }
        $left = ['left' => 1];
        $right = ['right' => 2];
        $outputs = [
            array_udiff_ukey($left, $right, 'compareAlwaysEqual'),
            array_uintersect_ukey($left, $right, 'compareAlwaysEqual'),
        ];
        echo count($outputs[0]), ':', count($outputs[1]);
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "0:1");
}

#[test]
fn every_user_array_diff_callback_variant_warns_for_reference_parameters() {
    let source = r#"<?php
        function compareByReference(&$left, &$right) { return 0; }
        $a = ['key' => 1]; $b = ['key' => 1];
        array_diff_uassoc($a, $b, 'compareByReference');
        array_diff_ukey($a, $b, 'compareByReference');
        array_intersect_uassoc($a, $b, 'compareByReference');
        array_intersect_ukey($a, $b, 'compareByReference');
        array_udiff($a, $b, 'compareByReference');
        array_udiff_assoc($a, $b, 'compareByReference');
        array_udiff_uassoc($a, $b, 'compareByReference', 'compareByReference');
        array_udiff_ukey($a, $b, 'compareByReference');
        array_uintersect($a, $b, 'compareByReference');
        array_uintersect_assoc($a, $b, 'compareByReference');
        array_uintersect_uassoc($a, $b, 'compareByReference', 'compareByReference');
        array_uintersect_ukey($a, $b, 'compareByReference');
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.diagnostics.len(), 12, "{:#?}", result.diagnostics);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
    let names = [
        "array_diff_uassoc",
        "array_diff_ukey",
        "array_intersect_uassoc",
        "array_intersect_ukey",
        "array_udiff",
        "array_udiff_assoc",
        "array_udiff_uassoc",
        "array_udiff_ukey",
        "array_uintersect",
        "array_uintersect_assoc",
        "array_uintersect_uassoc",
        "array_uintersect_ukey",
    ];
    for (diagnostic, name) in result.diagnostics.iter().zip(names) {
        assert!(
            diagnostic.message.starts_with(&format!("{name}():")),
            "{diagnostic:?}"
        );
    }
}

#[test]
fn diff_callback_reference_warning_waits_until_that_comparator_is_called() {
    let source = r#"<?php
        function compareValuesNormally($left, $right) { return $left <=> $right; }
        function compareKeysByReference(&$left, &$right) { return $left <=> $right; }
        array_udiff_uassoc(['left' => 1], ['right' => 2], 'compareValuesNormally', 'compareKeysByReference');
        array_udiff_uassoc(['same' => 1], ['same' => 1], 'compareValuesNormally', 'compareKeysByReference');
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.diagnostics.len(), 1);
    assert!(result.diagnostics[0].message.starts_with(
        "array_udiff_uassoc(): callback expects a parameter to be passed by reference"
    ));
}

#[test]
fn array_filter_reference_callback_arguments_are_temporary_even_with_both_mode() {
    let source = r#"<?php
        function rewrite_filter_inputs(&$value, &$key) {
            $value = 99;
            $key = 'rewritten';
            return true;
        }
        $source = ['original' => 1];
        $filtered = array_filter($source, 'rewrite_filter_inputs', ARRAY_FILTER_USE_BOTH);
        echo array_key_first($filtered), ':', $filtered['original'], '|', array_key_first($source), ':', $source['original'];
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "original:1|original:1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(
        result.diagnostics[0].level,
        php_runtime::DiagnosticLevel::Warning
    );
    assert!(result.diagnostics[0]
        .message
        .contains("array_filter(): callback expects a parameter to be passed by reference"));
}

#[test]
fn array_callback_reference_parameters_do_not_bind_reference_backed_source_slots() {
    let source = r#"<?php
        function overwriteCallbackValue(&$value) { $value = 99; return true; }
        $shared = 7;
        $source = [&$shared];
        $mapped = array_map('overwriteCallbackValue', $source);
        $filtered = array_filter($source, 'overwriteCallbackValue');
        echo $shared, ':', $source[0], ':', $mapped[0], ':', $filtered[0];
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "7:7:1:7");
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic
        .message
        .contains("callback expects a parameter to be passed by reference")));
}

#[test]
fn static_string_callbacks_support_and_diagnose_by_reference_parameters() {
    let source = r#"<?php
        class StaticReferenceArrayCallback {
            public static function doubleValue(&$value): int { $value *= 2; return $value; }
            public static function doubleNormally($value): int { return $value * 2; }
        }
        $source = [3, 4];
        $mapped = array_map('StaticReferenceArrayCallback::doubleValue', $source);
        $ordinary = array_map('StaticReferenceArrayCallback::doubleNormally', $source);
        echo implode(',', $mapped), ':', implode(',', $ordinary), ':', implode(',', $source), ':',
            is_callable('StaticReferenceArrayCallback::doubleNormally');
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "6,8:6,8:3,4:1");
    assert_eq!(result.diagnostics.len(), 1);
    assert!(result.diagnostics[0]
        .message
        .contains("array_map(): callback expects a parameter to be passed by reference"));
}

#[test]
fn array_walk_static_method_strings_preserve_the_writable_value_argument() {
    let source = r#"<?php
        class StaticWalkCallback {
            public static function doubleValue(&$value, $key) { $value *= 2; }
            public static function rewriteKey(&$value, &$key) { $value += 1; $key = 'changed'; }
        }
        $values = ['left' => 3];
        array_walk($values, 'StaticWalkCallback::doubleValue');
        echo $values['left'], ':', array_key_first($values), '|';
        array_walk($values, 'StaticWalkCallback::rewriteKey');
        echo $values['left'], ':', array_key_first($values);
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "6:left|7:left");
    assert_eq!(result.diagnostics.len(), 1);
    assert!(result.diagnostics[0].message.contains(
        "array_walk(): callback expects the key or userdata parameter to be passed by reference"
    ));
}

#[test]
fn callback_reference_warnings_require_an_actual_callback_invocation() {
    let source = r#"<?php
        function mutateCallbackInputs(&$value) { $value = 99; return true; }
        $empty = [];
        array_map('mutateCallbackInputs', $empty);
        array_filter($empty, 'mutateCallbackInputs');
        array_reduce($empty, 'mutateCallbackInputs');
        usort($empty, 'mutateCallbackInputs');
        $single = [1];
        usort($single, 'mutateCallbackInputs');
        $mapped = array_map('mutateCallbackInputs', $single);
        echo $single[0], ':', $mapped[0];
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "1:1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(
        result.diagnostics[0].level,
        php_runtime::DiagnosticLevel::Warning
    );
    assert!(result.diagnostics[0]
        .message
        .starts_with("array_map(): callback expects a parameter to be passed by reference"));
}

#[test]
fn key_sorts_honor_sort_flags_for_mixed_integer_and_string_keys() {
    let source = r#"<?php
        $values = ['a' => 'letter', 2 => 'two'];
        ksort($values, SORT_REGULAR);
        echo implode(',', array_keys($values)), "\n";
        $values = ['a' => 'letter', 2 => 'two'];
        ksort($values, SORT_NUMERIC);
        echo implode(',', array_keys($values)), "\n";
        $values = ['a' => 'letter', 2 => 'two'];
        krsort($values, SORT_NUMERIC);
        echo implode(',', array_keys($values)), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "2,a\na,2\n2,a\n"
    );
}

#[test]
fn array_multisort_locale_string_mode_converts_stringable_objects() {
    let source = r#"<?php
        class LocaleSortableValue {
            public function __construct(public string $text) {}
            public function __toString(): string { return $this->text; }
        }
        $values = [new LocaleSortableValue('z'), new LocaleSortableValue('a')];
        array_multisort($values, SORT_LOCALE_STRING);
        echo $values[0], ':', $values[1];
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "a:z");
}

#[test]
fn array_rand_uses_catchable_type_and_value_errors() {
    let source = r#"<?php
        try { array_rand([1], 0); } catch (ValueError $error) { echo get_class($error), ':'; }
        try { array_rand('not array'); } catch (TypeError $error) { echo get_class($error), ':'; }
        try { array_rand([1], 'nope'); } catch (TypeError $error) { echo get_class($error); }
        echo "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "ValueError:TypeError:TypeError\n"
    );
}

#[test]
fn callback_helpers_dispatch_named_reference_and_callable_targets() {
    let source = r#"<?php
        function joinParts($left, $right) { return $left . $right; }
        echo call_user_func_array('joinParts', ['right' => 'B', 'left' => 'A']), "\n";
        echo call_user_func('joinParts', 'N', 'O'), "\n";
        echo call_user_func('joinParts', right: 'Q', left: 'P'), "\n";
        echo call_user_func_array('strlen', ['string' => 'abc']), "\n";
        echo call_user_func('strlen', 'hello'), "\n";
        $closure = function ($left, $right) { return $left . $right; };
        echo call_user_func_array($closure, ['left' => 'C', 'right' => 'D']), "\n";
        class CallableTarget {
            public function join($left, $right) { return $left . $right; }
            public function __invoke($value) { return '[' . $value . ']'; }
            public static function staticJoin($left, $right) { return $left . $right; }
        }
        $target = new CallableTarget();
        echo call_user_func_array([$target, 'join'], ['left' => 'E', 'right' => 'F']), "\n";
        echo [$target, 'join']('G', 'H'), "\n";
        echo call_user_func_array($target, ['value' => 'I']), "\n";
        echo call_user_func_array('CallableTarget::staticJoin', ['left' => 'J', 'right' => 'K']), "\n";
        echo call_user_func_array(['CallableTarget', 'staticJoin'], ['left' => 'L', 'right' => 'M']), "\n";
        function changeByReference(&$value) { $value = 'changed'; }
        $value = 'before';
        call_user_func_array('changeByReference', [&$value]);
        echo $value, "\n";
        $value = 'before';
        call_user_func('changeByReference', $value);
        echo $value, "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(
        output,
        "AB\nNO\nPQ\n3\n5\nCD\nEF\nGH\n[I]\nJK\nLM\nchanged\nbefore\n"
    );
}

#[test]
fn callable_and_symbol_introspection_reports_supported_runtime_symbols() {
    let source = r#"<?php
        function flag($value) { return $value ? '1' : '0'; }
        function knownFunction() {}
        interface KnownContract { public function run(); }
        class KnownClass implements KnownContract {
            public function run() {}
            public static function make() {}
            private function hidden() {}
            public function __invoke() {}
        }
        $object = new KnownClass();
        echo flag(function_exists('knownFunction')), flag(function_exists('strlen')), flag(function_exists('notThere')), "\n";
        echo flag(class_exists('KnownClass')), flag(class_exists('KnownContract')), flag(interface_exists('KnownContract')), "\n";
        echo flag(method_exists($object, 'run')), flag(method_exists('KnownClass', 'hidden')), flag(method_exists('KnownClass', 'missing')), "\n";
        $label = 'unset';
        echo flag(is_callable([$object, 'run'], false, $label)), ':', $label, "\n";
        echo flag(is_callable(['KnownClass', 'make'])), ':', flag(is_callable([$object, 'hidden'])), "\n";
        echo flag(is_callable($object)), ':', flag(is_callable(function () {})), "\n";
        echo flag(is_callable('KnownClass::missing', true)), ':', flag(is_callable('notThere')), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "110\n101\n110\n1:knownclass::run\n1:0\n1:1\n1:0\n");
}

#[test]
fn is_callable_recognizes_public_magic_call_methods() {
    let source = r#"<?php
        class MagicCallable {
            public function __call($name, $arguments) {}
            public static function __callStatic($name, $arguments) {}
        }
        $object = new MagicCallable();
        echo is_callable([$object, 'missing']), ':', is_callable('MagicCallable::missing'), ':', is_callable(['MagicCallable', 'missing']), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "1:1:1\n");
}

#[test]
fn prefix_and_postfix_increment_support_instance_properties() {
    let source = r#"<?php
        class IncrementTarget { public $value = 4; public static $total = 1; }
        $target = new IncrementTarget();
        echo $target->value++, ':', $target->value, ':', ++$target->value, ':', $target->value--, ':', $target->value, "\n";
        $increment = fn() => $target->value++;
        echo $increment(), ':', $target->value, "\n";
        echo IncrementTarget::$total++, ':', ++IncrementTarget::$total, ':', IncrementTarget::$total, "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "4:5:6:6:5\n5:6\n1:3:3\n");
}

#[test]
fn increment_lvalues_support_computed_instance_property_names() {
    let source = r#"<?php
        class ComputedIncrementTarget { public $count = 4; }
        $target = new ComputedIncrementTarget();
        $property = 'count';
        echo $target->{$property}++, ':', ++$target->{$property}, ':', $target->{$property}, "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "4:6:6\n"
    );
}

#[test]
fn increment_respects_typed_and_readonly_instance_properties() {
    let source = r#"<?php
        class TypedIncrementTarget { public int $count = 3; public readonly int $locked; public function __construct() { $this->locked = 8; } }
        $target = new TypedIncrementTarget();
        echo $target->count++, ':', ++$target->count, ':', $target->count, "\n";
        try { ++$target->locked; } catch (Error $error) { echo get_class($error), ':', $error->getMessage(), "\n"; }
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(
        output,
        "3:5:5\nError:Cannot modify readonly property typedincrementtarget::$locked\n"
    );
}

#[test]
fn increment_operators_support_variable_variables_and_nested_offsets() {
    let source = r#"<?php
        $targetName = 'counter';
        $counter = 4;
        echo $$targetName++, ':', $counter, ':', ++$$targetName, ':', $counter, "\n";
        $targetName = 'rows';
        $rows = ['item' => ['count' => 8]];
        echo $$targetName['item']['count']++, ':', $rows['item']['count'], ':';
        echo ++$$targetName['item']['count'], ':', $rows['item']['count'], "\n";
        $bump = function () use ($targetName, &$rows) { return ++$$targetName['item']['count']; };
        echo $bump(), ':', $rows['item']['count'], "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "4:5:6:6\n8:9:10:10\n11:11\n"
    );
}

#[test]
fn increment_operators_support_string_offset_lvalues() {
    let source = r#"<?php
        $text = 'Az';
        echo $text[0]++, ':', $text, ':', ++$text[1], ':', $text, ':', $text[-1], "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "A:Bz:aa:Ba:a\n"
    );
}

#[test]
fn range_emits_php_83_boundary_and_character_step_warnings() {
    let source = r#"<?php
        echo count(range('', '2')), ':', count(range('ab', 'd')), ':';
        echo count(range(1, 'x')), ':', count(range('a', 'c', 1.0));
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "3:4:2:3");
    assert_eq!(result.diagnostics.len(), 4);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
    assert!(result.diagnostics[0].message.contains("must not be empty"));
    assert!(result.diagnostics[1]
        .message
        .contains("must be a single byte, subsequent bytes are ignored"));
    assert!(result.diagnostics[2]
        .message
        .contains("other boundary is numeric"));
    assert!(result.diagnostics[3]
        .message
        .contains("generating an array of characters"));
    assert_eq!(result.diagnostics[0].source_line, Some(2));
    assert_eq!(result.diagnostics[1].source_line, Some(2));
    assert_eq!(result.diagnostics[2].source_line, Some(3));
    assert_eq!(result.diagnostics[3].source_line, Some(3));
}

#[test]
fn range_dereferences_numeric_string_and_character_boundaries() {
    let source = r#"<?php
        $decimalStart = '1.5';
        $decimalEnd = '3.5';
        $letterStart = 'a';
        $letterEnd = 'c';
        echo implode(',', range($decimalStart, $decimalEnd)), '|';
        echo implode(',', range($letterStart, $letterEnd));
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1.5,2.5,3.5|a,b,c"
    );
}

#[test]
fn range_uses_php_83_step_coercion_and_negative_step_rules() {
    let source = r#"<?php
        $integerStep = range(1, 5, 2.0);
        $fractionalStep = range(1, 5, 1.5);
        echo implode(',', array_map('gettype', $integerStep)), '|';
        echo implode(',', array_map('gettype', $fractionalStep)), '|';
        echo implode(',', range(5, 1, -2)), '|';
        echo implode(',', range('e', 'a', -2)), "\n";
        try { range(1, 5, -2); } catch (ValueError $error) { echo get_class($error), ':', $error->getMessage(); }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "integer,integer,integer|double,double,double|5,3,1|e,c,a\nValueError:range(): Argument #3 ($step) must be greater than 0 for increasing ranges",
    );
}

#[test]
fn range_numeric_strings_trim_only_php_ascii_whitespace() {
    let source = r#"<?php
        echo implode(',', range(' 1', 2)), '|';
        echo implode(',', range(" \t1\r\n", '3'));
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "0,1,2|1,2,3");
    assert_eq!(result.diagnostics.len(), 1);
    assert!(result.diagnostics[0]
        .message
        .contains("must be numeric when the other boundary is numeric"));
}

#[test]
fn range_uses_first_utf8_bytes_for_non_ascii_character_endpoints() {
    let source = r#"<?php
        echo count(range('é', 'ê')), ':', implode('', range('é', 'ê'));
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "1:�");
    assert_eq!(result.diagnostics.len(), 4);
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic
        .message
        .contains("single byte, subsequent bytes are ignored")));
}

#[test]
fn array_map_dispatches_array_builtin_callbacks() {
    let source = r#"<?php
        $rows = [[1, 2], [3, 4], [5]];
        echo implode(',', array_map('array_sum', $rows)), '|';
        echo implode(',', array_map('array_product', $rows)), '|';
        echo implode(',', array_map('array_is_list', $rows)), '|';
        $reversed = array_map('array_reverse', [[1, 2], [3, 4]]);
        echo count($reversed[0]), ':', $reversed[0][0], '|';
        $cased = array_map('array_change_key_case', [['Name' => 1], ['AGE' => 2]]);
        echo isset($cased[0]['name']), ':', isset($cased[1]['age']), '|';
        $chunks = array_map('array_chunk', [[1, 2, 3], [4, 5]], [2, 1]);
        echo count($chunks[0]), ':', count($chunks[0][0]), ':', count($chunks[1]), '|';
        $counts = array_map('array_count_values', [[1, 1, 'x'], [2, 3]]);
        $filled = array_map('array_fill_keys', [['a', 'b'], ['x']], [7, 9]);
        echo $counts[0][1], ':', $counts[0]['x'], ':', $counts[1][3], ':', $filled[0]['b'], ':', $filled[1]['x'], '|';
        $padded = array_map('array_pad', [[1], [2, 3]], [3, -4], [0, 9]);
        $filledRanges = array_map('array_fill', [0, 2], [2, 1], ['a', 'b']);
        echo implode(',', $padded[0]), ':', implode(',', $padded[1]), ':', implode(',', $filledRanges[0]), ':', $filledRanges[1][2];
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "3,7,5|2,12,5|1,1,1|2:2|1:1|2:2:2|2:1:1:7:9|1,0,0:9,9,2,3:a,a:b"
    );
}

#[test]
fn array_pad_preserves_string_keys_and_reindexes_numeric_keys() {
    let source = r#"<?php
        $shared = 'shared';
        $input = [8 => &$shared, 'label' => 'keep', 15 => 'tail'];
        $right = array_pad($input, 5, 'pad');
        $left = array_pad($input, -5, 'pad');
        echo implode(',', array_keys($right)), '|', implode(',', array_keys($left)), '|';
        $right[0] = 'updated';
        echo $shared, ':', $right['label'], ':', $left['label'];
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "0,label,1,2,3|0,1,2,label,3|updated:keep:keep"
    );
}

#[test]
fn loose_array_search_and_diff_use_php8_string_number_comparisons() {
    let source = r#"<?php
        echo in_array(0, ['not numeric']), ':', in_array(0, ['0']), '|';
        echo array_search(0, ['not numeric', '0']), '|';
        echo implode(',', array_values(array_diff(['not numeric', '0'], [0]))), '|';
        echo implode(',', array_values(array_diff(['00', '0'], [0]))), ':';
        echo implode(',', array_values(array_intersect(['00', '0'], [0])));
        class ArrayDiffStringable { public function __toString(): string { return 'match'; } }
        echo '|', count(array_diff([new ArrayDiffStringable()], ['match'])), ':', count(array_intersect([new ArrayDiffStringable()], ['match']));
        echo ':', count(array_diff_uassoc([new ArrayDiffStringable()], ['match'], fn($left, $right) => $left <=> $right));
        echo ':', count(array_intersect_uassoc([new ArrayDiffStringable()], ['match'], fn($left, $right) => $left <=> $right));
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        ":1|1|not numeric|00:0|0:1:0:1"
    );
}

#[test]
fn strict_array_search_requires_nested_array_key_order() {
    let source = r#"<?php
        $ordered = ['first' => 'a', 'second' => 'b'];
        $reordered = ['second' => 'b', 'first' => 'a'];
        echo in_array($ordered, [$reordered], true), ':', in_array($ordered, [$ordered], true), ':';
        echo array_search($ordered, [$reordered], true) === false;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), ":1:1");
}

#[test]
fn strict_array_search_uses_identity_for_objects_generators_and_streams() {
    let source = r#"<?php
        $object = new stdClass();
        $otherObject = new stdClass();
        $generator = (function () { yield 1; })();
        $stream = fopen('php://temp', 'r+');
        $otherStream = fopen('php://temp', 'r+');
        echo in_array($object, [$object], true), ':', in_array($object, [$otherObject], true), ':';
        echo in_array($generator, [$generator], true), ':', in_array($stream, [$stream], true), ':';
        echo in_array($stream, [$otherStream], true);
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1::1:1:"
    );
}

#[test]
fn array_map_dispatches_array_query_and_transform_callbacks() {
    let source = r#"<?php
        $rows = [['a' => 1, 'b' => 2], ['x' => 3]];
        $flipped = array_map('array_flip', $rows);
        $merged = array_map('array_merge', $rows, [['c' => 4], ['y' => 5]]);
        $replaced = array_map('array_replace', $rows, [['a' => 9], ['x' => 8]]);
        $diff = array_map('array_diff', $rows, [['b' => 2], ['x' => 3]]);
        $combine = array_map('array_combine', [['left'], ['right']], [['L'], ['R']]);
        $keys = array_map('array_key_first', $rows);
        $values = array_map('array_first', $rows);
        $found = array_map('array_search', [1, 3], $rows);
        echo $flipped[0][1], ':', $flipped[1][3], '|';
        echo $merged[0]['c'], ':', $merged[1]['y'], '|';
        echo $replaced[0]['a'], ':', $replaced[1]['x'], '|';
        echo count($diff[0]), ':', count($diff[1]), '|';
        echo $combine[0]['left'], ':', $combine[1]['right'], '|';
        echo $keys[0], ':', $keys[1], ':', $values[0], ':', $values[1], '|';
        echo $found[0], ':', $found[1];
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "a:x|4:5|9:8|1:0|L:R|a:x:1:3|a:x"
    );
}

#[test]
fn array_map_forwards_diagnostics_from_array_utility_callbacks() {
    let source = r#"<?php
        $flipped = array_map('array_flip', [['valid' => 'key', 'invalid' => true]]);
        echo $flipped[0]['key'];
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "valid");
    assert_eq!(result.diagnostics.len(), 1);
    assert!(result.diagnostics[0]
        .message
        .contains("array_flip(): Can only flip STRING and INTEGER values"));
}

#[test]
fn array_map_unique_callback_uses_runtime_string_conversion() {
    let source = r#"<?php
        class UniqueCallbackValue {
            public string $value;
            public function __construct(string $value) { $this->value = $value; }
            public function __toString(): string { return $this->value; }
        }
        $values = [new UniqueCallbackValue('same'), new UniqueCallbackValue('same'), new UniqueCallbackValue('other')];
        $unique = array_map('array_unique', [$values]);
        echo count($unique[0]), ':', (string)$unique[0][0], ':', (string)$unique[0][2];
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "2:same:other"
    );
}

#[test]
fn array_map_count_callback_supports_arrays_and_countable_objects() {
    let source = r#"<?php
        class CountCallbackValue implements Countable {
            public function count(): int { return 7; }
        }
        echo implode(',', array_map('count', [[1, 2], [], new CountCallbackValue()]));
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2,0,7");
}

#[test]
fn array_map_count_values_callback_preserves_warnings_for_invalid_items() {
    let source = r#"<?php $counts = array_map('array_count_values', [[1, true, 2]]); echo count($counts[0]);"#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "2");
    assert_eq!(result.diagnostics.len(), 1);
    assert!(result.diagnostics[0].message.contains("array_count_values"));
}

#[test]
fn array_map_dispatches_array_column_callback_for_nested_records() {
    let source = r#"<?php
        class ArrayColumnCallbackRecord { public function __construct(public string $name) {} }
        $records = [[['name' => 'Ada'], ['name' => 'Lin']],[['name' => 'Sam']]];
        $columns = array_map('array_column', $records, ['name', 'name']);
        $objects = [[new ArrayColumnCallbackRecord('Jo'), new ArrayColumnCallbackRecord('Mo')]];
        $objectColumns = array_map('array_column', $objects, ['name']);
        echo $columns[0][1], ':', $columns[1][0], ':', $objectColumns[0][1];
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "Lin:Sam:Mo"
    );
}

#[test]
fn string_increment_carries_across_ascii_digits_and_letters() {
    let source = r#"<?php
        $lower = 'z';
        $upper = 'Z';
        $digits = '99';
        $mixed = 'a9';
        $items = ['y'];
        class StringSequence { public $value = 'z'; }
        $object = new StringSequence();
        echo $lower++, ':', $lower, ':', ++$upper, ':', ++$digits, ':', ++$mixed, ':', ++$items[0], ':', ++$object->value, "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "z:aa:AA:100:b0:z:aa\n");
}

#[test]
fn increment_operators_handle_numeric_empty_and_non_alphanumeric_strings() {
    let source = r#"<?php
        $integer = '5'; $integer++;
        $float = '1.5'; ++$float;
        $exponent = '5e0'; ++$exponent;
        $empty = ''; ++$empty;
        $punctuation = 'a-b'; ++$punctuation;
        $nonnumeric = 'hello'; $nonnumeric--;
        $empty_decrement = ''; $empty_decrement--;
        $boolean = true; $boolean++;
        echo gettype($integer), ':', $integer, ':', gettype($float), ':', $float, ':',
            gettype($exponent), ':', $exponent, ':', $empty, ':', $punctuation, ':', $nonnumeric, ':',
            $empty_decrement, ':', gettype($boolean), ':', $boolean, "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "integer:6:double:2.5:double:6:1:a-b:hello:-1:boolean:1\n"
    );
}

#[test]
fn incrementing_non_ascii_string_preserves_utf8_bytes_and_reports_nonnumeric_deprecation() {
    let source = "<?php $value = 'é'; ++$value; echo bin2hex($value);";
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "c3a9");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(
        result.diagnostics[0].level,
        php_runtime::DiagnosticLevel::Deprecated
    );
    assert!(result.diagnostics[0]
        .message
        .contains("Increment on non-numeric string is deprecated"));
}

#[test]
fn increment_deprecations_use_custom_error_handlers_and_handler_exceptions_are_catchable() {
    let source = r#"<?php
        $captured = [];
        set_error_handler(function ($severity, $message, $file, $line) use (&$captured) {
            $captured = [$severity, $message, basename($file), $line];
            return true;
        }, E_DEPRECATED);
        error_reporting(0);
        $value = 'word';
        ++$value;
        restore_error_handler();
        echo $captured[0], ':', str_contains($captured[1], 'Increment on non-numeric string'), ':',
            $captured[2], ':', $captured[3], ':', $value, ':';
        try {
            set_error_handler(function () { throw new RuntimeException('increment handler'); }, E_DEPRECATED);
            $other = 'text';
            ++$other;
        } catch (RuntimeException $error) {
            echo $error->getMessage();
        }
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(
        result.output,
        "8192:1:Command line code:9:wore:increment handler"
    );
    assert!(result.diagnostics.is_empty());
}

#[test]
fn increment_operators_apply_php_85_boolean_null_and_string_diagnostics() {
    let source = r#"<?php
        $boolean = true; $boolean++;
        $null_increment = null; ++$null_increment;
        $null_decrement = null; $null_decrement--;
        $empty_increment = ''; ++$empty_increment;
        $empty_decrement = ''; $empty_decrement--;
        $alpha = 'z'; ++$alpha;
        $word = 'hello'; $word--;
        echo $boolean, ':', $null_increment, ':', gettype($null_decrement), ':', $empty_increment, ':',
            $empty_decrement, ':', $alpha, ':', $word;
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "1:1:NULL:1:-1:aa:hello");
    assert_eq!(result.diagnostics.len(), 6);
    assert_eq!(
        result.diagnostics[0].level,
        php_runtime::DiagnosticLevel::Warning
    );
    assert_eq!(
        result.diagnostics[1].level,
        php_runtime::DiagnosticLevel::Warning
    );
    assert!(result.diagnostics[0]
        .message
        .contains("Increment on type bool"));
    assert!(result.diagnostics[1]
        .message
        .contains("Decrement on type null"));
    assert!(result.diagnostics[2]
        .message
        .contains("Increment on non-numeric string is deprecated"));
    assert!(result.diagnostics[3]
        .message
        .contains("Decrement on empty string is deprecated as non-numeric"));
    assert!(result.diagnostics[2..]
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Deprecated));
    assert_eq!(
        result
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.source_line)
            .collect::<Vec<_>>(),
        vec![Some(2), Some(4), Some(5), Some(6), Some(7), Some(8)]
    );

    let array_error = compile_source(
        "<?php $value = []; try { ++$value; } catch (TypeError $error) { echo get_class($error); }",
    )
    .unwrap()
    .execute()
    .unwrap();
    assert_eq!(array_error, "TypeError");
}

#[test]
fn error_reporting_masks_warning_and_deprecation_diagnostics() {
    let source = r#"<?php
        $initial = error_reporting();
        error_reporting(error_level: 0);
        $flag = true;
        $flag++;
        error_reporting(E_DEPRECATED);
        $text = '';
        $text--;
        error_reporting(E_WARNING);
        $flag++;
        $previous = error_reporting(E_ALL);
        echo $initial, ':', $previous, ':', E_WARNING, ':', E_DEPRECATED, ':', function_exists('error_reporting'), "\n";
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(std::collections::HashMap::new())
        .unwrap();
    assert_eq!(result.output, "32767:2:2:8192:1\n");
    assert_eq!(result.diagnostics.len(), 2);
    assert_eq!(
        result.diagnostics[0].level,
        php_runtime::DiagnosticLevel::Deprecated
    );
    assert_eq!(
        result.diagnostics[1].level,
        php_runtime::DiagnosticLevel::Warning
    );
}

#[test]
fn prefix_and_postfix_increment_support_array_paths_in_properties() {
    let source = r#"<?php
        class IncrementContainers {
            public $values;
            public static $shared;
        }
        $container = new IncrementContainers();
        $container->values = ['row' => ['z']];
        IncrementContainers::$shared = ['count' => '9'];
        echo $container->values['row'][0]++, ':', $container->values['row'][0], ':',
            ++$container->values['row'][0], ':', IncrementContainers::$shared['count']++, ':',
            IncrementContainers::$shared['count'], ':', ++IncrementContainers::$shared['count'], "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "z:aa:ab:9:10:11\n"
    );
}

#[test]
fn variadic_array_merge_binds_its_php_named_parameter() {
    let source = r#"<?php
        echo count(array_merge()), ':';
        echo array_merge(arrays: ['answer' => 42])['answer'], ':';
        echo array_merge_recursive(arrays: ['key' => ['value']])['key'][0], "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "0:42:value\n"
    );
}

#[test]
fn array_callback_diff_variadics_bind_named_callback_and_arrays() {
    let source = r#"<?php
        function compareValues($left, $right) { return $left <=> $right; }
        $keys = array_diff_ukey(key_compare_func: 'strcasecmp', arrays: ['a' => 2], array: ['A' => 1]);
        $values = array_udiff(value_compare_func: 'compareValues', arrays: ['left' => 3], array: ['right' => 3]);
        echo count($keys), ':', count($values), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "0:0\n");
}

#[test]
fn array_callback_diff_binds_both_value_and_key_comparators_by_name() {
    let source = r#"<?php
        function compareValues($left, $right) { return $left <=> $right; }
        $first = ['A' => 1, 'b' => 2]; $second = ['a' => 1];
        $common = array_uintersect_uassoc(key_compare_func: 'strcasecmp', arrays: $second,
            value_compare_func: 'compareValues', array: $first);
        $difference = array_udiff($first, $second, value_compare_func: 'compareValues');
        echo count($common), ':', array_key_first($common), ':', count($difference), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:A:1\n"
    );
}

#[test]
fn incrementing_undefined_variables_emits_php_warning_with_source_location() {
    let source = "<?php\n$before = $missing++;\n++$otherMissing;\necho $before, ':', $missing, ':', $otherMissing;";
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, ":1:1");
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
    assert_eq!(
        result
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Undefined variable $missing",
            "Undefined variable $otherMissing"
        ]
    );
    assert_eq!(
        result
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.source_line)
            .collect::<Vec<_>>(),
        vec![Some(2), Some(3)]
    );
}

#[test]
fn incrementing_missing_array_keys_emits_source_located_warning() {
    let source = "<?php\n$values = [];\n$previous = $values['missing']++;\n++$values[4];\necho gettype($previous), ':', $values['missing'], ':', $values[4];";
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "NULL:1:1");
    assert_eq!(result.diagnostics.len(), 2);
    assert_eq!(
        result
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>(),
        vec!["Undefined array key \"missing\"", "Undefined array key 4"]
    );
    assert_eq!(
        result
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.source_line)
            .collect::<Vec<_>>(),
        vec![Some(3), Some(4)]
    );
}

#[test]
fn missing_array_key_reads_warn_but_isset_coalesce_and_empty_stay_silent() {
    let source = "<?php\n$values = [];\n$read = $values['missing'];\n$exists = isset($values['missing']);\n$fallback = $values['missing'] ?? 'fallback';\n$isEmpty = empty($values['missing']);\nerror_reporting(0);\n$suppressed = $values['missing'];\necho gettype($read), ':', $exists, ':', $fallback, ':', $isEmpty;";
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "NULL::fallback:1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(
        result.diagnostics[0].level,
        php_runtime::DiagnosticLevel::Warning
    );
    assert_eq!(
        result.diagnostics[0].message,
        "Undefined array key \"missing\""
    );
    assert_eq!(result.diagnostics[0].source_line, Some(3));
}

#[test]
fn incrementing_missing_property_array_keys_emits_php_warnings() {
    let source = r#"<?php
        class IncrementDiagnostics {
            public $values = [];
            public static $shared = [];
        }
        $object = new IncrementDiagnostics();
        $object->values['instance']++;
        ++IncrementDiagnostics::$shared['static'];
        echo $object->values['instance'], ':', IncrementDiagnostics::$shared['static'];
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "1:1");
    assert_eq!(result.diagnostics.len(), 2);
    assert_eq!(
        result
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Undefined array key \"instance\"",
            "Undefined array key \"static\""
        ]
    );
}

#[test]
fn pos_aliases_current_for_direct_named_and_callback_calls() {
    let source = r#"<?php
        $values = ['first' => 10, 'second' => 20];
        echo function_exists('pos'), ':', pos($values), ':', pos(array: $values), ':';
        echo implode(',', array_map('pos', [$values, ['only' => 7]])), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:10:10:10,7\n"
    );
}

#[test]
fn array_first_and_last_follow_insertion_order_without_moving_array_pointer() {
    let source = r#"<?php
        $values = [7 => 'first', 'middle' => 'next', -2 => 'last'];
        next($values);
        echo array_first(array: $values), ':', array_last($values), ':', current($values), ':';
        echo array_first([]) === null, ':', array_last([]) === null, ':';
        echo implode(',', array_map('array_first', [[], ['sparse' => 'one', 9 => 'two']])), ':';
        echo function_exists('array_first'), function_exists('array_last');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "first:last:next:1:1:,one:11"
    );
}
