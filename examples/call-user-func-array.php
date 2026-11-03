<?php
function joinValues($left, $right) {
    return $left . $right;
}
$named = ["right" => "B", "left" => "A"];
echo call_user_func_array("joinValues", $named), "\n";
echo call_user_func_array("strlen", ["string" => "hello"]), "\n";

$closure = function ($left, $right) { return $left . $right; };
echo call_user_func_array($closure, ["left" => "C", "right" => "D"]), "\n";

class CallableExample {
    public function join($left, $right) { return $left . $right; }
    public function __invoke($value) { return "[" . $value . "]"; }
}
$callable = new CallableExample();
echo call_user_func_array([$callable, "join"], ["left" => "E", "right" => "F"]), "\n";
echo [$callable, "join"]("G", "H"), "\n";
echo call_user_func_array($callable, ["value" => "I"]), "\n";

function setByReference(&$value) { $value = "updated"; }
$value = "before";
call_user_func_array("setByReference", [&$value]);
echo $value, "\n";
