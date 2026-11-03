<?php
$base = [2 => "two", "name" => "Ada"];
$combined = [0 => "zero", ...$base, 4 => "four", ...["name" => "Grace", 6 => "six"]];

echo $combined[0], " ", $combined[1], " ", $combined["name"], " ", $combined[4], " ", $combined[5], "\n";
