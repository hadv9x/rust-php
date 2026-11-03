<?php
$integer = 12;
$float = 12.0;
$sum = 10 + 5;
$quotient = 10 / 5;
$decoded = json_decode('[7,7.0]', true);
function showCoercedNumbers(int $whole, float $decimal) {
    echo gettype($whole), ":", $whole, ":", gettype($decimal), ":", $decimal, "\n";
}
class NumericArgumentView {
    public function show(int $value) { return gettype($value) . ":" . $value; }
}
$showDecimal = function (float $value) { return gettype($value) . ":" . $value; };
echo gettype($integer), ":", gettype($float), ":", gettype($sum), ":", gettype($quotient), "\n";
echo is_int($integer), ":", is_int($float), ":", is_float($float), ":", get_debug_type($decoded[0]), ":", get_debug_type($decoded[1]), "\n";
echo implode(":", array_map("gettype", [1, 1.0, 1e0])), "\n";
showCoercedNumbers("7.8", 4);
echo (new NumericArgumentView())->show("9.4"), ":", $showDecimal(2), "\n";
