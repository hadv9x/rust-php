<?php
echo (int) '12.9tail', ':', (integer) '1e2', "\n";
echo (float) '2.5 meters', ':', (double) true, "\n";
echo (string) true, ':', (string) false, ':', (string) null, "\n";
echo (bool) '0', ':', (boolean) '0.0', ':', (binary) 42, ':', (real) 2, "\n";

class CastableValue {
    public function __toString(): string { return 'object text'; }
}
echo (string) new CastableValue(), "\n";

$single = (array) 7;
$record = (object) ['name' => 'Ada', 'role' => 'admin'];
$roundTrip = (array) $record;
echo $single[0], ':', get_class($record), ':', $roundTrip['role'], ':', get_class((object) null), "\n";
