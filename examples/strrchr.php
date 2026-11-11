<?php
$text = 'archive.tar.gz';
echo strrchr($text, '.'), "\n";
echo strrchr(haystack: 'one.two.three', needle: '.', before_needle: true), "\n";
echo strrchr('a.b', 'missing') === false, "\n";
echo implode(',', array_map('strrchr', ['a.b', 'x.y'], ['.', '.'])), "\n";
