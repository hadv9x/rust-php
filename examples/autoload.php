<?php
spl_autoload_register();
echo class_exists('ExampleAutoloaded') ? ExampleAutoloaded::value() : 'missing';
