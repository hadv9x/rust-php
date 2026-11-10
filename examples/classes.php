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

class ReflectionOriginExample {}
$userReflection = new ReflectionClass(ReflectionOriginExample::class);
$internalReflection = new ReflectionClass('stdClass');
echo 'reflection-origin:', (int) $userReflection->isUserDefined(), ':', (int) $internalReflection->isInternal(), "\n";
$countableReflection = new ReflectionClass('Countable');
echo 'reflection-interface:', (int) $countableReflection->isInterface(), ':', (int) $countableReflection->isInstantiable(), ':', (int) $countableReflection->isAbstract(), "\n";

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
$promotedId = (new ReflectionClass("ServiceHolder"))->getProperty("id");
echo "promoted-property:", $promotedId->isPromoted(), ":", $promotedId->isReadOnly(), "\n";
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

class HookFrameNotice {
    public function __construct(private string $label) {}
    public function __destruct() { echo "hook-frame-destroyed:", $this->label, "\n"; }
}
class ConstructorFrameExample {
    public function __construct() {
        $temporary = new HookFrameNotice("constructor");
        echo "constructor-returning\n";
    }
    public function __get($name) {
        $temporary = new HookFrameNotice("magic-get");
        echo "get-returning\n";
        return 7;
    }
}
$constructorFrameExample = new ConstructorFrameExample();
echo "constructed\n", $constructorFrameExample->missing, "\n";

class ShutdownNotice {
    public function __construct(private string $label) {}
    public function __destruct() { echo "destroyed:", $this->label, "\n"; }
    public function releaseMethodLocal() {
        $methodNotice = new ShutdownNotice("method-frame-end");
        echo "method-returning\n";
    }
}
$destructureSlot = new ShutdownNotice("destructuring-slot");
[$destructureSlot] = ["replacement"];
echo "destructuring-replaced\n";
function finishDestructorFrame() {
    $frameNotice = new ShutdownNotice("function-frame-end");
    echo "function-returning\n";
}
finishDestructorFrame();
$methodFrameOwner = new ShutdownNotice("method-owner");
$methodFrameOwner->releaseMethodLocal();
$closureFrame = function() {
    $closureNotice = new ShutdownNotice("closure-frame-end");
    echo "closure-returning\n";
};
$closureFrame();
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

trait FinalGreeting {
    public function lockedGreeting(): string { return 'trait greeting'; }
}
class GreetingBase { public function lockedGreeting(): string { return 'base greeting'; } }
class FinalGreetingCard extends GreetingBase {
    use FinalGreeting { FinalGreeting::lockedGreeting as final; }
}
$anonymousFinalGreeting = new class {
    use FinalGreeting { FinalGreeting::lockedGreeting as final; }
};
echo (new FinalGreetingCard())->lockedGreeting(), ':', $anonymousFinalGreeting->lockedGreeting(), "\n";

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

