<?php
$path = __FILE__;

echo 'exists: ', file_exists($path), "\n";
echo 'size: ', filesize($path), "\n";
echo 'modified timestamp available: ', filemtime(filename: $path) > 0, "\n";
echo 'access timestamp available: ', fileatime(filename: $path) > 0, "\n";
echo 'callback timestamp available: ', array_map('filemtime', [$path])[0] > 0, "\n";
