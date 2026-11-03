<?php
#[Attribute(Attribute::TARGET_PARAMETER | Attribute::IS_REPEATABLE)]
class ArgumentTag {
    public $label;
    public function __construct($label) { $this->label = $label; }
}

$worker = fn (#[ArgumentTag('input'), ArgumentTag('payload')] $value) => $value;
$parameter = (new ReflectionFunction($worker))->getParameters()[0];
$attributes = $parameter->getAttributes(ArgumentTag::class);
echo $parameter->getName(), ':', count($attributes), ':', $attributes[0]->newInstance()->label, ':', $attributes[0]->isRepeated(), "\n";
?>