class ReadonlyCloneValue {
    public function __construct(public readonly int $value) {}
    public function __clone() { $this->value = 11; }
}
$readonlyCloneSource = new ReadonlyCloneValue(10);
$readonlyClone = clone $readonlyCloneSource;
echo "readonly-clone:", $readonlyCloneSource->value, ":", $readonlyClone->value, "\n";
$standardObject = new stdClass();
$standardObject->example = 'dynamic';
echo "stdClass:", get_class($standardObject), ":", $standardObject->example, "\n";

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
$capturedForReflection = "capture-value";
$reflectionClosure = function () use ($capturedForReflection) {};
$reflectionVariables = (new ReflectionFunction($reflectionClosure))->getStaticVariables();
echo "function-captures:", $reflectionVariables["capturedForReflection"], "\n";
function exampleReflectionCounter() { static $count = 4; return ++$count; }
$counterReflection = new ReflectionFunction("exampleReflectionCounter");
echo "function-static-before:", $counterReflection->getStaticVariables()["count"], "\n";
exampleReflectionCounter();
echo "function-static-after:", $counterReflection->getStaticVariables()["count"], "\n";

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
class ReflectionConstructorBypassExample { public $value = "default"; public function __construct() { echo "constructor should be skipped\n"; } }
$constructorBypass = (new ReflectionClass(ReflectionConstructorBypassExample::class))->newInstanceWithoutConstructor();
echo "reflection-without-constructor:", $constructorBypass->value, "\n";
class ReflectionFactoryExample { public $label; public function __construct($label, $suffix = "!") { $this->label = $label . $suffix; } }
$reflectionFactory = new ReflectionClass(ReflectionFactoryExample::class);
echo "reflection-new-instance:", $reflectionFactory->newInstance("direct")->label, ":", $reflectionFactory->newInstanceArgs(["args", "suffix" => "?"])->label, "\n";
class ReflectionPrivateFactoryExample { private function __construct() {} }
try { (new ReflectionClass(ReflectionPrivateFactoryExample::class))->newInstance(); }
catch (ReflectionException $error) { echo "reflection-private-constructor:", get_class($error), "\n"; }
class ReflectionNoConstructorExample {}
try { (new ReflectionClass(ReflectionNoConstructorExample::class))->newInstanceArgs(["unexpected"]); }
catch (ReflectionException $error) { echo "reflection-no-constructor-arguments:", get_class($error), "\n"; }
echo "reflection-flags:", $reflectionLeaf->isFinal(), ":", (new ReflectionClass("ReflectionMarker"))->isInterface(), ":", (new ReflectionClass("ReflectionBase"))->isAbstract(), ":", (new ReflectionClass("ReflectionState"))->isEnum(), "\n";
echo "reflection-modifiers:", (new ReflectionClass("ReflectionBase"))->getModifiers(), ":", ReflectionClass::IS_EXPLICIT_ABSTRACT, ":", (new ReflectionClass("ReflectionLeaf"))->getModifiers(), ":", ReflectionClass::IS_FINAL, "\n";
echo "reflection-modifier-names:", implode(',', Reflection::getModifierNames($reflectionLeaf->getModifiers())), ":", implode(',', Reflection::getModifierNames(ReflectionClass::IS_EXPLICIT_ABSTRACT)), "\n";
echo "reflection-name-parts:", $reflectionLeaf->getShortName(), ":", $reflectionLeaf->getNamespaceName(), ":", $reflectionLeaf->inNamespace(), "\n";
class ReflectionCloneableExample {}
class ReflectionNonCloneableExample { private function __clone() {} }
echo "reflection-cloneable:", (new ReflectionClass(ReflectionCloneableExample::class))->isCloneable(), ":", (new ReflectionClass(ReflectionNonCloneableExample::class))->isCloneable(), "\n";
echo "reflection-instance:", (new ReflectionClass(ReflectionMarker::class))->isInstance($reflectionLeaf), ":", (new ReflectionEnum(ReflectionState::class))->isInstance(ReflectionState::Ready), "\n";
echo "reflection-parent:", (new ReflectionClass("ReflectionBranch"))->getParentClass()->getName(), ":", (new ReflectionClass("ReflectionRoot"))->getParentClass(), "\n";
class ReflectionAccessibleMethodExample { private function hidden() {} }
$reflectionHiddenMethod = (new ReflectionClass(ReflectionAccessibleMethodExample::class))->getMethod("hidden");
echo "reflection-method-accessible:", ($reflectionHiddenMethod->setAccessible(false) === null), "\n";
class ReflectionPropertyParentExample { protected string $token = "parent"; public static int $count = 2; }
class ReflectionPropertyChildExample extends ReflectionPropertyParentExample {}
$reflectedToken = (new ReflectionClass("ReflectionPropertyChildExample"))->getProperty("token");
echo "reflection-property-declaring:", $reflectedToken->getDeclaringClass()->getName(), ":", $reflectedToken->getType()->getName(), ":", $reflectedToken->hasType(), ":", $reflectedToken->getModifiers(), ":", $reflectedToken->isProtected(), "\n";
$reflectionPropertyObject = new ReflectionPropertyChildExample();
echo "reflection-property-value:", $reflectedToken->getValue($reflectionPropertyObject), "\n";
$reflectedToken->setValue($reflectionPropertyObject, "updated");
echo "reflection-property-set-value:", $reflectedToken->getValue($reflectionPropertyObject), "\n";
echo "reflection-property-accessible:", ($reflectedToken->setAccessible(false) === null), ":", $reflectedToken->getValue($reflectionPropertyObject), "\n";
$reflectedCount = (new ReflectionClass("ReflectionPropertyChildExample"))->getProperty("count");
echo "reflection-property-static:", $reflectedCount->getModifiers(), ":", $reflectedCount->isStatic(), ":", ReflectionProperty::IS_STATIC, "\n";
class PrivateReflectionLookupParentExample { private $hidden = "parent"; }
class PrivateReflectionLookupChildExample extends PrivateReflectionLookupParentExample {}
$privateLookup = new ReflectionClass(PrivateReflectionLookupChildExample::class);
try { $privateLookup->getProperty("hidden"); }
catch (ReflectionException $error) { echo "reflection-parent-private:", get_class($error), "\n"; }
class UninitializedPropertyExample { public string $value; public $optional; }
$uninitializedObject = new UninitializedPropertyExample();
$uninitializedReflection = new ReflectionClass("UninitializedPropertyExample");
echo "reflection-property-initialized:", ($uninitializedReflection->getProperty("value")->isInitialized($uninitializedObject) ? 1 : 0), ":", ($uninitializedReflection->getProperty("optional")->isInitialized($uninitializedObject) ? 1 : 0), "\n";
class PropertyDefaultsExample { public $implicit; public ?int $uninitialized; public ?int $explicitNull = null; }
$propertyDefaults = new ReflectionClass("PropertyDefaultsExample");
echo "reflection-property-defaults:", ($propertyDefaults->getProperty("implicit")->hasDefaultValue() ? 1 : 0), ":", ($propertyDefaults->getProperty("uninitialized")->hasDefaultValue() ? 1 : 0), ":", ($propertyDefaults->getProperty("explicitNull")->hasDefaultValue() ? 1 : 0), "\n";
class DefaultPropertyBaseExample { protected $inheritedDefault = "base"; }
class DefaultPropertyChildExample extends DefaultPropertyBaseExample { private $ownDefault = "child"; public $unsetDefault; }
$reflectedDefaults = (new ReflectionClass(DefaultPropertyChildExample::class))->getDefaultProperties();
echo "reflection-default-properties:", $reflectedDefaults["inheritedDefault"], ":", $reflectedDefaults["ownDefault"], ":", ($reflectedDefaults["unsetDefault"] === null), "\n";
class StaticPropertyBaseExample { protected static $baseCount = 2; }
class StaticPropertyChildExample extends StaticPropertyBaseExample { public static $childCount = 3; }
StaticPropertyChildExample::$childCount = 4;
$reflectedStatics = (new ReflectionClass(StaticPropertyChildExample::class))->getStaticProperties();
echo "reflection-static-properties:", $reflectedStatics["baseCount"], ":", $reflectedStatics["childCount"], "\n";
echo "reflection-static-property-value:", (new ReflectionClass(StaticPropertyChildExample::class))->getStaticPropertyValue("childCount"), ":", (new ReflectionClass(StaticPropertyChildExample::class))->getStaticPropertyValue("missing", "fallback"), "\n";
(new ReflectionClass(StaticPropertyChildExample::class))->setStaticPropertyValue("childCount", 12);
echo "reflection-static-property-write:", StaticPropertyChildExample::$childCount, "\n";
echo "reflection-property-filter-count:", count((new ReflectionClass(StaticPropertyChildExample::class))->getProperties(filter: ReflectionProperty::IS_STATIC)), "\n";
class PropertyHookTemporaryExample { public function __destruct() { echo "hook temporary released\n"; } }
class PropertyHookLifetimeExample {
    public int $value = 6;
    public int $computed { get { $temporary = new PropertyHookTemporaryExample(); return $this->value; } }
}
echo "property-hook-lifetime:", (new PropertyHookLifetimeExample())->computed, "\n";
echo "reflection-relations:", $reflectionLeaf->implementsInterface("ReflectionMarker"), ":", $reflectionLeaf->isSubclassOf("ReflectionMarker"), "\n";
class ReflectionCtorParentExample { public function __construct() {} }
class ReflectionCtorChildExample extends ReflectionCtorParentExample {}
class ReflectionCtorEmptyExample {}
echo "reflection-constructor:", (new ReflectionClass("ReflectionCtorChildExample"))->getConstructor()->isConstructor(), ":", ((new ReflectionClass("ReflectionCtorEmptyExample"))->getConstructor() === null), "\n";

