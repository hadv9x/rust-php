<?php
function runtimeGreeting() {}
interface CallableContract { public function run(); }
class CallableExample implements CallableContract {
    public function run() {}
    public static function build() {}
    private function hidden() {}
    public function __invoke() {}
}
function flag($value) { return $value ? "yes" : "no"; }

$object = new CallableExample();
echo flag(function_exists("runtimeGreeting")), ",", flag(function_exists("strlen")), ",", flag(function_exists("missingFunction")), "\n";
echo flag(class_exists("CallableExample")), ",", flag(class_exists("CallableContract")), ",", flag(interface_exists("CallableContract")), "\n";
echo flag(method_exists($object, "run")), ",", flag(method_exists("CallableExample", "hidden")), ",", flag(method_exists("CallableExample", "missing")), "\n";
$callableName = "";
echo flag(is_callable([$object, "run"], false, $callableName)), ":", $callableName, "\n";
echo flag(is_callable(["CallableExample", "build"])), ",", flag(is_callable([$object, "hidden"])), "\n";
echo flag(is_callable($object)), ",", flag(is_callable(function () {})), "\n";
