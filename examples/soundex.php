<?php
$names = ['Euler', 'Ellery', 'Gauss', 'Ghosh', 'Hilbert', 'Heilbronn', 'Knuth', 'Kant'];
echo implode(',', array_map('soundex', $names)), "\n";
echo soundex(string: 'Lukasiewicz'), "\n";
echo soundex('!!!'), "\n";
echo 'available: ', function_exists('soundex'), "\n";
