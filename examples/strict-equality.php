<?php
function flag($value) { return $value ? 'true' : 'false'; }

echo flag(1 === 1), ':', flag(1 === 1.0), ':', flag('1' !== 1), "\n";
echo flag([1] === [1.0]), ':', flag(['a' => 1, 'b' => 2] === ['b' => 2, 'a' => 1]), "\n";

class IdentitySample {}
$same = new IdentitySample();
$alias = $same;
$copy = clone $same;
echo flag($same === $alias), ':', flag($same !== $copy), "\n";
