<?php
$extensions = get_loaded_extensions();
$zendExtensions = get_loaded_extensions(zend_extensions: true);
$versions = array_map('phpversion', $extensions);

echo implode(', ', $extensions), "\n";
echo 'Zend extensions: ', count($zendExtensions), "\n";
echo 'PDO loaded: ', extension_loaded('PDO') ? 'yes' : 'no', "\n";
echo 'Version count matches: ', count($versions) === count($extensions) ? 'yes' : 'no', "\n";
