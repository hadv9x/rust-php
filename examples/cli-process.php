<?php

$pid = getmypid();
$callbackPid = call_user_func('getmypid');

echo 'process id: ', $pid, "\n";
echo 'callback returns process id: ', is_int($callbackPid) && $callbackPid > 0, "\n";
echo 'discoverable: ', function_exists('getmypid'), "\n";
