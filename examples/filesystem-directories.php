<?php
$root = tempnam(sys_get_temp_dir(), 'php-rust-directories');
unlink($root);
$nested = $root . '/first/second';
$callbackPath = $root . '/callback';

echo mkdir(directory: $nested, permissions: 0750, recursive: true), ':', is_dir($nested), "\n";
$created = array_map('mkdir', [$callbackPath]);
echo $created[0], ':', is_dir($callbackPath), "\n";

$removed = array_map('rmdir', [$nested, $root . '/first', $callbackPath, $root]);
echo implode('', array_map('intval', $removed)), ':', function_exists('mkdir'), function_exists('rmdir'), "\n";
