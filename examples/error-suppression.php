<?php
error_reporting(E_ALL);

set_error_handler(function ($severity) {
    echo 'handler:', $severity, ':', error_reporting(), "\n";
    return false;
});

@$missing;
echo 'restored:', error_reporting(), ':last=', error_get_last()['type'], "\n";
