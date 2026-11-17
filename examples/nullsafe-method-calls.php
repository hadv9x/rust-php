<?php

class NullsafeFactory
{
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
}

echo (new NullsafeHolder(null))->create('item'), "\n";
echo (new NullsafeHolder(new NullsafeFactory()))->create('item'), "\n";
