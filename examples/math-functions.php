<?php
echo abs(-7), ':', ceil(1.2), ':', floor(1.8), ':';
echo round(1.235, 2), ':', round(1.25, 1, 3), ':', round(1.25, 1, 4), ':';
echo sqrt(9), ':', intdiv(-7, 3), ':', is_finite(1), ':', is_infinite(fdiv(1, 0)), ':', is_nan(sqrt(-1)), "\n";
echo log(num: 8, base: 2), ':', sin(deg2rad(30)), ':', hypot(3, 4), ':', fmod(8, 3), ':';
echo max(1, 9, 3), ':', min([-2, 4, 1]), "\n";
echo base_convert(num: 'ff', from_base: 16, to_base: 2), ':', decbin(num: 13), ':', dechex(num: 255), ':', decoct(num: 8), "\n";
echo number_format(num: 1234.5678, decimals: 2), ':', number_format(1234.5678, 2, ',', ' '), "\n";
echo implode(',', array_map('intval', ['ff', '101'], [16, 2])), "\n";
echo implode('|', str_getcsv('name,"value, with comma","say ""hi"""', escape: '')), "\n";
$legacyCsvFields = str_getcsv('legacy,default escape');
echo implode('|', $legacyCsvFields), "\n";
echo 'hex round trip: ', bin2hex(hex2bin(string: '4869')), "\n";
echo 'chr bytes: ', chr(codepoint: 65), ':', chr(833), "\n";
$letterCounts = count_chars('Mississippi', mode: 1);
echo 'count_chars: ', $letterCounts[ord('s')], ':', strlen(count_chars('Mississippi', 3)), "\n";

echo 'nl2br: ', nl2br("first\r\nsecond\nthird", use_xhtml: false), "\n";
echo 'nl2br callback: ', implode('|', array_map('nl2br', ["one\ntwo", "three\rfour"], [false, true])), "\n";
