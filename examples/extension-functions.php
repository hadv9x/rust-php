<?php
$jsonFunctions = get_extension_funcs('json');
$hashFunctions = get_extension_funcs('hash');
$byExtension = array_map('get_extension_funcs', ['json', 'hash', 'unknown']);

echo implode(', ', $jsonFunctions), "\n";
echo 'hash_equals available: ', in_array('hash_equals', $hashFunctions) ? 'yes' : 'no', "\n";
echo 'unknown extension: ', $byExtension[2] === false ? 'false' : 'listed', "\n";
