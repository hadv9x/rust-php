<?php
$encoded = array_map('base64_encode', ['PHP', 'Rust']);
$decoded = base64_decode($encoded[0], strict: true);
echo $encoded[0], ':', $encoded[1], ':', $decoded, ':', function_exists('base64_decode'), "\n";
