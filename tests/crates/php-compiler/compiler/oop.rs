use php_compiler::compile_source;

#[test]
fn invalid_object_and_array_keys_raise_catchable_type_errors() {
    let source = r#"<?php
        class InvalidArrayKeyObject {}
        $values = [];
        try { $values[new InvalidArrayKeyObject()] = 'x'; }
        catch (TypeError $error) { echo get_class($error), ':'; }
        try { $values[[]] = 'x'; }
        catch (TypeError $error) { echo get_class($error); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "TypeError:TypeError");
}

#[test]
fn var_dump_uses_debug_info_and_hides_actual_object_properties() {
    let source = r#"<?php
        class DebugInfoSubject {
            public $secret = 'real';
            public function __debugInfo(): array { return ['visible' => 'custom']; }
        }
        echo function_exists('var_dump'), ':';
        var_dump(new DebugInfoSubject());
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "1:object(DebugInfoSubject) {\n  [\"visible\"]=>\nstring(6) \"custom\"\n}\n");
    assert!(!output.contains("secret"));
}

#[test]
fn var_dump_marks_recursive_array_references_without_overflowing() {
    let source = r#"<?php
        $cycle = [];
        $cycle['self'] =& $cycle;
        var_dump($cycle);
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert!(output.contains("*RECURSION*"), "{output}");
    assert!(output.starts_with("array(1) {\n"), "{output}");
}

#[test]
fn reflection_enum_exposes_cases_and_backed_values() {
    let source = r#"<?php
        #[Attribute] class ReflectedEnumTag {}
        #[ReflectedEnumTag] enum Direction { case North; case South; }
        enum Suit: string { case Heart = 'H'; case Club = 'C'; }
        $unit = new ReflectionEnum('Direction');
        $backed = new ReflectionEnum('Suit');
        $unitCases = $unit->getCases();
        $backedCases = $backed->getCases();
        echo $unit->getName(), ':', $unit->isBacked(), ':', count($unit->getAttributes()), ':', $unitCases[0]->getName(), ':', ($unitCases[0]->getValue() === Direction::North), ':';
        $backingType = $backed->getBackingType();
        echo $backed->isBacked(), ':', $backedCases[1]->getName(), ':', $backedCases[1]->getBackingValue(), ':', $backingType->getName(), ':', $backingType->isBuiltin(), ':', $backingType->allowsNull(), ':';
        echo ($unit->getBackingType() === null), ':', class_exists('ReflectionEnum'), ':', method_exists('ReflectionEnum', 'getCases');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "Direction::1:North:1:1:Club:C:string:1::1:1:1");
}

#[test]
fn serialization_and_debug_magic_methods_validate_signatures() {
    compile_source(r#"<?php
        class MagicMethodSignatures {
            public function __serialize(): array { return []; }
            public function __unserialize(array $data): void {}
            public function __sleep(): array { return []; }
            public function __wakeup(): void {}
            public static function __set_state(array $data): object { return new self(); }
            public function __debugInfo(): array { return []; }
        }
        class SerializationCanThrow { public function __serialize(): never { throw new Exception('not serializable'); } }
    "#).expect("valid declarations for the remaining magic methods should compile");

    for invalid in [
        "<?php class PrivateDebugInfo { private function __debugInfo() {} }",
        "<?php class StaticSerialize { public static function __serialize() {} }",
        "<?php class ParameterizedWakeup { public function __wakeup($data) {} }",
        "<?php class InstanceSetState { public function __set_state($data) {} }",
        "<?php class MissingUnserializeData { public function __unserialize() {} }",
        "<?php class WrongSerializeReturn { public function __serialize(): string {} }",
        "<?php class WrongUnserializeParameter { public function __unserialize(string $data): void {} }",
        "<?php class WrongSetStateReturn { public static function __set_state(array $data): string {} }",
    ] {
        assert!(compile_source(invalid).is_err(), "invalid declaration unexpectedly compiled: {invalid}");
    }
}

#[test]
fn trait_exists_reports_declared_traits_and_missing_traits() {
    let source = r#"<?php trait PresentTrait {} echo trait_exists('PresentTrait'), ':', trait_exists('Tests\\CreatesApplication'), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:\n");
}

#[test]
fn get_called_class_tracks_late_static_binding_and_requires_class_context() {
    let source = r#"<?php
        class CalledBase { public static function who(): string { return get_called_class(); } }
        class CalledChild extends CalledBase {}
        echo CalledBase::who(), ':', CalledChild::who(), ':', function_exists('get_called_class'), ':';
        try { get_called_class(); } catch (Error $error) { echo get_class($error); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "CalledBase:CalledChild:1:Error");
}

#[test]
fn inactive_anonymous_class_defers_missing_trait_resolution() {
    let source = r#"<?php if (false) { $value = new class { use MissingTrait; }; } echo "ok\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ok\n");
}

#[test]
fn instantiated_anonymous_class_reports_missing_trait() {
    let source = r#"<?php $value = new class { use MissingTrait; };"#;
    let error = compile_source(source).unwrap().execute().unwrap_err();
    assert!(error.contains("Trait 'missingtrait' not found"), "{error}");
}

#[test]
fn concrete_trait_method_satisfies_abstract_trait_method() {
    let source = r#"<?php
        trait RequiresHook { abstract protected function hook(string $value): void; }
        trait ProvidesHook { protected function hook(string $value): void {} }
        class HookConsumer { use RequiresHook, ProvidesHook; public function run(): void { $this->hook('ok'); } }
        (new HookConsumer())->run(); echo "ok\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ok\n");
}

#[test]
fn class_method_satisfies_duplicate_abstract_trait_requirements() {
    let source = r#"<?php
        trait RequiresRunA { abstract public function run(): void; }
        trait RequiresRunB { abstract public function run(): void; }
        class ImplementsRun { use RequiresRunA, RequiresRunB; public function run(): void {} }
        (new ImplementsRun())->run(); echo "ok\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ok\n");
}

#[test]
fn promoted_property_merges_with_compatible_trait_property() {
    let source = r#"<?php
        trait ProvidesScroll { public int $scroll; }
        class ScrollableTable { use ProvidesScroll; public function __construct(public int $scroll = 10) {} }
        echo (new ScrollableTable())->scroll, "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "10\n");
}

#[test]
fn promoted_property_still_rejects_incompatible_trait_property() {
    let source = r#"<?php
        trait ProvidesScrollText { public string $scroll; }
        class InvalidScrollableTable { use ProvidesScrollText; public function __construct(public int $scroll = 10) {} }
    "#;
    let error = match compile_source(source) { Err(error) => error, Ok(_) => panic!("expected incompatible promoted property error") };
    assert!(error.contains("Promoted property $scroll conflicts"), "{error}");
}


#[test]
fn anonymous_classes_compile_properties_methods_constructors_and_inheritance() {
    let source = r#"<?php
        class AnonymousBase { public function prefix() { return 'base:'; } }
        $value = new class('item') extends AnonymousBase {
            public $label;
            public function __construct($label) { $this->label = $label; }
            public function describe() { return parent::prefix() . $this->label; }
        };
        echo $value->describe(), ':', ($value instanceof AnonymousBase), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "base:item:1\n");
}

#[test]
fn clone_magic_method_accepts_private_and_protected_visibility() {
    let source = r#"<?php
        class PrivateCloneHook {
            public $value = 0;
            private function __clone() { $this->value = 1; }
        }
        class ProtectedCloneHook {
            public $value = 0;
            protected function __clone() { $this->value = 2; }
        }
        $private = clone new PrivateCloneHook();
        $protected = clone new ProtectedCloneHook();
        echo $private->value, ':', $protected->value, "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:2\n");

    for invalid in [
        "<?php class StaticCloneHook { public static function __clone() {} }",
        "<?php class ParameterizedCloneHook { private function __clone($value) {} }",
    ] {
        let error = compile_source(invalid).err().expect("invalid clone magic method must fail compilation");
        assert!(error.contains("__clone() must be"));
    }
}

