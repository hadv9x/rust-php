<?php
function nextId() {
    static $id = 0;
    return ++$id;
}

function recursiveCalls($depth) {
    static $calls = 0;
    $calls++;
    if ($depth > 0) {
        recursiveCalls($depth - 1);
    }
    return $calls;
}

echo nextId(), " ", nextId(), "\n";
echo recursiveCalls(2), " ", recursiveCalls(0), "\n";

class Counter {
    public function next() {
        static $value = 0;
        return ++$value;
    }
}
$first = new Counter();
$second = new Counter();
echo $first->next(), " ", $second->next(), "\n";

$left = function () {
    static $value = 0;
    return ++$value;
};
$right = function () {
    static $value = 0;
    return ++$value;
};
echo $left(), " ", $left(), " ", $right(), "\n";

$initializerCalls = 0;
function createInitialValue($seed) {
    global $initializerCalls;
    $initializerCalls++;
    return $seed * 2;
}
function dynamicStaticInitializer($seed) {
    static $from_call = createInitialValue($seed);
    static $from_parameter = $seed + 10;
    return $from_call . ":" . $from_parameter;
}
echo dynamicStaticInitializer(3), " ", dynamicStaticInitializer(9), " ", $initializerCalls, "\n";

function recursiveStaticInitializer($depth) {
    static $value = $depth < 1 ? 7 : recursiveStaticInitializer($depth - 1) + 100;
    return $value;
}
echo recursiveStaticInitializer(2), " ", recursiveStaticInitializer(9), "\n";

$seededClosure = function ($seed) {
    static $cached = createInitialValue($seed);
    return $cached;
};
echo $seededClosure(5), ":", $seededClosure(9), ":", $initializerCalls, "\n";

class StaticInitializerFactory {
    public $seed;
    public function __construct($seed) { $this->seed = $seed; }
    public function cachedValue() {
        static $cached = createInitialValue($this->seed);
        return $cached;
    }
}
$factoryA = new StaticInitializerFactory(6);
$factoryB = new StaticInitializerFactory(20);
echo $factoryA->cachedValue(), " ", $factoryB->cachedValue(), ":", $initializerCalls, "\n";

$initializerAttempts = 0;
function mayFailDuringStaticInitialization($fail) {
    global $initializerAttempts;
    $initializerAttempts++;
    if ($fail) { throw new Exception("initializer failed"); }
    return 41;
}
function retryStaticInitializer($fail) {
    static $value = mayFailDuringStaticInitialization($fail);
    return $value;
}
try {
    retryStaticInitializer(true);
} catch (Throwable $error) {
    echo $error, "\n";
}
echo retryStaticInitializer(false), " ", retryStaticInitializer(true), " ", $initializerAttempts, "\n";