class CalledClassBase { public static function called(): string { return get_called_class(); } }
class CalledClassChild extends CalledClassBase {}
echo "called-class:", CalledClassBase::called(), ":", CalledClassChild::called(), "\n";

interface ReflectionRootInterface {}
interface ReflectionChildInterface extends ReflectionRootInterface {}
class ReflectionInterfaceParent implements ReflectionRootInterface {}
class ReflectionInterfaceChild extends ReflectionInterfaceParent implements ReflectionChildInterface {}
$interfaceReflection = new ReflectionClass("ReflectionInterfaceChild");
$interfaceNames = $interfaceReflection->getInterfaceNames();
$interfaceObjects = $interfaceReflection->getInterfaces();
echo "interface-names:", implode(",", $interfaceNames), ":", implode(",", array_keys($interfaceObjects)), ":", $interfaceObjects["ReflectionChildInterface"]->getName(), "\n";

trait ReflectionMarkerTrait {}
trait ReflectionComposedTrait { use ReflectionMarkerTrait; }
echo "reflection-is-trait:", (new ReflectionClass(ReflectionMarkerTrait::class))->isTrait(), "\n";
class ReflectionTraitExample { use ReflectionComposedTrait; }
$traitReflection = new ReflectionClass("ReflectionTraitExample");
$traitNames = $traitReflection->getTraitNames();
$traitObjects = $traitReflection->getTraits();
echo "trait-names:", implode(",", $traitNames), ":", $traitObjects["ReflectionComposedTrait"]->getName(), "\n";

trait ReflectionAliasTrait { public function originalMethod(): string { return "alias"; } }
class ReflectionAliasExample { use ReflectionAliasTrait { ReflectionAliasTrait::originalMethod as publicAlias; } }
echo "trait-aliases:", json_encode((new ReflectionClass("ReflectionAliasExample"))->getTraitAliases()), "\n";
echo "reflection-members:", (new ReflectionClass("ReflectionAliasExample"))->hasMethod("publicALIAS"), ":", (new ReflectionClass("ReflectionAliasExample"))->hasProperty("missing"), "\n";

