<?php
// A by-reference parameter writes its final value back to the caller's variable.
function addTo(int &$value, int $amount = 1) {
    $value = $value + $amount;
}

// Both supported parameter spellings accept an ordinary variable argument.
function resetValue(&$value) {
    $value = 0;
}

$total = 10;
addTo($total, 5);
echo $total, "\n";
resetValue($total);
echo $total, "\n";

class DynamicPathReferenceBox {
    public $values = ['item' => 'before'];
}
$box = new DynamicPathReferenceBox();
$property = 'values';
$shared = 'linked';
$box->{$property}['item'] =& $shared;
$shared = 'changed through variable';
echo $box->values['item'], "\n";
$box->values['item'] = 'changed through property';
echo $shared, "\n";

class PrivatePathReferenceBox {
    private $values = ['item' => 'private'];
    public function bind(&$value) { $this->values['item'] =& $value; }
    public function read() { return $this->values['item']; }
}
$privateBox = new PrivatePathReferenceBox();
$privateValue = 'linked privately';
$privateBox->bind($privateValue);
$privateValue = 'updated privately';
echo $privateBox->read(), "\n";

class TypedPathReferenceBox {
    public array $values = ['item' => 'before'];
    public function bind(&$value) { $this->values['item'] =& $value; }
}
$typedBox = new TypedPathReferenceBox();
$typedValue = 'shared';
$typedBox->bind($typedValue);
echo $typedBox->values['item'], "\n";

class ReadonlyPathReferenceBox {
    public readonly array $values;
    public function __construct() { $this->values = ['item' => 'before']; }
}
$readonlyBox = new ReadonlyPathReferenceBox();
try { $readonlyBox->values['item'] =& $typedValue; }
catch (Error $error) { echo get_class($error), "\n"; }

// A regular read warns; null coalescing returns its fallback without warning.
echo $undefinedValue, "\n";
echo ($optionalValue ?? 'fallback'), "\n";
$dynamicReferenceName = 'missingDynamicValue';
echo $$dynamicReferenceName, "\n";
echo ($$dynamicReferenceName ?? 'dynamic fallback'), "\n";
echo isset($$dynamicReferenceName) ? "set\n" : "not set\n";
echo empty($undefinedEmptyValue) ? "empty\n" : "not empty\n";

$missingReadArray = [];
$missingRead = $missingReadArray['missing'];
echo 'missing array read: ', gettype($missingRead), "\n";
echo 'silent missing checks: ', isset($missingReadArray['missing']), ':', empty($missingReadArray['missing']), ':', ($missingReadArray['missing'] ?? 'fallback'), "\n";
