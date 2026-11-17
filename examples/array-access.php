<?php
class LoggingArrayAccess implements ArrayAccess
{
    public function offsetExists($offset): bool
    {
        return $offset === 'item';
    }

    public function offsetGet($offset): mixed
    {
        return "read:$offset";
    }

    public function offsetSet($offset, $value): void
    {
        echo $offset === null ? "append:$value\n" : "write:$offset=$value\n";
    }

    public function offsetUnset($offset): void
    {
        echo "unset:$offset\n";
    }
}

$items = new LoggingArrayAccess();
$items['item'] = 'value';
$items[] = 'next';
echo $items['item'], "\n";
echo isset($items['item']) ? "exists\n" : "missing\n";
unset($items['item']);