abstract class ReflectionMethodBase {
    abstract protected function needsImplementation();
    final public static function fixedStatic() {}
    public function signature(int $required, string $optional = 'x'): ?string { return null; }
    private function hidden() {}
    public function __construct() {}
    public function __destruct() {}
}
class ReflectionMethodChild extends ReflectionMethodBase { protected function needsImplementation() {} }
$methodReflection = new ReflectionClass("ReflectionMethodChild");
echo "reflection-method-declaring:", $methodReflection->getMethod("hidden")->getDeclaringClass()->getName(), "\n";
echo "reflection-method-flags:", $methodReflection->getMethod("needsImplementation")->isProtected(), ":", (new ReflectionClass("ReflectionMethodBase"))->getMethod("needsImplementation")->isAbstract(), ":", $methodReflection->getMethod("hidden")->isPrivate(), ":", $methodReflection->getMethod("fixedStatic")->isStatic(), ":", $methodReflection->getMethod("fixedStatic")->isFinal(), ":", $methodReflection->getMethod("__construct")->isConstructor(), ":", $methodReflection->getMethod("__destruct")->isDestructor(), "\n";
$signatureMethod = (new ReflectionClass("ReflectionMethodChild"))->getMethod("signature");
echo "reflection-method-signature:", $signatureMethod->getNumberOfParameters(), ":", $signatureMethod->getNumberOfRequiredParameters(), ":", $signatureMethod->hasReturnType(), ":", $signatureMethod->getReturnType()->getName(), ":", $signatureMethod->getReturnType()->allowsNull(), "\n";
foreach ($signatureMethod->getParameters() as $signatureParameter) {
    echo "reflection-parameter:", $signatureParameter->getName(), ":", $signatureParameter->hasType(), ":", $signatureParameter->isOptional(), ":", $signatureParameter->isVariadic(), ":", $signatureParameter->isPassedByReference(), ":", $signatureParameter->isDefaultValueAvailable(), "\n";
}
function reflectedFunctionSignature($required, $optional = null): bool { return true; }
$signatureFunction = new ReflectionFunction("reflectedFunctionSignature");
echo "reflection-function-signature:", $signatureFunction->getNumberOfParameters(), ":", $signatureFunction->getNumberOfRequiredParameters(), ":", $signatureFunction->getReturnType()->getName(), "\n";
function reflectedParameterNullability($untyped, int $required, ?string $nullable, mixed $mixed, string $defaultNull = null, string $defaultValue = 'x') {}
$nullabilityParameters = (new ReflectionFunction("reflectedParameterNullability"))->getParameters();
echo "reflection-parameter-nullability:", implode("", array_map(static fn($parameter) => $parameter->allowsNull() ? "1" : "0", $nullabilityParameters)), "\n";
class ParameterOriginExample { public function run($value) {} }
$originFunctionParameter = (new ReflectionFunction("reflectedFunctionSignature"))->getParameters()[0];
$originMethodParameter = (new ReflectionMethod("ParameterOriginExample", "run"))->getParameters()[0];
echo "reflection-parameter-origin:", $originFunctionParameter->getDeclaringFunction()->getName(), ":", ($originFunctionParameter->getDeclaringClass() === null ? "null" : "class"), ":", $originMethodParameter->getDeclaringFunction()->getName(), ":", $originMethodParameter->getDeclaringClass()->getName(), "\n";
$directFunctionParameter = new ReflectionParameter("reflectedParameterNullability", 1);
$directMethodParameter = new ReflectionParameter(["ParameterOriginExample", "run"], "value");
echo "reflection-parameter-constructor:", $directFunctionParameter->getName(), ":", $directFunctionParameter->getType()->getName(), ":", $directMethodParameter->getDeclaringClass()->getName(), "\n";
class PromotedParameterExample { public function __construct(public string $name, private int $level, $ordinary = null) {} }
$promotedParameters = (new ReflectionMethod("PromotedParameterExample", "__construct"))->getParameters();
echo "reflection-parameter-promoted:", ($promotedParameters[0]->isPromoted() ? "1" : "0"), ($promotedParameters[1]->isPromoted() ? "1" : "0"), ($promotedParameters[2]->isPromoted() ? "1" : "0"), "\n";
const REFLECTION_PARAMETER_DEFAULT = 17;
function reflectedConstantDefault($named = REFLECTION_PARAMETER_DEFAULT, $literal = 2) {}
$constantDefaultParameters = (new ReflectionFunction("reflectedConstantDefault"))->getParameters();
echo "reflection-parameter-constant-default:", ($constantDefaultParameters[0]->isDefaultValueConstant() ? "1" : "0"), ":", $constantDefaultParameters[0]->getDefaultValueConstantName(), ":", ($constantDefaultParameters[1]->isDefaultValueConstant() ? "1" : "0"), "\n";
function reflectedReferencePassing(&$reference, $value) {}
$passingParameters = (new ReflectionFunction("reflectedReferencePassing"))->getParameters();
echo "reflection-parameter-pass-by-value:", ($passingParameters[0]->canBePassedByValue() ? "1" : "0"), ":", ($passingParameters[1]->canBePassedByValue() ? "1" : "0"), "\n";
class ReflectionFunctionAbstractExample {
    public int $stored = 1;
    public function &referenceResult(): int { return $this->stored; }
    public function variadicMethod(...$values) {}
}
function &reflectionFunctionAbstractReference(): int { static $value = 2; return $value; }
function reflectionFunctionAbstractVariadic(...$values) {}
$abstractMethodReflection = new ReflectionClass("ReflectionFunctionAbstractExample");
$abstractClosureReflection = new ReflectionFunction(static function (...$values) {});
echo "reflection-function-abstract:", $abstractMethodReflection->getMethod("referenceResult")->returnsReference(), ":", $abstractMethodReflection->getMethod("variadicMethod")->isVariadic(), ":", (new ReflectionFunction("reflectionFunctionAbstractReference"))->returnsReference(), ":", (new ReflectionFunction("reflectionFunctionAbstractVariadic"))->isVariadic(), ":", $abstractClosureReflection->isClosure(), ":", $abstractClosureReflection->isStatic(), ":", $abstractClosureReflection->isVariadic(), "\n";
function reflectedCallableExample(string $name, string $suffix = "!"): string { return $name . $suffix; }
$reflectedCallable = new ReflectionFunction("reflectedCallableExample");
echo "reflection-function-invoke:", $reflectedCallable->invoke("direct"), ":", $reflectedCallable->invokeArgs(["suffix" => "?", "name" => "named"]), ":", $reflectedCallable->getClosure()("closure"), "\n";
class ReflectionInvokeExample {
    private function secret(string $value): string { return "secret:" . $value; }
    protected static function sum(int $left, int $right): int { return $left + $right; }
}
$invokeExample = new ReflectionInvokeExample();
$privateMethod = new ReflectionMethod($invokeExample, "secret");
$staticMethod = ReflectionMethod::createFromMethodName("ReflectionInvokeExample::sum");
$privateClosure = $privateMethod->getClosure($invokeExample);
$staticClosure = $staticMethod->getClosure();
echo "reflection-invoke:", $privateMethod->invoke($invokeExample, "ok"), ":", $staticMethod->invokeArgs(null, ["left" => 2, "right" => 3]), ":", $privateClosure("closure"), ":", $staticClosure(3, 4), "\n";

