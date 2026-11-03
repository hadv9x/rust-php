<?php
$field = "color";
$color = "blue";
echo $$field;
echo "\n";

$$field = "green";
echo $color;
echo "\n";

echo ${$field};
echo "\n";
${$field} = "orange";
echo $color;
echo "\n";

$prefix = "co";
$suffix = "lor";
echo ${$prefix . $suffix};
echo "\n";

echo (${$field} += 2), ":", $color, "\n";
echo ($$field = "purple"), ":", $color, "\n";
echo (${$field} ??= "fallback"), ":", $color, "\n";
$missingField = "missing";
echo (${$missingField} ??= "created"), ":", $missing, "\n";
