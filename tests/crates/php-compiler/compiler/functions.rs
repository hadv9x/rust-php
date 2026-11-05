use php_compiler::compile_source;

#[test]
fn unpack_expands_positional_named_and_reference_arguments() {
    let source = r#"<?php
        function collect($first, ...$rest) { echo $first, ':', $rest['label'], ':', $rest[0], "\n"; }
        collect(...[0 => 'A', 1 => 'C', 'label' => 'B']);
        function update(&$value) { $value += 4; }
        $original = 3;
        $refs = [&$original];
        update(...$refs);
        echo $original, "\n";
        function combine($left, $right, $last) { return $left . $right . $last; }
        echo combine(...['x', 'y'], last: 'z'), "\n";
        $closure = function ($left, $right) { return $left . $right; };
        echo $closure(...['p', 'q']), "\n";
        class UnpackedCtor { public $value; function __construct($a, $b) { $this->value = $a . $b; } }
        echo (new UnpackedCtor(...['m', 'n']))->value, "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "A:B:C\n7\nxyz\npq\nmn\n");
}

#[test]
fn unpack_rejects_non_arrays() {
    let program = compile_source("<?php function take($value) {} take(...42);").unwrap();
    assert_eq!(program.execute().unwrap_err(), "Only arrays and Traversable objects can be unpacked");
}

#[test]
fn traversable_unpack_reports_php_duplicate_named_argument_error() {
    let source = r#"<?php
        function duplicate_names() { yield 'value' => 1; yield 'value' => 2; }
        function accept_value($value) { return $value; }
        accept_value(...duplicate_names());
    "#;
    let error = compile_source(source).unwrap().execute().unwrap_err();
    assert!(error.contains("Named parameter $value overwrites previous argument"), "{error}");
    let direct = compile_source("<?php function one($value) {} one(value: 1, value: 2);").unwrap().execute().unwrap_err();
    assert!(direct.contains("Named parameter $value overwrites previous argument"), "{direct}");
}

#[test]
fn traversable_unpack_passes_by_reference_parameters_by_value_with_warning() {
    let source = r#"<?php
        function overwrite(&$value) { $value = 99; echo $value, "\n"; }
        function generated_values() { yield 0 => 3; }
        overwrite(...generated_values());
    "#;
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "99\n");
    assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert!(result.diagnostics[0].message.contains("Cannot pass by-reference argument 1"), "{:?}", result.diagnostics[0]);
    assert!(result.diagnostics[0].message.contains("by unpacking a Traversable, passing by-value instead"), "{:?}", result.diagnostics[0]);

    let variadic = r#"<?php
        function overwrite_many(&...$values) { foreach ($values as &$value) { $value = 8; } }
        function generated_many() { yield 0 => 1; yield 1 => 2; }
        overwrite_many(...generated_many());
    "#;
    let result = compile_source(variadic).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.diagnostics.len(), 2, "{:?}", result.diagnostics);
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Warning));
}

#[test]
fn traversable_unpack_warns_for_builtin_array_reference_parameters() {
    let source = r#"<?php
        function generated_array_args() { yield 0 => [3, 1]; yield 1 => 2; }
        $pushed = array_push(...generated_array_args());
        function generated_sort_args() { yield 0 => [3, 1]; }
        $sorted = sort(...generated_sort_args());
        function generated_similarity_args() { yield 0 => 'abc'; yield 1 => 'abc'; yield 2 => 0; }
        $similar = similar_text(...generated_similarity_args());
        function generated_multisort_args() { yield 0 => [3, 1]; yield 1 => [30, 10]; }
        $multisorted = array_multisort(...generated_multisort_args());
        echo $pushed, ':', $sorted, ':', $similar, ':', $multisorted;
    "#;
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "3:1:3:1");
    assert_eq!(result.diagnostics.len(), 5, "{:?}", result.diagnostics);
    assert!(result.diagnostics.iter().all(|diagnostic| {
        diagnostic.level == php_runtime::DiagnosticLevel::Warning
            && diagnostic.message.contains("by unpacking a Traversable, passing by-value instead")
    }), "{:?}", result.diagnostics);
    assert!(result.diagnostics[0].message.contains("argument 1 of array_push "));
    assert!(result.diagnostics[1].message.contains("argument 1 of sort "));
    assert!(result.diagnostics[2].message.contains("argument 3 of similar_text "));
    assert!(result.diagnostics[3].message.contains("argument 1 of array_multisort "));
    assert!(result.diagnostics[4].message.contains("argument 2 of array_multisort "));
}

#[test]
fn traversable_unpack_warns_for_array_walk_source_and_still_invokes_callback() {
    let source = r#"<?php
        function bump_walk_value(&$value) { $value++; }
        function generated_walk_args() { yield 0 => [1, 2]; yield 1 => 'bump_walk_value'; }
        echo array_walk(...generated_walk_args());
    "#;
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "1");
    assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
    assert_eq!(result.diagnostics[0].level, php_runtime::DiagnosticLevel::Warning);
    assert!(result.diagnostics[0].message.contains("argument 1 of array_walk "));
    assert!(result.diagnostics[0].message.contains("by unpacking a Traversable, passing by-value instead"));
}

