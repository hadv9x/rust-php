<?php
class AnonymousExampleBase {
    public function prefix() { return 'base:'; }
}
$anonymousExample = new class('local') extends AnonymousExampleBase {
    public $label;
    public function __construct($label) { $this->label = $label; }
    public function describe() { return parent::prefix() . $this->label; }
};
echo "anonymous:", $anonymousExample->describe(), "\n";
class TypedStringExample {
    public function __toString(): string { return 'typed-string'; }
}
echo new TypedStringExample(), "\n";

interface Describable {
    const KIND = "describable";

    public function description(): string;
    public function version(): int;
    public function identify(string $prefix): string;
}

trait HasLabel {
    public function labelText() {
        return $this->label . ":";
    }
}

abstract class Formatter {
    abstract public function format($value);

    final public function describe($value) {
        return "formatted=" . $this->format($value);
    }
}

class UpperFormatter extends Formatter {
    public function format($value) {
        return "formatted-" . $value;
    }
}

final class ClosedType {
    public function name() {
        return "closed";
    }
}

class Vault {
    private $pin = 2468;
    protected $label = "vault";

    private function pinValue() {
        return $this->pin;
    }

    public function unlock() {
        return $this->pinValue();
    }
}

class NamedVault extends Vault {
    public function label() {
        return $this->label;
    }
}

class UserRecord {
    public const TYPE = "user";
    public final const MAX_ID = 100;
    public int $id;
    public readonly string $name;
    public static int $created = 0;

    public function __construct($id, $name) {
        $this->id = $id;
        $this->name = $name;
        self::$created = self::$created + 1;
    }

    public function display() {
        return self::TYPE . ":" . $this->id . ":" . $this->name;
    }
}

class Counter {
    public $value = 0;
    public static $created = 0;

    // Constructors and methods support scalar defaults too.
    public function __construct($start = 0) {
        $this->value = $start;
        self::$created = self::$created + 1;
    }

    public function increment($step = 1) {
        $this->value = $this->value + $step;
        return $this->value;
    }

    public function current() {
        return $this->value;
    }

    public static function doubleValue($value) {
        return $value * 2;
    }

    public static function createdCount() {
        return self::$created;
    }
}

class TextLabel {
    public function __toString() { return "text-label"; }
}

class LabeledCounter extends Counter implements Describable {
    use HasLabel;
    public $label = "count";

    public function __construct($start) {
        parent::__construct($start);
    }

    public function description(): string {
        return $this->labelText() . parent::current();
    }

    public function version(): int {
        return 1;
    }

    public function identify(string $prefix): string {
        return $prefix . $this->label;
    }
}

$counter = new Counter(10);
echo $counter->increment(2), "\n";
echo $counter->current(), "\n";
$labeled = new LabeledCounter(5);
echo $labeled->description(), "\n";
echo $labeled->version(), "\n";
echo $labeled->identify("item-"), "\n";
echo $labeled->increment(3), "\n";
echo Counter::doubleValue(6), "\n";
echo Counter::$created, "\n";
echo Counter::createdCount(), "\n";
$formatter = new UpperFormatter();
echo $formatter->describe("php"), "\n";
echo (new ClosedType())->name(), "\n";
echo (new Vault())->unlock(), "\n";
echo (new NamedVault())->label(), "\n";
$user = new UserRecord(7, "Ada");
echo $user->display(), "\n";
echo UserRecord::$created, "\n";
echo "prefix:" . new TextLabel() . ":suffix\n";
echo UserRecord::MAX_ID, "\n";
echo Describable::KIND, ":", LabeledCounter::KIND, "\n";

class ServiceBase {
    public function name(): string {
        return "service";
    }
}

class ServiceChild extends ServiceBase {}

class ServiceHolder {
    public ?string $note = null;
    public ServiceBase $service;

    public function __construct(public int $id, ServiceBase $service) {
        $this->service = $service;
    }

    public function summary(): string {
        return $this->id . ":" . $this->service->name();
    }
}

$holder = new ServiceHolder(42, new ServiceChild());
echo $holder->summary(), "\n";
echo $holder->note, "\n";

class StringableInvoice {
    public function __construct(private int $number) {}

    public function __toString() {
        return "invoice-" . $this->number;
    }
}

echo new StringableInvoice(73), "\n";
echo (new StringableInvoice(74) instanceof Stringable) ? "stringable\n" : "not stringable\n";
echo strlen(new StringableInvoice(75)), "\n";

