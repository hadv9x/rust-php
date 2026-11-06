<?php
$language = 'PHP';
$version = '8.5';

echo "Hello, $language!\n";
echo "Version: {$version}\n";
echo 'Single quotes keep $language literal.' . "\n";
echo "An escaped variable marker: \\$language\n";
$details = ['runtime' => 'Rust'];
$object = new stdClass();
$object->maintainer = 'PHP';
echo "{$object->maintainer} runs on {$details['runtime']}.\n";
$property = 'maintainer';
echo "Dynamic property: {$object->{$property}}\n";
$selected = 'language';
$language = 'PHP';
echo "Variable variable: {$$selected}\n";
echo "Deprecated legacy interpolation: ${language}\n";
echo "Deprecated variable-variable interpolation: ${$selected}\n";