#[test]
fn dynamic_static_property_name_uses_variable_variable_value() {
    let source = r#"<?php
        class StaticValues { public static $selected = 'found'; }
        StaticValues::$selected = 'found';
        $class = 'StaticValues';
        $property = 'selected';
        echo StaticValues::$selected, '|', $property, '|';
        echo $class::$$property;
        echo '|', isset($class::$$property);
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "found|selected|found|1");
}

#[test]
fn tostring_is_used_when_objects_are_concatenated() {
    let source = r#"<?php
        class Label {
            public function __toString() { return 'label'; }
        }
        $value = new Label();
        echo 'left:' . $value . ':' . 'right', "\n";
        echo $value . ':tail', "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "left:label:right\nlabel:tail\n"
    );
}

#[test]
fn tostring_weakly_coerces_scalar_results_and_rejects_arrays() {
    let source = r#"<?php
        class IntegerStringifier { public function __toString() { return 42; } }
        class ArrayStringifier { public function __toString() { return []; } }
        class TextStringifier { public function __toString() { return 'text'; } }
        echo new IntegerStringifier(), "\n";
        echo strval(new TextStringifier()), ':', implode(',', array_map('strval', [new TextStringifier()])), "\n";
        try { echo new ArrayStringifier(); }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(), "\n"; }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "42\ntext:text\nError:ArrayStringifier::__toString() must return a string\n");
}

#[test]
fn self_parent_and_static_return_types_follow_class_context() {
    let source = r#"<?php
        class ReturnTypeRoot {}
        class ReturnTypeParent extends ReturnTypeRoot {
            public function selfValue(): self { return $this; }
            public function parentValue(): parent { return new ReturnTypeRoot(); }
            public function staticValue(): static { return new static(); }
            public static function staticFactory(): static { return new static(); }
            public function acceptsSelf(self $candidate): bool { return true; }
            public function acceptsParent(parent $candidate): bool { return true; }
            public function invalidStaticValue(): static { return new ReturnTypeParent(); }
        }
        class ReturnTypeChild extends ReturnTypeParent {}
        $value = new ReturnTypeChild();
        echo get_class($value->selfValue()), ':', get_class($value->parentValue()), ':', get_class($value->staticValue()), ':';
        echo get_class(ReturnTypeChild::staticFactory()), ':', $value->acceptsSelf(new ReturnTypeChild()), ':', $value->acceptsParent(new ReturnTypeRoot()), ':';
        $instanceCallable = [$value, 'staticValue'];
        $staticCallable = ['ReturnTypeChild', 'staticFactory'];
        echo get_class($instanceCallable()), ':', get_class($staticCallable()), ':';
        try { $value->invalidStaticValue(); }
        catch (TypeError $error) { echo get_class($error); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ReturnTypeChild:ReturnTypeRoot:ReturnTypeChild:ReturnTypeChild:1:1:ReturnTypeChild:ReturnTypeChild:TypeError");
}

#[test]
fn self_and_parent_property_types_resolve_for_instance_and_static_storage() {
    let source = r#"<?php
        class PropertyTypeRoot {}
        trait HasPropertyTypeSelf { public self $traitChild; }
        class PropertyTypeChild extends PropertyTypeRoot {
            use HasPropertyTypeSelf;
            public self $child;
            public parent $root;
            public static self $staticChild;
            public static parent $staticRoot;
        }
        $child = new PropertyTypeChild();
        $child->child = $child;
        $child->root = new PropertyTypeRoot();
        $child->traitChild = $child;
        PropertyTypeChild::$staticChild = $child;
        PropertyTypeChild::$staticRoot = new PropertyTypeRoot();
        echo 'assigned:';
        try { $child->child = new PropertyTypeRoot(); }
        catch (TypeError $error) { echo get_class($error), ':'; }
        try { $child->traitChild = new PropertyTypeRoot(); }
        catch (TypeError $error) { echo get_class($error); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "assigned:TypeError:TypeError");
}

#[test]
fn string_sorts_use_object_tostring_values() {
    let source = r#"<?php
        class SortLabel {
            public $label;
            public function __construct($label) { $this->label = $label; }
            public function __toString() { return $this->label; }
        }
        $values = [new SortLabel('zeta'), new SortLabel('alpha')];
        sort($values, SORT_STRING);
        echo $values[0]->label, ':', $values[1]->label, ':';
        $natural = [new SortLabel('item10'), new SortLabel('item2')];
        natsort($natural);
        echo array_values($natural)[0]->label, ':', array_values($natural)[1]->label, ':';
        $unique = array_unique([new SortLabel('same'), new SortLabel('same')]);
        echo count($unique), ':', $unique[0]->label;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "alpha:zeta:item2:item10:1:same");
}

#[test]
fn tostring_classes_automatically_implement_stringable() {
    let source = r#"<?php
        class LabelValue { public function __toString(): string { return 'label'; } }
        class ExplicitLabel implements Stringable { public function __toString(): string { return 'explicit'; } }
        $implicit = new LabelValue();
        $explicit = new ExplicitLabel();
        echo interface_exists('Stringable'), ':', method_exists('Stringable', '__toString'), ':';
        echo ($implicit instanceof Stringable), ':', is_a($implicit, 'Stringable'), ':';
        echo ($explicit instanceof Stringable), ':', (string) $implicit, ':', (string) $explicit;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:1:1:1:1:label:explicit");
}

#[test]
fn destructors_run_once_in_reverse_creation_order_at_script_shutdown() {
    let source = r#"<?php
        class DestructorTrace {
            private $label;
            public function __construct($label) { $this->label = $label; }
            public function __destruct() { echo 'destroy:', $this->label, "\n"; }
        }
        $first = new DestructorTrace('first');
        $second = new DestructorTrace('second');
        echo "body\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "body\ndestroy:second\ndestroy:first\n");
    for invalid in [
        "<?php class StaticDestructor { public static function __destruct() {} }",
        "<?php class ParameterizedDestructor { public function __destruct($value) {} }",
    ] {
        let error = compile_source(invalid).err().expect("malformed destructor must fail compilation");
        assert!(error.contains("__destruct() must be"));
    }
    for invalid in [
        "<?php class TypedDestructor { public function __destruct(): void {} }",
        "<?php class TypedConstructor { public function __construct(): void {} }",
    ] {
        let error = compile_source(invalid).err().expect("constructor/destructor return type must fail compilation");
        assert!(error.contains("cannot declare a return type"), "{error}");
    }
}

#[test]
fn private_and_protected_destructors_run_when_objects_are_destroyed() {
    let source = r#"<?php
        class PrivateDestructor { private function __destruct() { echo 'private:', "\n"; } }
        class ProtectedDestructor { protected function __destruct() { echo 'protected:', "\n"; } }
        $private = new PrivateDestructor();
        $protected = new ProtectedDestructor();
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "protected:\nprivate:\n");
}

#[test]
fn destructor_runs_when_unset_releases_the_last_variable_reference() {
    let source = r#"<?php
        class UnsetDestructor {
            public function __destruct() { echo "destroyed\n"; }
        }
        $value = new UnsetDestructor();
        echo "before\n";
        unset($value);
        echo "after\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "before\ndestroyed\nafter\n");
}

