<?php
$path = tempnam(sys_get_temp_dir(), 'php-chmod-');
$changed = chmod(filename: $path, permissions: 416);
$callback = array_map('chmod', [$path], [384])[0];
echo $changed, ':', (fileperms($path) & 511), ':', $callback, ':', function_exists('chmod'), "\n";
unlink($path);
