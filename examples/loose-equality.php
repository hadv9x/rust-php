<?php
function flag($value) { return $value ? 'true' : 'false'; }

echo flag(0 == 'foo'), ':', flag(0 == '0'), ':', flag('01' == 1), "\n";
echo flag(null == ''), ':', flag(null == 0), ':', flag(true == 'anything'), "\n";
echo flag([1] == [1.0]), ':', flag(['a' => 1, 'b' => 2] == ['b' => 2, 'a' => 1]), "\n";