class ScalarStringifier {
    public function __toString() { return 42; }
}
echo new ScalarStringifier(), "\n";

class ShutdownNotice {
    public function __construct(private string $label) {}
    public function __destruct() { echo "destroyed:", $this->label, "\n"; }
}
$shutdownNotice = new ShutdownNotice("script-end");
$arrayNotice = [new ShutdownNotice("array-unset")];
unset($arrayNotice[0]);
$reassignedArrayNotice = [new ShutdownNotice("array-slot-replaced")];
$reassignedArrayNotice[0] = new ShutdownNotice("array-slot-current");
$replacedNotice = new ShutdownNotice("reassigned");
$replacedNotice = null;

trait PrimaryGreeting {
    public function greet() {
        return "hello";
    }
}

trait AlternateGreeting {
    public function greet() {
        return "hi";
    }
}

class GreetingCard {
    use PrimaryGreeting, AlternateGreeting {
        PrimaryGreeting::greet insteadof AlternateGreeting;
        AlternateGreeting::greet as private alternateGreeting;
    }

    public function otherGreeting() {
        return $this->alternateGreeting();
    }
}

$card = new GreetingCard();
echo $card->greet(), ":", $card->otherGreeting(), "\n";

interface ServiceFactory {
    public function make(): ServiceBase;
}

class SpecializedFactory implements ServiceFactory {
    public function make(): ServiceChild {
        return new ServiceChild();
    }
}

interface ChildServiceConsumer {
    public function consume(ServiceChild $service): string;
}

class GeneralServiceConsumer implements ChildServiceConsumer {
    public function consume(ServiceBase $service): string {
        return $service->name();
    }
}

echo (new SpecializedFactory())->make()->name(), "\n";
echo (new GeneralServiceConsumer())->consume(new ServiceChild()), "\n";

interface IterableFactory {
    public function values(): iterable;
}

class ArrayFactory implements IterableFactory {
    public function values(): array {
        return [3, 5, 8];
    }
}

interface ArrayInput {
    public function accept(array $values): string;
}

class IterableInput implements ArrayInput {
    public function accept(iterable $values): string {
        return "accepted-" . $values[0];
    }
}

echo (new ArrayFactory())->values()[1], "\n";
echo (new IterableInput())->accept([9]), "\n";

class LateStaticBase {
    public static $label = "base";
    public const NAME = "base";

    public static function label() {
        return static::$label;
    }

    public static function declaredName() {
        return static::NAME;
    }

    public static function updateLabel($value) {
        static::$label = $value;
    }

    public static function spawn() {
        return new static();
    }

    public function dynamicName() {
        return static::NAME;
    }
}

class LateStaticChild extends LateStaticBase {
    public static $label = "child";
    public const NAME = "child";
}

echo LateStaticBase::label(), ":", LateStaticChild::label(), "\n";
LateStaticChild::updateLabel("changed");
echo LateStaticBase::label(), ":", LateStaticChild::label(), "\n";
echo LateStaticChild::declaredName(), ":", (new LateStaticChild())->dynamicName(), "\n";
echo LateStaticChild::spawn()->dynamicName(), "\n";


class VirtualBox {
    private $secret = "hidden";

    // Return a virtual value for undeclared or inaccessible properties.
    public function __get($name) {
        return "virtual-" . $name;
    }

    // Handle writes that do not target a directly accessible declaration.
    public function __set($name, $value) {
        echo "stored-" . $name . "=" . $value, "\n";
    }

    // Define which overloaded property names count as set.
    public function __isset($name) {
        return $name == "secret";
    }

    // Observe removal requests for overloaded properties.
    public function __unset($name) {
        echo "unset-" . $name, "\n";
    }
}

$box = new VirtualBox();
echo $box->secret, "\n";
echo isset($box->secret), ":", isset($box->missing), "\n";
$box->color = "blue";
unset($box->secret);

class IncrementVirtualBox {
    private $value = 4;
    public $calls = [];
    public function __get($name) { $this->calls[] = 'get:' . $name; return $this->value; }
    public function __set($name, $value) { $this->calls[] = 'set:' . $name . ':' . $value; $this->value = $value; }
    public function actual() { return $this->value; }
}
$incrementBox = new IncrementVirtualBox();
echo $incrementBox->virtual++, ':', ++$incrementBox->virtual, ':', $incrementBox->actual(), ':', implode('|', $incrementBox->calls), "\n";

