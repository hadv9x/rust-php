<?php

class NullsafeFactory
{
    public $label = 'factory';

    public function create($name)
    {
        return $name;
    }
}

class NullsafeHolder
{
    private $factory;

    public function __construct($factory)
    {
        $this->factory = $factory;
    }

    public function create($name)
    {
        return $this->factory?->create($name) ?? 'no factory';
    }

    public function label()
    {
        return $this->factory?->label ?? 'no label';
    }
}

echo (new NullsafeHolder(null))->create('item'), "\n";
echo (new NullsafeHolder(new NullsafeFactory()))->create('item'), "\n";
echo (new NullsafeHolder(null))->label(), "\n";
echo (new NullsafeHolder(new NullsafeFactory()))->label(), "\n";
