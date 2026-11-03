<?php
// Switch compares values loosely and runs later cases until a break is reached.
$status = 2;
switch ($status) {
    case 1:
        echo "new", "\n";
        break;
    case 2:
        echo "in progress", "\n";
    case 3:
        echo "active", "\n";
        break;
    default:
        echo "unknown", "\n";
}

// A switch nested in a loop lets break 2 exit both constructs.
for ($i = 0; $i < 3; $i++) {
    switch ($i) {
        case 1:
            echo "stop at ", $i, "\n";
            break 2;
        default:
            echo "continue at ", $i, "\n";
    }
}