$emptyMagicCalls = [];
class EmptyVirtualBox {
    public function __isset($name) { $GLOBALS['emptyMagicCalls'][] = 'isset:' . $name; return $name !== 'missing'; }
    public function __get($name) {
        $GLOBALS['emptyMagicCalls'][] = 'get:' . $name;
        return $name === 'nested' ? ['value' => 0] : '0';
    }
}
$emptyBox = new EmptyVirtualBox();
$emptyDynamicName = 'nested';
echo empty($emptyBox->missing), ':', empty($emptyBox->zero), ':', empty($emptyBox->nested['value']), ':', empty($emptyBox->{$emptyDynamicName}['value']), ':', implode(',', $emptyMagicCalls), "\n";

class DynamicOverloadedBag {
    public function __get($name) { return ["nested" => ["value" => 11]]; }
    public function __isset($name) { return $name == "items"; }
}
$dynamicProperty = "items";
echo isset((new DynamicOverloadedBag())->{$dynamicProperty}["nested"]["value"]), "\n";

$nestedMagicCalls = [];
class NestedOverloadedNode {
    public function __isset($name) { $GLOBALS['nestedMagicCalls'][] = 'isset:' . $name; return true; }
    public function __get($name) { $GLOBALS['nestedMagicCalls'][] = 'get:' . $name; return new NestedOverloadedNode(); }
}
$nestedOverloaded = new NestedOverloadedNode();
echo isset($nestedOverloaded->first->second), ':', implode(',', $nestedMagicCalls), "\n";
$dynamicFirst = 'first';
$dynamicSecond = 'second';
$nestedMagicCalls = [];
echo isset($nestedOverloaded->{$dynamicFirst}->{$dynamicSecond}), ':', implode(',', $nestedMagicCalls), "\n";

$offsetMagicCalls = [];
class OffsetMagicNode {
    public function __isset($name) { $GLOBALS['offsetMagicCalls'][] = 'isset:' . $name; return true; }
    public function __get($name) {
        $GLOBALS['offsetMagicCalls'][] = 'get:' . $name;
        return ['child' => new OffsetMagicNode()];
    }
}
$offsetMagic = new OffsetMagicNode();
echo isset($offsetMagic->first['child']->second), ':', implode(',', $offsetMagicCalls), "\n";

class MagicRouter {
    // Route unknown instance methods through a small dynamic dispatcher.
    public function __call($name, $arguments) {
        return $name . ":" . $arguments[0];
    }

    // Route unknown static methods through the same dynamic naming convention.
    public static function __callStatic($name, $arguments) {
        return "static-" . $name . ":" . $arguments[0];
    }
}

echo (new MagicRouter())->route("home"), "\n";
echo MagicRouter::lookup("id"), "\n";

class CloneCounter {
    public $value = 1;
    public $wasCloned = false;

    // Mark the copied object so the example proves the clone hook ran.
    public function __clone() {
        $this->wasCloned = true;
    }
}

$source = new CloneCounter();
$copy = clone $source;
$copy->value = 2;
echo $source->value, ":", $copy->value, ":", $copy->wasCloned, "\n";

// Omitting constructor and method arguments selects their declared defaults.
$defaultCounter = new Counter();
echo $defaultCounter->increment(), "\n";

class ParentPrivateValue {
    private $value = "parent";
    public function readParentValue() { return $this->value; }
}
class ChildPrivateValue extends ParentPrivateValue {
    private $value = "child";
    public function readChildValue() { return $this->value; }
}
$privateValues = new ChildPrivateValue();
echo $privateValues->readParentValue(), ":", $privateValues->readChildValue(), "\n";

function addMarker(&$value) { $value .= "!"; }
class PrivateReferenceValue {
    private $value = "private";
    public function update() { addMarker($this->value); }
    public function read() { return $this->value; }
}
$privateReference = new PrivateReferenceValue();
$privateReference->update();
echo $privateReference->read(), "\n";

function replaceClassValue(&$value) { $value = "updated"; }
class PrivateArrayReferenceValue {
    private $values = ["item" => "before"];
    public function update() { replaceClassValue($this->values["item"]); }
    public function read() { return $this->values["item"]; }
}
$privateArrayReference = new PrivateArrayReferenceValue();
$privateArrayReference->update();
echo $privateArrayReference->read(), "\n";

