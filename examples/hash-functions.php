<?php
$checksums = array_map('crc32', ['', '123456789', 'The quick brown fox jumps over the lazy dog']);
echo $checksums[0], ':', $checksums[1], ':', $checksums[2], ':', function_exists('crc32'), "\n";
echo md5(''), ':', md5('abc'), ':', (md5('abc', binary: true) !== md5('abc')), ':', function_exists('md5'), "\n";
echo sha1(''), ':', sha1('abc'), ':', (sha1('abc', binary: true) !== sha1('abc')), ':', function_exists('sha1'), "\n";
$file = tempnam(sys_get_temp_dir(), 'hash-');
file_put_contents($file, "hash me");
echo md5_file(filename: $file), ':', function_exists('md5_file'), "\n";
unlink($file);
