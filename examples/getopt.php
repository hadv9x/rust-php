<?php
$rest_index = null;
$options = getopt('v::o:', ['verbose', 'output:'], $rest_index);
echo function_exists('getopt'), ':', count($options), ':', $rest_index, "\n";
?>
