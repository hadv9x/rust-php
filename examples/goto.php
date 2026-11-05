<?php
$attempts = 0;
start:
try {
    $attempts++;
    if ($attempts < 2) {
        goto start;
    }
    goto done;
} finally {
    echo 'finally ', $attempts, "\n";
}

done:
echo 'done', "\n";
