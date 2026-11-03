<?php
$value = "start";
function &sharedValue() { global $value; return $value; }
$alias =& sharedValue();
$alias = "linked";
echo $value, ":", $alias, "\n";
$copy = sharedValue();
$copy = "separate";
echo $value, ":", $copy, "\n";

$closure = function &() use (&$value) { return $value; };
$closureAlias =& $closure();
$closureAlias = "closure";
echo $value, "\n";

class ReferenceBox {
    public $item = "box";
    public function &itemReference() { return $this->item; }
    public function appendReference(&$value) { $value .= "!"; }
}
$box = new ReferenceBox();
$itemAlias =& $box->itemReference();
$itemAlias = "object";
echo $box->item, "\n";

$itemCallable = [$box, "itemReference"];
$callableAlias =& $itemCallable();
$callableAlias = "callable-array";
$literalAlias =& [$box, "itemReference"]();
$literalAlias = "callable-literal";
echo $box->item, "\n";
$appendCallable = [$box, "appendReference"];
$argument = "variable";
$appendCallable($argument);
$literalArgument = "literal";
[$box, "appendReference"]($literalArgument);
echo $argument, ":", $literalArgument, "\n";

class StaticReferenceSlots { public static $items = ['nested' => ['item' => 'before']]; }
function replaceStaticItem(&$value) { $value = 'after'; }
replaceStaticItem(StaticReferenceSlots::$items['nested']['item']);
$staticItemAlias =& StaticReferenceSlots::$items['nested']['item'];
$staticItemAlias .= '!';
echo StaticReferenceSlots::$items['nested']['item'], "\n";
