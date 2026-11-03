<?php
class CompoundCounter {
    public int $count = 4;
    public $optional;
}
class MagicCompoundCounter {
    private $count = 2;
    private $optional;
    public function __get($name) {
        if ($name == "optional") { return $this->optional; }
        return $this->count;
    }
    public function __set($name, $value) {
        if ($name == "optional") { $this->optional = $value; }
        else { $this->count = $value; }
    }
}
class StaticCompoundCounter {
    public static int $count = 3;
    public static $optional;
    public static int $initialized;
}

$total = 10;
$total += 5;
$total -= 3;
$total *= 2;
$total /= 4;
echo $total;
echo "\n";

$label = "PHP";
$label .= " in Rust";
echo $label;
echo "\n";

$key_calls = 0;
$fallback_calls = 0;
function propertyFallback() {
    global $fallback_calls;
    $fallback_calls++;
    return 99;
}
function compoundKey() {
    global $key_calls;
    $key_calls++;
    return "count";
}
$stats = ["nested" => ["count" => 4]];
$stats["nested"][compoundKey()] += 3;
$stats["nested"]["count"] .= "!";
echo $stats["nested"]["count"], ":", $key_calls, "\n";
$sequence = ["nested" => ["count" => 7]];
$previous = $sequence["nested"][compoundKey()]++;
$next = ++$sequence["nested"][compoundKey()];
echo $previous, ":", $next, ":", $sequence["nested"]["count"], ":", $key_calls, "\n";
$counter = new CompoundCounter();
$counter->count += 3;
$magic = new MagicCompoundCounter();
$magic->count *= 4;
$counter->optional ??= 5;
$counter->optional ??= propertyFallback();
$magic->optional ??= 9;
echo $counter->count, ":", $magic->count, ":", $counter->optional, ":", $magic->optional, ":", $fallback_calls, "\n";
StaticCompoundCounter::$count += 4;
StaticCompoundCounter::$count *= 2;
StaticCompoundCounter::$optional ??= 11;
StaticCompoundCounter::$optional ??= propertyFallback();
StaticCompoundCounter::$initialized ??= 5;
echo StaticCompoundCounter::$count, ":", StaticCompoundCounter::$optional, ":", StaticCompoundCounter::$initialized, ":", $fallback_calls, "\n";