#[test]
fn destructor_runs_when_unset_removes_last_array_or_property_element_reference() {
    let source = r#"<?php
        class ElementDestructor {
            public $label;
            public function __construct($label) { $this->label = $label; }
            public function __destruct() { echo 'drop:', $this->label, "\n"; }
        }
        class DestructorHolder { public $items = []; }
        class StaticDestructorHolder { public static $items = []; }
        $items = [new ElementDestructor('array')];
        unset($items[0]);
        echo "array-unset-done\n";
        $holder = new DestructorHolder();
        $holder->items = [new ElementDestructor('property-array')];
        unset($holder->items[0]);
        echo "property-unset-done\n";
        StaticDestructorHolder::$items = [new ElementDestructor('static-array')];
        unset(StaticDestructorHolder::$items[0]);
        echo "static-unset-done\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "drop:array\narray-unset-done\ndrop:property-array\nproperty-unset-done\ndrop:static-array\nstatic-unset-done\n"
    );
}

#[test]
fn destructor_runs_on_reassignment_only_after_last_alias_is_replaced() {
    let source = r#"<?php
        class ReassignmentDestructor {
            public $label;
            public function __construct($label) { $this->label = $label; }
            public function __destruct() { echo 'drop:', $this->label, "\n"; }
        }
        class ReassignmentHolder { public $item; }
        class StaticReassignmentHolder { public static $item; }
        $value = new ReassignmentDestructor('direct');
        $value = new ReassignmentDestructor('final');
        echo "after-direct\n";
        $alias = $value;
        $value = new ReassignmentDestructor('replacement');
        echo "alias-still-live\n";
        $alias = null;
        echo "after-alias\n";
        $holder = new ReassignmentHolder();
        $holder->item = new ReassignmentDestructor('property-old');
        $holder->item = new ReassignmentDestructor('property-new');
        echo "after-property\n";
        StaticReassignmentHolder::$item = new ReassignmentDestructor('static-old');
        StaticReassignmentHolder::$item = new ReassignmentDestructor('static-new');
        echo "after-static\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "drop:direct\nafter-direct\nalias-still-live\ndrop:final\nafter-alias\ndrop:property-old\nafter-property\ndrop:static-old\nafter-static\ndrop:static-new\ndrop:property-new\ndrop:replacement\n"
    );
}

#[test]
fn destructor_tracks_objects_replaced_in_array_slots_and_preserves_copied_array_aliases() {
    let source = r#"<?php
        class ArraySlotDestructor {
            public $label;
            public function __construct($label) { $this->label = $label; }
            public function __destruct() { echo 'drop:', $this->label, "\n"; }
        }
        class ArraySlotHolder { public $items = []; }
        class StaticArraySlotHolder { public static $items = []; }
        $items = [new ArraySlotDestructor('local-old')];
        $items[0] = new ArraySlotDestructor('local-new');
        echo "local-replaced\n";
        $copy = [new ArraySlotDestructor('shared-old')];
        $otherCopy = $copy;
        $copy[0] = new ArraySlotDestructor('copy-new');
        echo "other-copy-live\n";
        unset($otherCopy[0]);
        echo "other-copy-released\n";
        $root = [new ArraySlotDestructor('root-alias-old')];
        $rootAlias = $root;
        $root[0] = new ArraySlotDestructor('root-new');
        echo "root-alias-live\n";
        $rootAlias = null;
        echo "root-alias-released\n";
        $holder = new ArraySlotHolder();
        $holder->items = [new ArraySlotDestructor('property-old')];
        $holder->items[0] = new ArraySlotDestructor('property-new');
        echo "property-replaced\n";
        StaticArraySlotHolder::$items = [new ArraySlotDestructor('static-old')];
        StaticArraySlotHolder::$items[0] = new ArraySlotDestructor('static-new');
        echo "static-replaced\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "drop:local-old\nlocal-replaced\nother-copy-live\ndrop:shared-old\nother-copy-released\nroot-alias-live\ndrop:root-alias-old\nroot-alias-released\ndrop:property-old\nproperty-replaced\ndrop:static-old\nstatic-replaced\ndrop:static-new\ndrop:property-new\ndrop:root-new\ndrop:copy-new\ndrop:local-new\n"
    );
}

#[test]
fn dynamic_static_property_array_paths_support_compound_assignment() {
    let source = r#"<?php
        class StaticCounters { public static $values; }
        $class = 'StaticCounters';
        $property = 'values';
        $class::${$property}['hits'] = 2;
        $class::${$property}['hits'] += 3;
        echo $class::${$property}['hits'];
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "5");
}

#[test]
fn object_property_array_paths_support_compound_assignment() {
    let source = r#"<?php
        class SectionStore {
            public array $sections = ['main' => 'hello'];
            public function append(string $key, string $text): void { $this->sections[$key] .= $text; }
        }
        $store = new SectionStore(); $store->append('main', ' world'); echo $store->sections['main'], "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "hello world\n");
}

#[test]
fn namespace_constant_imports_preserve_case_and_grouped_aliases() {
    let source = r#"<?php
        namespace Library;
        define('Library\CamelCase', 'one');
        define('Library\OtherValue', 'two');
        namespace Consumer;
        use const Library\CamelCase as LocalValue;
        use const Library\{OtherValue as SecondValue};
        echo LocalValue, ':', SecondValue, "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "one:two\n");
}

#[test]
fn grouped_namespace_imports_can_mix_classes_functions_and_constants() {
    let source = r#"<?php
        namespace Library;
        define('Library\\CamelCase', 'mixed');
        function greeting($name) { return 'hello:' . $name; }
        class Exported { public static function label() { return 'class'; } }
        namespace Consumer;
        use Library\{const CamelCase as LocalValue, function greeting as sayHello, Exported as ImportedClass};
        echo LocalValue, ':', sayHello('PHP'), ':', ImportedClass::label(), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "mixed:hello:PHP:class\n");
}

#[test]
fn namespace_magic_constant_tracks_named_and_global_namespace_blocks() {
    let source = r#"<?php
        namespace App\Tools { echo __NAMESPACE__, "\n"; }
        namespace Tests { echo __NAMESPACE__, "\n"; }
        namespace { echo '[', __NAMESPACE__, "]\n"; }
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "App\\Tools\nTests\n[]\n");
}

#[test]
fn line_magic_constant_reports_the_expression_source_line() {
    let source = "<?php\necho __LINE__, \"\\n\";\necho\n    __LINE__, \"\\n\";\n";
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "2\n4\n");
}

#[test]
fn thrown_exception_records_the_throw_statement_line() {
    let source = "<?php\ntry {\n    $error = new RuntimeException('boom');\n    throw $error;\n} catch (RuntimeException $caught) {\n    echo $caught->getLine(), \"\\n\";\n}\n";
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "3\n");
}

#[test]
fn native_runtime_exceptions_capture_source_line_and_active_call_frames() {
    let source = r#"<?php
        function triggerNativeFailure() { return 7 / 0; }
        try { triggerNativeFailure(); }
        catch (DivisionByZeroError $error) {
            $trace = $error->getTrace();
            echo $error->getLine(), ':', $trace[0]['function'], ':', count($trace);
        }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2:triggerNativeFailure:1");
}

#[test]
fn builtin_exception_objects_expose_core_fields_and_throwable_hierarchy() {
    let source = r#"<?php
        $base = new Exception('bad input', 12);
        echo $base->getMessage(), ':', $base->getCode(), ':', $base, ':', is_a($base, 'Throwable'), "\n";
        $error = new TypeError('wrong type');
        echo get_class($error), ':', get_parent_class($error), ':', ($error instanceof Error), ':', class_exists('Exception'), ':', interface_exists('Throwable'), ':', method_exists('Throwable', 'getMessage') ? 1 : 0, ':', method_exists('Throwable', '__construct') ? 1 : 0, ':', method_exists('Throwable', '__toString') ? 1 : 0, "\n";
        try { throw $error; }
        catch (Exception $wrong) { echo "wrong catch\n"; }
        catch (Throwable $caught) { echo $caught->getMessage(), "\n"; }
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "bad input:12:Exception: bad input:1\nTypeError:Error:1:1:1:1:0:0\nwrong type\n");
}

