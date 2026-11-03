<?php
function greet($name) {
    return "Hello, " . $name;
}

function add($left, $right) {
    return $left + $right;
}

// Missing optional arguments are filled from their scalar defaults.
function formatName($name, $prefix = "Hello", $suffix = "!") {
    return $prefix . " " . $name . $suffix;
}

// Defaults may use null, booleans, numbers, strings, and negative numbers.
function optionSummary($enabled = true, $retries = -2, $label = null) {
    return $enabled . ":" . $retries . ":" . $label;
}

function factorial($n) {
    if ($n <= 1) {
        return 1;
    }
    return $n * factorial($n - 1);
}

echo greet("PHP"), "\n";
echo add(6, 7), "\n";
echo formatName("An"), "\n";
echo formatName("An", "Chao"), "\n";
echo optionSummary(), "\n";
echo factorial(6), "\n";
