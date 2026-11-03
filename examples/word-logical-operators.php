<?php
$assigned = false or true;
echo "assignment: ", $assigned, "\n";

$and_before_xor = true and false xor true;
$xor_before_or = true xor true or false;
$and_before_or = false or true and false;
echo "and/xor/or: ", $and_before_xor, ":", $xor_before_or, ":", $and_before_or, "\n";

$evaluated = 0;
false and ($evaluated = 1);
true or ($evaluated = 2);
echo "short circuit: ", $evaluated, "\n";
?>
