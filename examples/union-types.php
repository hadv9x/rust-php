<?php
function preserveScalar(int|string $value): int|string { return $value; }
function numericValue(int|float $value): int|float { return $value; }

interface NumericValue { public function read(int $fallback): int|float; }
class StoredNumericValue implements NumericValue {
    public int|float $value;
    public function __construct(int|float $value) { $this->value = $value; }
    public function read(int|float $fallback): int|float { return $this->value; }
}

$closure = function (int|string $value): int|string { return $value; };
$record = new StoredNumericValue('4.5');
echo preserveScalar(7), ':', preserveScalar('text'), ':', numericValue('3.25'), ':';
echo $closure(8), ':', $record->read(0), "\n";
