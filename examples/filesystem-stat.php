<?php
$path = 'examples/filesystem-stat.php';
$status = stat($path);
$linkStatus = lstat($path);

echo 'size: ', $status['size'], "\n";
echo 'numeric and named keys agree: ', $status[7] === $status['size'], "\n";
echo 'modification time: ', $status['mtime'], "\n";
echo 'type and mode: ', filetype($path), ':', decoct(fileperms($path) & 0777), "\n";
echo 'owner and group available: ', fileowner($path) >= 0, ':', filegroup($path) >= 0, "\n";
echo 'callback size: ', array_map('stat', [$path])[0]['size'], "\n";
echo 'lstat size: ', $linkStatus['size'], "\n";
