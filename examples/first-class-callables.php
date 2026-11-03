<?php
function greet($name) { return 'hello ' . $name; }
function increment(&$value) { $value += 1; }

class Formatter {
    public function surround($value, $side = '!') { return $side . $value . $side; }
    public static function decorate($value) { return '[' . $value . ']'; }
}
class ArrayCallableTarget {
    public function add(&$value, $amount = 1) { $value += $amount; }
    public static function label($value) { return '[' . $value . ']'; }
}

class BaseFormatter {
    public static function label() { return 'base'; }
    public static function callable() { return static::label(...); }
}
class ChildFormatter extends BaseFormatter {
    public static function label() { return 'child'; }
}

$greetCallable = greet(...);
$lengthCallable = strlen(...);
echo gettype($greetCallable), ':', get_debug_type($greetCallable), ':', is_callable($greetCallable) ? 'callable' : 'invalid', "\n";
echo $greetCallable('Ada'), ':', $lengthCallable('four'), "\n";
echo greet(...)('Lin'), "\n";
echo implode(',', array_map($greetCallable, ['A', 'B'])), "\n";

$formatter = new Formatter();
$surround = $formatter->surround(...);
$decorate = Formatter::decorate(...);
echo $surround(value: 'PHP', side: '*'), ':', $decorate('Rust'), "\n";

// A callable array can also use first-class callable syntax. By-reference
// parameters and named arguments keep their normal method-call behavior.
$arrayTarget = new ArrayCallableTarget();
$addCallable = [$arrayTarget, 'add'](...);
$number = 5;
$addCallable($number, amount: 2);
$labelCallable = ['ArrayCallableTarget', 'label'](...);
echo $number, ':', $labelCallable(value: 'ok'), "\n";

$incrementValue = increment(...);
$value = 40;
$incrementValue($value);
echo $value, "\n";

$lateStatic = ChildFormatter::callable();
echo $lateStatic(), "\n";