$methodNames = [];
foreach ($methodReflection->getMethods() as $reflectedMethod) { $methodNames[] = $reflectedMethod->getName(); }
$finalOrStatic = [];
foreach ($methodReflection->getMethods(ReflectionMethod::IS_FINAL | ReflectionMethod::IS_STATIC) as $reflectedMethod) { $finalOrStatic[] = $reflectedMethod->getName(); }
echo "reflection-method-list:", implode(",", $methodNames), ":", implode(",", $finalOrStatic), ":", $methodReflection->getMethod("fixedStatic")->getModifiers(), "\n";

class ReflectedConstantBase { protected const string TOKEN = "base-token"; }
#[Attribute(Attribute::TARGET_CLASS_CONSTANT | Attribute::IS_REPEATABLE)] class ConstantLabel { public function __construct(public string $label) {} }
class ReflectedConstantChild extends ReflectedConstantBase {
    #[ConstantLabel("first"), ConstantLabel(label: "second")]
    final public const int CODE = 42;
}
$constantReflection = new ReflectionClass("ReflectedConstantChild");
$tokenReflection = $constantReflection->getReflectionConstant("TOKEN");
$codeReflection = new ReflectionClassConstant("ReflectedConstantChild", "CODE");
$constantAttribute = $codeReflection->getAttributes(ConstantLabel::class)[1];
echo "reflection-constants:", $tokenReflection->getDeclaringClass()->getName(), ":", $tokenReflection->getValue(), ":", $codeReflection->getName(), ":", $codeReflection->getValue(), ":", $codeReflection->isFinal(), ":", $codeReflection->getType()->getName(), ":", $codeReflection->hasType(), ":", $constantAttribute->newInstance()->label, "\n";

interface ReflectedTypeB {} interface ReflectedTypeC {}
class ReflectedDnfConstant { public const int|(ReflectedTypeB&ReflectedTypeC) VALUE = 1; }
$dnf = (new ReflectionClassConstant("ReflectedDnfConstant", "VALUE"))->getType();
echo "reflection-dnf:", get_class($dnf), ":", get_class($dnf->getTypes()[1]), ":", count($dnf->getTypes()[1]->getTypes()), "\n";

