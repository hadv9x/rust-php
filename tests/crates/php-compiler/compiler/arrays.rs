use php_compiler::compile_source;

#[test]
fn assignment_appends_at_multiple_levels_of_an_array_path() {
    let source = r#"<?php
        $body = [];
        $body['personalizations'][]['to'][]['email'] = 'ada@example.test';
        echo $body['personalizations'][0]['to'][0]['email'];
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ada@example.test");
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

    let invalid = compile_source("<?php array_filter([1], 'is_int', 3);").unwrap().execute().unwrap_err();
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
fn regular_array_sort_does_not_treat_unicode_whitespace_as_numeric_string_padding() {
    let source = r#"<?php
        $values = [" 2", '10', '2'];
        sort($values, SORT_REGULAR);
        echo implode('|', $values), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2|10|\u{00a0}2\n");
}

#[test]
fn regular_array_sort_compares_large_integer_strings_without_float_rounding() {
    let source = r#"<?php
        $values = ['9007199254740993', '9007199254740992'];
        sort($values, SORT_REGULAR);
        echo implode(',', $values), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "9007199254740992,9007199254740993\n");
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "A,a,z,İ\nZ1,z2,z10,İ2\n");
}

#[test]
fn regular_array_sort_compares_objects_by_class_and_properties() {
    let source = r#"<?php
        class SortableRecord { public $rank; public $label; function __construct($rank, $label) { $this->rank = $rank; $this->label = $label; } }
        $records = [new SortableRecord(2, 'b'), new SortableRecord(1, 'a'), new SortableRecord(2, 'a')];
        sort($records, SORT_REGULAR);
        echo $records[0]->rank, ':', $records[0]->label, ',', $records[1]->rank, ':', $records[1]->label, ',', $records[2]->rank, ':', $records[2]->label, "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:a,2:a,2:b\n");
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:2:1:5\n");
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
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "Error:Nesting level too deep - recursive dependency?\n");
}

#[test]
fn array_multisort_reports_catchable_value_and_type_errors() {
    let source = r#"<?php
        try { array_multisort([1, 2], [3]); }
        catch (ValueError $error) { echo get_class($error), ':'; }
        try { array_multisort('not an array'); }
        catch (TypeError $error) { echo get_class($error); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ValueError:TypeError");
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2:10:20:30:2\n");
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "20:10:2:2");
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2:12:11:11");
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
fn sorting_a_copied_array_separates_keys_but_keeps_element_references() {
    let source = r#"<?php
        $shared = 2;
        $original = ['named' => &$shared, 'other' => 1];
        $copy = $original;
        sort($copy, SORT_NUMERIC);
        $copy[1] = 9;
        echo array_key_exists('named', $original), ':', $original['other'], ':', $original['named'], ':', $shared, ':', implode(',', $copy), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:1:9:9:1,9\n");
}

#[test]
fn array_mutators_reject_literal_array_arguments_with_php_diagnostics() {
    for (source, expected) in [
        ("<?php array_push([], 1);", "array_push(): Argument #1 ($array) cannot be passed by reference"),
        ("<?php array_pop(array: []);", "array_pop(): Argument #1 ($array) cannot be passed by reference"),
        ("<?php sort(array: [2, 1]);", "sort(): Argument #1 ($array) cannot be passed by reference"),
    ] {
        let error = compile_source(source).unwrap().execute().unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
    let caught = compile_source(r#"<?php
        try { array_push([], 1); }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(); }
    "#).unwrap().execute().unwrap();
    assert_eq!(caught, "Error:array_push(): Argument #1 ($array) cannot be passed by reference");
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "lower|LOWER,ALSO LOWER\n");
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:1:1::1,1,:1:1:7:7:4:4:2:TypeError:TypeError:TypeError:ValueError");
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1,235,12|2,3|-1,1|fox,fox");
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "open:closed");
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
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.diagnostics.len(), 13);
    assert_eq!(result.output, "99:1|8:original:source");
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
    assert!(result.diagnostics[0].message.contains("array_map"));
    assert!(result.diagnostics[1].message.contains("array_filter"));
    assert!(result.diagnostics[2].message.contains("array_reduce"));
    assert!(result.diagnostics[3].message.contains("usort"));
    assert!(result.diagnostics[4].message.contains("array_map"));
    assert!(result.diagnostics[5].message.contains("array_walk"));
    assert!(result.diagnostics[4].message.contains("array_map"));
    assert!(result.diagnostics[5].message.contains("array_walk"));
    assert!(result.diagnostics[6].message.contains("array_udiff"));
    assert!(result.diagnostics[7].message.contains("array_udiff_uassoc"));
    assert!(result.diagnostics[8].message.contains("array_map"));
    assert!(result.diagnostics[9].message.contains("array_all"));
    assert!(result.diagnostics[10].message.contains("array_any"));
    assert!(result.diagnostics[11].message.contains("array_find"));
    assert!(result.diagnostics[12].message.contains("array_find_key"));
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2,a\na,2\n2,a\n");
}

#[test]
fn array_rand_uses_catchable_type_and_value_errors() {
    let source = r#"<?php
        try { array_rand([1], 0); } catch (ValueError $error) { echo get_class($error), ':'; }
        try { array_rand('not array'); } catch (TypeError $error) { echo get_class($error), ':'; }
        try { array_rand([1], 'nope'); } catch (TypeError $error) { echo get_class($error); }
        echo "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ValueError:TypeError:TypeError\n");
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
    assert_eq!(output, "AB\nNO\nPQ\n3\n5\nCD\nEF\nGH\n[I]\nJK\nLM\nchanged\nbefore\n");
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "4:6:6\n");
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
    assert_eq!(output, "3:5:5\nError:Cannot modify readonly property typedincrementtarget::$locked\n");
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "4:5:6:6\n8:9:10:10\n11:11\n");
}

