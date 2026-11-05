<?php
$path = getenv(name: 'PATH', local_only: true);
$environment = getenv();
$mapped = array_map('getenv', ['PATH']);
$localName = 'RUST_PHP_COMPAT_EXAMPLE_LOCAL_ENV';
$setLocal = putenv($localName . '=temporary-value');
$localValue = getenv($localName, true);
$allWithLocal = getenv();
$removeLocal = putenv($localName);
$missing = getenv('RUST_PHP_COMPAT_TEST_VARIABLE_THAT_IS_NOT_SET');

echo 'PATH available: ', is_string($path) && strlen($path) > 0, "\n";
echo 'Environment listed: ', array_key_exists('PATH', $environment), "\n";
echo 'Callback agrees: ', $mapped[0] === getenv('PATH'), "\n";
echo 'Request local putenv: ', $setLocal && $localValue === 'temporary-value', ':', array_key_exists($localName, $allWithLocal), ':', $removeLocal, "\n";
echo 'Missing value is false: ', $missing === false, "\n";
echo 'getenv exists: ', function_exists('getenv'), "\n";