class ScopedForeachExample {
    private $secret = 'private';
    protected $state = 'pending';
    public $label = 'visible';
    public function complete() {
        $keys = [];
        foreach ($this as $key => &$value) {
            $keys[] = $key;
            if ($value === 'pending') { $value = 'complete'; }
        }
        unset($value);
        echo "scoped-foreach:", implode(',', $keys), ":", $this->secret, ":", $this->state, "\n";
    }
}
(new ScopedForeachExample())->complete();

trait PreferredMessageTrait { public function message() { return 'preferred'; } }
trait AlternateMessageTrait { public function message() { return 'alternate'; } }
trait MessageCompositionTrait {
    use PreferredMessageTrait, AlternateMessageTrait {
        PreferredMessageTrait::message insteadof AlternateMessageTrait;
        AlternateMessageTrait::message as alternateMessage;
    }
}
class MessageCompositionExample { use MessageCompositionTrait; }
$messages = new MessageCompositionExample();
echo "trait-adaptation:", $messages->message(), ':', $messages->alternateMessage(), "\n";

trait SharedTraitState { public int $items = 2; }
trait NestedSharedTraitState { use SharedTraitState; }
trait MatchingTraitState { public int $items = 2; }
class TraitStateExample { use NestedSharedTraitState, MatchingTraitState; }
echo "trait-property:", (new TraitStateExample())->items, "\n";

trait BaseTraitConstantsExample { public const int BASE = 4; }
trait DerivedTraitConstantsExample { use BaseTraitConstantsExample; public const int DERIVED = self::BASE + 3; }
trait MatchingTraitConstantsExample { public const int BASE = 4; }
class TraitConstantExample {
    use DerivedTraitConstantsExample, MatchingTraitConstantsExample;
    public const int BASE = 4;
}
echo "trait-constant:", TraitConstantExample::DERIVED, ':', TraitConstantExample::BASE, "\n";

class VarDumpLayoutExample {
    private $secret = 'kept';
    protected $state = 'ready';
    public $nested = ['answer' => 42];
}
var_dump([new VarDumpLayoutExample()]);
var_dump("raw\nbytes: " . chr(0) . chr(1));

interface InstantiableExampleContract { public function run(); }
abstract class InstantiableExampleBase { abstract public function run(); }
class InstantiableExampleChild extends InstantiableExampleBase implements InstantiableExampleContract { public function run() {} }
class InstantiableExamplePrivateConstructor { private function __construct() {} }
echo "reflection-instantiable:",
    ((new ReflectionClass('InstantiableExampleChild'))->isInstantiable() ? 'yes' : 'no'), ':',
    ((new ReflectionClass('InstantiableExamplePrivateConstructor'))->isInstantiable() ? 'yes' : 'no'), "\n";

class TypedMagicMethodsExample {
    private array $values = [];
    public function __get(string $name): mixed { return $this->values[$name] ?? null; }
    public function __set(string $name, mixed $value): void { $this->values[$name] = $value; }
    public function __isset(string $name): bool { return isset($this->values[$name]); }
    public function __unset(string $name): void { unset($this->values[$name]); }
    public function __call(string $name, array $arguments): mixed { return $name . count($arguments); }
    public static function __callStatic(string $name, array $arguments): mixed { return $name . count($arguments); }
    public function __toString(): string { return 'typed-magic'; }
    public function __clone(): void {}
    public function __invoke(...$values): mixed { return count($values); }
}
$typedMagic = new TypedMagicMethodsExample();
$typedMagic->note = 'ok';
$typedMagicClone = clone $typedMagic;
echo "typed-magic:", $typedMagic->note, ':', isset($typedMagic->note) ? 'set' : 'unset', ':',
    $typedMagic->missing(1, 2), ':', TypedMagicMethodsExample::missing(3), ':',
    (string)$typedMagicClone, ':', $typedMagicClone(1, 2), "\n";

class AsymmetricPropertyReflectionExample {
    public protected(set) string $protected = 'p';
    public private(set) string $private = 'q';
}
$asymmetricReflection = new ReflectionClass(AsymmetricPropertyReflectionExample::class);
echo "asymmetric-set:",
    $asymmetricReflection->getProperty('protected')->isProtectedSet() ? 'protected' : 'other', ':',
    $asymmetricReflection->getProperty('private')->isPrivateSet() ? 'private' : 'other', "\n";
echo "asymmetric-modifiers:",
    implode(',', Reflection::getModifierNames($asymmetricReflection->getProperty('protected')->getModifiers())), ':',
    implode(',', Reflection::getModifierNames($asymmetricReflection->getProperty('private')->getModifiers())), "\n";

