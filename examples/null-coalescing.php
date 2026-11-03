<?php
$name = null;
echo $name ?? "guest";
echo "\n";

$enabled = false;
echo $enabled ?? true;
echo "\n";

$fallback = "unused";
echo "ready" ?? $fallback;
echo "\n";
