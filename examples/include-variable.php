<?php
$file = __DIR__ . '/hello.php';
include $file;
echo "Included path: ", basename($file), "\n";

$runtimeFile = dirname($_SERVER['SCRIPT_FILENAME']) . '/include-variable-runtime.php';
$runtimeResult = include $runtimeFile;
echo "Runtime include result: ", $runtimeResult, " (", $runtimeScopeValue, ")\n";