#[test]
fn standard_exception_families_have_php_parent_classes_and_catch_behavior() {
    let source = r#"<?php
        $logic = new BadMethodCallException('bad call');
        $runtime = new OutOfBoundsException('missing');
        echo get_parent_class($logic), ':', get_parent_class('BadMethodCallException'), ':', $logic instanceof LogicException, ':', $logic instanceof Exception, "\n";
        echo get_parent_class($runtime), ':', $runtime instanceof RuntimeException, ':', class_exists('UnexpectedValueException'), "\n";
        try { throw $runtime; }
        catch (LogicException $wrong) { echo "wrong\n"; }
        catch (RuntimeException $caught) { echo $caught->getMessage(), "\n"; }
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "BadFunctionCallException:BadFunctionCallException:1:1\nRuntimeException:1:1\nmissing\n");
}

#[test]
fn native_exception_constructors_bind_named_arguments() {
    let source = r#"<?php
        $previous = new LogicException('inner');
        $error = new RuntimeException(previous: $previous, code: 9, message: 'outer');
        echo $error->getMessage(), ':', $error->getCode(), ':', $error->getPrevious()->getMessage(), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "outer:9:inner\n");
}

#[test]
fn native_exception_methods_reject_unknown_named_arguments() {
    for method in ["getMessage", "getTraceAsString"] {
        let source = format!("<?php (new Exception('x'))->{method}(unknown: true);");
        let error = compile_source(&source).unwrap().execute().unwrap_err();
        assert!(error.contains("unknown named parameter '$unknown'"), "{method}: {error}");
    }
}

#[test]
fn user_exception_subclasses_inherit_native_state_and_catch_types() {
    let source = r#"<?php
        class DomainFailure extends Exception {
            public function __construct($message) { parent::__construct($message, 23); }
        }
        class DomainTypeFailure extends TypeError {}
        $failure = new DomainFailure('domain');
        $typeFailure = new DomainTypeFailure('type');
        echo get_class($failure), ':', $failure->getMessage(), ':', $failure->getCode(), ':', ($failure instanceof Throwable), ':', method_exists($failure, 'getMessage'), ':', method_exists('DomainFailure', 'getPrevious'), ':', method_exists($failure, '__construct'), ':', is_callable([$failure, 'getMessage']), "\n";
        try { throw $typeFailure; }
        catch (Exception $wrong) { echo "wrong exception\n"; }
        catch (Error $caught) { echo $caught->getMessage(), ':', get_parent_class($caught), "\n"; }
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "DomainFailure:domain:23:1:1:1:1:1\ntype:TypeError\n");
}

#[test]
fn invalid_throw_operands_become_errors_and_only_throwables_match_catches() {
    let source = r#"<?php
        class OrdinaryObject {}
        try { throw 'text'; }
        catch (Exception $wrong) { echo "wrong exception\n"; }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(), "\n"; }
        try { throw new OrdinaryObject(); }
        catch (Exception $wrong) { echo "wrong object catch\n"; }
        catch (Throwable $error) { echo get_class($error), ':', $error->getMessage(), "\n"; }
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "Error:Can only throw objects that implement Throwable\nError:Can only throw objects that implement Throwable\n");
}

#[test]
fn throwable_cannot_be_implemented_directly_but_can_be_extended_by_an_interface() {
    let class_error = compile_source("<?php class InvalidThrowable implements Throwable {}").err().expect("direct Throwable implementation must fail");
    assert!(class_error.contains("cannot implement interface Throwable directly"));
    compile_source("<?php interface InvalidThrowable extends Throwable {}")
        .expect("user interfaces may extend Throwable");
}

#[test]
fn exception_previous_argument_must_implement_throwable() {
    let invalid = compile_source("<?php class PlainObject {} new Exception('outer', 0, new PlainObject());")
        .unwrap().execute().unwrap_err();
    assert!(invalid.contains("Argument #3 ($previous) must be of type ?Throwable"));
    let valid = compile_source("<?php class DomainFailure extends Exception {} $previous = new DomainFailure('inner'); $outer = new Exception('outer', 0, $previous); echo $outer->getPrevious()->getMessage();")
        .unwrap().execute().unwrap();
    assert_eq!(valid, "inner");
}

#[test]
fn reference_returning_functions_and_closures_preserve_aliases_only_when_requested() {
    let source = r#"<?php
        $shared = 2;
        function &sharedValue() { global $shared; return $shared; }
        $alias =& sharedValue();
        $alias = 10;
        echo $shared, ':', $alias, "\n";
        $copy = sharedValue();
        $copy = 20;
        echo $shared, ':', $copy, "\n";
        $closure = function &() use (&$shared) { return $shared; };
        $closureAlias =& $closure();
        $closureAlias = 30;
        echo $shared, ':', $closureAlias, "\n";
        $items = ['before'];
        function &firstItem() { global $items; return $items[0]; }
        $item =& firstItem();
        $item = 'after';
        echo $items[0], "\n";
        interface ReferenceProvider { public function &valueReference(); }
        class ReferenceBox implements ReferenceProvider {
            public $value = 'box';
            public function &valueReference() { return $this->value; }
        }
        $box = new ReferenceBox();
        $boxAlias =& $box->valueReference();
        $boxAlias = 'changed';
        echo $box->value, "\n";
        $box->value = 'property assignment';
        echo $boxAlias, "\n";
        $copyArray = [sharedValue()];
        $copyArray[0] = 40;
        echo $shared, ':', $copyArray[0], "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "10:10\n10:20\n30:30\nafter\nchanged\nproperty assignment\n30:40\n");
}

#[test]
fn reference_returns_reject_non_reference_results_and_non_reference_callables() {
    let invalid_return = compile_source("<?php function &badReference() { return 42; } badReference();").unwrap();
    assert_eq!(invalid_return.execute().unwrap_err(), "badreference() must return a reference");
    let invalid_assignment = compile_source("<?php function valueOnly() { return 42; } $value =& valueOnly();").unwrap();
    assert_eq!(invalid_assignment.execute().unwrap_err(), "Only a reference-returning function can be assigned by reference");
}

#[test]
fn union_catch_matches_each_declared_class_and_keeps_finally_order() {
    let source = r#"<?php
        class FirstFailure extends Exception {}
        class SecondFailure extends Exception {}
        class ThirdFailure extends Exception {}
        try {
            throw new SecondFailure();
        } catch (FirstFailure $error) {
            echo "wrong catch\n";
        } catch (SecondFailure|ThirdFailure $error) {
            echo "caught second clause\n";
        } finally { echo "outer finally\n"; }
        try { throw new FirstFailure(); }
        catch (FirstFailure) { echo "caught without binding\n"; }
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "caught second clause\nouter finally\ncaught without binding\n");
}

#[test]
fn instanceof_checks_classes_parents_interfaces_and_runtime_names() {
    let source = r#"<?php
        interface Renderable {}
        class BaseView {}
        class PageView extends BaseView implements Renderable {
            public function matchesSelf($value) { return $value instanceof self; }
            public function matchesParent($value) { return $value instanceof parent; }
            public function matchesStatic($value) { return $value instanceof static; }
        }
        class DetailedPageView extends PageView {}
        $view = new PageView();
        $detailedView = new DetailedPageView();
        $parentName = 'BaseView';
        echo ($view instanceof PageView), ':', ($view instanceof BaseView), ':', ($view instanceof Renderable), ':', ($view instanceof $parentName), ':', (42 instanceof PageView), ':', $view->matchesSelf($view), ':', $view->matchesParent(new BaseView()), ':', $detailedView->matchesStatic($detailedView), ':', $detailedView->matchesStatic($view), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "1:1:1:1::1:1:1:\n");
}

