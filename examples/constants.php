<?php
namespace Demo;

const BASE = 6;
const DOUBLE = BASE * 2;
const OPTIONS = ['answer' => DOUBLE, 'enabled' => true];

echo DOUBLE, ':', OPTIONS['answer'], ':', defined('Demo\\DOUBLE'), ':', constant('Demo\\BASE'), "\n";
