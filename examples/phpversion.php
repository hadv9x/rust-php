<?php
$versions = array_map('phpversion', [null, 'Core', 'PDO', 'json', 'not-an-extension']);

echo PHP_VERSION, ':', phpversion(), ':', phpversion(extension: 'core'), "\n";
echo implode(':', $versions), ':', function_exists('phpversion'), "\n";