#[test]
fn class_introspection_preserves_declared_name_and_method_context() {
    let source = r#"<?php
        class ParentObject {}
        class FancyObject extends ParentObject {
            public function classInfo() { return get_class() . ':' . get_parent_class(); }
        }
        $object = new FancyObject();
        echo get_class($object), ':', get_parent_class($object), ':', get_parent_class('FancyObject'), ':', $object->classInfo(), ':', get_class(object: $object), ':', get_parent_class(object_or_class: $object), ':', function_exists('get_class'), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "FancyObject:ParentObject:ParentObject:FancyObject:ParentObject:FancyObject:ParentObject:1\n");
}

#[test]
fn is_a_checks_object_hierarchy_interfaces_and_allowed_class_strings() {
    let source = r#"<?php
        interface SerializableView {}
        class RootView {}
        class CardView extends RootView implements SerializableView {}
        $view = new CardView();
        $className = 'CardView';
        echo is_a($view, 'CardView'), ':', is_a($view, 'RootView'), ':', is_a($view, 'SerializableView'), ':', is_a($view, 'MissingView'), ':', is_a($className, 'RootView'), ':', is_a($className, 'RootView', true), ':', is_a(object_or_class: $view, class: 'RootView'), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "1:1:1:::1:1\n");
}

#[test]
fn is_subclass_of_excludes_same_type_and_honors_allow_string() {
    let source = r#"<?php
        interface Savable {}
        class Entity {}
        class UserEntity extends Entity implements Savable {}
        $user = new UserEntity();
        echo is_subclass_of($user, 'UserEntity'), ':', is_subclass_of($user, 'Entity'), ':', is_subclass_of($user, 'Savable'), ':', is_subclass_of('UserEntity', 'Entity'), ':', is_subclass_of('UserEntity', 'Entity', true), ':', is_subclass_of(object_or_class: $user, class: 'Entity'), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, ":1:1::1:1\n");
}

#[test]
fn property_exists_sees_inherited_private_static_null_and_dynamic_properties() {
    let source = r#"<?php
        class PropertyBase { private $hidden; protected $empty = null; public static $shared; }
        class PropertyChild extends PropertyBase { public $late; }
        $item = new PropertyChild();
        $dynamic = json_decode('{"dynamicvalue":1}');
        echo property_exists($item, 'hidden'), ':', property_exists($item, 'empty'), ':', property_exists($item, 'shared'), ':', property_exists($item, 'late'), ':', property_exists($item, 'dynamicvalue'), ':', property_exists($dynamic, 'dynamicvalue'), ':', property_exists($item, 'missing'), ':', property_exists('PropertyChild', 'hidden'), ':', property_exists('PropertyChild', 'shared'), ':', property_exists(object_or_class: $item, property: 'late'), ':', function_exists('property_exists'), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "1:1:1:1::1::1:1:1:1\n");
}
#[test]
fn nested_isset_on_overloaded_property_calls_isset_before_get() {
    let source = r#"<?php
        class OverloadedArray {
            public $calls = [];
            public function __isset($name) { $this->calls[] = 'isset:' . $name; return true; }
            public function __get($name) { $this->calls[] = 'get:' . $name; return ['key' => ['nested' => 7]]; }
        }
        $value = new OverloadedArray();
        echo isset($value->data['key']), '|', implode(',', $value->calls), "\n";
        echo isset($value->data['missing']), '|', implode(',', $value->calls), "\n";
        $property = 'data';
        echo isset($value->{$property}['key']['nested']), '|', implode(',', $value->calls), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1|isset:data,get:data\n|isset:data,get:data,isset:data,get:data\n1|isset:data,get:data,isset:data,get:data,isset:data,get:data\n"
    );
}

#[test]
fn nested_overloaded_property_chains_call_isset_before_each_intermediate_get() {
    let source = r#"<?php
        $calls = [];
        class NestedOverload {
            public function __isset($name) { $GLOBALS['calls'][] = 'isset:' . $name; return $name !== 'stop'; }
            public function __get($name) { $GLOBALS['calls'][] = 'get:' . $name; return new NestedOverload(); }
        }
        $value = new NestedOverload();
        echo isset($value->first->second), ':', implode(',', $calls), "\n";
        $calls = [];
        echo isset($value->stop->never), ':', implode(',', $calls), "\n";
        $calls = [];
        $first = 'first'; $second = 'second';
        echo isset($value->{$first}->{$second}), ':', implode(',', $calls), "\n";
        $calls = [];
        echo isset($value->first->{$second}), ':', implode(',', $calls), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:isset:first,get:first,isset:second\n:isset:stop\n1:isset:first,get:first,isset:second\n1:isset:first,get:first,isset:second\n"
    );
}

#[test]
fn nested_isset_property_chains_preserve_magic_dispatch_across_array_offsets() {
    let source = r#"<?php
        $calls = [];
        class OffsetBetweenMagicProperties {
            public function __isset($name) { $GLOBALS['calls'][] = 'isset:' . $name; return true; }
            public function __get($name) {
                $GLOBALS['calls'][] = 'get:' . $name;
                return ['child' => new OffsetBetweenMagicProperties()];
            }
        }
        $value = new OffsetBetweenMagicProperties();
        echo isset($value->first['child']->second), ':', implode(',', $calls), "\n";
        $calls = [];
        $first = 'first'; $second = 'second';
        echo isset($value->{$first}['child']->{$second}), ':', implode(',', $calls), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:isset:first,get:first,isset:second\n1:isset:first,get:first,isset:second\n"
    );
}

#[test]
fn isset_defers_array_offset_side_effects_until_overloaded_property_checks_pass() {
    let source = r#"<?php
        $calls = []; $offsetCount = 0;
        function nextOffset() { $GLOBALS['calls'][] = 'key'; $GLOBALS['offsetCount']++; return 'child'; }
        class ShortCircuitMagicOffset {
            public function __isset($name) { $GLOBALS['calls'][] = 'isset:' . $name; return $name !== 'stop'; }
            public function __get($name) { $GLOBALS['calls'][] = 'get:' . $name; return ['child' => new ShortCircuitMagicOffset()]; }
        }
        $value = new ShortCircuitMagicOffset();
        echo isset($value->first[nextOffset()]->stop[nextOffset()]->last), ':', implode('|', $calls), ':', $offsetCount, "\n";
        $calls = []; $offsetCount = 0;
        echo isset($value->stop[nextOffset()]->last), ':', implode('|', $calls), ':', $offsetCount, "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        ":isset:first|get:first|key|isset:stop:1\n:isset:stop:0\n"
    );
}

#[test]
fn by_reference_read_of_overloaded_property_uses_reference_returning_getter() {
    let source = r#"<?php
        class ReferenceGetter {
            private $value = 'before';
            public function &__get($name) { return $this->value; }
            public function read() { return $this->value; }
        }
        $object = new ReferenceGetter();
        $alias =& $object->missing;
        $alias = 'after';
        $object->dynamic = 'initial';
        $dynamicAlias =& $object->dynamic;
        $dynamicAlias = 'changed';
        echo $object->read(), ':', $object->dynamic;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "after:changed");
    let invalid = compile_source("<?php class ValueGetter { public function __get($name) { return 1; } } $object = new ValueGetter(); $alias =& $object->missing;").unwrap();
    assert!(invalid.execute().unwrap_err().contains("__get() must return by reference"));
}

