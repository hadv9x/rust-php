<?php
$path = tempnam(sys_get_temp_dir(), 'php-touch-');
$changed = touch(filename: $path, mtime: 123456789, atime: 234567890);
$callback = array_map('touch', [$path])[0];
echo $changed, ':', $callback, ':', file_exists($path), ':', function_exists('touch'), "\n";
unlink($path);
