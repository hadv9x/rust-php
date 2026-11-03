<?php
$samples = ['42', 42, [], false, null, function () {}, ' 1.25e2 ', '12cats'];
foreach ($samples as $sample) {
    echo is_string($sample), ':', is_array($sample), ':', is_bool($sample), ':', is_null($sample), ':';
    echo is_object($sample), ':', is_scalar($sample), ':', is_numeric($sample), "\n";
}
