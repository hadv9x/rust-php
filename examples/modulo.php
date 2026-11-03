<?php
echo 17 % 5, ":", -17 % 5, ":", 17 % -5, ":", -17 % -5, "\n";

$remaining = 17;
$remaining %= 5;
echo "remaining: ", $remaining, "\n";

echo "precedence: ", 17 % 5 * 2, "\n";
?>