#[test]
fn increment_of_overloaded_property_runs_getter_then_setter() {
    let source = r#"<?php
        class IncrementOverloaded {
            private $value = 4;
            public $calls = [];
            public function __get($name) { $this->calls[] = 'get:' . $name; return $this->value; }
            public function __set($name, $value) { $this->calls[] = 'set:' . $name . ':' . $value; $this->value = $value; }
            public function actual() { return $this->value; }
        }
        $object = new IncrementOverloaded();
        echo $object->missing++, ':', ++$object->missing, ':', $object->actual(), ':', implode('|', $object->calls);
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "4:6:6:get:missing|set:missing:5|get:missing|set:missing:6");
}

#[test]
fn reflection_class_reads_class_attributes_and_arguments() {
    let source = r#"<?php
        #[Attribute] class ExampleMarker { public $value; public function __construct($value) { $this->value = $value; } }
        #[Attribute] class ExampleOptions { public $mode; public $retries; public function __construct($mode, $retries) { $this->mode = $mode; $this->retries = $retries; } }
        #[ExampleMarker('first'), ExampleOptions(mode: 'safe', retries: 3)]
        class ReflectedService {}
        $reflection = new ReflectionClass(objectOrClass: 'ReflectedService');
        $all = $reflection->getAttributes();
        $filtered = $reflection->getAttributes(name: 'ExampleOptions');
        $arguments = $filtered[0]->getArguments();
        $instance = $filtered[0]->newInstance();
        $fromObject = new ReflectionClass(new ReflectedService());
        echo $reflection->getName(), ':', $fromObject->getName(), ':', count($all), ':', $all[0]->getName(), ':', $filtered[0]->getTarget(), ':', $filtered[0]->isRepeated(), ':', $arguments['mode'], ':', $arguments['retries'], ':', $instance->mode, ':', $instance->retries, ':', count($reflection->getAttributes('Missing')), ':', class_exists('ReflectionClass'), ':', method_exists('ReflectionAttribute', 'newInstance');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ReflectedService:ReflectedService:2:ExampleMarker:1::safe:3:safe:3:0:1:1");
    let invalid = compile_source("<?php class NotAnAttribute {} #[NotAnAttribute] class Holder {} (new ReflectionClass('Holder'))->getAttributes()[0]->newInstance();").unwrap();
    assert!(invalid.execute().unwrap_err().contains("Attempting to use non-attribute class"));
    for method in ["getTarget", "isRepeated"] {
        let source = format!("<?php #[Attribute] class Marker {{}} #[Marker] class Holder {{}} (new ReflectionClass('Holder'))->getAttributes()[0]->{method}(unexpected: true);");
        let error = compile_source(&source).unwrap().execute().unwrap_err();
        assert!(error.contains("unknown named parameter '$unexpected'"), "{method}: {error}");
    }
}

#[test]
fn reflection_class_reports_enum_interface_abstract_and_final_flags() {
    let source = r#"<?php
        interface ReflectableContract {}
        abstract class ReflectableBase {}
        final class ReflectableFinal implements ReflectableContract {}
        enum ReflectableEnum { case Ready; }
        $ordinary = new ReflectionClass('ReflectableFinal');
        $abstract = new ReflectionClass('ReflectableBase');
        $interface = new ReflectionClass('ReflectableContract');
        $enum = new ReflectionClass('ReflectableEnum');
        echo $ordinary->isEnum(), ':', $ordinary->isInterface(), ':', $ordinary->isAbstract(), ':', $ordinary->isFinal(), ':';
        echo $abstract->isAbstract(), ':', $interface->isInterface(), ':', $enum->isEnum(), ':';
        echo method_exists('ReflectionClass', 'isEnum'), ':', method_exists('ReflectionClass', 'isFinal');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), ":::1:1:1:1:1:1");
}

#[test]
fn reflection_class_returns_parent_metadata_or_false() {
    let source = r#"<?php
        class ReflectionParent {}
        class ReflectionChild extends ReflectionParent {}
        $parent = new ReflectionClass('ReflectionParent');
        $child = new ReflectionClass('ReflectionChild');
        echo $child->getParentClass()->getName(), ':', $parent->getParentClass(), ':', method_exists('ReflectionClass', 'getParentClass');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ReflectionParent::1");
}

#[test]
fn reflection_class_checks_parent_and_interface_relationships() {
    let source = r#"<?php
        interface ReflectsContract {}
        class ReflectsBase {}
        class ReflectsChild extends ReflectsBase implements ReflectsContract {}
        $reflection = new ReflectionClass('ReflectsChild');
        echo $reflection->isSubclassOf(class: 'ReflectsBase'), ':', $reflection->implementsInterface(interface: 'ReflectsContract'), ':';
        echo $reflection->isSubclassOf('ReflectsChild'), ':', $reflection->implementsInterface('ReflectsContract'), ':', method_exists('ReflectionClass', 'implementsInterface');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:1::1:1");
}

#[test]
fn attribute_target_and_repeatable_flags_are_enforced() {
    let source = r#"<?php
        #[Attribute(Attribute::TARGET_CLASS | Attribute::IS_REPEATABLE)]
        class RepeatableTag {}
        #[RepeatableTag, RepeatableTag]
        class MarkedTwice {}
        $attributes = (new ReflectionClass('MarkedTwice'))->getAttributes();
        echo count($attributes), ':', $attributes[1]->isRepeated(), ':', get_class($attributes[0]->newInstance());
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2:1:RepeatableTag");

    let invalid_target = compile_source("<?php #[Attribute(Attribute::TARGET_METHOD)] class MethodTag {} #[MethodTag] class InvalidTarget {} (new ReflectionClass('InvalidTarget'))->getAttributes()[0]->newInstance();").unwrap();
    assert!(invalid_target.execute().unwrap_err().contains("cannot target this declaration"));

    let invalid_repeat = compile_source("<?php #[Attribute(Attribute::TARGET_CLASS)] class SingleTag {} #[SingleTag, SingleTag] class RepeatedInvalid {} (new ReflectionClass('RepeatedInvalid'))->getAttributes()[0]->newInstance();").unwrap();
    assert!(invalid_repeat.execute().unwrap_err().contains("must not be repeated"));
}

#[test]
fn reflection_method_reads_method_attributes_and_instantiates_them() {
    let source = r#"<?php
        #[Attribute(Attribute::TARGET_METHOD)]
        class RouteTag { public $path; public function __construct($path) { $this->path = $path; } }
        class Controller { #[RouteTag('/items')] public function listItems() {} }
        $method = (new ReflectionClass('Controller'))->getMethod('listItems');
        $attributes = $method->getAttributes();
        $attribute = $attributes[0];
        $route = $attribute->newInstance();
        echo $method->getName(), ':', $attribute->getName(), ':', $attribute->getTarget(), ':', $attribute->getArguments()[0], ':', $route->path;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "listItems:RouteTag:4:/items:/items");
}

#[test]
fn reflection_property_reads_property_attributes_and_instantiates_them() {
    let source = r#"<?php
        #[Attribute(Attribute::TARGET_PROPERTY)]
        class ColumnTag { public $column; public function __construct($column) { $this->column = $column; } }
        class RecordModel { #[ColumnTag(column: 'record_id')] public int $id; }
        $property = (new ReflectionClass('RecordModel'))->getProperty('id');
        $attribute = $property->getAttributes(name: 'ColumnTag')[0];
        echo $property->getName(), ':', $attribute->getName(), ':', $attribute->getTarget(), ':', $attribute->getArguments()['column'], ':', $attribute->newInstance()->column;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "id:ColumnTag:8:record_id:record_id");
}

