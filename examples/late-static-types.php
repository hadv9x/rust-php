<?php
class EntityRoot {}

trait HasEntitySelfReference {
    public self $reference;
}

class EntityRecord extends EntityRoot {
    use HasEntitySelfReference;
    public parent $ancestor;
    public static self $prototype;
    public static parent $rootPrototype;

    public function selfValue(): self { return $this; }
    public function parentValue(): parent { return new EntityRoot(); }
    public function staticValue(): static { return new static(); }
    public static function factory(): static { return new static(); }
    public function acceptsSelf(self $value): bool { return true; }
    public function acceptsParent(parent $value): bool { return true; }
    public function invalidStaticValue(): static { return new EntityRecord(); }
}

class SpecialEntityRecord extends EntityRecord {}

interface DnfReadable {}
interface DnfCountable {}
interface DnfLoggable {}
interface DnfExtra {}

class DnfBoth implements DnfReadable, DnfCountable {}

interface DnfContract {
    public function handle((DnfReadable&DnfCountable)|DnfLoggable $value): (DnfReadable&DnfCountable)|DnfLoggable;
}

class DnfHandler implements DnfContract {
    public function handle((DnfReadable&DnfCountable)|DnfLoggable|DnfExtra $value): (DnfReadable&DnfCountable)|DnfLoggable {
        return $value;
    }
}

$record = new SpecialEntityRecord();
$record->reference = $record;
$record->ancestor = new EntityRoot();
EntityRecord::$prototype = $record;
EntityRecord::$rootPrototype = new EntityRoot();
echo get_class($record->selfValue()), ':', get_class($record->parentValue()), ':';
echo get_class($record->staticValue()), ':', get_class(SpecialEntityRecord::factory()), ':';
echo $record->acceptsSelf($record), ':', $record->acceptsParent(new EntityRoot()), ':';
echo 'typed-properties:';

try {
    $record->reference = new EntityRoot();
} catch (TypeError $error) {
    echo get_class($error), ':';
}

try {
    $record->invalidStaticValue();
} catch (TypeError $error) {
    echo get_class($error), "\n";
}

echo 'dnf:', get_class((new DnfHandler())->handle(new DnfBoth())), "\n";