class ExampleCallable {
    public function __invoke($value) { return "called:" . $value; }
}
$callable = new ExampleCallable();
echo $callable("object"), ":", implode(",", array_map($callable, ["a", "b"])), "\n";

#[Attribute]
class ExampleLabel {
    public $value;
    public function __construct($name) { $this->value = $name; }
}

#[ExampleLabel(name: "service")]
class TaggedService {}
$tag = (new ReflectionClass("TaggedService"))->getAttributes()[0];
$tagInstance = $tag->newInstance();
echo "attribute:", $tag->getName(), ":", $tag->getArguments()["name"], ":", $tagInstance->value, "\n";

#[Attribute(Attribute::TARGET_METHOD)]
class ExampleAction {
    public $name;
    public function __construct($name) { $this->name = $name; }
}
class ExampleController {
    #[ExampleAction("index")]
    public function index() {}
}
$action = (new ReflectionClass("ExampleController"))->getMethod("index")->getAttributes()[0];
echo "method-attribute:", $action->getTarget(), ":", $action->newInstance()->name, "\n";

#[Attribute(Attribute::TARGET_PROPERTY)]
class ExampleColumn {
    public $name;
    public function __construct($name) { $this->name = $name; }
}
class ExampleRecord {
    #[ExampleColumn("record_id")]
    public int $id;
}
$column = (new ReflectionClass("ExampleRecord"))->getProperty("id")->getAttributes()[0];
echo "property-attribute:", $column->getTarget(), ":", $column->newInstance()->name, "\n";

#[Attribute(Attribute::TARGET_PARAMETER)]
class ExampleParameterTag {
    public $name;
    public function __construct($name) { $this->name = $name; }
}
class ExampleCommand {
    public function execute(#[ExampleParameterTag("payload")] $input, $mode) {}
}
$parameters = (new ReflectionClass("ExampleCommand"))->getMethod("execute")->getParameters();
$inputAttribute = $parameters[0]->getAttributes()[0];
echo "parameter-attribute:", $parameters[0]->getName(), ":", $parameters[0]->getPosition(), ":", $inputAttribute->getTarget(), ":", $inputAttribute->newInstance()->name, "\n";

#[Attribute(Attribute::TARGET_FUNCTION)]
class ExampleFunctionTag {
    public $name;
    public function __construct($name) { $this->name = $name; }
}
#[ExampleFunctionTag("worker")]
function exampleWorker() {}
$workerAttribute = (new ReflectionFunction("exampleWorker"))->getAttributes()[0];
echo "function-attribute:", $workerAttribute->getTarget(), ":", $workerAttribute->newInstance()->name, "\n";

$anonymousTagged = new #[ExampleLabel("anonymous")] class {};
$anonymousTag = (new ReflectionClass($anonymousTagged))->getAttributes()[0];
echo "anonymous-attribute:", $anonymousTag->getTarget(), ":", $anonymousTag->newInstance()->value, "\n";

#[ExampleLabel("contract")]
interface ExampleTaggedContract {}
$contractTag = (new ReflectionClass("ExampleTaggedContract"))->getAttributes()[0];
echo "interface-attribute:", $contractTag->getTarget(), ":", $contractTag->newInstance()->value, "\n";

class DebugView {
    public $secret = "hidden";
    public function __debugInfo(): array { return ["display" => "custom debug view"]; }
}
var_dump(new DebugView());

interface ReflectionMarker {}
abstract class ReflectionBase {}
final class ReflectionLeaf implements ReflectionMarker {}
class ReflectionRoot {}
class ReflectionBranch extends ReflectionRoot {}
enum ReflectionState { case Ready; }
$reflectionLeaf = new ReflectionClass("ReflectionLeaf");
echo "reflection-flags:", $reflectionLeaf->isFinal(), ":", (new ReflectionClass("ReflectionMarker"))->isInterface(), ":", (new ReflectionClass("ReflectionBase"))->isAbstract(), ":", (new ReflectionClass("ReflectionState"))->isEnum(), "\n";
echo "reflection-parent:", (new ReflectionClass("ReflectionBranch"))->getParentClass()->getName(), ":", (new ReflectionClass("ReflectionRoot"))->getParentClass(), "\n";
echo "reflection-relations:", $reflectionLeaf->implementsInterface("ReflectionMarker"), ":", $reflectionLeaf->isSubclassOf("ReflectionMarker"), "\n";
