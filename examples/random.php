<?php
$value = random_int(max: 10, min: 1);
echo ($value >= 1 && $value <= 10), "\n";
$legacy = mt_rand(max: 3);
$alias = rand(8, 10);
echo ($legacy >= 0 && $legacy <= 3), ':', ($alias >= 8 && $alias <= 10), ':', getrandmax() === mt_getrandmax(), "\n";
echo strlen(str_shuffle(string: 'php-runtime')), "\n";
?>