#[test]
fn reflection_parameter_reads_parameter_attributes_and_instantiates_them() {
    let source = r#"<?php
        #[Attribute(Attribute::TARGET_PARAMETER)]
        class ParamTag { public $label; public function __construct($label) { $this->label = $label; } }
        class Handler { public function run(#[ParamTag(label: 'payload')] $value, $other) {} }
        $method = (new ReflectionClass('Handler'))->getMethod('run');
        $parameters = $method->getParameters();
        $parameter = $parameters[0];
        $attribute = $parameter->getAttributes(name: 'ParamTag')[0];
        echo count($parameters), ':', $parameter->getName(), ':', $parameter->getPosition(), ':', $attribute->getTarget(), ':', $attribute->getArguments()['label'], ':', $attribute->newInstance()->label, ':', class_exists('ReflectionParameter'), ':', method_exists('ReflectionMethod', 'getParameters');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2:value:0:32:payload:payload:1:1");

    let invalid_target = compile_source("<?php #[Attribute(Attribute::TARGET_METHOD)] class MethodOnly {} class Subject { public function run(#[MethodOnly] $value) {} } (new ReflectionClass('Subject'))->getMethod('run')->getParameters()[0]->getAttributes()[0]->newInstance();").unwrap();
    assert!(invalid_target.execute().unwrap_err().contains("cannot target this declaration"));
}

#[test]
fn reflection_function_reads_function_attributes_and_instantiates_them() {
    let source = r#"<?php
        #[Attribute(Attribute::TARGET_FUNCTION)]
        class FunctionTag { public $label; public function __construct($label) { $this->label = $label; } }
        #[FunctionTag(label: 'worker')]
        function processTask() {}
        $reflection = new ReflectionFunction('processTask');
        $attribute = $reflection->getAttributes()[0];
        echo $reflection->getName(), ':', $attribute->getTarget(), ':', $attribute->getArguments()['label'], ':', $attribute->newInstance()->label, ':', class_exists('ReflectionFunction'), ':', method_exists('ReflectionFunction', 'getAttributes');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "processTask:2:worker:worker:1:1");

    let invalid_target = compile_source("<?php #[Attribute(Attribute::TARGET_METHOD)] class MethodOnly {} #[MethodOnly] function utility() {} (new ReflectionFunction('utility'))->getAttributes()[0]->newInstance();").unwrap();
    assert!(invalid_target.execute().unwrap_err().contains("cannot target this declaration"));
}

#[test]
fn reflection_class_reads_interface_attributes() {
    let source = r#"<?php
        #[Attribute(Attribute::TARGET_CLASS)]
        class ContractTag { public $name; public function __construct($name) { $this->name = $name; } }
        #[ContractTag('cacheable')]
        interface CacheableContract {}
        $attribute = (new ReflectionClass('CacheableContract'))->getAttributes()[0];
        echo $attribute->getTarget(), ':', $attribute->newInstance()->name;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:cacheable");
}

#[test]
fn anonymous_class_attributes_are_reflectable() {
    let source = r#"<?php
        #[Attribute(Attribute::TARGET_CLASS)]
        class AnonymousTag { public $name; public function __construct($name) { $this->name = $name; } }
        $object = new #[AnonymousTag('ephemeral')] class {};
        $attribute = (new ReflectionClass($object))->getAttributes()[0];
        echo $attribute->getTarget(), ':', $attribute->newInstance()->name;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:ephemeral");
}

#[test]
fn parent_and_child_private_properties_with_the_same_name_have_distinct_slots() {
    let source = r#"<?php
        class PrivateParent {
            private $value = 'parent';
            public function readParent() { return $this->value; }
            public function writeParent($value) { $this->value = $value; }
        }
        class PrivateChild extends PrivateParent {
            private $value = 'child';
            public function readChild() { return $this->value; }
            public function writeChild($value) { $this->value = $value; }
        }
        $value = new PrivateChild();
        echo $value->readParent(), ':', $value->readChild(), "\n";
        $value->writeParent('parent-updated');
        $value->writeChild('child-updated');
        echo $value->readParent(), ':', $value->readChild(), "\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "parent:child\nparent-updated:child-updated\n"
    );
}

#[test]
fn by_reference_arguments_bind_to_the_private_property_slot_of_the_calling_scope() {
    let source = r#"<?php
        function appendMark(&$value) { $value .= '!'; }
        class ReferenceParent {
            private $value = 'parent';
            public function updateParent() { appendMark($this->value); }
            public function readParent() { return $this->value; }
        }
        class ReferenceChild extends ReferenceParent {
            private $value = 'child';
            public function updateChild() { appendMark($this->value); }
            public function readChild() { return $this->value; }
        }
        $value = new ReferenceChild();
        $value->updateParent();
        $value->updateChild();
        echo $value->readParent(), ':', $value->readChild(), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "parent!:child!\n");
}

#[test]
fn by_reference_arguments_bind_nested_offsets_of_object_properties() {
    let source = r#"<?php
        function replaceValue(&$value) { $value = 'shared'; }
        class ArrayReferenceProperty {
            private $values = ['item' => 'before'];
            public function update() { replaceValue($this->values['item']); }
            public function read() { return $this->values['item']; }
        }
        $box = new ArrayReferenceProperty();
        $box->update();
        echo $box->read(), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "shared\n");
}

#[test]
fn dynamic_object_property_array_offsets_bind_variable_references() {
    let source = r#"<?php
        class DynamicReferenceContainer { public $values = ['item' => 'before']; }
        $container = new DynamicReferenceContainer();
        $property = 'values';
        $source = 'shared';
        $container->{$property}['item'] =& $source;
        $source = 'from-variable';
        echo $container->values['item'], ':';
        $container->values['item'] = 'from-property';
        echo $source, "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "from-variable:from-property\n");
}

#[test]
fn private_object_property_array_references_respect_calling_scope() {
    let source = r#"<?php
        class PrivateReferenceParent {
            private $values = ['item' => 'parent'];
            public function bind(&$source) { $this->values['item'] =& $source; }
            public function read() { return $this->values['item']; }
        }
        class PrivateReferenceChild extends PrivateReferenceParent {
            private $values = ['item' => 'child'];
            public function bindChild(&$source) { $this->values['item'] =& $source; }
            public function readChild() { return $this->values['item']; }
        }
        $box = new PrivateReferenceChild();
        $parent = 'p'; $child = 'c';
        $box->bind($parent); $box->bindChild($child);
        $parent = 'parent alias'; $child = 'child alias';
        echo $box->read(), ':', $box->readChild(), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "parent alias:child alias\n");
}

#[test]
fn inaccessible_private_object_property_array_reference_raises_error() {
    let source = r#"<?php
        class InaccessibleReferenceBox { private $values = ['item' => 'before']; }
        $box = new InaccessibleReferenceBox();
        $source = 'after';
        try { $box->values['item'] =& $source; }
        catch (Error $error) { echo get_class($error), "\n"; }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "Error\n");
}

#[test]
fn empty_on_overloaded_properties_calls_isset_then_get_only_when_present() {
    let source = r#"<?php
        class EmptyMagicProperties {
            public function __isset($name) { echo 'i', $name, ';'; return $name !== 'missing'; }
            public function __get($name) {
                echo 'g', $name, ';';
                if ($name === 'nested') return ['value' => 0];
                return 0;
            }
        }
        $box = new EmptyMagicProperties();
        $name = 'zero';
        echo empty($box->missing) ? 'yes:' : 'no:';
        echo empty($box->zero) ? 'yes:' : 'no:';
        echo empty($box->nested['value']) ? 'yes:' : 'no:';
        echo empty($box->nested['absent']) ? 'yes:' : 'no:';
        echo empty($box->{$name}) ? 'yes:' : 'no:';
        $nestedName = 'nested';
        echo empty($box->{$nestedName}['value']) ? 'yes' : 'no', "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "imissing;yes:izero;gzero;yes:inested;gnested;yes:inested;gnested;yes:izero;gzero;yes:inested;gnested;yes\n");
}

