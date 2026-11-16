<?php
$phpFiles = glob('examples/*.php');
$selected = glob(pattern: 'examples/{arrays,functions}.php', flags: GLOB_BRACE);
$directories = glob('crates/*', GLOB_ONLYDIR | GLOB_MARK);

echo count($phpFiles) > 0, ':', basename($phpFiles[0]), ':';
echo implode(',', array_map('basename', $selected)), ':';
echo count($directories) > 0, ':', function_exists('glob'), ':', GLOB_AVAILABLE_FLAGS, "\n";
