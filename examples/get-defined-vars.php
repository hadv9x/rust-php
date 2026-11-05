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
$callback = array_map('inspectCallbackOrder', [1]);
echo $callback[0], "\n";
$variables = get_defined_vars();
echo isset($variables['outer']) ? $variables['outer'] : 'missing', ':';
echo isset($variables['argument']) ? 'leaked' : 'isolated', "\n";

function inspectCallbackOrder($zeta) {
    $alpha = 'later';
    return implode('|', array_keys(get_defined_vars()));
}

class InspectMethodVariables {
    public function inspect($zeta) {
        $alpha = 'later';
        echo implode('|', array_keys(get_defined_vars())), "\n";
    }
}

(new InspectMethodVariables())->inspect('parameter');
