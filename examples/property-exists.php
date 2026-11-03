<?php
class PropertyBase {
    private $secretField;
    protected $nullableValue = null;
    public static $itemCount;
}
class PropertyRecord extends PropertyBase {
    public $pendingValue;
}

$record = new PropertyRecord();
$record->dynamicField = 'present';
echo property_exists($record, 'secretField'), ':', property_exists($record, 'nullableValue'), ':', property_exists($record, 'itemCount'), ':', property_exists($record, 'pendingValue'), ':', property_exists($record, 'dynamicField'), ':', property_exists('PropertyRecord', 'secretField'), ':', property_exists($record, 'absent'), "\n";
