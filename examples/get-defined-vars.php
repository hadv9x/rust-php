<?php
$outer = 'global';

function inspectScope($argument) {
    $local = 'function';
    $variables = get_defined_vars();
    echo isset($variables['argument']) ? $variables['argument'] : 'missing', ':';
    echo isset($variables['local']) ? $variables['local'] : 'missing', ':';
    echo isset($variables['outer']) ? 'leaked' : 'isolated', "\n";
}

inspectScope('parameter');
$variables = get_defined_vars();
echo isset($variables['outer']) ? $variables['outer'] : 'missing', ':';
echo isset($variables['argument']) ? 'leaked' : 'isolated', "\n";
