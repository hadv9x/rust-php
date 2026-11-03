<?php
// Short-circuited right-hand expressions must not update the counter.
$calls = 0;
$andResult = false && ++$calls;
$orResult = true || ++$calls;
echo "short-circuit calls: ", $calls, "\n";

// `!` binds before comparisons, while && binds more tightly than ||.
$enabled = true;
$blocked = false;
if ($enabled && !$blocked || false) {
    echo "condition passed", "\n";
}

// Parentheses can make the intended grouping explicit.
$grouped = $enabled && ($blocked || !false);
if ($grouped) {
    echo "grouped condition passed", "\n";
}
