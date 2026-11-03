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
