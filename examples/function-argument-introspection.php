<?php
function inspectArguments($first, $second = "default") {
    echo func_num_args(), ":", func_get_arg(1), ":";
    $first = "updated";
    echo func_get_args()[0], "\n";
}
inspectArguments(first: "A", second: "B");
inspectArguments(second: "last", first: "start");

function inspectSkipped($first = "one", $second = "two", $third = "three") {
    echo func_num_args(), ":", implode("|", func_get_args()), "\n";
}
inspectSkipped(third: "custom");

function inspectVariadic($first, ...$rest) {
    echo func_num_args(), ":", count($rest), ":", implode("|", func_get_args()), "\n";
}
inspectVariadic("head", "middle", "tail");
inspectVariadic(first: "head", extra: "ignored-by-func-get-args");

function inspectByReference(&$value) {
    $value = "changed";
    echo func_get_args()[0], "\n";
}
$value = "before";
inspectByReference($value);

function inspectUndeclaredArguments() {
    echo func_num_args(), ":", func_get_arg(1), ":", implode("|", func_get_args()), "\n";
}
inspectUndeclaredArguments("first", "second", "third");

function nestedArguments($value) {
    $readOwnArguments = function ($inner) { echo func_num_args(), ":", func_get_arg(0), "\n"; };
    $readOwnArguments("inner", "extra");
    echo func_num_args(), ":", func_get_arg(0), "\n";
}
nestedArguments("outer");