#[test]
fn increment_operators_support_string_offset_lvalues() {
    let source = r#"<?php
        $text = 'Az';
        echo $text[0]++, ':', $text, ':', ++$text[1], ':', $text, ':', $text[-1], "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "A:Bz:aa:Ba:a\n");
}

#[test]
fn range_emits_php_83_boundary_and_character_step_warnings() {
    let source = r#"<?php
        echo count(range('', '2')), ':', count(range('ab', 'd')), ':';
        echo count(range(1, 'x')), ':', count(range('a', 'c', 1.0));
    "#;
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "3:4:2:3");
    assert_eq!(result.diagnostics.len(), 4);
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
    assert!(result.diagnostics[0].message.contains("must not be empty"));
    assert!(result.diagnostics[1].message.contains("must be a single byte, subsequent bytes are ignored"));
    assert!(result.diagnostics[2].message.contains("other boundary is numeric"));
    assert!(result.diagnostics[3].message.contains("generating an array of characters"));
    assert_eq!(result.diagnostics[0].source_line, Some(2));
    assert_eq!(result.diagnostics[1].source_line, Some(2));
    assert_eq!(result.diagnostics[2].source_line, Some(3));
    assert_eq!(result.diagnostics[3].source_line, Some(3));
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "integer:6:double:2.5:double:6:1:a-b:hello:-1:boolean:1\n");
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
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "1:1:NULL:1:-1:aa:hello");
    assert_eq!(result.diagnostics.len(), 6);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(result.diagnostics[1].level, php_runtime::DiagnosticLevel::Warning);
    assert!(result.diagnostics[0].message.contains("Increment on type bool"));
    assert!(result.diagnostics[1].message.contains("Decrement on type null"));
    assert!(result.diagnostics[2].message.contains("Increment on non-numeric string is deprecated"));
    assert!(result.diagnostics[3].message.contains("Decrement on empty string is deprecated as non-numeric"));
    assert!(result.diagnostics[2..].iter().all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Deprecated));
    assert_eq!(result.diagnostics.iter().map(|diagnostic| diagnostic.source_line).collect::<Vec<_>>(), vec![Some(2), Some(4), Some(5), Some(6), Some(7), Some(8)]);

    let array_error = compile_source("<?php $value = []; try { ++$value; } catch (TypeError $error) { echo get_class($error); }")
        .unwrap().execute().unwrap();
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
    let result = compile_source(source).unwrap().execute_with_globals(std::collections::HashMap::new()).unwrap();
    assert_eq!(result.output, "32767:2:2:8192:1\n");
    assert_eq!(result.diagnostics.len(), 2);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Deprecated);
    assert_eq!(result.diagnostics[1].level, php_runtime::DiagnosticLevel::Warning);
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "z:aa:ab:9:10:11\n");
}

#[test]
fn variadic_array_merge_binds_its_php_named_parameter() {
    let source = r#"<?php
        echo count(array_merge()), ':';
        echo array_merge(arrays: ['answer' => 42])['answer'], ':';
        echo array_merge_recursive(arrays: ['key' => ['value']])['key'][0], "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "0:42:value\n");
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
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:A:1\n");
}

#[test]
fn incrementing_undefined_variables_emits_php_warning_with_source_location() {
    let source = "<?php\n$before = $missing++;\n++$otherMissing;\necho $before, ':', $missing, ':', $otherMissing;";
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, ":1:1");
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
    assert_eq!(result.diagnostics.iter().map(|diagnostic| diagnostic.message.as_str()).collect::<Vec<_>>(), vec!["Undefined variable $missing", "Undefined variable $otherMissing"]);
    assert_eq!(result.diagnostics.iter().map(|diagnostic| diagnostic.source_line).collect::<Vec<_>>(), vec![Some(2), Some(3)]);
}

#[test]
fn incrementing_missing_array_keys_emits_source_located_warning() {
    let source = "<?php\n$values = [];\n$previous = $values['missing']++;\n++$values[4];\necho gettype($previous), ':', $values['missing'], ':', $values[4];";
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "NULL:1:1");
    assert_eq!(result.diagnostics.len(), 2);
    assert_eq!(result.diagnostics.iter().map(|diagnostic| diagnostic.message.as_str()).collect::<Vec<_>>(), vec!["Undefined array key \"missing\"", "Undefined array key 4"]);
    assert_eq!(result.diagnostics.iter().map(|diagnostic| diagnostic.source_line).collect::<Vec<_>>(), vec![Some(3), Some(4)]);
}

#[test]
fn missing_array_key_reads_warn_but_isset_coalesce_and_empty_stay_silent() {
    let source = "<?php\n$values = [];\n$read = $values['missing'];\n$exists = isset($values['missing']);\n$fallback = $values['missing'] ?? 'fallback';\n$isEmpty = empty($values['missing']);\nerror_reporting(0);\n$suppressed = $values['missing'];\necho gettype($read), ':', $exists, ':', $fallback, ':', $isEmpty;";
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "NULL::fallback:1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert_eq!(result.diagnostics[0].message, "Undefined array key \"missing\"");
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
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "1:1");
    assert_eq!(result.diagnostics.len(), 2);
    assert_eq!(result.diagnostics.iter().map(|diagnostic| diagnostic.message.as_str()).collect::<Vec<_>>(), vec!["Undefined array key \"instance\"", "Undefined array key \"static\""]);
}
