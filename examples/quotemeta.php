<?php
$pattern = '.\\+*?[^]($)';
echo quotemeta(string: $pattern), "\n";
echo quotemeta('') === false, "\n";
echo implode(',', array_map('quotemeta', ['a.b', 'x|y', 'é.'])), "\n";
echo 'available: ', function_exists('quotemeta'), "\n";
