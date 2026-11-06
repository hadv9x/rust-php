<?php
error_reporting(0);
$ignored = $missingForErrorGetLast;
$last = error_get_last();
echo $last['type'], ':', $last['message'], ':', $last['line'], "\n";
error_clear_last();
echo error_get_last() === null, "\n";
trigger_error('User notice', E_USER_NOTICE);
$last = error_get_last();
error_reporting(0);
trigger_error(message: 'User warning', error_level: E_USER_WARNING);
echo $last['type'], ':', $last['message'], ':', error_get_last()['type'], "\n";
set_error_handler(function ($severity, $message, $file, $line) {
    echo "handled:", $severity, ':', $message, ':', $line, "\n";
    return true;
}, E_USER_WARNING);
trigger_error('Captured by a custom handler', E_USER_WARNING);
restore_error_handler();
user_error('The legacy alias also records the user error type.', E_USER_NOTICE);
?>