#[test]
fn function_argument_introspection_tracks_named_variadic_and_reference_values() {
    let source = r#"<?php
        function inspect($first = 'A', $second = 'B', $third = 'C') {
            echo func_num_args(), ':', implode('|', func_get_args()), "\n";
            $first = 'changed';
            echo func_get_arg(position: 0), "\n";
        }
        inspect(third: 'Z');
        function inspectVariadic($head, ...$tail) {
            echo func_num_args(), ':', count($tail), ':', implode('|', func_get_args()), "\n";
        }
        inspectVariadic(head: 'H', unknown: 'ignored');
        inspectVariadic('H', 'M', 'T');
        function inspectReference(&$value) { $value = 'updated'; echo func_get_args()[0], "\n"; }
        $value = 'before';
        inspectReference($value);
        function inspectExtraArguments() { echo func_num_args(), ':', func_get_arg(1), ':', implode('|', func_get_args()), "\n"; }
        inspectExtraArguments('first', 'second', 'third');
        $outer = function () {
            $inner = function ($value) { echo func_num_args(), ':', func_get_arg(0), "\n"; };
            $inner('inside', 'extra');
            echo func_num_args(), "\n";
        };
        $outer('outside');
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "3:A|B|Z\nchanged\n1:1:H\n3:2:H|M|T\nupdated\n3:second:first|second|third\n2:inside\n1\n");
    let invalid = compile_source("<?php get_defined_vars(unknown: true);").unwrap().execute().unwrap_err();
    assert!(invalid.contains("unknown named parameter '$unknown'"), "{invalid}");
}

#[test]
fn get_defined_vars_reports_only_the_active_scope() {
    let source = r#"<?php
        $outside = 'outer';
        function inspectScope($argument) {
            $local = 'inside';
            $vars = get_defined_vars();
            echo isset($vars['argument']) ? $vars['argument'] : 'missing', ':';
            echo isset($vars['local']) ? $vars['local'] : 'missing', ':';
            echo isset($vars['outside']) ? 'leaked' : 'isolated', "\n";
        }
        inspectScope('parameter');
        $vars = get_defined_vars();
        echo isset($vars['outside']) ? $vars['outside'] : 'missing', ':';
        echo isset($vars['argument']) ? 'leaked' : 'isolated', "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "parameter:inside:isolated\nouter:isolated\n");
}

#[test]
fn global_symbol_tables_preserve_variable_insertion_order() {
    let source = r#"<?php
        $zeta = 1;
        $alpha = 2;
        echo implode('|', array_slice(array_keys($GLOBALS), -2)), ":";
        echo implode('|', array_slice(array_keys(get_defined_vars()), -2)), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "zeta|alpha:zeta|alpha\n");
}

#[test]
fn removed_global_name_is_reinserted_at_the_end() {
    let source = r#"<?php
        $first = 1;
        $second = 2;
        unset($first);
        $first = 3;
        echo implode('|', array_slice(array_keys($GLOBALS), -2)), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "second|first\n");
}

#[test]
fn local_defined_vars_preserve_parameter_and_assignment_order() {
    let source = r#"<?php
        function inspect($zeta, $alpha) {
            $middle = 1;
            $tail = 2;
            unset($middle);
            $middle = 3;
            echo implode('|', array_keys(get_defined_vars())), "\n";
        }
        inspect(1, 2);
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "zeta|alpha|tail|middle\n");
}

#[test]
fn closure_defined_vars_preserve_capture_source_order() {
    let source = r#"<?php
        $first = 1;
        $second = 2;
        $inspect = function () use ($second, &$first) {
            echo implode('|', array_keys(get_defined_vars())), "\n";
        };
        $inspect();
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "second|first\n");
}

#[test]
fn internally_dispatched_function_callbacks_preserve_defined_variable_order() {
    let source = r#"<?php
        function inspect_callback_frame($zeta) {
            $alpha = 'assigned later';
            return implode('|', array_keys(get_defined_vars()));
        }
        echo array_map('inspect_callback_frame', [1])[0], "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "zeta|alpha\n");
}

#[test]
fn magic_method_frames_preserve_this_parameter_and_assignment_order() {
    let source = r#"<?php
        class MissingMethodOrder {
            public function __call($method, $arguments) {
                $alpha = true;
                echo implode('|', array_keys(get_defined_vars())), "\n";
            }
        }
        (new MissingMethodOrder())->unknown();
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "this|method|arguments|alpha\n");
}