readonly class ReadonlyClassReflectionExample { public int $id; }
echo "readonly-class:", (new ReflectionClass(ReadonlyClassReflectionExample::class))->isReadOnly() ? 'yes' : 'no', "\n";
$readonlyAnonymous = new readonly class { public int $id; };
echo "readonly-anonymous:", (new ReflectionClass($readonlyAnonymous))->isReadOnly() ? 'yes' : 'no', "\n";

class ParentSetVisibilityExample { public protected(set) int $value = 1; }
class ChildSetVisibilityExample extends ParentSetVisibilityExample { public int $value = 2; }
echo "inherited-set-visibility:", (new ChildSetVisibilityExample())->value, "\n";

class UnionPropertyParentExample { public int|string $value; }
class UnionPropertyChildExample extends UnionPropertyParentExample { public string|int $value = 'ok'; }
echo "equivalent-inherited-union:", (new UnionPropertyChildExample())->value ?? 'unset', "\n";

class UninitializedCoalescePropertyExample { public int $value; }
$uninitializedCoalesce = new UninitializedCoalescePropertyExample();
echo "typed-property-coalesce:", ($uninitializedCoalesce->value ?? 'fallback'), "\n";
echo "computed-property-coalesce:", ($uninitializedCoalesce->{'value'} ?? 'computed-fallback'), "\n";
$uninitializedCoalesce->value ??= 3;
echo "typed-property-coalesce-assign:", $uninitializedCoalesce->value, "\n";
class MagicCoalesceVisibilityExample {
    private int $hidden;
    public function __isset($name) { echo "isset:$name "; return false; }
    public function __get($name) { echo "get:$name "; return 9; }
}
$magicCoalesce = new MagicCoalesceVisibilityExample();
echo "magic-property-coalesce:", ($magicCoalesce->hidden ?? 'fallback'), "\n";
$anonymousReflectionExample = new class {};
echo "anonymous-reflection:", (new ReflectionClass($anonymousReflectionExample))->isAnonymous() ? 'yes' : 'no', "\n";

class DeprecatedParameterTypeExample {}
function deprecatedParameterTypeExample(DeprecatedParameterTypeExample $object, array|int $items, callable|string $callback, $untyped) {}
$deprecatedTypeParameters = (new ReflectionFunction('deprecatedParameterTypeExample'))->getParameters();
echo "deprecated-parameter-types:", strtolower($deprecatedTypeParameters[0]->getClass()->getName()), ':',
    $deprecatedTypeParameters[1]->isArray() ? 'array' : 'other', ':',
    $deprecatedTypeParameters[2]->isCallable() ? 'callable' : 'other', ':',
    $deprecatedTypeParameters[3]->getClass() === null ? 'untyped' : 'class', "\n";

interface ReflectedIterableContract extends Traversable {}
class ReflectedIterableBase implements ReflectedIterableContract {}
class ReflectedIterableChild extends ReflectedIterableBase {}
echo "reflection-iterable:", (new ReflectionClass(ReflectedIterableChild::class))->isIterable() ? 'yes' : 'no', ':',
    (new ReflectionClass(ReflectedIterableChild::class))->isIterateable() ? 'yes' : 'no', "\n";

class ExplicitCycleCollectionExample {
    public $link;
    public function __destruct() { echo 'cycle-destroyed '; }
}
$cycleLeft = new ExplicitCycleCollectionExample();
$cycleRight = new ExplicitCycleCollectionExample();
$cycleLeft->link = ['next' => $cycleRight];
$cycleRight->link = ['next' => $cycleLeft];
unset($cycleLeft, $cycleRight);
echo "cycles-collected:", gc_collect_cycles(), "\n";

class ClosureCycleCollectionExample {
    public $link;
    public function __destruct() { echo 'closure-cycle-destroyed '; }
}
$closureCycleNode = new ClosureCycleCollectionExample();
$closureCycleNode->link = function () use ($closureCycleNode) { return $closureCycleNode; };
unset($closureCycleNode);
echo "closure-cycle-collected:", gc_collect_cycles(), "\n";

function generatorCycleCollectionExample($value) { yield $value; }
class GeneratorCycleCollectionExample {
    public $generator;
    public function __destruct() { echo 'generator-cycle-destroyed '; }
}
$generatorCycleNode = new GeneratorCycleCollectionExample();
$generatorCycleNode->generator = generatorCycleCollectionExample($generatorCycleNode);
unset($generatorCycleNode);
echo "generator-cycle-collected:", gc_collect_cycles(), "\n";

class OverridePropertyContract { public string $label = ''; }
class OverridePropertyImplementation extends OverridePropertyContract {
    #[\Override]
    public string $label = 'property override';
}
echo (new OverridePropertyImplementation())->label, "\n";

