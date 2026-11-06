<?php
$first = hash_hkdf('sha1', 'input key material', 16, 'encryption');
$second = hash_hkdf('sha1', 'input key material', 16, 'authentication');
echo (hash_equals($first, $first) ? 'stable' : 'different'), "\n";
echo ($first !== $second ? 'separate keys' : 'same key'), "\n";
