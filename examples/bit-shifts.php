<?php
echo (3 << 2), ':', (16 >> 2), ':', (1 + 2 << 1), "\n";

$flags = 15;
$flags >>= 2;
$flags <<= 1;
echo $flags, ':', (~1), ':', (-8 >> 2), "\n";

echo (1 << 2 . 'x'), "\n"; // 4x: shift binds tighter than concatenation.
