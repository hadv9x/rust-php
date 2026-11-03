<?php
function combineValues($left, $right) {
    return $left . $right;
}
echo call_user_func("combineValues", "A", "B"), "\n";
echo call_user_func("combineValues", right: "D", left: "C"), "\n";
echo call_user_func("strlen", "hello"), "\n";

$combine = function ($left, $right) { return $left . $right; };
echo call_user_func($combine, "E", "F"), "\n";

class UserFuncTarget {
    public static function combine($left, $right) { return $left . $right; }
    public function __invoke($value) { return "[" . $value . "]"; }
}
echo call_user_func("UserFuncTarget::combine", "G", "H"), "\n";
$target = new UserFuncTarget();
echo call_user_func($target, "I"), "\n";

function replaceByReference(&$value) { $value = "inside"; }
$value = "outside";
call_user_func("replaceByReference", $value);
echo $value, "\n";
