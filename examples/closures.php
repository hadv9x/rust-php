<?php
// A closure captures selected variables by value when it is created.
$factor = 3;
$multiply = function ($value) use ($factor) {
    return $value * $factor;
};
echo $multiply(4), "\n";

$factor = 5;
$arrowMultiply = fn(int $value): int => $value * $factor;
echo $arrowMultiply(3), "\n";
$shadowed = fn($factor) => $factor + 1;
echo $shadowed(8), "\n";
$arrowMapper = fn($value) => $value + $factor;
$mapped = array_map($arrowMapper, [1, 2]);
echo $mapped[0], ":", $mapped[1], "\n";
$makeAdder = fn($base) => fn($value) => $base + $value;
$addSeven = $makeAdder(7);
echo $addSeven(4), "\n";
$staticArrow = static fn($value) => $value + $factor;
echo $staticArrow(2), "\n";
$staticClosure = static function ($value) use ($factor) {
    return $value * $factor;
};
echo $staticClosure(3), "\n";

// Mutating a by-value capture changes the closure's private state across calls.
$count = 0;
$next = function () use ($count) {
    $count = $count + 1;
    return $count;
};
echo $next(), ":", $next(), "\n";

// Closures can be passed directly to the array callback functions.
$mapped = array_map(function ($value) { return $value * 2; }, [1, 2, 3]);
$filtered = array_filter($mapped, function ($value) { return $value > 2; });
$total = array_reduce($filtered, function ($carry, $value) { return $carry + $value; }, 0);
echo $total, "\n";
