<?php
$manifest = 'Cargo.toml';
echo file_get_contents(filename: $manifest, offset: 0, length: 10), ':';
echo file_exists(filename: $manifest), ':', is_file($manifest), ':', is_dir('crates'), ':', is_link($manifest), ':', is_readable(filename: $manifest), ':';
echo filesize($manifest) > 0, ':', realpath($manifest) !== false, ':', file_exists('missing-file.php'), "\n";
echo is_writable(filename: 'missing-file.php'), ':', is_executable(filename: $manifest), ':', strlen(getcwd()) > 0, ':', function_exists('is_executable'), "\n";
$stream = fopen(filename: $manifest, mode: 'rb');
echo stream_get_contents($stream, 10), ':', fclose($stream), ':', file_exists($manifest), "\n";
$temporary = tempnam(directory: sys_get_temp_dir(), prefix: 'php-rust-example');
echo file_put_contents(filename: $temporary, data: ["temporary\n", "\n", 'data']), ':', file_get_contents($temporary), ':';
$previousIncludePath = set_include_path(dirname($temporary));
echo get_include_path(), ':', file_get_contents(filename: basename($temporary), use_include_path: true), ':';
set_include_path($previousIncludePath);
echo implode('|', file(filename: $temporary, flags: FILE_IGNORE_NEW_LINES | FILE_SKIP_EMPTY_LINES)), ':', unlink($temporary), "\n";
?>