#[test]
fn user_static_class_variables_remain_visible_without_shadowing_late_static_binding() {
    let source = r#"<?php
        $static_class = 'global';
        class StaticClassNameParent {
            public static function identify() {
                $static_class = 'local';
                echo implode('|', array_keys(get_defined_vars())), ':', $static_class, "\n";
                return get_called_class();
            }
        }
        class StaticClassNameChild extends StaticClassNameParent {}
        echo StaticClassNameChild::identify(), ':', implode('|', array_slice(array_keys(get_defined_vars()), -1)), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "static_class:local\nStaticClassNameChild:static_class\n");
}

#[test]
fn lifecycle_and_conversion_hooks_preserve_defined_variable_order() {
    let source = r#"<?php
        class DefinedVarsHooks implements JsonSerializable {
            public function __toString() { $alpha = 1; return implode('|', array_keys(get_defined_vars())); }
            public function __clone() { $alpha = 1; echo 'clone:', implode('|', array_keys(get_defined_vars())), "\n"; }
            public function __debugInfo() { $alpha = 1; echo 'debug:', implode('|', array_keys(get_defined_vars())), "\n"; return []; }
            public function jsonSerialize() { $alpha = 1; echo 'json:', implode('|', array_keys(get_defined_vars())), "\n"; return 1; }
            public function __destruct() { $alpha = 1; echo 'destroy:', implode('|', array_keys(get_defined_vars())), "\n"; }
        }
        $object = new DefinedVarsHooks();
        echo $object, "\n";
        $copy = clone $object;
        var_dump($object);
        echo json_encode($object), "\n";
        unset($object, $copy);
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output.matches("this|alpha\n").count(), 6, "{output}");
    assert!(output.contains("clone:this|alpha\n"), "{output}");
    assert!(output.contains("debug:this|alpha\n"), "{output}");
    assert!(output.contains("json:this|alpha\n"), "{output}");
    assert!(output.contains("destroy:this|alpha\n"), "{output}");
    assert!(!output.contains("static_class"), "{output}");
}

#[test]
fn instance_method_and_constructor_frames_place_this_before_parameters() {
    let source = r#"<?php
        class DefinedVarsMethodOrder {
            public function __construct($zeta) {
                $alpha = 1;
                echo 'construct:', implode('|', array_keys(get_defined_vars())), "\n";
            }
            public function inspect($zeta) {
                $alpha = 1;
                echo 'method:', implode('|', array_keys(get_defined_vars())), "\n";
            }
        }
        $object = new DefinedVarsMethodOrder(1);
        $object->inspect(2);
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "construct:this|zeta|alpha\nmethod:this|zeta|alpha\n");
}

#[test]
fn first_class_callables_cover_functions_methods_named_and_reference_arguments() {
    let source = r#"<?php
        function greet($name) { return 'hello ' . $name; }
        function increment(&$value) { $value += 1; }
        class Formatter {
            public function surround($value, $side = '!') { return $side . $value . $side; }
            public static function decorate($value) { return '[' . $value . ']'; }
        }
        class BaseFormatter {
            public static function label() { return 'base'; }
            public static function callable() { return static::label(...); }
        }
        class ChildFormatter extends BaseFormatter {
            public static function label() { return 'child'; }
        }
        $greet = greet(...);
        $length = strlen(...);
        echo gettype($greet), ':', get_debug_type($greet), ':', is_callable($greet) ? 'callable' : 'invalid', "\n";
        echo $greet('Ada'), ':', $length('four'), "\n";
        echo greet(...)('Lin'), "\n";
        echo implode(',', array_map($greet, ['A', 'B'])), "\n";
        $formatter = new Formatter();
        $surround = $formatter->surround(...);
        $decorate = Formatter::decorate(...);
        echo $surround(value: 'PHP', side: '*'), ':', $decorate('Rust'), "\n";
        $incrementValue = increment(...);
        $value = 40;
        $incrementValue($value);
        echo $value, "\n";
        $lateStatic = ChildFormatter::callable();
        echo $lateStatic(), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "object:Closure:callable\nhello Ada:4\nhello Lin\nhello A,hello B\n*PHP*:[Rust]\n41\nchild\n");
    let invalid = compile_source("<?php $missing = noSuchFunction(...);").unwrap().execute().unwrap_err();
    assert!(invalid.contains("Failed to create closure from callable"));
}

#[test]
fn first_class_callable_arrays_preserve_instance_static_and_reference_targets() {
    let source = r#"<?php
        class CallableArrayTarget {
            public function add(&$value, $amount = 1) { $value += $amount; }
            public static function label($value) { return '[' . $value . ']'; }
        }
        $target = new CallableArrayTarget();
        $instanceCallable = [$target, 'add'](...);
        $value = 4;
        $instanceCallable($value, amount: 3);
        $staticCallable = ['CallableArrayTarget', 'label'](...);
        echo $value, ':', $staticCallable(value: 'ok'), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "7:[ok]\n");
}
