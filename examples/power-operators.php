<?php
// Power is right-associative and binds more tightly than unary minus.
echo 2 ** 3 ** 2, ":", -2 ** 2, ":", (-2) ** 2, "\n";

$value = 3;
echo ($value **= 3), ":", $value, "\n";

$values = [2];
echo ($values[0] **= 4), ":", $values[0], "\n";

$matrix = [[2]];
echo ($matrix[0][0] **= 3), ":", $matrix[0][0], "\n";

class PowerBox {
    public $value = 2;
    public static $shared = 3;
}
$box = new PowerBox();
echo ($box->value **= 3), ":", $box->value, "\n";
echo (PowerBox::$shared **= 2), ":", PowerBox::$shared, "\n";

$dynamicName = "dynamicNumber";
$dynamicNumber = 4;
echo ($$dynamicName **= 2), ":", $dynamicNumber, "\n";
