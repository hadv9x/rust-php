<?php
$calls = 0;
function nextMatchValue(&$calls) {
    $calls++;
    return 2;
}

echo match (nextMatchValue($calls)) {
    1 => 'one',
    2, 3 => 'two or three',
    default => 'other',
}, ':', $calls, "\n";

echo match ('2') {
    2 => 'integer',
    default => 'string',
}, "\n";

try {
    match (9) { 1 => 'unreachable' };
} catch (UnhandledMatchError $error) {
    echo get_class($error), ':', $error->getMessage(), "\n";
}
