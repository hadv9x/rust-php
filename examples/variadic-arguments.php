<?php
function collect($first, ...$rest) {
    echo $first, ":", count($rest), ":", $rest[0], ":", $rest[1], "\n";
}
collect("items", "one", "two");

function sumValues(int ...$values): int {
    return array_sum($values);
}
echo sumValues(2, 3, 5), "\n";
echo sumValues(), "\n";

function withDefault($prefix = "p", ...$values) {
    echo $prefix, ":", count($values), "\n";
}
withDefault();
withDefault("values", 1, 2);

class VariadicMethods {
    public static function countItems(...$items) {
        return count($items);
    }
}
echo VariadicMethods::countItems("a", "b", "c"), "\n";

interface AcceptsMany {
    public function countItems(...$items);
}
class VariadicImplementation implements AcceptsMany {
    public function countItems(...$items) { return count($items); }
}
echo (new VariadicImplementation())->countItems(1, 2, 3, 4), "\n";

$countItems = function (...$items) { return count($items); };
echo $countItems(4, 5), "\n";

function increaseReferences(&...$values) {
    foreach ($values as &$value) {
        $value++;
    }
}
$first = 10;
$second = 20;
$bucket = ["count" => 30];
increaseReferences($first, $second, $bucket["count"]);
echo $first, ":", $second, ":", $bucket["count"], "\n";

$increaseClosure = function (&...$values) {
    foreach ($values as &$value) {
        $value += 2;
    }
};
$third = 40;
$fourth = 50;
$increaseClosure(left: $third, right: $fourth);
echo $third, ":", $fourth, "\n";

function describeThree($first, $second, $third) {
    return $first . ":" . $second . ":" . $third;
}
$positional = ["left", "middle", "right"];
echo describeThree(...$positional), "\n";
echo describeThree(...["one", "two"], third: "three"), "\n";

function collectNamedUnpack($first, ...$rest) {
    echo $first, ":", $rest["second"], ":", $rest["third"], "\n";
}
$named = ["third" => "C", "second" => "B"];
collectNamedUnpack("A", ...$named);

function updateUnpacked(&$value) { $value += 5; }
$source = 7;
$references = [&$source];
updateUnpacked(...$references);
echo $source, "\n";

class UnpackCalls {
    public function join($left, $right) { return $left . $right; }
    public static function build($left, $right) { return $left . $right; }
}
$pair = ["x", "y"];
$unpackCalls = new UnpackCalls();
echo $unpackCalls->join(...$pair), ":", UnpackCalls::build(...$pair), "\n";
$joinClosure = function ($left, $right) { return $left . $right; };
echo $joinClosure(...$pair), "\n";

class UnpackConstructor {
    public $value;
    public function __construct($left, $right) { $this->value = $left . $right; }
}
$constructed = new UnpackConstructor(...$pair);
echo $constructed->value, "\n";
