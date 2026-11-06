<?php
$source = tempnam(sys_get_temp_dir(), 'php-rust-rename-src');
$target = tempnam(sys_get_temp_dir(), 'php-rust-rename-dst');
$callbackSource = tempnam(sys_get_temp_dir(), 'php-rust-rename-map-src');
$callbackTarget = tempnam(sys_get_temp_dir(), 'php-rust-rename-map-dst');

file_put_contents($source, 'renamed payload');
echo rename(from: $source, to: $target), ':', file_get_contents($target), "\n";

file_put_contents($callbackSource, 'callback payload');
$renamed = array_map('rename', [$callbackSource], [$callbackTarget]);
echo $renamed[0], ':', file_get_contents($callbackTarget), ':', function_exists('rename'), "\n";

unlink($source);
unlink($target);
unlink($callbackSource);
unlink($callbackTarget);
