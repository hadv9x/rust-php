<?php
$source = tempnam(sys_get_temp_dir(), 'php-rust-copy-src');
$target = tempnam(sys_get_temp_dir(), 'php-rust-copy-dst');
$callbackTarget = tempnam(sys_get_temp_dir(), 'php-rust-copy-map');

file_put_contents($source, 'copy payload');
echo copy(from: $source, to: $target), ':', file_get_contents($target), "\n";

$copied = array_map('copy', [$source], [$callbackTarget]);
echo $copied[0], ':', file_get_contents($callbackTarget), ':', function_exists('copy'), "\n";

unlink($source);
unlink($target);
unlink($callbackTarget);
