<?php
// while repeats until its condition becomes false and supports continue/break.
$i = 0;
while ($i < 5) {
    $i++;
    if ($i == 2) {
        continue;
    }
    if ($i == 4) {
        break;
    }
    echo "while:", $i, "\n";
}

// do-while always executes its body once before checking the condition.
$j = 0;
do {
    $j++;
} while ($j < 2);
echo "do-while:", $j, "\n";

// for supports multiple initializer/update clauses and post-increment.
for ($k = 0; $k < 4; $k++) {
    if ($k == 1) {
        continue;
    }
    echo "for:", $k, "\n";
}

// foreach preserves array order and lets break stop iteration early.
foreach (["a", "b", "c"] as $index => $letter) {
    if ($index == 2) {
        break;
    }
    echo "foreach:", $letter, "\n";
}

// Nested loops keep break scoped to the innermost active loop.
for ($outer = 0; $outer < 2; $outer++) {
    for ($inner = 0; $inner < 3; $inner++) {
        if ($inner == 1) {
            break;
        }
        echo "nested:", $outer, ":", $inner, "\n";
    }
}

// Multiple comma-separated initializer and update clauses are supported.
for ($left = 0, $right = 2; $left < 2; $left++, $right--) {
    echo "pair:", $left, ":", $right, "\n";
}

// A numeric break level exits the requested number of nested loops.
for ($outer = 0; $outer < 3; $outer++) {
    for ($inner = 0; $inner < 3; $inner++) {
        if ($inner == 1) {
            break 2;
        }
        echo "break-level:", $outer, ":", $inner, "\n";
    }
}

// A numeric continue level skips the rest of each requested outer iteration.
$outer = 0;
while ($outer < 2) {
    $outer++;
    for ($inner = 0; $inner < 2; $inner++) {
        continue 2;
    }
}
echo "continue-level:", $outer, "\n";

// A variable can choose the nesting level at runtime.
$level = 2;
for ($outer = 0; $outer < 2; $outer++) {
    for ($inner = 0; $inner < 3; $inner++) {
        echo "computed-break:", $outer, ":", $inner, "\n";
        if ($inner == 1) {
            break $level;
        }
    }
}

// PHP loop bodies may also consist of one statement without braces.
$single = 0;
while ($single < 1) $single++;
echo "single-body:", $single, "\n";
