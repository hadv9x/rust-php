<?php
#[Attribute(Attribute::TARGET_CONSTANT | Attribute::IS_REPEATABLE)]
class ConfigTag {
    public $label;
    public function __construct($label) { $this->label = $label; }
}

#[ConfigTag('primary'), ConfigTag('legacy')]
const SERVICE_NAME = 'catalog';

$constant = new ReflectionConstant('SERVICE_NAME');
$attributes = $constant->getAttributes();
echo $constant->getName(), ':', $constant->getValue(), ':', count($attributes), ':';
echo $attributes[0]->getTarget(), ':', $attributes[0]->newInstance()->label, ':', $attributes[1]->isRepeated(), "\n";
?>
