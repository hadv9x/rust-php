<?php
declare(strict_types=1);

function formatCount(int $count): string {
    return 'count=' . $count;
}
function strictIntegerResult(): int { return '8'; }
function strictFloatResult(): float { return 8; }

class StrictCounter {
    public function __construct(public int $count) {}
    public function add(int $amount): int { return $this->count + $amount; }
    public static function asFloat(float $value): string { return gettype($value); }
}

echo formatCount(3), ':', StrictCounter::asFloat(4), "\n";
echo gettype(strictFloatResult()), "\n";
$counter = new StrictCounter(5);
echo $counter->add(2), "\n";

try {
    formatCount('6');
} catch (TypeError $error) {
    echo get_class($error), "\n";
}
try {
    strictIntegerResult();
} catch (TypeError $error) {
    echo get_class($error), "\n";
}
