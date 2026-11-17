<?php

$map = new WeakMap();
$key = new stdClass();
$map[$key] = 'cached';
echo $map->get($key), ':', count($map), ':', isset($map[$key]) ? 'present' : 'missing', "\n";

unset($key);
echo count($map), "\n";