class ReflectionFilterExample { public function present() {} }
try { (new ReflectionClass(ReflectionFilterExample::class))->getMethod('missing'); }
catch (ReflectionException $error) { echo 'missing reflected method: ', get_class($error), "\n"; }
try { (new ReflectionClass(ReflectionFilterExample::class))->getMethods([]); }
catch (TypeError $error) { echo 'invalid reflection filter: ', get_class($error), "\n"; }

class ReadonlyUnsetExample {
    public readonly int $id;
    public readonly int $uninitialized;
    public function __construct() { $this->id = 42; }
    public function clearUninitialized() { unset($this->uninitialized); }
}
$readonlyUnsetExample = new ReadonlyUnsetExample();
$readonlyUnsetExample->clearUninitialized();
try { unset($readonlyUnsetExample->id); }
catch (Error $error) { echo 'readonly unset: ', $error->getMessage(), "\n"; }

class ReflectionPrototypeBase { public function work(): string { return 'base'; } }
class ReflectionPrototypeChild extends ReflectionPrototypeBase { public function work(): string { return 'child'; } }
$prototype = (new ReflectionClass(ReflectionPrototypeChild::class))->getMethod('work');
echo "reflection-prototype:", $prototype->hasPrototype(), ':', $prototype->getPrototype()->getDeclaringClass()->getName(), "\n";

interface ReflectionPrototypeContract { public function render(): string; }
class ReflectionPrototypeImplementation implements ReflectionPrototypeContract { public function render(): string { return 'ok'; } }
$contractMethod = (new ReflectionClass(ReflectionPrototypeImplementation::class))->getMethod('render')->getPrototype();
echo "reflection-interface-prototype:", $contractMethod->getDeclaringClass()->getName(), "\n";

class OptionalOverrideBase { public function fetch(int $id): string { return (string) $id; } }
class OptionalOverrideChild extends OptionalOverrideBase {
    public function fetch(int $id = 1, string $format = 'plain'): string { return (string) $id . ':' . $format; }
}
echo "optional override:", (new OptionalOverrideChild())->fetch(), "\n";

class FixedSignatureBase { public function route(int $id, string $label): string { return $label; } }
class VariadicSignatureChild extends FixedSignatureBase {
    public function route(int $id, string ...$labels): string { return $labels[0]; }
}
echo "variadic override:", (new VariadicSignatureChild())->route(2, 'child'), "\n";

class InvariantPropertyBase { public int $value = 1; }
class InvariantPropertyChild extends InvariantPropertyBase { public int $value = 2; }
echo "inherited property type:", (new InvariantPropertyChild())->value, "\n";

trait PreferredAdaptationExample {
    public function selectAdaptation() { return 'preferred'; }
}
trait ExcludedAdaptationExample {
    public function selectAdaptation() { return 'excluded'; }
}
class TraitAdaptationPrecedenceExample {
    use PreferredAdaptationExample, ExcludedAdaptationExample {
        PreferredAdaptationExample::selectAdaptation insteadof ExcludedAdaptationExample;
        ExcludedAdaptationExample::selectAdaptation as protected;
    }
}
echo 'trait-adaptation:', (new TraitAdaptationPrecedenceExample())->selectAdaptation(), "\n";

$anonymousNameExample = new class {};
$anonymousNameReflection = new ReflectionClass($anonymousNameExample);
echo 'anonymous reflection name: ', $anonymousNameReflection->getShortName(), ':', $anonymousNameReflection->isAnonymous(), "\n";

class ExportStateExample {
    public function __construct(public string $name) {}
    public static function __set_state(array $state): object {
        return new self($state['name']);
    }
}
echo "var_export state expression: ", var_export(new ExportStateExample('Ada'), true), "\n";
enum ExportStateStatus: string { case Ready = 'ready'; }
echo "var_export enum case: ", var_export(ExportStateStatus::Ready, true), "\n";
$exportStdClassExample = new stdClass();
$exportStdClassExample->answer = 42;
echo "var_export stdClass: ", var_export($exportStdClassExample, true), "\n";
$exportCycleExample = [];
$exportCycleExample['self'] =& $exportCycleExample;
echo "var_export circular state: ", var_export($exportCycleExample, true), "\n";

class ClosureCycleCollectionExample {
    public $link;
    public function __destruct() { echo 'closure-cycle-destroyed '; }
}
$closureCycleNode = new ClosureCycleCollectionExample();
$closureCycleNode->link = function () use ($closureCycleNode) { return $closureCycleNode; };
unset($closureCycleNode);
echo "closure-cycle-collected:", gc_collect_cycles(), "\n";

function generatorCycleCollectionExample($value) { yield $value; }
class GeneratorCycleCollectionExample {
    public $generator;
    public function __destruct() { echo 'generator-cycle-destroyed '; }
}
$generatorCycleNode = new GeneratorCycleCollectionExample();
$generatorCycleNode->generator = generatorCycleCollectionExample($generatorCycleNode);
unset($generatorCycleNode);
echo "generator-cycle-collected:", gc_collect_cycles(), "\n";
