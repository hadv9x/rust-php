<?php
$text = '<p>© à ß &amp; "quotes"</p>';
echo htmlentities($text, double_encode: false), "\n";
echo implode(',', array_map('htmlentities', ['©', '€'])), "\n";
