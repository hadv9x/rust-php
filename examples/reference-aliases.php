<?php
$first = 1;
$second =& $first;
$second = 5;
echo $first, " ", $second, "\n";

function readValue($value) {
    echo $value, "\n";
}
function setAndRead(&$value) {
    $value = 9;
    readValue($value);
}
setAndRead($first);
echo $second, "\n";

$third = 20;
$second =& $third;
$third = 8;
echo $first, " ", $second, " ", $third, "\n";

function unsetAlias(&$value) {
    unset($value);
}
unsetAlias($first);
echo $first, "\n";

class RefMethods {
    public function update(&$value) {
        $value = 12;
    }
    public static function updateStatic(&$value) {
        $value = 14;
    }
}
$constructorValue = 1;
class RefConstructor {
    public function __construct(&$value) {
        $value = 10;
    }
}
new RefConstructor($constructorValue);
echo $constructorValue, "\n";
$methodValue = 4;
(new RefMethods())->update($methodValue);
echo $methodValue, "\n";
RefMethods::updateStatic($methodValue);
echo $methodValue, "\n";

$closure = function (&$value) { $value = 16; };
$closure($methodValue);
echo $methodValue, "\n";
$capturingClosure = function () use (&$methodValue) { $methodValue = 18; };
$capturingClosure();
echo $methodValue, "\n";

$globalA = 2;
$globalB = 3;
$globalA =& $globalB;
$globalA = 7;
echo $globalB, "\n";

$detached = 1;
$destination = 2;
$detached =& $destination;
unset($detached);
echo $detached ?? "unset", " ", $destination, "\n";

$referenceData = ["nested" => ["value" => 6]];
$outerKey = "nested";
$innerKey = "value";
$arrayAlias =& $referenceData[$outerKey][$innerKey];
$arrayAlias = 11;
echo $referenceData["nested"]["value"], " ";
$referenceData["nested"]["value"] = 17;
echo $arrayAlias, "\n";
$missingAlias =& $referenceData["new"]["leaf"];
$missingAlias = "created";
echo $referenceData["new"]["leaf"], "\n";

class AliasBox {
    public $value = "before";
    public function &valueReference() { return $this->value; }
}
$box = new AliasBox();
$propertyAlias =& $box->value;
$propertyAlias = "property";
echo $box->value, " ";
$callAlias =& $box->valueReference();
$callAlias = "returned";
echo $box->value, "\n";
