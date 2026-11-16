<?php
namespace Demo;

const BASE = 6;
const DOUBLE = BASE * 2;
const OPTIONS = ['answer' => DOUBLE, 'enabled' => true];
class ConstantComparisons
{
    public const IS_64_BIT = PHP_INT_SIZE >= 8;
    public const LESS = 3 < 4;
    public const GREATER = 5 > 4;
}

echo DOUBLE, ':', OPTIONS['answer'], ':', defined('Demo\\DOUBLE'), ':', constant('Demo\\BASE'), "\n";
echo ConstantComparisons::IS_64_BIT, ConstantComparisons::LESS, ConstantComparisons::GREATER, "\n";
