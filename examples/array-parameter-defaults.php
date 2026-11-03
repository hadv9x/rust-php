<?php
function makeOptions($options = ['mode' => 'safe', 'flags' => [1, 2]]) {
    return $options;
}

$first = makeOptions();
$first['flags'][0] = 99;
$second = makeOptions();
echo $second['mode'], ':', $second['flags'][0], "\n";

$handler = function ($labels = ['created', 'updated']) {
    return $labels[1];
};
echo $handler(), "\n";

function defaultLabel($label = 'php' . '-' . 'runtime', $retries = 2 * 3) {
    echo $label, ':', $retries, "\n";
}
defaultLabel();

class DefaultConfig {
    public const MODE = 'stable';
}
function configured($mode = DefaultConfig::MODE) {
    echo $mode, "\n";
}
configured();
