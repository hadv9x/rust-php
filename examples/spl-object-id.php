<?php
$object = new stdClass();
$firstId = spl_object_id($object);

echo is_int($firstId), ':', $firstId === spl_object_id($object), ':', function_exists('spl_object_id'), "\n";
