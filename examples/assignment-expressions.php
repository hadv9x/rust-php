<?php
class AssignmentBox {
    public $value;
    public $optional;
    public static $count;
    public static $optionalcount;
}

$value = 2;
echo ($value = 5);
echo " ";
echo $value;
echo "\n";

echo ($value += 3);
echo " ";
echo $value;
echo "\n";

$left = ($right = 4) + 1;
echo $left, " ", $right, "\n";

$settings = ["database" => ["port" => 8000, "timeout" => null]];
echo ($settings["database"]["port"] += 50), " ", $settings["database"]["port"], "\n";
echo ($settings["database"]["port"] = 9000), " ", $settings["database"]["port"], "\n";
echo ($settings["database"]["timeout"] ??= 15), " ", $settings["database"]["timeout"], "\n";
echo ($settings["database"]["timeout"] ??= 25), " ", $settings["database"]["timeout"], "\n";

$box = new AssignmentBox();
echo ($box->value = 10), " ", ($box->value += 5), " ", $box->value, "\n";
echo ($box->optional ??= 20), " ", ($box->optional ??= 30), " ", $box->optional, "\n";
echo (AssignmentBox::$count = 4), " ", (AssignmentBox::$count += 3), " ", AssignmentBox::$count, "\n";
echo (AssignmentBox::$optionalcount ??= 8), " ", (AssignmentBox::$optionalcount ??= 9), " ", AssignmentBox::$optionalcount, "\n";
