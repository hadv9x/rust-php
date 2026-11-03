<?php
class Counter {
    public $value = 4;
    public $labels;
    public static $total = 1;
    public static $counts;
}

$counter = new Counter();
$counter->labels = ['row' => ['z']];
Counter::$counts = ['items' => '9'];
echo $counter->value++, ":", $counter->value, ":", ++$counter->value, ":", $counter->value--, ":", $counter->value, "\n";
$increment = fn() => $counter->value++;
echo $increment(), ":", $counter->value, "\n";
echo Counter::$total++, ":", ++Counter::$total, ":", Counter::$total, "\n";
echo $counter->labels['row'][0]++, ":", $counter->labels['row'][0], ":", ++$counter->labels['row'][0], "\n";
echo Counter::$counts['items']++, ":", Counter::$counts['items'], ":", ++Counter::$counts['items'], "\n";
$class = 'Counter';
$totalProperty = 'total';
$countsProperty = 'counts';
echo ++$class::$totalProperty, ":", $class::$totalProperty++, ":", ++$class::$countsProperty['items'], ":", $class::$countsProperty['items'], "\n";
