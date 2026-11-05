<?php
$lowercase = 'z';
$uppercase = 'Z';
$digits = '99';
$mixed = 'a9';

echo ++$lowercase, "\n"; // aa
echo ++$uppercase, "\n"; // AA
echo ++$digits, "\n";    // 100
echo ++$mixed, "\n";     // b0
// Numeric strings use numeric arithmetic; empty ++ produces "1", while
// non-numeric strings with punctuation remain unchanged by ++ and --.
$numeric = '5';
$empty = '';
$punctuation = 'a-b';
$punctuation++;
echo gettype($numeric++), ':', $numeric, ':', ++$empty, ':', $punctuation, "\n";
$unicode = 'é';
++$unicode; // Non-ASCII strings remain unchanged and follow nonnumeric-string diagnostics.
echo bin2hex($unicode), "\n";
echo str_increment('A9'), ':', str_decrement('AA'), "\n";
foreach ([fn() => str_increment(''), fn() => str_increment('a-b'), fn() => str_decrement('A')] as $operation) {
    try {
        $operation();
    } catch (ValueError $error) {
        echo get_class($error), "\n";
    }
}
