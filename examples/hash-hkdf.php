<?php
$first = hash_hkdf('sha1', 'input key material', 16, 'encryption');
$second = hash_hkdf('sha1', 'input key material', 16, 'authentication');
$sha256Key = hash_hkdf('sha256', 'input key material', 32, 'encryption');
echo (hash_equals($first, $first) ? 'stable' : 'different'), "\n";
echo ($first !== $second ? 'separate keys' : 'same key'), "\n";
echo strlen($sha256Key), "\n";
$sha224Key = hash_hkdf('sha224', 'input key material', 28, 'encryption');
echo strlen($sha224Key), "\n";
$sha384Key = hash_hkdf('sha384', 'input key material', 48, 'encryption');
echo strlen($sha384Key), "\n";
$sha512Key = hash_hkdf('sha512', 'input key material', 64, 'encryption');
echo strlen($sha512Key), "\n";
