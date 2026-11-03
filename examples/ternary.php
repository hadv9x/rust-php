<?php
// The ternary expression evaluates only the branch selected by its condition.
$calls = 0;
$selected = true ? "chosen" : ++$calls;
echo $selected, ":", $calls, "\n";

// Ternary has lower precedence than logical operators and supports nesting.
$enabled = true;
$blocked = false;
$message = $enabled && !$blocked ? "ready" : "waiting";
$fallback = false ? "first" : true ? "second" : "third";
echo $message, ":", $fallback, "\n";
