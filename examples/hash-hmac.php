<?php
$message = 'what do ya want for nothing?';
$key = 'Jefe';

echo hash_hmac('md5', $message, $key), "\n";
echo hash_hmac(algo: 'sha1', data: $message, key: $key), "\n";
echo hash_hmac('sha256', $message, $key), "\n";
echo hash_hmac('sha224', $message, $key), "\n";
echo hash_hmac('sha3-256', $message, $key), "\n";
echo hash_hmac('sha384', $message, $key), "\n";
echo hash_hmac('sha512', $message, $key), "\n";
echo hash_equals(hash_hmac('sha1', $message, $key), hash_hmac('sha1', $message, $key)) ? "match\n" : "mismatch\n";
