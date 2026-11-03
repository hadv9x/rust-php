<?php
error_reporting(0);
$ignored = $missingForErrorGetLast;
$last = error_get_last();
echo $last['type'], ':', $last['message'], ':', $last['line'], "\n";
error_clear_last();
echo error_get_last() === null, "\n";
?>
