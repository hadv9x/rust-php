<?php
$flags = 6;
echo ($flags & 3), ':', ($flags ^ 3), ':', ($flags | 3), "\n";

$flags &= 3;
$flags |= 8;
echo $flags, "\n";

echo ('a' & 'b'), ':', ('a' ^ 'b'), ':', ('a' | 'b'), "\n";
