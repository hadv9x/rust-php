<?php
$jsonFunctions = get_extension_funcs('json');
$hashFunctions = get_extension_funcs('hash');
$dateFunctions = get_extension_funcs('date');
$byExtension = array_map('get_extension_funcs', ['json', 'hash', 'date', 'unknown']);

echo implode(', ', $jsonFunctions), "\n";
echo 'hash_equals available: ', in_array('hash_equals', $hashFunctions) ? 'yes' : 'no', "\n";
echo 'strtotime available: ', in_array('strtotime', $dateFunctions) ? 'yes' : 'no', "\n";
echo 'unknown extension: ', $byExtension[3] === false ? 'false' : 'listed', "\n";