#[test]
fn typed_property_array_references_bind_and_readonly_targets_raise_catchable_error() {
    let source = r#"<?php
        class TypedReferenceBox {
            public array $values = ['item' => 'before'];
            public function bind(&$value) { $this->values['item'] =& $value; }
        }
        $typed = new TypedReferenceBox();
        $source = 'after';
        $typed->bind($source);
        $source = 'shared';
        echo $typed->values['item'], ':';
        class ReadonlyReferenceBox {
            public readonly array $values;
            public function __construct() { $this->values = ['item' => 'before']; }
        }
        $readonly = new ReadonlyReferenceBox();
        try { $readonly->values['item'] =& $source; }
        catch (Error $error) { echo get_class($error), "\n"; }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "shared:Error\n");
}

#[test]
fn direct_typed_and_readonly_property_references_use_php_errors() {
    let source = r#"<?php
        class TypedAliasTarget { public int $value = 4; }
        class ReadonlyAliasTarget {
            public readonly int $value;
            public function __construct() { $this->value = 5; }
        }
        $typed = new TypedAliasTarget();
        $readonly = new ReadonlyAliasTarget();
        try { $alias =& $typed->value; }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(), "\n"; }
        try { $alias =& $readonly->value; }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(), "\n"; }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(),
        "Error:Typed property TypedAliasTarget::$value must not be referenced\nError:Cannot indirectly modify readonly property ReadonlyAliasTarget::$value\n");
}

#[test]
fn typed_static_property_reference_uses_typed_property_error() {
    let source = r#"<?php
        class StaticTypedAliasTarget { public static int $value = 4; }
        function touch(&$value) { $value = 8; }
        class StaticTypedAliasGetter {
            public static function &get() { return StaticTypedAliasTarget::$value; }
        }
        class StaticRefArgument { public static $value = 4; }
        class StaticArrayRefArgument { public static $values = ['nested' => ['item' => 5]]; }
        function touch_static(&$value) { $value = 9; }
        function touch_static_array_item(&$value) { $value = 12; }
        touch_static(StaticRefArgument::$value);
        echo StaticRefArgument::$value, "\n";
        touch_static_array_item(StaticArrayRefArgument::$values['nested']['item']);
        $staticItemAlias =& StaticArrayRefArgument::$values['nested']['item'];
        $staticItemAlias++;
        echo StaticArrayRefArgument::$values['nested']['item'], "\n";
        try { $alias =& StaticTypedAliasGetter::get(); }
        catch (Error $error) { echo $error->getMessage(), "\n"; }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(),
        "9\n13\nTyped property statictypedaliastarget::$value must not be referenced\n");
}

#[test]
fn by_reference_overloaded_read_of_typed_property_checks_getter_value() {
    let source = r#"<?php
        class TypedOverloadedReference {
            private int $value = 6;
            private $backing = 7;
            public function &__get($name) { return $this->backing; }
        }
        $object = new TypedOverloadedReference();
        $alias =& $object->value;
        $alias = 'untyped alias';
        echo $alias;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "untyped alias");
}

#[test]
fn by_value_overloaded_read_of_typed_property_checks_getter_value() {
    let source = r#"<?php
        class InvalidTypedGetter {
            private int $value = 1;
            public function __get($name) { return 'wrong'; }
        }
        try { echo (new InvalidTypedGetter())->value; }
        catch (TypeError $error) { echo get_class($error), ':', $error->getMessage(); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(),
        "TypeError:Value of type string returned from InvalidTypedGetter::__get() must be compatible with unset property InvalidTypedGetter::$value of type int");
}

#[test]
fn invoke_magic_method_supports_callable_objects_and_array_callbacks() {
    let source = r#"<?php
        class PrefixCallable {
            public function __invoke($value, $suffix = '!') { return $value . $suffix; }
        }
        $callable = new PrefixCallable();
        echo is_callable($callable), ':', $callable('direct'), ':', $callable(value: 'named'), ':';
        echo implode(',', array_map($callable, ['a', 'b'])), ':';
        class WalkCallable { public function __invoke(&$value, $key) { $value = $key . $value; } }
        $values = ['x' => 'a'];
        array_walk($values, new WalkCallable());
        echo $values['x'], "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:direct!:named!:a!,b!:xa\n");
}

#[test]
fn magic_invoke_and_to_string_reject_invalid_visibility_or_staticness() {
    for source in [
        "<?php class InvalidInvoker { private function __invoke() {} }",
        "<?php class StaticInvoker { public static function __invoke() {} }",
        "<?php class PrivateStringifier { private function __toString() { return 'x'; } }",
        "<?php class StaticStringifier { public static function __toString() { return 'x'; } }",
        "<?php class StringifierWithArgument { public function __toString($value) { return 'x'; } }",
        "<?php class StringifierWithWrongReturnType { public function __toString(): int { return 1; } }",
    ] {
        assert!(compile_source(source).is_err(), "accepted invalid magic declaration: {source}");
    }
}

#[test]
fn enum_implements_unit_enum_and_backed_enum_interfaces() {
    let source = r#"<?php
        enum PlainStatus { case Ready; }
        enum HttpStatus: int { case Ok = 200; }
        echo interface_exists('UnitEnum'), ':', interface_exists('BackedEnum'), ':';
        echo method_exists('UnitEnum', 'cases'), ':', method_exists('BackedEnum', 'from'), ':';
        echo (PlainStatus::Ready instanceof UnitEnum), ':', (PlainStatus::Ready instanceof BackedEnum), ':';
        echo (HttpStatus::Ok instanceof UnitEnum), ':', (HttpStatus::Ok instanceof BackedEnum), ':';
        echo is_a(HttpStatus::Ok, 'BackedEnum'), ':', is_a(PlainStatus::Ready, 'UnitEnum'), ':';
        echo is_a(RoundingMode::HalfEven, 'UnitEnum'), ':', is_a(SortDirection::Ascending, 'UnitEnum');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:1:1:1:1::1:1:1:1:1:1");
}

#[test]
fn reflection_closure_and_arrow_parameters_expose_attributes() {
    let source = r#"<?php
        #[Attribute(Attribute::TARGET_PARAMETER)]
        class CallableParamTag { public $name; public function __construct($name) { $this->name = $name; } }
        #[Attribute(Attribute::TARGET_PARAMETER | Attribute::IS_REPEATABLE)]
        class RepeatableParamTag {}
        $closure = function(#[CallableParamTag(name: 'closure')] $value) {};
        $arrow = fn (#[CallableParamTag(name: 'arrow')] $value) => $value;
        $repeated = fn (#[RepeatableParamTag, RepeatableParamTag] $value) => $value;
        $closureReflection = new ReflectionFunction($closure);
        $arrowReflection = new ReflectionFunction($arrow);
        $closureParameter = $closureReflection->getParameters()[0];
        $arrowParameter = $arrowReflection->getParameters()[0];
        $repeatedParameter = (new ReflectionFunction($repeated))->getParameters()[0];
        echo $closureParameter->getName(), ':', $closureParameter->getAttributes()[0]->newInstance()->name, ':';
        echo $arrowParameter->getName(), ':', $arrowParameter->getAttributes()[0]->getArguments()['name'], ':';
        echo $repeatedParameter->getAttributes()[0]->isRepeated(), ':', method_exists('ReflectionFunction', 'getParameters');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "value:closure:value:arrow:1:1");
}

#[test]
fn reflection_function_parameters_keep_parameter_attributes() {
    let source = r#"<?php
        #[Attribute(Attribute::TARGET_PARAMETER)]
        class NamedFunctionParamTag { public $label; function __construct($label) { $this->label = $label; } }
        function processArgument(#[NamedFunctionParamTag('function')] $argument) {}
        $parameter = (new ReflectionFunction('processArgument'))->getParameters()[0];
        echo $parameter->getName(), ':', $parameter->getAttributes()[0]->newInstance()->label;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "argument:function");
}
