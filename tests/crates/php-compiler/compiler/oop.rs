use php_compiler::compile_source;

#[test]
fn reflection_class_get_modifiers_reports_class_kind_flags() {
    let source = r#"<?php
        interface ModifierContract {}
        abstract class ModifierAbstract {}
        final class ModifierFinal {}
        readonly class ModifierReadonly { public int $value; }
        trait ModifierTrait {}
        enum ModifierEnum { case Item; }
        $abstract = new ReflectionClass(ModifierAbstract::class);
        $readonly = new ReflectionClass(ModifierReadonly::class);
        $interface = new ReflectionClass(ModifierContract::class);
        $trait = new ReflectionClass(ModifierTrait::class);
        $enum = new ReflectionEnum(ModifierEnum::class);
        echo $abstract->getModifiers(), ':', ReflectionClass::IS_EXPLICIT_ABSTRACT, ':';
        echo (new ReflectionClass(ModifierFinal::class))->getModifiers(), ':', ReflectionClass::IS_FINAL, ':';
        echo $readonly->getModifiers(), ':', ReflectionClass::IS_READONLY, ':';
        echo $interface->getModifiers(), ':', $trait->getModifiers(), ':', $enum->getModifiers(), ':';
        echo ReflectionClass::IS_IMPLICIT_ABSTRACT, ':';
        echo method_exists('ReflectionClass', 'getModifiers'), ':', method_exists('ReflectionEnum', 'getModifiers');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "64:64:32:32:65536:65536:0:0:32:16:1:1"
    );
}

#[test]
fn reflection_get_modifier_names_decodes_class_and_method_flags() {
    let source = r#"<?php
        final class ModifierNamesSubject {
            public protected(set) string $protected = 'p';
            public private(set) string $private = 'q';
            final public static function run() {}
        }
        abstract class ModifierNamesAbstract {}
        $class = new ReflectionClass(ModifierNamesSubject::class);
        $method = new ReflectionMethod(ModifierNamesSubject::class, 'run');
        echo class_exists('Reflection'), ':', method_exists('Reflection', 'getModifierNames'), ':';
        echo implode(',', Reflection::getModifierNames($class->getModifiers())), ':';
        echo implode(',', Reflection::getModifierNames(modifiers: $method->getModifiers())), ':';
        echo implode(',', Reflection::getModifierNames(ReflectionClass::IS_READONLY)), ':';
        echo implode(',', Reflection::getModifierNames((new ReflectionClass(ModifierNamesAbstract::class))->getModifiers())), ':';
        echo implode(',', Reflection::getModifierNames($class->getProperty('protected')->getModifiers())), ':';
        echo implode(',', Reflection::getModifierNames($class->getProperty('private')->getModifiers())), ':';
        echo implode(',', Reflection::getModifierNames(512)), "\n";
        try { Reflection::getModifierNames([]); }
        catch (TypeError $error) { echo get_class($error), ':', $error->getMessage(); }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:1:final:final,public,static:readonly:abstract:public,protected(set):public,private(set):virtual\nTypeError:Reflection::getModifierNames(): Argument #1 ($modifiers) must be of type int, array given"
    );
}

#[test]
fn reflection_class_distinguishes_internal_and_user_defined_types() {
    let source = r#"<?php
        class UserReflectionTarget {}
        interface UserReflectionContract {}
        enum UserReflectionEnum { case Item; }
        $user = new ReflectionClass(UserReflectionTarget::class);
        $interface = new ReflectionClass(UserReflectionContract::class);
        $enum = new ReflectionEnum(UserReflectionEnum::class);
        $internal = new ReflectionClass('stdClass');
        $exception = new ReflectionClass('Exception');
        $reflection = new ReflectionClass(ReflectionClass::class);
        $internalException = new ReflectionClass('Exception');
        echo json_encode([
            $user->isInternal(), $user->isUserDefined(),
            $interface->isInternal(), $interface->isUserDefined(),
            $enum->isInternal(), $enum->isUserDefined(),
            $internal->isInternal(), $internal->isUserDefined(),
            $exception->isInternal(), $exception->isUserDefined(),
            $reflection->isInternal(), $reflection->isUserDefined(),
            $internalException->isAbstract(), $internalException->isFinal(), $internalException->isInstantiable(),
            $internal->isInstantiable(),
        ]), ':';
        echo method_exists('ReflectionClass', 'isInternal'), ':', method_exists('ReflectionEnum', 'isUserDefined');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "[false,true,false,true,false,true,true,false,true,false,true,false,false,false,true,true]:1:1"
    );
}

#[test]
fn reflection_object_inspects_the_concrete_object_and_inherits_reflection_class_apis() {
    let source = r#"<?php
        class ReflectedObjectSubject { public function ready() { return 'yes'; } }
        $object = new ReflectedObjectSubject();
        $reflection = new ReflectionObject($object);
        echo get_class($reflection), ':', $reflection->getName(), ':', $reflection->getMethod('ready')->getName(), ':';
        echo is_a($reflection, ReflectionClass::class), ':', class_exists('ReflectionObject'), ':';
        echo method_exists('ReflectionObject', 'getMethod'), ':';
        try { new ReflectionObject('ReflectedObjectSubject'); }
        catch (TypeError $error) { echo get_class($error); }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "ReflectionObject:ReflectedObjectSubject:ready:1:1:1:TypeError"
    );
}

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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "TypeError:TypeError"
    );
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
    assert_eq!(
        output,
        "1:object(DebugInfoSubject)#1 (1) {\n  [\"visible\"]=>\n  string(6) \"custom\"\n}\n"
    );
    assert!(!output.contains("secret"));
}

#[test]
fn var_export_produces_reusable_php_state_expressions() {
    let source = r#"<?php
        class ExportableRecord {
            public $name = 'Ada';
            public static function __set_state(array $state): object {
                return new self($state['name']);
            }
            public function __construct($name = 'default') { $this->name = $name; }
        }
        echo function_exists('var_export'), ':', var_export(['ready' => true], true), "\n";
        echo var_export(new ExportableRecord('Ada'), true);
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:array (\n  'ready' => true,\n)\n\\ExportableRecord::__set_state(array(\n  'name' => 'Ada',\n))"
    );
    let output_mode = compile_source("<?php $result = var_export(5); echo $result === null ? ':null' : ':value';")
        .unwrap()
        .execute()
        .unwrap();
    assert_eq!(output_mode, "5:null");
}

#[test]
fn var_export_uses_enum_case_expressions() {
    let source = r#"<?php
        enum ExportUnitCase { case Ready; }
        enum ExportBackedCase: string { case Ready = 'ready'; }
        echo var_export(ExportUnitCase::Ready, true), ':', var_export(ExportBackedCase::Ready, true);
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "\\ExportUnitCase::Ready:\\ExportBackedCase::Ready"
    );
}

#[test]
fn var_export_warns_and_emits_null_for_circular_references() {
    let source = r#"<?php
        $value = [];
        $value['self'] =& $value;
        echo var_export($value, true);
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "array (\n  'self' => NULL,\n)");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(
        result.diagnostics[0].message,
        "var_export does not handle circular references"
    );
    assert_eq!(
        result.diagnostics[0].level,
        php_runtime::DiagnosticLevel::Warning
    );
    let mutual_source = r#"<?php
        $first = [];
        $second = [];
        $first['second'] =& $second;
        $second['first'] =& $first;
        echo var_export($first, true);
    "#;
    let mutual = compile_source(mutual_source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(
        mutual.output,
        "array (\n  'second' => array (\n    'first' => NULL,\n  ),\n)"
    );
    assert_eq!(mutual.diagnostics.len(), 1);
}

#[test]
fn var_export_serializes_stdclass_as_an_object_array_cast() {
    let source = r#"<?php
        $value = new stdClass();
        $value->answer = 42;
        echo var_export($value, true);
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "(object) array(\n  'answer' => 42,\n)"
    );
}

#[test]
fn var_dump_indents_nested_values_and_preserves_object_ids_and_private_properties() {
    let source = r#"<?php
        class DumpSubject {
            public $visible = ['nested' => 7];
            private $secret = 'hidden';
            protected $state = 'ready';
        }
        $subject = new DumpSubject();
        var_dump([$subject, $subject], $subject);
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert!(output.contains("array(2) {\n  [0]=>\n  object(DumpSubject)#1 (3) {\n    [\"visible\"]=>\n    array(1) {\n      [\"nested\"]=>\n      int(7)"), "{output}");
    assert!(
        output.contains("[\"secret\":\"DumpSubject\":private]=>\n    string(6) \"hidden\""),
        "{output}"
    );
    assert!(
        output.contains("[\"state\":protected]=>\n    string(5) \"ready\""),
        "{output}"
    );
    assert_eq!(
        output.matches("object(DumpSubject)#1 (3)").count(),
        3,
        "{output}"
    );
}

#[test]
fn var_dump_writes_string_bytes_and_array_keys_without_escaping() {
    let source = r#"<?php
        $value = "a\n\t" . chr(0) . chr(1) . "\"";
        var_dump($value, ["k\n" => $value]);
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    let value = "a\n\t\0\u{1}\"";
    let key = "k\n";
    let expected = format!(
        "string(6) \"{value}\"\narray(1) {{\n  [\"{key}\"]=>\n  string(6) \"{value}\"\n}}\n"
    );
    assert_eq!(output, expected);
}

#[test]
fn var_dump_prints_stream_resource_ids_and_types() {
    let source = r#"<?php
        $stream = fopen('Cargo.toml', 'r');
        var_dump($stream);
        fclose($stream);
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    let resource_id = output
        .strip_prefix("resource(")
        .and_then(|rest| rest.split_once(") of type (stream)\n"))
        .map(|(id, _)| id)
        .expect("stream resource dump should include an ID and type");
    assert!(resource_id.parse::<u64>().is_ok(), "{output}");
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
        echo $unit->getName(), ':', $unit->isBacked(), ':', count($unit->getAttributes()), ':', $unitCases[0]->getName(), ':', ($unitCases[0]->getValue() === Direction::North), ':', $unitCases[0]->getEnum()->getName(), ':', $unitCases[0]->getDeclaringClass()->getName(), ':', $unitCases[0]->getModifiers(), ':', $unitCases[0]->hasType(), ':', $unitCases[0]->isEnumCase(), ':', $unit->hasCase(name: 'South'), ':', $unit->hasCase('south'), ':', $unit->getCase(name: 'South')->getValue() === Direction::South, ':';
        $backingType = $backed->getBackingType();
        echo $backed->isBacked(), ':', $backedCases[1]->getName(), ':', $backed->getCase('Club')->getBackingValue(), ':', $backedCases[1]->getBackingValue(), ':', $backedCases[1]->getEnum()->getName(), ':', $backedCases[1]->getType() === null, ':', $backingType->getName(), ':', $backingType->isBuiltin(), ':', $backingType->allowsNull(), ':';
        try { $unit->getCase('Missing'); } catch (ReflectionException $error) { echo get_class($error), ':'; }
        echo ($unit->getBackingType() === null), ':', $backed->getConstant('Heart') === Suit::Heart, ':', $backed->hasConstant('Club'), ':', count($backed->getConstants()), ':';
        echo class_exists('ReflectionEnum'), ':', method_exists('ReflectionEnum', 'getCases'), ':', method_exists('ReflectionEnum', 'getCase'), ':', method_exists('ReflectionEnum', 'hasCase'), ':', method_exists('ReflectionEnumUnitCase', 'getEnum'), ':', method_exists('ReflectionEnumBackedCase', 'getEnum'), ':', method_exists('ReflectionEnum', 'getConstants');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "Direction::1:North:1:Direction:Direction:1::1:1::1:1:Club:C:C:Suit:1:string:1::ReflectionException:1:1:1:2:1:1:1:1:1:1:1");
}

#[test]
fn reflection_enum_cases_expose_attributes() {
    let source = r#"<?php
        #[Attribute(Attribute::TARGET_CLASS_CONSTANT | Attribute::IS_REPEATABLE)]
        class CaseLabel { public function __construct(public string $label) {} }
        enum Color { #[CaseLabel('rouge'), CaseLabel(label: 'red')] case Red; }
        enum BackedColor: string { #[CaseLabel('blue')] case Blue = 'B'; }
        $case = (new ReflectionEnum('Color'))->getCase('Red');
        $attributes = $case->getAttributes(CaseLabel::class);
        echo count($attributes), ':', $attributes[0]->getTarget(), ':', $attributes[0]->newInstance()->label, ':';
        echo $attributes[1]->getArguments()['label'], ':', $attributes[0]->isRepeated(), ':';
        $backedCase = (new ReflectionEnum('BackedColor'))->getCase('Blue');
        echo method_exists($case, 'getAttributes'), ':', $backedCase->getAttributes()[0]->newInstance()->label;
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "2:16:rouge:red:1:1:blue"
    );
}

#[test]
fn reflection_enum_case_names_accept_stringable_and_reject_arrays() {
    let source = r#"<?php
        enum ReflectionCaseName { case Ready; }
        class ReflectionCaseNameString implements Stringable {
            public function __toString(): string { return 'Ready'; }
        }
        $reflection = new ReflectionEnum(ReflectionCaseName::class);
        echo $reflection->hasCase(new ReflectionCaseNameString()), ':';
        echo $reflection->getCase(new ReflectionCaseNameString())->getName(), ':';
        try { $reflection->hasCase([]); }
        catch (TypeError $error) { echo get_class($error), ':'; }
        try { $reflection->getCase([]); }
        catch (TypeError $error) { echo get_class($error), ':', $error->getMessage(); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:Ready:TypeError:TypeError:ReflectionEnum::getCase(): Argument #1 ($name) must be of type string, array given");
}

#[test]
fn reflection_class_and_enum_report_short_names_and_namespaces() {
    let source = r#"<?php
        namespace Acme\Catalog;
        trait ProductTag {}
        class Product {}
        enum Status { case Ready; }
        $class = new \ReflectionClass(Product::class);
        $trait = new \ReflectionClass(ProductTag::class);
        $enum = new \ReflectionEnum(Status::class);
        echo $class->getName(), ':', $class->getShortName(), ':', $class->getNamespaceName(), ':', $class->inNamespace(), ':';
        echo $enum->getName(), ':', $enum->getShortName(), ':', $enum->getNamespaceName(), ':', $enum->inNamespace(), ':';
        echo $trait->isTrait(), ':', $enum->isTrait(), ':', method_exists(\ReflectionClass::class, 'getShortName'), ':', method_exists(\ReflectionEnum::class, 'inNamespace'), ':', method_exists(\ReflectionClass::class, 'isTrait');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "Acme\\Catalog\\Product:Product:Acme\\Catalog:1:Acme\\Catalog\\Status:Status:Acme\\Catalog:1:1::1:1:1");
}

#[test]
fn reflection_class_is_instance_checks_class_interface_and_enum_membership() {
    let source = r#"<?php
        interface InstanceMarker {}
        class InstanceBase implements InstanceMarker {}
        class InstanceChild extends InstanceBase {}
        class InstanceOther {}
        enum InstanceState { case Ready; }
        $class = new ReflectionClass(InstanceMarker::class);
        $enum = new ReflectionEnum(InstanceState::class);
        echo $class->isInstance(new InstanceChild()), ':', $class->isInstance(new InstanceOther()), ':';
        echo $enum->isInstance(InstanceState::Ready), ':', method_exists(ReflectionClass::class, 'isInstance'), ':';
        try { $class->isInstance([]); }
        catch (TypeError $error) { echo get_class($error), ':', $error->getMessage(); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1::1:1:TypeError:ReflectionClass::isInstance(): Argument #1 ($object) must be of type object, array given");
}

#[test]
fn reflected_enum_cases_expose_reflection_class_constant_behavior() {
    let source = r#"<?php
        enum ReflectedCaseKind: string { case Active = 'active'; }
        $case = (new ReflectionEnum('ReflectedCaseKind'))->getCase('Active');
        echo ($case instanceof ReflectionEnumUnitCase), ':', ($case instanceof ReflectionClassConstant), ':';
        echo get_parent_class('ReflectionEnumBackedCase'), ':', get_parent_class('ReflectionEnumUnitCase'), ':';
        echo $case->getDeclaringClass()->getName(), ':', $case->getModifiers(), ':', ($case->getType() === null), ':';
        echo $case->hasType(), ':', $case->isEnumCase(), ':', $case->isPublic(), ':', $case->isProtected(), ':', $case->isPrivate(), ':', $case->isFinal();
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:1:ReflectionEnumUnitCase:ReflectionClassConstant:ReflectedCaseKind:1:1::1:1:::"
    );
}

#[test]
fn reflection_class_get_constructor_returns_inherited_constructor_or_null() {
    let source = r#"<?php
        class ReflectionCtorParent { public function __construct() {} }
        class ReflectionCtorChild extends ReflectionCtorParent {}
        class ReflectionCtorMissing {}
        enum ReflectionCtorEnum { case Ready; }
        $constructor = (new ReflectionClass('ReflectionCtorChild'))->getConstructor();
        $enumConstructor = (new ReflectionEnum('ReflectionCtorEnum'))->getConstructor();
        echo $constructor->getName(), ':', $constructor->isConstructor(), ':',
            ((new ReflectionClass('ReflectionCtorMissing'))->getConstructor() === null), ':';
        echo ($enumConstructor === null), ':', method_exists('ReflectionClass', 'getConstructor'), ':',
            method_exists('ReflectionEnum', 'getConstructor');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "__construct:1:1:1:1:1"
    );
}

#[test]
fn reflection_property_reports_the_declaring_class_for_inherited_properties() {
    let source = r#"<?php
        class ReflectedPropertyParent { protected string $token = 'parent'; public static int $count = 2; }
        class ReflectedPropertyChild extends ReflectedPropertyParent { public $plain; public readonly int $id; }
        $property = (new ReflectionClass('ReflectedPropertyChild'))->getProperty('token');
        $plain = (new ReflectionClass('ReflectedPropertyChild'))->getProperty('plain');
        $count = (new ReflectionClass('ReflectedPropertyChild'))->getProperty('count');
        $id = (new ReflectionClass('ReflectedPropertyChild'))->getProperty('id');
        echo $property->getName(), ':', $property->getDeclaringClass()->getName(), ':', $property->getType()->getName(), ':', $property->hasType(), ':';
        echo $plain->hasType(), ':', ($plain->getType() === null), ':', method_exists('ReflectionProperty', 'getDeclaringClass'), ':';
        echo method_exists('ReflectionProperty', 'getType'), ':', method_exists('ReflectionProperty', 'hasType'), ':';
        echo $property->getModifiers(), ':', $property->isProtected(), ':', $property->isPublic(), ':';
        echo $count->getModifiers(), ':', $count->isStatic(), ':', $id->getModifiers(), ':', $id->isReadOnly(), ':';
        echo ReflectionProperty::IS_PUBLIC, ':', ReflectionProperty::IS_PROTECTED, ':', ReflectionProperty::IS_PRIVATE, ':', ReflectionProperty::IS_STATIC, ':', ReflectionProperty::IS_READONLY;
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "token:ReflectedPropertyParent:string:1::1:1:1:1:2:1::17:1:129:1:1:2:4:16:128"
    );
}

#[test]
fn reflection_property_reads_private_inherited_and_static_values() {
    let source = r#"<?php
        class ReflectionValueParent { private string $secret = 'hidden'; protected string $token = 'parent'; public static int $count = 7; }
        class ReflectionValueChild extends ReflectionValueParent {}
        $object = new ReflectionValueChild();
        $parent = new ReflectionClass('ReflectionValueParent');
        $child = new ReflectionClass('ReflectionValueChild');
        echo $parent->getProperty('secret')->getValue($object), ':', $child->getProperty('token')->getValue($object), ':';
        echo $child->getProperty('count')->getValue(), ':', method_exists('ReflectionProperty', 'getValue');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "hidden:parent:7:1"
    );
}

#[test]
fn reflection_property_sets_private_instance_and_static_values() {
    let source = r#"<?php
        class ReflectionSetParent { private string $secret = 'old'; public static int $count = 7; }
        class ReflectionSetChild extends ReflectionSetParent {}
        $object = new ReflectionSetChild();
        $parent = new ReflectionClass('ReflectionSetParent');
        $secret = $parent->getProperty('secret');
        $count = $parent->getProperty('count');
        $secret->setValue($object, 'new');
        $count->setValue(null, 9);
        echo $secret->getValue($object), ':', $count->getValue(), ':', ($secret->setValue($object, 'again') === null), ':';
        try { $secret->setValue($object, 10); } catch (TypeError $error) { echo get_class($error); }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "new:9:1:TypeError"
    );
}

#[test]
fn reflection_property_get_value_invokes_property_get_hook() {
    let source = r#"<?php
        class ReflectedHookedValue {
            public int $value {
                get { return $this->value + 1; }
                set { $this->value = $value; }
            }
        }
        $object = new ReflectedHookedValue();
        $property = (new ReflectionClass(ReflectedHookedValue::class))->getProperty('value');
        $property->setValue($object, 4);
        echo $property->getValue($object);
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "5");
}

#[test]
fn reflection_property_set_accessible_is_a_deprecated_no_op_in_php_85() {
    let source = r#"<?php
        class ReflectionAccessibleProperty { private string $secret = 'hidden'; }
        $object = new ReflectionAccessibleProperty();
        $property = (new ReflectionClass(ReflectionAccessibleProperty::class))->getProperty('secret');
        echo ($property->setAccessible(accessible: false) === null), ':', $property->getValue($object), ':';
        echo ($property->setAccessible(true) === null), ':', method_exists('ReflectionProperty', 'setAccessible');
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "1:hidden:1:1");
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Deprecated));
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic
        .message
        .contains("setAccessible() is deprecated since 8.5")));
}

#[test]
fn reflection_method_set_accessible_is_a_deprecated_no_op_in_php_85() {
    let source = r#"<?php
        class ReflectionAccessibleMethod { private function hidden() {} }
        $method = (new ReflectionClass(ReflectionAccessibleMethod::class))->getMethod('hidden');
        echo ($method->setAccessible(accessible: false) === null), ':', method_exists('ReflectionMethod', 'setAccessible');
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "1:1");
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(
        result.diagnostics[0].level,
        php_runtime::DiagnosticLevel::Deprecated
    );
    assert!(result.diagnostics[0]
        .message
        .contains("ReflectionMethod::setAccessible() is deprecated since 8.5"));
}

#[test]
fn reflection_class_get_default_properties_includes_inherited_defaults_and_nulls() {
    let source = r#"<?php
        class DefaultPropertyParent { protected $inherited = 'parent'; private $parentPrivate = 'hidden'; public static $baseStatic = 'base'; }
        class DefaultPropertyChild extends DefaultPropertyParent {
            public $own = 'child';
            private $secret = 'private';
            public $implicit;
            public ?int $uninitialized;
            public static $childStatic = 'static';
        }
        $defaults = (new ReflectionClass(DefaultPropertyChild::class))->getDefaultProperties();
        echo count($defaults), ':', $defaults['inherited'], ':', $defaults['baseStatic'], ':', $defaults['own'], ':';
        echo $defaults['secret'], ':', ($defaults['implicit'] === null), ':', ($defaults['uninitialized'] === null), ':';
        echo $defaults['childStatic'], ':', array_key_exists('parentPrivate', $defaults), ':';
        echo method_exists('ReflectionClass', 'getDefaultProperties'), ':';
        enum ReflectionDefaultEmpty { case Ready; }
        echo count((new ReflectionClass(ReflectionDefaultEmpty::class))->getDefaultProperties());
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "7:parent:base:child:private:1:1:static::1:0"
    );
}

#[test]
fn reflection_class_get_static_properties_reads_current_inherited_slots() {
    let source = r#"<?php
        class StaticPropertyParent { protected static int $inherited = 2; private static int $parentPrivate = 3; }
        class StaticPropertyChild extends StaticPropertyParent { public static int $count = 4; private static int $secret = 5; public static ?int $uninitialized; }
        StaticPropertyChild::$count = 9;
        $properties = (new ReflectionClass(StaticPropertyChild::class))->getStaticProperties();
        echo count($properties), ':', $properties['inherited'], ':', $properties['count'], ':', $properties['secret'], ':';
        echo ($properties['uninitialized'] === null), ':', array_key_exists('parentPrivate', $properties), ':';
        echo method_exists('ReflectionClass', 'getStaticProperties'), ':';
        enum ReflectionStaticEmpty { case Ready; }
        echo count((new ReflectionEnum(ReflectionStaticEmpty::class))->getStaticProperties());
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "4:2:9:5:1::1:0"
    );
}

#[test]
fn reflection_class_reads_static_property_values_and_honors_default() {
    let source = r#"<?php
        class StaticValueParent { private static $secret = 'parent'; protected static $shared = 3; }
        class StaticValueChild extends StaticValueParent { public static $shared = 8; public static $local = 11; }
        $reflection = new ReflectionClass(StaticValueChild::class);
        echo $reflection->getStaticPropertyValue('shared'), ':', $reflection->getStaticPropertyValue('local'), ':';
        echo (new ReflectionClass(StaticValueParent::class))->getStaticPropertyValue('secret'), ':';
        echo $reflection->getStaticPropertyValue('missing', 'fallback'), ':';
        echo $reflection->getStaticPropertyValue(default: 'named-fallback', name: 'missing'), ':';
        echo $reflection->getStaticPropertyValue('secret', 'private-fallback'), ':';
        try { $reflection->getStaticPropertyValue('missing'); }
        catch (ReflectionException $error) { echo get_class($error), ':'; }
        try { $reflection->getStaticPropertyValue(name: 'missing'); }
        catch (ReflectionException $error) { echo get_class($error), ':'; }
        echo method_exists('ReflectionClass', 'getStaticPropertyValue');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "8:11:parent:fallback:named-fallback:private-fallback:ReflectionException:ReflectionException:1"
    );
}

#[test]
fn reflection_class_sets_static_property_values_across_visibility() {
    let source = r#"<?php
        class StaticWriteParent { protected static int $shared = 2; private static string $parentSecret = 'hidden'; }
        class StaticWriteChild extends StaticWriteParent { private static string $secret = 'old'; public static int $count = 1; }
        $reflection = new ReflectionClass(StaticWriteChild::class);
        $reflection->setStaticPropertyValue('shared', 7);
        $reflection->setStaticPropertyValue(name: 'secret', value: 'new');
        $reflection->setStaticPropertyValue('count', 9);
        echo $reflection->getStaticPropertyValue('shared'), ':', $reflection->getStaticPropertyValue('secret'), ':', StaticWriteChild::$count, ':';
        try { $reflection->setStaticPropertyValue('missing', 5); }
        catch (ReflectionException $error) { echo get_class($error), ':'; }
        try { $reflection->setStaticPropertyValue('parentSecret', 'changed'); }
        catch (ReflectionException $error) { echo get_class($error), ':'; }
        try { $reflection->setStaticPropertyValue('count', 'wrong'); }
        catch (TypeError $error) { echo get_class($error), ':'; }
        echo method_exists('ReflectionClass', 'setStaticPropertyValue');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "7:new:9:ReflectionException:ReflectionException:TypeError:1"
    );
}

#[test]
fn reflection_class_can_instantiate_without_running_constructor() {
    let source = r#"<?php
        $constructed = 0;
        class ReflectionConstructorBypass {
            public int $ready = 5;
            public string $uninitialized;
            public function __construct() { $GLOBALS['constructed']++; $this->ready = 9; }
        }
        $reflection = new ReflectionClass(ReflectionConstructorBypass::class);
        $instance = $reflection->newInstanceWithoutConstructor();
        echo $instance->ready, ':', $constructed, ':';
        try { echo $instance->uninitialized; }
        catch (Error $error) { echo 'uninitialized'; }
        echo ':';
        echo method_exists('ReflectionClass', 'newInstanceWithoutConstructor');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "5:0:uninitialized:1"
    );
    let abstract_error = compile_source(r#"<?php abstract class ReflectionCannotInstantiate { } (new ReflectionClass(ReflectionCannotInstantiate::class))->newInstanceWithoutConstructor();"#)
        .unwrap()
        .execute()
        .unwrap_err();
    assert!(abstract_error.contains("ReflectionException: Cannot instantiate abstract class ReflectionCannotInstantiate"), "{abstract_error}");
    let non_classes = compile_source(r#"<?php
        interface ReflectionCannotInstantiateInterface {}
        trait ReflectionCannotInstantiateTrait {}
        enum ReflectionCannotInstantiateEnum { case Ready; }
        foreach ([ReflectionCannotInstantiateInterface::class, ReflectionCannotInstantiateTrait::class, ReflectionCannotInstantiateEnum::class] as $name) {
            try { (new ReflectionClass($name))->newInstanceWithoutConstructor(); }
            catch (ReflectionException $error) { echo get_class($error), ':'; }
        }
    "#)
    .unwrap()
    .execute()
    .unwrap();
    assert_eq!(non_classes, "ReflectionException:ReflectionException:ReflectionException:");
}

#[test]
fn reflection_class_new_instance_factories_forward_arguments_to_constructor() {
    let source = r#"<?php
        class ReflectionFactoryTarget {
            public $label;
            public function __construct($label, $suffix = '!') { $this->label = $label . $suffix; }
        }
        class ReflectionFactoryReferenceTarget { public function __construct(&$value) { $value++; } }
        $reflection = new ReflectionClass(ReflectionFactoryTarget::class);
        echo $reflection->newInstance('direct')->label, ':';
        echo $reflection->newInstanceArgs(['args', 'suffix' => '?'])->label, ':';
        echo $reflection->newInstanceArgs(['suffix' => '.', 'label' => 'named'])->label, ':';
        echo method_exists('ReflectionClass', 'newInstance'), ':', method_exists('ReflectionClass', 'newInstanceArgs');
        $directReference = 4;
        (new ReflectionClass(ReflectionFactoryReferenceTarget::class))->newInstance($directReference);
        $arrayReference = 6;
        (new ReflectionClass(ReflectionFactoryReferenceTarget::class))->newInstanceArgs([&$arrayReference]);
        echo ':', $directReference, ':', $arrayReference;
        class ReflectionFactoryPrivateConstructor { private function __construct() {} }
        try { (new ReflectionClass(ReflectionFactoryPrivateConstructor::class))->newInstance(); }
        catch (ReflectionException $error) { echo ':', get_class($error); }
        class ReflectionFactoryNoConstructor {}
        try { (new ReflectionClass(ReflectionFactoryNoConstructor::class))->newInstanceArgs([1]); }
        catch (ReflectionException $error) { echo ':', get_class($error); }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "direct!:args?:named.:1:1:5:7:ReflectionException:ReflectionException"
    );
    let missing = compile_source(r#"<?php
        class ReflectionFactoryNeedsArgument { public function __construct($required) {} }
        (new ReflectionClass(ReflectionFactoryNeedsArgument::class))->newInstance();
    "#)
    .unwrap()
    .execute()
    .unwrap_err();
    assert!(missing.contains("ArgumentCountError"), "{missing}");
}

#[test]
fn reflection_class_does_not_expose_parent_private_properties_as_inherited() {
    let source = r#"<?php
        class PrivateLookupParent { private $secret = 'parent'; private $parentOnly = 'hidden'; private static $staticSecret = 'parent-static'; }
        class PrivateLookupChild extends PrivateLookupParent { private $secret = 'child'; }
        $reflection = new ReflectionClass(PrivateLookupChild::class);
        echo $reflection->getProperty('secret')->getDeclaringClass()->getName(), ':';
        echo $reflection->hasProperty('parentOnly'), ':';
        try { $reflection->getProperty('parentOnly'); }
        catch (ReflectionException $error) { echo get_class($error), ':'; }
        echo $reflection->hasProperty('staticSecret'), ':';
        try { $reflection->getProperty('staticSecret'); }
        catch (ReflectionException $error) { echo get_class($error), ':'; }
        echo method_exists('ReflectionClass', 'getProperty');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "PrivateLookupChild::ReflectionException::ReflectionException:1"
    );
}

#[test]
fn reflection_class_get_method_throws_reflection_exception_for_missing_methods() {
    let source = r#"<?php
        class ReflectionMethodLookup { public function present() {} }
        $reflection = new ReflectionClass(ReflectionMethodLookup::class);
        try { $reflection->getMethod('missing'); }
        catch (ReflectionException $error) { echo get_class($error), ':', $error->getMessage(); }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "ReflectionException:Method ReflectionMethodLookup::missing() does not exist"
    );
}

#[test]
fn reflection_class_get_properties_filters_visibility_static_and_readonly_members() {
    let source = r#"<?php
        class FilteredPropertyParent {
            private $parentPrivate = 1;
            protected $inherited = 2;
            public static $baseStatic = 3;
        }
        class FilteredPropertyChild extends FilteredPropertyParent {
            public $name = 'child';
            private int $secret;
            public readonly int $locked;
            public static $childStatic = 4;
        }
        $reflection = new ReflectionClass(FilteredPropertyChild::class);
        $all = $reflection->getProperties();
        $allNames = [];
        foreach ($all as $property) { $allNames[] = $property->getName(); }
        sort($allNames);
        $publicNames = [];
        foreach ($reflection->getProperties(filter: ReflectionProperty::IS_PUBLIC) as $property) { $publicNames[] = $property->getName(); }
        sort($publicNames);
        $staticNames = [];
        foreach ($reflection->getProperties(ReflectionProperty::IS_STATIC) as $property) { $staticNames[] = $property->getName(); }
        sort($staticNames);
        $readonlyNames = [];
        foreach ($reflection->getProperties(ReflectionProperty::IS_READONLY) as $property) { $readonlyNames[] = $property->getName(); }
        echo implode(',', $allNames), ':', implode(',', $publicNames), ':', implode(',', $staticNames), ':';
        echo implode(',', $readonlyNames), ':', $reflection->hasProperty('parentPrivate'), ':';
        echo count((new ReflectionEnum(ReflectionPropertyEnum::class))->getProperties()), ':';
        echo method_exists('ReflectionClass', 'getProperties');
    "#;
    let source = source
        .replace("ReflectionPropertyEnum", "FilteredPropertyEnum")
        .replace(
            "$reflection =",
            "enum FilteredPropertyEnum { case Ready; }\n        $reflection =",
        );
    assert_eq!(compile_source(&source).unwrap().execute().unwrap(), "baseStatic,childStatic,inherited,locked,name,secret:baseStatic,childStatic,locked,name:baseStatic,childStatic:locked::0:1");
}

#[test]
fn reflection_method_reports_parent_and_interface_prototypes() {
    let source = r#"<?php
        #[Attribute(Attribute::TARGET_METHOD)] class PrototypeMethodMark {}
        interface PrototypeContract { #[PrototypeMethodMark] public function run(): void; }
        class PrototypeBase implements PrototypeContract { public function run(): void {} }
        class PrototypeChild extends PrototypeBase { public function run(): void {} }
        class PrototypeStandalone { public function stop(): void {} }
        $base = (new ReflectionClass(PrototypeBase::class))->getMethod('run');
        $child = (new ReflectionClass(PrototypeChild::class))->getMethod('run');
        $standalone = (new ReflectionClass(PrototypeStandalone::class))->getMethod('stop');
        $interfacePrototype = $base->getPrototype();
        echo $base->hasPrototype(), ':', $interfacePrototype->getDeclaringClass()->getName(), ':', $interfacePrototype->getAttributes()[0]->getName(), ':';
        echo $child->hasPrototype(), ':', $child->getPrototype()->getDeclaringClass()->getName(), ':';
        echo $standalone->hasPrototype(), ':', method_exists('ReflectionMethod', 'getPrototype'), ':';
        try { $standalone->getPrototype(); } catch (ReflectionException $error) { echo $error->getMessage(); }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:PrototypeContract:PrototypeMethodMark:1:PrototypeBase::1:Method PrototypeStandalone::stop() does not have a prototype",
    );
}

#[test]
fn inherited_and_interface_signatures_support_optional_and_variadic_parameters() {
    let valid = r#"<?php
        interface OptionalExtraContract { public function fromInterface(int $value): int; }
        interface ConstructorSignatureContract { public function __construct(int $value); }
        interface VariadicContract { public function fromInterfaceVariadic(int $id, string $label): string; }
        class OptionalExtraParent {
            public function fromParent(int $value): int { return $value; }
            public function optionalParent(int $value = 1): int { return $value; }
            private function privateParent(int $value): int { return $value; }
            public function fromParentVariadic(int $id, string $label): string { return $label; }
        }
        class OptionalExtraChild extends OptionalExtraParent implements OptionalExtraContract, VariadicContract {
            public function fromParent(int $value = 2, string $extra = 'x'): int { return $value; }
            public function optionalParent(int $value = 3): int { return $value; }
            public function fromInterface(int $value, string $extra = 'x'): int { return $value; }
            public function privateParent(string $value, bool $extra): string { return $value; }
            public function fromParentVariadic(int $id, string ...$labels): string { return $labels[0]; }
            public function fromInterfaceVariadic(int $id, string ...$labels): string { return $labels[0]; }
        }
        class ConstructorWithoutContractMethod implements ConstructorSignatureContract {}
        $child = new OptionalExtraChild();
        new ConstructorWithoutContractMethod();
        echo $child->fromParent(), ':', $child->optionalParent(), ':', $child->fromInterface(4), ':';
        echo $child->fromParentVariadic(5, 'parent'), ':', $child->fromInterfaceVariadic(6, 'interface');
    "#;
    assert_eq!(
        compile_source(valid).unwrap().execute().unwrap(),
        "2:3:4:parent:interface"
    );

    let invalid = r#"<?php
        class RequiredExtraParent { public function run(int $value): void {} }
        class RequiredExtraChild extends RequiredExtraParent {
            public function run(int $value, string $extra): void {}
        }
    "#;
    match compile_source(invalid) {
        Err(error) => assert!(error.contains("incompatible parameter or return types with parent")),
        Ok(_) => panic!("required extra override parameter should be rejected"),
    }

    let invalid_variadic = r#"<?php
        interface VarianceContract { public function run(string $value): void; }
        class VarianceImplementation implements VarianceContract {
            public function run(int ...$values): void {}
        }
    "#;
    match compile_source(invalid_variadic) {
        Err(error) => assert!(
            error.contains("does not match interface variancecontract signature"),
            "{error}"
        ),
        Ok(_) => panic!("incompatible variadic type should be rejected"),
    }
}

#[test]
fn inherited_property_types_and_readonly_status_are_invariant() {
    let valid = r#"<?php
        class PropertyInvariantParent { public int $value = 1; }
        class PropertyInvariantChild extends PropertyInvariantParent { public int $value = 2; }
        class PrivatePropertyParent { private int $value = 1; }
        class PrivatePropertyChild extends PrivatePropertyParent { public string $value = 'child'; }
        class UnionTypeParent { public int|string $value; }
        class UnionTypeChild extends UnionTypeParent { public string|int $value; }
        class NullableTypeParent { public ?int $value; }
        class NullableTypeChild extends NullableTypeParent { public int|null $value; }
        interface PropertyTypeA {}
        interface PropertyTypeB {}
        class IntersectionTypeParent { public PropertyTypeA&PropertyTypeB $value; }
        class IntersectionTypeChild extends IntersectionTypeParent { public PropertyTypeB&PropertyTypeA $value; }
        echo (new PropertyInvariantChild())->value, ':', (new PrivatePropertyChild())->value;
    "#;
    assert_eq!(compile_source(valid).unwrap().execute().unwrap(), "2:child");

    let wrong_type = r#"<?php
        class TypedPropertyParent { public int $value; }
        class TypedPropertyChild extends TypedPropertyParent { public string $value; }
    "#;
    match compile_source(wrong_type) {
        Err(error) => assert!(
            error.contains("Property TypedPropertyChild::$value cannot change inherited type")
        ),
        Ok(_) => panic!("inherited property type change should be rejected"),
    }

    let wrong_readonly = r#"<?php
        class ReadonlyPropertyParent { public readonly int $value; }
        class ReadonlyPropertyChild extends ReadonlyPropertyParent { public int $value; }
    "#;
    match compile_source(wrong_readonly) {
        Err(error) => {
            assert!(error
                .contains("Property ReadonlyPropertyChild::$value cannot change readonly status"))
        }
        Ok(_) => panic!("inherited readonly status change should be rejected"),
    }
}

#[test]
fn null_coalescing_handles_uninitialized_typed_properties_like_isset() {
    let source = r#"<?php
        class UninitializedCoalesceExample {
            public int|string $value;
            public array $items;
            public static int $staticValue;
        }
        class MagicCoalesceExample {
            private int $hidden;
            private int $shown;
            public function __isset($name) { echo 'i'; return $name === 'shown'; }
            public function __get($name) { echo 'g'; return 9; }
        }
        $object = new UninitializedCoalesceExample();
        echo ($object->value ?? 'fallback'), ':', ($object->items['missing'] ?? 'nested'), ':';
        echo (UninitializedCoalesceExample::$staticValue ?? 'static'), ':';
        $magic = new MagicCoalesceExample();
        echo ($magic->hidden ?? 'hidden'), ':', ($magic->shown ?? 'fallback'), ':';
        $propertyName = 'value';
        echo ($object->{$propertyName} ?? 'computed'), ':';
        $object->value ??= 7;
        UninitializedCoalesceExample::$staticValue ??= 8;
        echo $object->value, ':', UninitializedCoalesceExample::$staticValue;
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "fallback:nested:static:ihidden:ig9:computed:7:8"
    );
}

#[test]
fn inherited_asymmetric_property_visibility_is_widened_and_private_set_is_final() {
    let valid = r#"<?php
        class SetVisibilityParent { public protected(set) int $value = 1; }
        class SetVisibilityChild extends SetVisibilityParent { public int $value = 2; }
        echo (new SetVisibilityChild())->value;
    "#;
    assert_eq!(compile_source(valid).unwrap().execute().unwrap(), "2");

    let narrowed = r#"<?php
        class SetVisibilityNarrowParent { public protected(set) int $value; }
        class SetVisibilityNarrowChild extends SetVisibilityNarrowParent { public private(set) int $value; }
    "#;
    match compile_source(narrowed) {
        Err(error) => assert!(
            error.contains(
                "Property SetVisibilityNarrowChild::$value cannot reduce inherited set visibility"
            ),
            "{error}"
        ),
        Ok(_) => panic!("inherited set visibility must not be narrowed"),
    }

    let private_set = r#"<?php
        class FinalSetVisibilityParent { public private(set) int $value; }
        class FinalSetVisibilityChild extends FinalSetVisibilityParent { public int $value; }
    "#;
    match compile_source(private_set) {
        Err(error) => assert!(
            error.contains("cannot override final private(set) property"),
            "{error}"
        ),
        Ok(_) => panic!("private(set) property must be final"),
    }
}

#[test]
fn reflection_property_reports_asymmetric_set_visibility() {
    let source = r#"<?php
        class AsymmetricReflectionExample {
            public protected(set) string $protected = 'p';
            public private(set) string $private = 'q';
            public string $ordinary = 'o';
            private string $privateProperty;
        }
        $reflection = new ReflectionClass(AsymmetricReflectionExample::class);
        foreach (['protected', 'private', 'ordinary', 'privateProperty'] as $name) {
            $property = $reflection->getProperty($name);
            echo $property->isPrivateSet(), ':', $property->isProtectedSet(), ':', $property->getModifiers(), '|';
        }
        $privateSetNames = [];
        foreach ($reflection->getProperties(ReflectionProperty::IS_PRIVATE_SET) as $property) { $privateSetNames[] = $property->getName(); }
        $protectedSetNames = [];
        foreach ($reflection->getProperties(ReflectionProperty::IS_PROTECTED_SET) as $property) { $protectedSetNames[] = $property->getName(); }
        echo ReflectionProperty::IS_PRIVATE_SET, ':', ReflectionProperty::IS_PROTECTED_SET, ':';
        echo implode(',', $privateSetNames), ':', implode(',', $protectedSetNames), ':';
        echo method_exists('ReflectionProperty', 'isPrivateSet'), ':', method_exists('ReflectionProperty', 'isProtectedSet');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        ":1:2049|1::4097|::1|::4|4096:2048:private:protected:1:1"
    );
}

#[test]
fn reflection_class_reports_readonly_declaration_modifier() {
    let source = r#"<?php
        readonly class ReadonlyReflectedClass { public int $value; }
        class MutableReflectedClass { public readonly int $value; }
        enum ReadonlyReflectionEnum { case Ready; }
        echo (new ReflectionClass(ReadonlyReflectedClass::class))->isReadOnly(), ':';
        echo (new ReflectionClass(MutableReflectedClass::class))->isReadOnly(), ':';
        echo (new ReflectionEnum(ReadonlyReflectionEnum::class))->isReadOnly(), ':';
        echo method_exists('ReflectionClass', 'isReadOnly'), ':';
        echo method_exists('ReflectionEnum', 'isReadOnly');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:::1:1"
    );
}

#[test]
fn reflection_class_identifies_anonymous_classes() {
    let source = r#"<?php
        class NamedReflectionClass {}
        enum ReflectionAnonymousEnum { case Ready; }
        $anonymous = new class {};
        echo (new ReflectionClass(NamedReflectionClass::class))->isAnonymous() ? 'yes' : 'no', ':';
        echo (new ReflectionClass($anonymous))->isAnonymous() ? 'yes' : 'no', ':';
        echo (new ReflectionEnum(ReflectionAnonymousEnum::class))->isAnonymous() ? 'yes' : 'no', ':';
        echo method_exists('ReflectionClass', 'isAnonymous'), ':', method_exists('ReflectionEnum', 'isAnonymous');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "no:yes:no:1:1"
    );
}

#[test]
fn anonymous_class_reflection_names_include_source_location() {
    let source = r#"<?php
        $anonymous = new class {};
        $reflection = new ReflectionClass($anonymous);
        echo $reflection->getName(), ':', get_class($anonymous), ':', $reflection->getShortName(), ':';
        echo $reflection->getNamespaceName(), ':', $reflection->inNamespace();
        class AnonymousReflectionParent {}
        $anonymousChild = new class extends AnonymousReflectionParent {};
        echo ':', get_class($anonymousChild);
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "class@anonymous\0Command line code:2$0:class@anonymous\0Command line code:2$0:class@anonymous:::AnonymousReflectionParent@anonymous\0Command line code:7$1",
    );
}

#[test]
fn reflection_class_checks_traversable_interfaces_and_parents() {
    let source = r#"<?php
        interface ReflectionWalkable extends Traversable {}
        class ReflectionWalkableParent implements ReflectionWalkable {}
        class ReflectionWalkableChild extends ReflectionWalkableParent {}
        class ReflectionNotWalkable {}
        enum ReflectionNotWalkableEnum { case Ready; }
        $iterable = (new ReflectionClass(ReflectionWalkableChild::class))->isIterable();
        $alias = (new ReflectionClass(ReflectionWalkableParent::class))->isIterateable();
        echo $iterable ? 'yes' : 'no', ':', $alias ? 'yes' : 'no', ':';
        echo (new ReflectionClass(ReflectionNotWalkable::class))->isIterable() ? 'yes' : 'no', ':';
        echo (new ReflectionEnum(ReflectionNotWalkableEnum::class))->isIterable() ? 'yes' : 'no', ':';
        echo method_exists('ReflectionClass', 'isIterable'), ':', method_exists('ReflectionClass', 'isIterateable');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "yes:yes:no:no:1:1"
    );
}

#[test]
fn readonly_class_constraints_cover_anonymous_classes_and_inheritance() {
    let source = r#"<?php
        readonly class ReadonlyParentClass {}
        readonly class ReadonlyChildClass extends ReadonlyParentClass {}
        $anonymous = new readonly class { public int $id; };
        echo (new ReflectionClass($anonymous))->isReadOnly();
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1");

    for source in [
        "<?php class MutableChild extends ReadonlyBase {} readonly class ReadonlyBase {}",
        "<?php class MutableBase {} readonly class ReadonlyChild extends MutableBase {}",
        "<?php readonly class InvalidReadonlyClass { public $untyped; }",
        "<?php readonly class InvalidReadonlyClass { public static int $static; }",
        "<?php new readonly stdClass();",
    ] {
        assert!(
            compile_source(source).is_err(),
            "accepted invalid readonly class declaration: {source}"
        );
    }
}

#[test]
fn reflection_property_reports_constructor_promotion() {
    let source = r#"<?php
        class PromotedPropertyExample {
            public function __construct(public readonly int $id) {}
            public string $name = 'plain';
        }
        $reflection = new ReflectionClass(PromotedPropertyExample::class);
        $promoted = $reflection->getProperty('id');
        $ordinary = $reflection->getProperty('name');
        echo $promoted->isPromoted(), ':', $promoted->isReadOnly(), ':', $ordinary->isPromoted(), ':';
        echo method_exists('ReflectionProperty', 'isPromoted');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:1::1");
}

#[test]
fn reflection_property_checks_initialized_state_for_instance_and_static_properties() {
    let source = r#"<?php
        class InitializedPropertyExample {
            public string $typed;
            public $untyped;
            public static string $typedStatic;
            public static $untypedStatic;
        }
        $object = new InitializedPropertyExample();
        $reflection = new ReflectionClass(InitializedPropertyExample::class);
        $typed = $reflection->getProperty('typed');
        $untyped = $reflection->getProperty('untyped');
        $typedStatic = $reflection->getProperty('typedStatic');
        $untypedStatic = $reflection->getProperty('untypedStatic');
        echo ($typed->isInitialized($object) ? 1 : 0), ':', ($untyped->isInitialized($object) ? 1 : 0), ':';
        $object->typed = 'ready';
        echo ($typed->isInitialized($object) ? 1 : 0), ':';
        unset($object->typed);
        echo ($typed->isInitialized($object) ? 1 : 0), ':';
        echo ($typedStatic->isInitialized() ? 1 : 0), ':', ($untypedStatic->isInitialized() ? 1 : 0), ':';
        InitializedPropertyExample::$typedStatic = 'ready';
        echo ($typedStatic->isInitialized() ? 1 : 0), ':';
        try { $untypedStatic->isInitialized(1); } catch (TypeError $error) { echo get_class($error), ':'; }
        echo method_exists('ReflectionProperty', 'isInitialized');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "0:1:1:0:0:1:1:TypeError:1"
    );
}

#[test]
fn reflection_property_distinguishes_implicit_and_declared_default_values() {
    let source = r#"<?php
        class PropertyDefaultsExample {
            public $implicit;
            public ?int $uninitialized;
            public ?int $explicitNull = null;
            public function __construct(public string $promoted = 'parameter-default') {}
        }
        $reflection = new ReflectionClass(PropertyDefaultsExample::class);
        $implicit = $reflection->getProperty('implicit');
        $typed = $reflection->getProperty('uninitialized');
        $explicit = $reflection->getProperty('explicitNull');
        $promoted = $reflection->getProperty('promoted');
        echo ($implicit->hasDefaultValue() ? 1 : 0), ':', ($typed->hasDefaultValue() ? 1 : 0), ':';
        echo ($explicit->hasDefaultValue() ? 1 : 0), ':', ($promoted->hasDefaultValue() ? 1 : 0), ':';
        echo ($implicit->getDefaultValue() === null ? 1 : 0), ':', ($explicit->getDefaultValue() === null ? 1 : 0), ':';
        echo ($typed->getDefaultValue() === null ? 1 : 0), ':', ($promoted->getDefaultValue() === null ? 1 : 0), ':';
        echo method_exists('ReflectionProperty', 'hasDefaultValue'), ':', method_exists('ReflectionProperty', 'getDefaultValue');
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "1:0:1:0:1:1:1:1:1:1");
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Deprecated));
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic
        .message
        .contains("ReflectionProperty::hasDefaultValue()")));
}

#[test]
fn reflection_class_reads_inherited_constants_and_checks_constant_presence() {
    let source = r#"<?php
        class ReflectionConstantParent { public const INHERITED = 14; }
        class ReflectionConstantChild extends ReflectionConstantParent { private const FALSE_VALUE = false; public const OWN = 'child'; }
        class ReflectionConstantOrder { public const ZETA = 1; public const ALPHA = 2; }
        $reflection = new ReflectionClass('ReflectionConstantChild');
        echo $reflection->getConstant(name: 'INHERITED'), ':', $reflection->getConstant('OWN'), ':';
        echo ($reflection->getConstant('FALSE_VALUE') === false), ':', $reflection->hasConstant('FALSE_VALUE'), ':';
        echo ($reflection->getConstant('Missing') === false), ':', $reflection->hasConstant('Missing'), ':';
        $public = $reflection->getConstants(filter: 1);
        $private = $reflection->getConstants(4);
        $order = (new ReflectionClass('ReflectionConstantOrder'))->getConstants();
        echo count($public), ':', $public['INHERITED'], ':', $public['OWN'], ':', count($private), ':', $private['FALSE_VALUE'], ':';
        echo implode(',', array_keys($order)), ':', method_exists('ReflectionClass', 'getConstant'), ':', method_exists('ReflectionClass', 'hasConstant'), ':', method_exists('ReflectionClass', 'getConstants');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "14:child:1:1:1::2:14:child:1::ZETA,ALPHA:1:1:1"
    );
}

#[test]
fn reflection_class_constant_exposes_metadata_and_inheritance() {
    let source = r#"<?php
        class ConstantMetadataParent { protected const string TOKEN = 'parent'; }
        class ConstantMetadataChild extends ConstantMetadataParent { final public const int OWN = 42; }
        $reflection = new ReflectionClass('ConstantMetadataChild');
        $token = $reflection->getReflectionConstant(name: 'TOKEN');
        $direct = new ReflectionClassConstant('ConstantMetadataParent', 'TOKEN');
        $own = $reflection->getReflectionConstant('OWN');
        echo $token->getName(), ':', $token->getValue(), ':', $token->isProtected(), ':', $token->getDeclaringClass()->getName(), ':', $direct->getValue(), ':';
        echo $own->getModifiers(), ':', $own->isPublic(), ':', $own->isFinal(), ':', ($reflection->getReflectionConstant('Missing') === false), ':';
        echo class_exists('ReflectionClassConstant'), ':', method_exists('ReflectionClass', 'getReflectionConstant'), ':', method_exists($own, 'getValue');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "TOKEN:parent:1:ConstantMetadataParent:parent:33:1:1:1:1:1:1"
    );
}

#[test]
fn reflection_class_constant_exposes_and_validates_attributes() {
    let source = r#"<?php
        #[Attribute(Attribute::TARGET_CLASS_CONSTANT | Attribute::IS_REPEATABLE)]
        class ConstantTag { public function __construct(public string $label) {} }
        #[Attribute(Attribute::TARGET_CLASS)] class ClassOnlyTag {}
        class TaggedConstants {
            #[ConstantTag('first'), ConstantTag(label: 'second')]
            #[ClassOnlyTag]
            public const VALUE = 7;
        }
        $constant = (new ReflectionClass('TaggedConstants'))->getReflectionConstant('VALUE');
        $attributes = $constant->getAttributes(ConstantTag::class);
        echo count($attributes), ':', $attributes[0]->getName(), ':', $attributes[1]->getArguments()['label'], ':', $attributes[0]->getTarget(), ':', $attributes[0]->isRepeated(), ':';
        echo $attributes[0]->newInstance()->label, ':';
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "2:ConstantTag:second:16:1:first:"
    );
    let invalid_target = compile_source("<?php #[Attribute(Attribute::TARGET_CLASS)] class ClassOnlyTag {} class TaggedConstant { #[ClassOnlyTag] public const VALUE = 1; } (new ReflectionClass('TaggedConstant'))->getReflectionConstant('VALUE')->getAttributes()[0]->newInstance();").unwrap();
    assert!(invalid_target
        .execute()
        .unwrap_err()
        .contains("cannot target this declaration"));
}

#[test]
fn reflection_class_constant_reports_types_and_enum_cases() {
    let source = r#"<?php
        interface DnfSecond {} interface DnfThird {}
        class TypedConstants { public const string TEXT = 'value'; public const int|string NUMBER = 3; public const int|(DnfSecond&DnfThird) DNF = 5; public const mixed ANY = null; public const RAW = 4; }
        enum TypedConstantEnum: string { case Ready = 'ready'; }
        $reflection = new ReflectionClass('TypedConstants');
        $text = $reflection->getReflectionConstant('TEXT');
        $union = $reflection->getReflectionConstant('NUMBER')->getType();
        $dnf = $reflection->getReflectionConstant('DNF')->getType();
        $intersection = $dnf->getTypes()[1];
        $raw = $reflection->getReflectionConstant('RAW');
        $mixed = $reflection->getReflectionConstant('ANY')->getType();
        $case = (new ReflectionEnum('TypedConstantEnum'))->getReflectionConstant('Ready');
        echo $text->hasType(), ':', $text->getType()->getName(), ':', $text->getType()->isBuiltin(), ':', $text->getType()->allowsNull(), ':';
        echo count($union->getTypes()), ':', $union->getTypes()[0]->getName(), ':', $union->allowsNull(), ':';
        echo count($dnf->getTypes()), ':', get_class($intersection), ':', count($intersection->getTypes()), ':';
        echo $raw->hasType(), ':', ($raw->getType() === null), ':', $case->isEnumCase(), ':', $case->hasType(), ':', ($case->getType() === null), ':', $mixed->allowsNull();
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:string:1::2:int::2:ReflectionIntersectionType:2::1:1::1:1"
    );
}

#[test]
fn typed_class_constants_validate_values_at_compile_time() {
    let valid = r#"<?php
        interface ConstantMarker {}
        enum ConstantEnum implements ConstantMarker { case Ready; }
        class ValidTypedConstants implements ConstantMarker { public const ?string NULL_VALUE = null; public const float NUMBER = 1; public const int|string TEXT_OR_NUMBER = 'ok'; public const ConstantMarker MARKER = ConstantEnum::Ready; }
    "#;
    compile_source(valid)
        .expect("nullable, int-to-float, and union typed constants should compile");
    let invalid =
        compile_source("<?php class InvalidTypedConstant { public const int VALUE = 'wrong'; }")
            .err()
            .expect("mismatched class constant type must fail compilation");
    assert_eq!(
        invalid,
        "Cannot assign string to class constant InvalidTypedConstant::VALUE of type int"
    );

    for invalid in [
        "<?php class CallableConstantType { public const callable VALUE = 'not-callable'; }",
        "<?php class VoidConstantType { public const void VALUE = null; }",
        "<?php class NeverConstantType { public const never VALUE = null; }",
        "<?php class UnionCallableConstantType { public const string|callable VALUE = 'text'; }",
    ] {
        let error = compile_source(invalid)
            .err()
            .expect("unsupported typed class constant should fail compilation");
        assert!(
            error.contains("cannot have type"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn reflection_get_constant_deprecates_missing_names_and_respects_error_reporting() {
    let source = r#"<?php
        class ReflectedConstants { public const EXISTS = 1; }
        $reflection = new ReflectionClass('ReflectedConstants');
        $reflection->getConstant('Missing');
        error_reporting(0);
        $reflection->getConstant('AlsoMissing');
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(
        result.diagnostics[0].level,
        php_runtime::DiagnosticLevel::Deprecated
    );
    assert_eq!(result.diagnostics[0].message, "ReflectionClass::getConstant() for a non-existent constant is deprecated, use ReflectionClass::hasConstant() to check if the constant exists");
    assert_eq!(result.diagnostics[0].source_line, Some(4));
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
fn property_and_call_magic_methods_validate_declared_types() {
    compile_source(r#"<?php
        class FullyTypedMagicMethods {
            public function __get(string $name): mixed { return null; }
            public function __set(string $name, mixed $value): void {}
            public function __isset(string $name): bool { return false; }
            public function __unset(string $name): void {}
            public function __call(string $name, array $arguments): mixed { return null; }
            public static function __callStatic(string $name, array $arguments): mixed { return null; }
            public function __toString(): string { return ''; }
            public function __clone(): void {}
            public function __invoke(string $value): bool { return true; }
        }
    "#).expect("PHP-compatible declared magic method types should compile");

    for invalid in [
        "<?php class WrongGetParameter { public function __get(int $name) {} }",
        "<?php class WrongSetValue { public function __set(string $name, string $value) {} }",
        "<?php class WrongIssetReturn { public function __isset(string $name): int { return 1; } }",
        "<?php class WrongUnsetReturn { public function __unset(string $name): bool {} }",
        "<?php class WrongCallArguments { public function __call(string $name, mixed $arguments) {} }",
        "<?php class WrongToStringReturn { public function __toString(): int { return 1; } }",
        "<?php class WrongCloneReturn { public function __clone(): int {} }",
    ] {
        assert!(compile_source(invalid).is_err(), "invalid magic method types compiled: {invalid}");
    }
}

#[test]
fn interface_variance_parses_parenthesized_dnf_types() {
    let valid = r#"<?php
        interface DnfReadable {}
        interface DnfCountable {}
        interface DnfLoggable {}
        interface DnfExtra {}
        interface DnfContract {
            public function handle((DnfReadable&DnfCountable)|DnfLoggable $value): (DnfReadable&DnfCountable)|DnfLoggable;
        }
        class DnfImplementation implements DnfContract {
            public function handle((DnfReadable&DnfCountable)|DnfLoggable|DnfExtra $value): (DnfReadable&DnfCountable)|DnfLoggable {
                return $value;
            }
        }
    "#;
    compile_source(valid)
        .expect("DNF parameter contravariance and return covariance should compile");

    for implementation in [
        "public function handle((DnfReadable&DnfCountable)|DnfExtra $value): (DnfReadable&DnfCountable)|DnfLoggable { return $value; }",
        "public function handle((DnfReadable&DnfCountable)|DnfLoggable $value): (DnfReadable&DnfCountable)|DnfLoggable|DnfExtra { return $value; }",
    ] {
        let source = format!(r#"<?php
            interface DnfReadable {{}}
            interface DnfCountable {{}}
            interface DnfLoggable {{}}
            interface DnfExtra {{}}
            interface DnfContract {{
                public function handle((DnfReadable&DnfCountable)|DnfLoggable $value): (DnfReadable&DnfCountable)|DnfLoggable;
            }}
            class InvalidDnfImplementation implements DnfContract {{ {implementation} }}
        "#);
        let error = match compile_source(&source) {
            Err(error) => error,
            Ok(_) => panic!("incompatible DNF variance unexpectedly compiled"),
        };
        assert!(error.to_ascii_lowercase().contains("does not match interface dnfcontract signature"), "{error}");
    }
}

#[test]
fn interface_variance_follows_builtin_throwable_stringable_inheritance() {
    let source = r#"<?php
        interface StringDisplayContract { public function render(): Stringable; }
        class ExceptionStringDisplay implements StringDisplayContract {
            public function render(): Exception { return new Exception('display'); }
        }
        class ErrorStringDisplay implements StringDisplayContract {
            public function render(): Error { return new Error('display'); }
        }
    "#;
    compile_source(source)
        .expect("Exception and Error satisfy Stringable through the Throwable interface");

    let invalid = r#"<?php
        interface StringInputContract { public function accept(Stringable $value): void; }
        class ExceptionOnlyInput implements StringInputContract {
            public function accept(Exception $value): void {}
        }
    "#;
    let error = match compile_source(invalid) {
        Err(error) => error,
        Ok(_) => panic!("narrowing Stringable to Exception must be rejected"),
    };
    assert!(error.contains("does not match interface stringinputcontract signature"), "{error}");
}

#[test]
fn traversable_subtypes_are_covariant_with_iterable_returns() {
    let valid = r#"<?php
        interface ReadableCollection extends Traversable {}
        interface IterableContract { public function values(): iterable; }
        class IterableImplementation implements IterableContract {
            public function values(): ReadableCollection {}
        }
        class ConcreteIterator implements Iterator {
            public function current(): mixed {}
            public function key(): mixed {}
            public function next(): void {}
            public function rewind(): void {}
            public function valid(): bool {}
        }
        interface ConcreteIterableContract { public function values(): iterable; }
        class ConcreteIterableImplementation implements ConcreteIterableContract {
            public function values(): ConcreteIterator { return new ConcreteIterator(); }
        }
        echo get_class((new ConcreteIterableImplementation())->values());
    "#;
    assert_eq!(
        compile_source(valid).unwrap().execute().unwrap(),
        "ConcreteIterator"
    );

    let invalid = r#"<?php
        interface ReadableCollection extends Traversable {}
        interface IterableParameterContract { public function accept(iterable $values): void; }
        class NarrowIterableParameter implements IterableParameterContract {
            public function accept(ReadableCollection $values): void {}
        }
    "#;
    let error = match compile_source(invalid) {
        Err(error) => error,
        Ok(_) => panic!("a narrower parameter cannot replace iterable"),
    };
    assert!(
        error
            .to_ascii_lowercase()
            .contains("does not match interface iterableparametercontract signature"),
        "{error}"
    );
}

#[test]
fn destructors_run_when_user_frames_unwind_through_exceptions() {
    let source = r#"<?php
        class FrameExitDestructor {
            public $label;
            public function __construct($label) { $this->label = $label; }
            public function __destruct() { echo $this->label, '|'; }
        }
        function failFromFunction() {
            $local = new FrameExitDestructor('function');
            throw new Exception('function failure');
        }
        class FrameExitMethod {
            public function fail() {
                $local = new FrameExitDestructor('method');
                throw new Exception('method failure');
            }
        }
        class FrameExitConstructor {
            public function __construct() {
                $local = new FrameExitDestructor('constructor');
                throw new Exception('constructor failure');
            }
        }
        try { failFromFunction(); } catch (Exception $error) { echo 'caught-function|'; }
        try { (new FrameExitMethod())->fail(); } catch (Exception $error) { echo 'caught-method|'; }
        try { new FrameExitConstructor(); } catch (Exception $error) { echo 'caught-constructor|'; }
        $fail = function () {
            $local = new FrameExitDestructor('closure');
            throw new Exception('closure failure');
        };
        try { $fail(); } catch (Exception $error) { echo 'caught-closure|'; }
        function failReturnType(): int {
            $local = new FrameExitDestructor('return-type');
            return 'wrong';
        }
        try { failReturnType(); } catch (TypeError $error) { echo 'caught-return-type|'; }
        echo 'end';
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "function|caught-function|method|caught-method|constructor|caught-constructor|closure|caught-closure|return-type|caught-return-type|end");
}

#[test]
fn property_hook_return_type_errors_release_hook_local_objects_before_catch() {
    let source = r#"<?php
        class PropertyHookTemporary {
            public function __destruct() { echo 'drop|'; }
        }
        class InvalidPropertyHookResult {
            public PropertyHookTemporary $item {
                get {
                    $temporary = new PropertyHookTemporary();
                    return 1;
                }
            }
        }
        try { (new InvalidPropertyHookResult())->item; }
        catch (TypeError $error) { echo 'caught|'; }
        echo 'end';
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "drop|caught|end"
    );
}

#[test]
fn property_hook_frames_release_temporaries_and_preserve_returned_objects() {
    let source = r#"<?php
        class PropertyHookLifetimeToken { public function __destruct() { echo 'drop|'; } }
        class PropertyHookLocalOwner {
            public int $value = 5;
            public int $computed {
                get {
                    $temporary = new PropertyHookLifetimeToken();
                    return $this->value;
                }
            }
        }
        class PropertyHookReturnedOwner {
            public PropertyHookLifetimeToken $value {
                get { return new PropertyHookLifetimeToken(); }
            }
        }
        echo (new PropertyHookLocalOwner())->computed, '|';
        $held = (new PropertyHookReturnedOwner())->value;
        echo 'held|';
        unset($held);
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "drop|5|held|drop|"
    );
}

#[test]
fn property_hook_exceptions_release_local_objects_before_the_caller_catch() {
    let source = r#"<?php
        class PropertyHookThrownTemporary { public function __destruct() { echo 'drop|'; } }
        class ThrowingPropertyHook {
            public int $value {
                get {
                    $temporary = new PropertyHookThrownTemporary();
                    throw new Exception('hook failed');
                }
            }
        }
        try { (new ThrowingPropertyHook())->value; }
        catch (Exception $error) { echo 'caught|'; }
        echo 'end';
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "drop|caught|end"
    );
}

#[test]
fn trait_exists_reports_declared_traits_and_missing_traits() {
    let source = r#"<?php trait PresentTrait {} echo trait_exists('PresentTrait'), ':', trait_exists('Tests\\CreatesApplication'), "\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1:\n");
}

#[test]
fn qualified_trait_use_keeps_the_full_namespace_path() {
    let source = r#"<?php
        namespace Example\Concerns; trait CallsCommands {}
        namespace Example;
        class ConsoleCommand { use Concerns\CallsCommands; }
        echo trait_exists('Example\\Concerns\\CallsCommands'), "\n";
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "1\n");
}

#[test]
fn get_called_class_tracks_late_static_binding_and_requires_class_context() {
    let source = r#"<?php
        class CalledBase { public static function who(): string { return get_called_class(); } }
        class CalledChild extends CalledBase {}
        echo CalledBase::who(), ':', CalledChild::who(), ':', function_exists('get_called_class'), ':';
        try { get_called_class(); } catch (Error $error) { echo get_class($error); }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "CalledBase:CalledChild:1:Error"
    );
}

#[test]
fn reflection_class_lists_direct_transitive_and_inherited_interfaces() {
    let source = r#"<?php
        interface ReflectRoot {}
        interface ReflectChild extends ReflectRoot {}
        interface ReflectSecond {}
        class ReflectParent implements ReflectRoot, ReflectSecond {}
        class ReflectLeaf extends ReflectParent implements ReflectChild {}
        $reflection = new ReflectionClass('ReflectLeaf');
        $names = $reflection->getInterfaceNames();
        $interfaces = $reflection->getInterfaces();
        echo implode(',', $names), ':', implode(',', array_keys($interfaces)), ':', $interfaces['ReflectChild']->getName(), ':', method_exists('ReflectionClass', 'getInterfaceNames'), ':', method_exists('ReflectionClass', 'getInterfaces');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ReflectRoot,ReflectSecond,ReflectChild:ReflectRoot,ReflectSecond,ReflectChild:ReflectChild:1:1");
}

#[test]
fn reflection_class_lists_only_traits_used_directly_by_the_reflected_type() {
    let source = r#"<?php
        trait ReflectionLeafTrait {}
        trait ReflectionNestedTrait { use ReflectionLeafTrait; }
        trait ReflectionDirectTrait {}
        class ReflectionTraitParent { use ReflectionDirectTrait; }
        class ReflectionTraitSubject extends ReflectionTraitParent { use ReflectionNestedTrait; }
        class ReflectionTraitChild extends ReflectionTraitSubject {}
        $reflection = new ReflectionClass('ReflectionTraitSubject');
        $names = $reflection->getTraitNames();
        $traits = $reflection->getTraits();
        $childNames = (new ReflectionClass('ReflectionTraitChild'))->getTraitNames();
        $trait = $traits['ReflectionNestedTrait'] ?? null;
        echo implode(',', $names), ':', implode(',', array_keys($traits)), ':', is_object($trait), ':', implode(',', $childNames), ':', method_exists('ReflectionClass', 'getTraitNames'), ':', method_exists('ReflectionClass', 'getTraits');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "ReflectionNestedTrait:ReflectionNestedTrait:1::1:1"
    );
}

#[test]
fn reflection_class_reports_trait_aliases_with_original_names() {
    let source = r#"<?php
        trait ReflectionAliasTrait { public function OriginalMethod(): string { return 'ok'; } }
        class ReflectionAliasSubject { use ReflectionAliasTrait { ReflectionAliasTrait::OriginalMethod as PublicAlias; } }
        class ReflectionAliasChild extends ReflectionAliasSubject {}
        $aliases = (new ReflectionClass('ReflectionAliasSubject'))->getTraitAliases();
        $childAliases = (new ReflectionClass('ReflectionAliasChild'))->getTraitAliases();
        echo implode(',', array_keys($aliases)), ':', $aliases['PublicAlias'], ':', count($childAliases), ':', method_exists('ReflectionClass', 'getTraitAliases');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "PublicAlias:ReflectionAliasTrait::OriginalMethod:0:1"
    );
}

#[test]
fn nested_trait_adaptations_validate_missing_methods_and_unresolved_conflicts() {
    let missing_method = compile_source(
        r#"<?php
        trait TraitMethodSource { public function present() {} }
        trait TraitMethodConsumer { use TraitMethodSource { TraitMethodSource::absent as alias; } }
    "#,
    )
    .err()
    .expect("missing nested trait method must fail");
    assert!(
        missing_method.contains("Trait method TraitMethodSource::absent not found"),
        "{missing_method}"
    );

    let unresolved_conflict = compile_source(
        r#"<?php
        trait FirstTraitChoice { public function choose() {} }
        trait SecondTraitChoice { public function choose() {} }
        trait UnusedConflictingTrait { use FirstTraitChoice, SecondTraitChoice; }
    "#,
    )
    .err()
    .expect("unresolved nested trait conflict must fail");
    assert!(
        unresolved_conflict
            .contains("Trait method conflict for choose in trait UnusedConflictingTrait"),
        "{unresolved_conflict}"
    );

    let unknown_excluded = compile_source(r#"<?php
        trait ExclusionSourceTrait { public function choose() {} }
        trait InvalidExclusionTrait { use ExclusionSourceTrait { ExclusionSourceTrait::choose insteadof MissingExclusionTrait; } }
    "#).err().expect("unknown excluded trait must fail");
    assert!(
        unknown_excluded.contains("Trait 'MissingExclusionTrait' is not part of the composition"),
        "{unknown_excluded}"
    );

    let class_unknown_excluded = compile_source(r#"<?php
        trait ClassExclusionSource { public function choose() {} }
        class InvalidClassExclusion { use ClassExclusionSource { ClassExclusionSource::choose insteadof MissingClassExclusion; } }
    "#).err().expect("class adaptation must reject a trait outside its composition");
    assert!(class_unknown_excluded.contains("Trait 'MissingClassExclusion' is not part of the composition of class InvalidClassExclusion"), "{class_unknown_excluded}");

    assert_eq!(compile_source(r#"<?php
        trait FirstResolvedChoice { public function choose() { return 'first'; } }
        trait SecondResolvedChoice { public function choose() { return 'second'; } }
        trait ResolvedNestedTrait {
            use FirstResolvedChoice, SecondResolvedChoice {
                FirstResolvedChoice::choose insteadof SecondResolvedChoice;
                SecondResolvedChoice::choose as chooseSecond;
            }
        }
    class UseResolvedNestedTrait { use ResolvedNestedTrait; }
        echo (new UseResolvedNestedTrait())->choose(), ':', (new UseResolvedNestedTrait())->chooseSecond();
    "#).unwrap().execute().unwrap(), "first:second");

    let nested_provider = r#"<?php
        trait NestedProviderLeaf { public function select() { return 'nested'; } }
        trait NestedProviderWrapper { use NestedProviderLeaf; }
        trait DirectProvider { public function select() { return 'direct'; } }
        trait ProviderConflictResolver {
            use NestedProviderWrapper, DirectProvider {
                NestedProviderWrapper::select insteadof DirectProvider;
                DirectProvider::select as selectDirect;
            }
        }
        class ProviderConflictSubject { use ProviderConflictResolver; }
        $subject = new ProviderConflictSubject();
        echo $subject->select(), ':', $subject->selectDirect();
    "#;
    assert_eq!(
        compile_source(nested_provider).unwrap().execute().unwrap(),
        "nested:direct"
    );
}

#[test]
fn visibility_adaptation_on_insteadof_loser_does_not_replace_selected_method() {
    let source = r#"<?php
        trait PreferredVisibilityTrait { public function choose() { return 'preferred'; } }
        trait LosingVisibilityTrait { public function choose() { return 'losing'; } }
        class VisibilityAdaptationWinner {
            use PreferredVisibilityTrait, LosingVisibilityTrait {
                PreferredVisibilityTrait::choose insteadof LosingVisibilityTrait;
                LosingVisibilityTrait::choose as protected;
            }
        }
        echo (new VisibilityAdaptationWinner())->choose();
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "preferred"
    );
}

#[test]
fn trait_method_final_adaptation_overrides_parent_and_blocks_child_override() {
    let source = r#"<?php
        class FinalAdaptationParent { public function identify() { return 'parent'; } }
        trait FinalAdaptationTrait { public function identify() { return 'trait'; } }
        class FinalAdaptationHost extends FinalAdaptationParent {
            use FinalAdaptationTrait { FinalAdaptationTrait::identify as final; }
        }
        class FinalAdaptationLocalOverride extends FinalAdaptationParent {
            use FinalAdaptationTrait { FinalAdaptationTrait::identify as final; }
            public function identify() { return 'local'; }
        }
        trait AnonymousFinalAdaptationTrait { public function identify() { return 'anonymous'; } }
        $anonymous = new class { use AnonymousFinalAdaptationTrait { AnonymousFinalAdaptationTrait::identify as final; } };
        echo (new FinalAdaptationHost())->identify(), ':', (new FinalAdaptationLocalOverride())->identify(), ':', $anonymous->identify();
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "trait:local:anonymous"
    );

    let child_override = compile_source(r#"<?php
        trait FinalAdaptationSource { public function identify() {} }
        class FinalAdaptationBase { use FinalAdaptationSource { FinalAdaptationSource::identify as final; } }
        class FinalAdaptationChild extends FinalAdaptationBase { public function identify() {} }
    "#).err().expect("child must not override a final trait-adapted method");
    assert!(
        child_override.contains("cannot override final method identify()"),
        "{child_override}"
    );
}

#[test]
fn trait_final_alias_adaptation_marks_only_the_alias_final() {
    let alias_only = r#"<?php
        trait AliasFinalSource { public function original() { return 'original'; } }
        class AliasFinalHost { use AliasFinalSource { AliasFinalSource::original as final aliasMethod; } }
        class AliasFinalChild extends AliasFinalHost { public function original() { return 'child'; } }
        echo (new AliasFinalChild())->original();
    "#;
    assert_eq!(
        compile_source(alias_only).unwrap().execute().unwrap(),
        "child"
    );

    let child_alias_override = compile_source(r#"<?php
        trait AliasFinalChildSource { public function original() {} }
        class AliasFinalParent { use AliasFinalChildSource { AliasFinalChildSource::original as final aliasMethod; } }
        class AliasFinalOverride extends AliasFinalParent { public function aliasMethod() {} }
    "#).err().expect("child must not override a final alias");
    assert!(
        child_alias_override.contains("cannot override final method aliasmethod()"),
        "{child_alias_override}"
    );
}

#[test]
fn nested_trait_final_adaptation_propagates_to_consuming_class() {
    let source = r#"<?php
        trait NestedFinalSource { public function locked() {} }
        trait NestedFinalAdapter { use NestedFinalSource { NestedFinalSource::locked as final; } }
        class NestedFinalParent { use NestedFinalAdapter; }
        class NestedFinalChild extends NestedFinalParent { public function locked() {} }
    "#;
    let error = compile_source(source)
        .err()
        .expect("nested final adaptation must propagate");
    assert!(
        error.contains("cannot override final method locked()"),
        "{error}"
    );
}

#[test]
fn trait_properties_compose_recursively_and_require_compatible_declarations() {
    let compatible = r#"<?php
        trait PropertyShapeOne { public int $value = 7; }
        trait PropertyShapeTwo { public int $value = 7; }
        trait NestedPropertyShape { use PropertyShapeOne; }
        class CompatiblePropertyShape { use NestedPropertyShape, PropertyShapeTwo; }
        trait UnionPropertyShapeOne { public int|string $item; }
        trait UnionPropertyShapeTwo { public string|int $item; }
        class CompatibleUnionPropertyShape { use UnionPropertyShapeOne, UnionPropertyShapeTwo; }
        echo (new CompatiblePropertyShape())->value;
    "#;
    assert_eq!(compile_source(compatible).unwrap().execute().unwrap(), "7");

    let incompatible_traits = compile_source(
        r#"<?php
        trait PropertyConflictOne { public int $value = 7; }
        trait PropertyConflictTwo { public int $value = 8; }
        class IncompatiblePropertyTraits { use PropertyConflictOne, PropertyConflictTwo; }
    "#,
    )
    .err()
    .expect("incompatible trait properties must fail");
    assert!(
        incompatible_traits
            .contains("Trait property conflict for $value in class IncompatiblePropertyTraits"),
        "{incompatible_traits}"
    );

    let incompatible_class = compile_source(r#"<?php
        trait PropertyClassSource { public string $value = 'same'; }
        class IncompatibleTraitPropertyOverride { use PropertyClassSource; public string $value = 'different'; }
    "#).err().expect("incompatible class property override must fail");
    assert!(
        incompatible_class.contains(
            "Property IncompatibleTraitPropertyOverride::$value conflicts with a trait property"
        ),
        "{incompatible_class}"
    );

    let enum_trait_property = compile_source(
        r#"<?php
        trait EnumPropertyTrait { public int $value = 1; }
        enum InvalidPropertyEnum { use EnumPropertyTrait; case One; }
    "#,
    )
    .err()
    .expect("enum trait properties must fail");
    assert!(
        enum_trait_property.contains("Enum InvalidPropertyEnum cannot include properties"),
        "{enum_trait_property}"
    );

    let case_sensitive = r#"<?php
        trait LowercaseProperty { public int $value = 1; }
        trait UppercaseProperty { public int $Value = 2; }
        class CaseSensitiveTraitProperties { use LowercaseProperty, UppercaseProperty; }
        $object = new CaseSensitiveTraitProperties();
        echo $object->value, ':', $object->Value;
    "#;
    assert_eq!(
        compile_source(case_sensitive).unwrap().execute().unwrap(),
        "1:2"
    );
}

#[test]
fn trait_constants_compose_recursively_and_require_compatible_declarations() {
    let compatible = r#"<?php
        trait BaseTraitConstants { public const int BASE = 4; }
        trait DerivedTraitConstants { use BaseTraitConstants; public const int DERIVED = self::BASE + 3; }
        trait MatchingTraitConstant { public const int BASE = 4; }
        class CompatibleTraitConstants {
            use DerivedTraitConstants, MatchingTraitConstant;
            public const int BASE = 4;
        }
        echo CompatibleTraitConstants::DERIVED, ':', (new ReflectionClass('CompatibleTraitConstants'))->getConstant('BASE');
    "#;
    assert_eq!(
        compile_source(compatible).unwrap().execute().unwrap(),
        "7:4"
    );

    let incompatible_traits = compile_source(
        r#"<?php
        trait FirstTraitConstant { public const string VALUE = 'first'; }
        trait SecondTraitConstant { public const string VALUE = 'second'; }
        class IncompatibleTraitConstants { use FirstTraitConstant, SecondTraitConstant; }
    "#,
    )
    .err()
    .expect("incompatible trait constants must fail");
    assert!(
        incompatible_traits
            .contains("Trait constant conflict for VALUE in class IncompatibleTraitConstants"),
        "{incompatible_traits}"
    );

    let incompatible_class = compile_source(r#"<?php
        trait TraitConstantOverride { public const string VALUE = 'trait'; }
        class IncompatibleClassConstant { use TraitConstantOverride; public const string VALUE = 'class'; }
    "#).err().expect("incompatible class constant override must fail");
    assert!(
        incompatible_class.contains(
            "Class constant IncompatibleClassConstant::VALUE conflicts with a trait constant"
        ),
        "{incompatible_class}"
    );
}

#[test]
fn reflection_class_checks_inherited_and_missing_members_by_name() {
    let source = r#"<?php
        function reflectionMemberFlag($value) { return $value ? '1' : '0'; }
        class ReflectionMemberBase { private function hiddenMethod() {} protected $hiddenProperty; private $privateProperty; public static $staticProperty; }
        class ReflectionMemberChild extends ReflectionMemberBase { public function visibleMethod() {} public $visibleProperty; }
        $reflection = new ReflectionClass('ReflectionMemberChild');
        echo reflectionMemberFlag($reflection->hasMethod(name: 'hiddenMethod')), ':', reflectionMemberFlag($reflection->hasMethod('VISIBLEMETHOD')), ':', reflectionMemberFlag($reflection->hasMethod('missingMethod')), ':';
        echo reflectionMemberFlag($reflection->hasProperty(name: 'hiddenProperty')), ':', reflectionMemberFlag($reflection->hasProperty('staticProperty')), ':', reflectionMemberFlag($reflection->hasProperty('visibleProperty')), ':', reflectionMemberFlag($reflection->hasProperty('VisibleProperty')), ':', reflectionMemberFlag($reflection->hasProperty('missingProperty')), ':', reflectionMemberFlag($reflection->hasProperty('privateProperty')), ':';
        echo reflectionMemberFlag(method_exists('ReflectionClass', 'hasMethod')), ':', reflectionMemberFlag(method_exists('ReflectionClass', 'hasProperty'));
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:1:0:1:1:1:0:0:0:1:1"
    );
}

#[test]
fn reflection_method_reports_visibility_modifiers_and_special_method_kind() {
    let source = r#"<?php
        abstract class ReflectedMethodBase {
            abstract protected function needsImplementation();
            final public static function fixedStatic() {}
            private function hidden() {}
            public function __construct() {}
            public function __destruct() {}
        }
        class ReflectedMethodChild extends ReflectedMethodBase { protected function needsImplementation() {} }
        $reflection = new ReflectionClass('ReflectedMethodChild');
        $base = new ReflectionClass('ReflectedMethodBase');
        echo $reflection->getMethod('hidden')->getDeclaringClass()->getName(), ':';
        echo $reflection->getMethod('needsImplementation')->isProtected() ? '1' : '0';
        echo $reflection->getMethod('hidden')->isPrivate() ? '1' : '0';
        echo $reflection->getMethod('fixedStatic')->isStatic() ? '1' : '0';
        echo $reflection->getMethod('fixedStatic')->isFinal() ? '1' : '0';
        echo $reflection->getMethod('__construct')->isConstructor() ? '1' : '0';
        echo $reflection->getMethod('__destruct')->isDestructor() ? '1' : '0';
        echo $base->getMethod('needsImplementation')->isAbstract() ? '1' : '0';
        echo $reflection->getMethod('fixedStatic')->isPublic() ? '1' : '0';
        echo $reflection->getMethod('needsImplementation')->isPrivate() ? '1' : '0';
        echo
            ':', method_exists('ReflectionMethod', 'isPublic'), method_exists('ReflectionMethod', 'isAbstract'), method_exists('ReflectionMethod', 'getDeclaringClass');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "ReflectedMethodBase:111111110:111"
    );
}

#[test]
fn reflection_class_lists_methods_in_declaration_order_and_applies_any_modifier_filter() {
    let source = r#"<?php
        trait ReflectedMethodListTrait { public function traitMethod() {} }
        class ReflectedMethodListBase {
            protected final function protectedFinal() {}
            public function inheritedMethod() {}
            public function replacedMethod() {}
        }
        class ReflectedMethodListChild extends ReflectedMethodListBase {
            use ReflectedMethodListTrait { ReflectedMethodListTrait::traitMethod as traitAlias; }
            private function childFirst() {}
            final public static function selectedStaticFinal() {}
            public function replacedMethod() {}
        }
        $reflection = new ReflectionClass('ReflectedMethodListChild');
        $names = [];
        foreach ($reflection->getMethods() as $method) { $names[] = $method->getName(); }
        $filtered = [];
        foreach ($reflection->getMethods(filter: ReflectionMethod::IS_STATIC | ReflectionMethod::IS_FINAL) as $method) { $filtered[] = $method->getName(); }
        $selected = $reflection->getMethod('selectedStaticFinal');
        echo implode(',', $names), ':', implode(',', $filtered), ':', $selected->getModifiers(), ':',
            ReflectionMethod::IS_PUBLIC | ReflectionMethod::IS_STATIC | ReflectionMethod::IS_FINAL, ':',
            method_exists('ReflectionClass', 'getMethods'), ':', method_exists('ReflectionMethod', 'getModifiers');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "traitMethod,traitAlias,childFirst,selectedStaticFinal,replacedMethod,protectedFinal,inheritedMethod:selectedStaticFinal,protectedFinal:49:49:1:1");
}

#[test]
fn reflection_class_filter_arguments_require_nullable_integers() {
    let source = r#"<?php
        class ReflectionFilterArguments { public function method() {} public $property; }
        $reflection = new ReflectionClass(ReflectionFilterArguments::class);
        echo count($reflection->getMethods('1.9')), ':', count($reflection->getProperties('1.9')), ';';
        try { $reflection->getMethods([]); }
        catch (TypeError $error) { echo get_class($error), ':', $error->getMessage(), ';'; }
        try { $reflection->getProperties('not numeric'); }
        catch (TypeError $error) { echo get_class($error), ':', $error->getMessage(); }
        try { $reflection->getMethods(new stdClass()); }
        catch (TypeError $error) { echo ';', get_class($error), ':', $error->getMessage(); }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), concat!(
        "1:1;TypeError:ReflectionClass::getMethods(): Argument #1 ($filter) must be of type ?int, array given;",
        "TypeError:ReflectionClass::getProperties(): Argument #1 ($filter) must be of type ?int, string given;",
        "TypeError:ReflectionClass::getMethods(): Argument #1 ($filter) must be of type ?int, stdClass given",
    ));
}

#[test]
fn readonly_properties_can_only_be_unset_before_initialization_from_declaring_scope() {
    let source = r#"<?php
        class ReadonlyUnsetFixture {
            public readonly int $initialized;
            public readonly int $uninitialized;
            public function __construct() { $this->initialized = 7; }
            public function unsetUninitialized() { unset($this->uninitialized); }
            public function unsetInitialized() { unset($this->initialized); }
        }
        $fixture = new ReadonlyUnsetFixture();
        try { unset($fixture->initialized); }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(), ';'; }
        try { unset($fixture->uninitialized); }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(), ';'; }
        $fixture->unsetUninitialized();
        try { $fixture->unsetInitialized(); }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(), ';'; }
        $name = 'initialized';
        try { unset($fixture->$name); }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(), ';'; }
        echo $fixture->initialized;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), concat!(
        "Error:Cannot unset readonly property ReadonlyUnsetFixture::$initialized;",
        "Error:Cannot unset readonly property ReadonlyUnsetFixture::$uninitialized from global scope;",
        "Error:Cannot unset readonly property ReadonlyUnsetFixture::$initialized;",
        "Error:Cannot unset readonly property ReadonlyUnsetFixture::$initialized;7",
    ));
}

#[test]
fn reflection_method_reports_parameter_counts_and_return_type() {
    let source = r#"<?php
        class MethodSignatureReflectionSubject {
            public function typed(int &$first, string $optional = 'x', mixed ...$rest): ?string { return null; }
            public function untyped($value) {}
        }
        function function_signature_reflection(int $first, $optional = null): bool { return true; }
        $typed = (new ReflectionClass('MethodSignatureReflectionSubject'))->getMethod('typed');
        $untyped = (new ReflectionClass('MethodSignatureReflectionSubject'))->getMethod('untyped');
        echo $typed->getNumberOfParameters(), ':', $typed->getNumberOfRequiredParameters(), ':';
        echo $typed->hasReturnType() ? $typed->getReturnType()->getName() . ':' . ($typed->getReturnType()->allowsNull() ? 'nullable' : 'required') : 'none', ':';
        echo $untyped->hasReturnType() ? 'typed' : 'none', ':', $untyped->getReturnType() === null ? 'null' : 'type';
        $parameters = $typed->getParameters();
        echo ':', $parameters[0]->getType()->getName(), ':', $parameters[0]->isPassedByReference(), ':', $parameters[1]->isOptional(), ':', $parameters[1]->getDefaultValue(), ':', $parameters[2]->isVariadic(), ':', $parameters[2]->isOptional();
        $function = new ReflectionFunction('function_signature_reflection');
        $functionParameters = $function->getParameters();
        echo ':', $function->getNumberOfParameters(), ':', $function->getNumberOfRequiredParameters(), ':', $function->getReturnType()->getName(), ':', $functionParameters[1]->isDefaultValueAvailable(), ':', $functionParameters[1]->getDefaultValue() === null;
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "3:1:string:nullable:none:null:int:1:1:x:1:1:2:1:bool:1:1"
    );
}

#[test]
fn reflection_method_invokes_non_public_and_static_methods() {
    let source = r#"<?php
        class InvokableReflectionSubject {
            private function secret(string $value, string $suffix = '!'): string { return $value . $suffix; }
            private function increment(int &$value): int { return ++$value; }
            protected static function combine(int $left, int $right): int { return $left + $right; }
        }
        $subject = new InvokableReflectionSubject();
        $secret = new ReflectionMethod($subject, 'secret');
        $combine = new ReflectionMethod('InvokableReflectionSubject::combine');
        $factoryMethod = ReflectionMethod::createFromMethodName('InvokableReflectionSubject::combine');
        $secretClosure = $secret->getClosure($subject);
        $combineClosure = $combine->getClosure();
        echo $secret->invoke($subject, 'hello'), ':', $secret->invokeArgs($subject, ['works', '?']), ':', $secret->invokeArgs($subject, ['suffix' => '?', 'value' => 'named']), ':';
        $number = 4;
        $increment = new ReflectionMethod('InvokableReflectionSubject', 'increment');
        echo $increment->invokeArgs($subject, ['value' => &$number]), ':', $number, ':';
        echo $combine->invoke(null, 2, 3), ':', $combine->invokeArgs(null, [4, 5]), ':', $factoryMethod->invoke(null, 6, 7), ':', $secretClosure('closure'), ':', $combineClosure(8, 9), ':';
        echo method_exists('ReflectionMethod', 'invoke'), method_exists('ReflectionMethod', 'invokeArgs'), method_exists('ReflectionMethod', 'getClosure'), method_exists('ReflectionMethod', 'createFromMethodName'), ':';
        try { new ReflectionMethod('InvokableReflectionSubject', 'missing'); }
        catch (ReflectionException $error) { echo 'missing'; }
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(
        result.output,
        "hello!:works?:named?:5:5:5:9:13:closure!:17:1111:missing"
    );
    assert_eq!(result.diagnostics.len(), 1);
    assert!(result.diagnostics[0]
        .message
        .contains("ReflectionMethod::__construct() with 1 argument is deprecated"));
}

#[test]
fn reflection_function_abstract_reports_variadic_reference_static_and_closure_metadata() {
    let source = r#"<?php
        class FunctionAbstractReflectionSubject {
            public int $stored = 1;
            public function &referenceResult(): int { return $this->stored; }
            public function variadicMethod(...$values) {}
        }
        function &function_reference_result(): int { static $stored = 2; return $stored; }
        function function_variadic_values(...$values) {}
        $class = new ReflectionClass('FunctionAbstractReflectionSubject');
        $methodReference = $class->getMethod('referenceResult');
        $methodVariadic = $class->getMethod('variadicMethod');
        $functionReference = new ReflectionFunction('function_reference_result');
        $functionVariadic = new ReflectionFunction('function_variadic_values');
        $closure = new ReflectionFunction(static function (...$values) {});
        echo ($methodReference->returnsReference() ? '1' : '0'), ':', ($methodReference->isVariadic() ? '1' : '0'), ':', ($methodReference->isClosure() ? '1' : '0'), '|';
        echo ($methodVariadic->returnsReference() ? '1' : '0'), ':', ($methodVariadic->isVariadic() ? '1' : '0'), '|';
        echo ($functionReference->returnsReference() ? '1' : '0'), ':', ($functionReference->isClosure() ? '1' : '0'), ':', ($functionReference->isStatic() ? '1' : '0'), '|';
        echo ($functionVariadic->isVariadic() ? '1' : '0'), '|';
        echo ($closure->isClosure() ? '1' : '0'), ':', ($closure->isStatic() ? '1' : '0'), ':', ($closure->isVariadic() ? '1' : '0');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:0:0|0:1|1:0:0|1|1:1:1"
    );
}

#[test]
fn reflection_function_invokes_named_and_closure_functions() {
    let source = r#"<?php
        function reflection_function_join(string $left, string $right = '!'): string { return $left . $right; }
        function reflection_function_increment(int &$value): int { return ++$value; }
        $reflection = new ReflectionFunction('reflection_function_join');
        $number = 4;
        $increment = new ReflectionFunction('reflection_function_increment');
        $closure = new ReflectionFunction(static function (string $value): string { return strtoupper($value); });
        echo $reflection->invoke('call'), ':', $reflection->invokeArgs(['right' => '?', 'left' => 'named']), ':';
        echo $increment->invokeArgs(['value' => &$number]), ':', $number, ':';
        echo $reflection->getClosure()('wrapped'), ':', $closure->invoke('closure'), ':', $closure->getClosure()('copy'), ':';
        echo method_exists('ReflectionFunction', 'invoke'), method_exists('ReflectionFunction', 'invokeArgs'), method_exists('ReflectionFunction', 'getClosure');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "call!:named?:5:5:wrapped!:CLOSURE:COPY:111"
    );
}

#[test]
fn reflection_parameter_reports_nullability_from_types_and_defaults() {
    let source = r#"<?php
        function reflected_nullability($untyped, int $required, ?string $nullable, mixed $mixed, string $defaultNull = null, string $defaultValue = 'x') {}
        $parameters = (new ReflectionFunction('reflected_nullability'))->getParameters();
        foreach ($parameters as $parameter) { echo $parameter->allowsNull() ? '1' : '0'; }
        echo ':', method_exists('ReflectionParameter', 'allowsNull');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "101110:1"
    );
}

#[test]
fn reflection_parameter_reports_its_declaring_function_and_class() {
    let source = r#"<?php
        function parameter_origin_function($value) {}
        class ParameterOriginSubject { public function run($value) {} }
        class ParameterOriginChild extends ParameterOriginSubject {}
        $functionParameter = (new ReflectionFunction('parameter_origin_function'))->getParameters()[0];
        $closureParameter = (new ReflectionFunction(static function ($value) {}))->getParameters()[0];
        $methodParameter = (new ReflectionMethod('ParameterOriginChild', 'run'))->getParameters()[0];
        echo $functionParameter->getDeclaringFunction()->getName(), ':', ($functionParameter->getDeclaringClass() === null ? 'null' : 'class'), '|';
        echo $closureParameter->getDeclaringFunction()->isClosure() ? 'closure' : 'function', ':', ($closureParameter->getDeclaringClass() === null ? 'null' : 'class'), '|';
        echo $methodParameter->getDeclaringFunction()->getName(), ':', $methodParameter->getDeclaringClass()->getName(), ':';
        echo method_exists('ReflectionParameter', 'getDeclaringFunction'), method_exists('ReflectionParameter', 'getDeclaringClass');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "parameter_origin_function:null|closure:null|run:ParameterOriginSubject:11"
    );
}

#[test]
fn reflection_parameter_constructor_resolves_functions_closures_and_callable_methods() {
    let source = r#"<?php
        #[Attribute(Attribute::TARGET_PARAMETER)] class DirectParameterTag { public $value; public function __construct($value) { $this->value = $value; } }
        function directly_reflected_function(#[DirectParameterTag('function')] int $number, ?string $label = null) {}
        class DirectParameterSubject { public function run(#[DirectParameterTag('method')] string $value = 'default') {} }
        class DirectInvokableSubject { public function __invoke(bool $enabled, ...$rest) {} }
        $functionByName = new ReflectionParameter('directly_reflected_function', 'number');
        $functionByPosition = new ReflectionParameter('directly_reflected_function', 1);
        $closure = static function (float $amount) {};
        $closureParameter = new ReflectionParameter($closure, 0);
        $methodByArray = new ReflectionParameter(['DirectParameterSubject', 'run'], 'value');
        $methodByString = new ReflectionParameter('DirectParameterSubject::run', 0);
        $objectMethod = new ReflectionParameter([new DirectParameterSubject(), 'run'], 'value');
        class DirectParameterChild extends DirectParameterSubject {}
        $inheritedMethod = new ReflectionParameter(['DirectParameterChild', 'run'], 'value');
        $invokable = new ReflectionParameter(new DirectInvokableSubject(), 0);
        echo $functionByName->getName(), ':', $functionByName->getType()->getName(), ':', $functionByName->getAttributes()[0]->newInstance()->value, '|';
        echo $functionByPosition->getName(), ':', $functionByPosition->getDefaultValue() === null, ':', $functionByPosition->allowsNull(), '|';
        echo $closureParameter->getDeclaringFunction()->isClosure(), ':', $closureParameter->getType()->getName(), '|';
        echo $methodByArray->getDeclaringClass()->getName(), ':', $methodByArray->getAttributes()[0]->newInstance()->value, ':', $methodByString->getDefaultValue(), ':', $objectMethod->getName(), ':', $inheritedMethod->getDeclaringClass()->getName(), '|';
        echo $invokable->getDeclaringFunction()->getName(), ':', ($invokable->isPassedByReference() ? '1' : '0'), ':';
        echo method_exists('ReflectionParameter', '__construct');
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "number:int:function|label:1:1|1:float|DirectParameterSubject:method:default:value:DirectParameterSubject|__invoke:0:1");

    let missing_name = compile_source("<?php function parameter_target($value) {} new ReflectionParameter('parameter_target', 'missing');").unwrap().execute().unwrap_err();
    assert!(
        missing_name.contains("ReflectionException: Parameter $missing does not exist"),
        "{missing_name}"
    );
    let missing_position = compile_source("<?php function parameter_target($value) {} new ReflectionParameter('parameter_target', 2);").unwrap().execute().unwrap_err();
    assert!(
        missing_position.contains("ReflectionException: Parameter number 2 does not exist"),
        "{missing_position}"
    );
}

#[test]
fn reflection_parameter_reports_constructor_property_promotion() {
    let source = r#"<?php
        class PromotedParameterSubject {
            public function __construct(public string $visible, private int $hidden, $ordinary = 'x') {}
        }
        $parameters = (new ReflectionMethod('PromotedParameterSubject', '__construct'))->getParameters();
        $direct = new ReflectionParameter(['PromotedParameterSubject', '__construct'], 'hidden');
        echo ($parameters[0]->isPromoted() ? '1' : '0'), ($parameters[1]->isPromoted() ? '1' : '0'), ($parameters[2]->isPromoted() ? '1' : '0'), ':';
        echo ($direct->isPromoted() ? '1' : '0'), ':', method_exists('ReflectionParameter', 'isPromoted');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "110:1:1"
    );
}

#[test]
fn reflection_parameter_reports_named_constant_defaults() {
    let source = r#"<?php
        const GLOBAL_PARAMETER_DEFAULT = 'global';
        class ConstantDefaultSubject {
            public const CLASS_PARAMETER_DEFAULT = 8;
            public function run($classConstant = self::CLASS_PARAMETER_DEFAULT, $literal = 3) {}
        }
        function constant_default_function($globalConstant = GLOBAL_PARAMETER_DEFAULT, $literal = 'text', $explicitNull = null) {}
        $function = (new ReflectionFunction('constant_default_function'))->getParameters();
        $method = (new ReflectionMethod('ConstantDefaultSubject', 'run'))->getParameters();
        echo ($function[0]->isDefaultValueConstant() ? '1' : '0'), ':', $function[0]->getDefaultValueConstantName(), ':', $function[0]->getDefaultValue(), '|';
        echo ($function[1]->isDefaultValueConstant() ? '1' : '0'), ':', ($function[1]->getDefaultValueConstantName() === null ? 'null' : 'name'), ':', $function[2]->getDefaultValueConstantName() === null, '|';
        echo ($method[0]->isDefaultValueConstant() ? '1' : '0'), ':', $method[0]->getDefaultValueConstantName(), ':', $method[0]->getDefaultValue(), '|';
        echo method_exists('ReflectionParameter', 'isDefaultValueConstant'), method_exists('ReflectionParameter', 'getDefaultValueConstantName');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:GLOBAL_PARAMETER_DEFAULT:global|0:null:1|1:self::CLASS_PARAMETER_DEFAULT:8|11"
    );

    let missing = compile_source("<?php function no_default_parameter($value) {} (new ReflectionFunction('no_default_parameter'))->getParameters()[0]->getDefaultValueConstantName();").unwrap().execute().unwrap_err();
    assert!(
        missing.contains("ReflectionException: Parameter $value does not have a default value"),
        "{missing}"
    );
}

#[test]
fn reflection_parameter_reports_value_passing_compatibility() {
    let source = r#"<?php
        function reflected_value_passing(&$reference, $ordinary, &...$references) {}
        $parameters = (new ReflectionFunction('reflected_value_passing'))->getParameters();
        foreach ($parameters as $parameter) {
            echo ($parameter->canBePassedByValue() ? '1' : '0'), ':', ($parameter->isPassedByReference() ? '1' : '0'), '|';
        }
        echo method_exists('ReflectionParameter', 'canBePassedByValue');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "0:1|1:0|0:1|1"
    );
}

#[test]
fn deprecated_reflection_parameter_type_helpers_inspect_named_and_union_types() {
    let source = r#"<?php
        class DeprecatedTypeSubject {}
        function deprecated_type_helpers(DeprecatedTypeSubject $object, array|int $array, callable|string $callback, int $number, $untyped) {}
        $parameters = (new ReflectionFunction('deprecated_type_helpers'))->getParameters();
        echo $parameters[0]->getClass()->getName(), '|';
        echo ($parameters[1]->isArray() ? '1' : '0'), ($parameters[2]->isCallable() ? '1' : '0');
        echo ($parameters[3]->getClass() === null ? '1' : '0'), ($parameters[3]->isArray() ? '1' : '0'), ($parameters[4]->isCallable() ? '1' : '0');
        echo ':', method_exists('ReflectionParameter', 'getClass'), method_exists('ReflectionParameter', 'isArray'), method_exists('ReflectionParameter', 'isCallable');
    "#;
    let result = compile_source(source)
        .unwrap()
        .execute_with_globals(Default::default())
        .unwrap();
    assert_eq!(result.output, "DeprecatedTypeSubject|11100:111");
    assert_eq!(result.diagnostics.len(), 6);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.level == php_runtime::DiagnosticLevel::Deprecated));
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic
        .message
        .contains("deprecated since 8.0, use ReflectionParameter::getType() instead")));
}

#[test]
fn gc_collect_cycles_runs_destructors_and_releases_unreachable_object_cycles() {
    let source = r#"<?php
        class CollectedObjectCycle {
            public $self;
            public function __destruct() { echo 'destroyed'; }
        }
        $live = new CollectedObjectCycle();
        $live->self = $live;
        echo gc_collect_cycles(), ':';
        unset($live);
        echo gc_collect_cycles(), ':', function_exists('gc_collect_cycles');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "0:destroyed1:1"
    );
}

#[test]
fn gc_collect_cycles_traces_object_edges_through_arrays_and_collects_pairs() {
    let source = r#"<?php
        class NestedObjectCycle {
            public $links = [];
            public function __destruct() { echo 'd'; }
        }
        $left = new NestedObjectCycle();
        $right = new NestedObjectCycle();
        $left->links = ['next' => $right];
        $right->links = ['next' => $left];
        unset($left, $right);
        echo ':', gc_collect_cycles();
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), ":dd2");
}

#[test]
fn gc_collect_cycles_preserves_objects_resurrected_by_their_destructor() {
    let source = r#"<?php
        class ResurrectedCycleObject {
            public $self;
            public function __destruct() { $GLOBALS['resurrected_cycle'] = $this; echo 'd'; }
        }
        $value = new ResurrectedCycleObject();
        $value->self = $value;
        unset($value);
        $collected = gc_collect_cycles();
        echo ':', $collected, ':', isset($GLOBALS['resurrected_cycle']->self) ? 'alive' : 'broken';
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "d:0:alive"
    );
}

#[test]
fn gc_collect_cycles_traces_closure_captures_and_suspended_generator_state() {
    let source = r#"<?php
        class ClosureOwnedCycle {
            public $callback;
            public function __destruct() { echo 'c'; }
        }
        $closureNode = new ClosureOwnedCycle();
        $closureNode->callback = function () use ($closureNode) { return $closureNode; };
        unset($closureNode);
        echo gc_collect_cycles(), ':';

        function holdGeneratorValue($value) { yield $value; }
        class GeneratorOwnedCycle {
            public $generator;
            public function __destruct() { echo 'g'; }
        }
        $generatorNode = new GeneratorOwnedCycle();
        $generatorNode->generator = holdGeneratorValue($generatorNode);
        unset($generatorNode);
        echo gc_collect_cycles();
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "c1:g1");
}

#[test]
fn inactive_anonymous_class_defers_missing_trait_resolution() {
    let source = r#"<?php if (false) { $value = new class { use MissingTrait; }; } echo "ok\n";"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ok\n");
}

#[test]
fn trait_exists_guard_defers_namespaced_missing_trait_in_anonymous_class() {
    let source = r#"<?php
        namespace Tests;
        if (trait_exists(\Tests\CreatesApplication::class)) {
            $application = new class { use CreatesApplication; };
        }
        echo "guarded\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "guarded\n"
    );
}

#[test]
fn instantiated_anonymous_class_reports_missing_trait() {
    let source = r#"<?php $value = new class { use MissingTrait; };"#;
    let error = compile_source(source).unwrap().execute().unwrap_err();
    assert!(error.contains("Trait 'MissingTrait' not found"), "{error}");
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
    let error = match compile_source(source) {
        Err(error) => error,
        Ok(_) => panic!("expected incompatible promoted property error"),
    };
    assert!(
        error.contains("Promoted property $scroll conflicts"),
        "{error}"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "base:item:1\n"
    );
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
        let error = compile_source(invalid)
            .err()
            .expect("invalid clone magic method must fail compilation");
        assert!(error.contains("__clone() must be"));
    }
}

#[test]
fn readonly_properties_can_be_reinitialized_in_clone_hook() {
    let source = r#"<?php
        class ReadonlyCloneBase {
            public function __construct(public readonly int $value) {}
            public function __CLONE() {
                $this->resetReadonlyValue();
                try { $this->resetReadonlyValue(); } catch (Error $error) { echo 'once:'; }
            }
            private function resetReadonlyValue() { $this->value = 11; }
        }
        class ReadonlyCloneChild extends ReadonlyCloneBase {}
        $original = new ReadonlyCloneChild(10);
        $copy = clone $original;
        echo $original->value, ':', $copy->value;
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "once:10:11"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "found|selected|found|1"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "42\ntext:text\nError:ArrayStringifier::__toString() must return a string\n"
    );

    let strict = r#"<?php
        declare(strict_types=1);
        class StrictScalarStringifier { public function __toString() { return 42; } }
        try { echo new StrictScalarStringifier(); }
        catch (TypeError $error) { echo 'caught'; }
    "#;
    assert_eq!(compile_source(strict).unwrap().execute().unwrap(), "caught");
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "assigned:TypeError:TypeError"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "alpha:zeta:item2:item10:1:same"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:1:1:1:1:label:explicit"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "body\ndestroy:second\ndestroy:first\n"
    );
    for invalid in [
        "<?php class StaticDestructor { public static function __destruct() {} }",
        "<?php class ParameterizedDestructor { public function __destruct($value) {} }",
    ] {
        let error = compile_source(invalid)
            .err()
            .expect("malformed destructor must fail compilation");
        assert!(error.contains("__destruct() must be"));
    }
    for invalid in [
        "<?php class TypedDestructor { public function __destruct(): void {} }",
        "<?php class TypedConstructor { public function __construct(): void {} }",
    ] {
        let error = compile_source(invalid)
            .err()
            .expect("constructor/destructor return type must fail compilation");
        assert!(error.contains("cannot declare a return type"), "{error}");
    }
}

#[test]
fn function_local_destructors_run_when_the_frame_returns_without_destroying_returned_objects() {
    let source = r#"<?php
        class FunctionFrameDestructor {
            private $label;
            public function __construct($label) { $this->label = $label; }
            public function __destruct() { echo 'destroy:', $this->label, "\n"; }
        }
        function make_frame_objects() {
            $local = new FunctionFrameDestructor('local');
            echo "returning\n";
            return new FunctionFrameDestructor('returned');
        }
        $held = make_frame_objects();
        echo "caller\n";
        unset($held);
        echo "released\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "returning\ndestroy:local\ncaller\ndestroy:returned\nreleased\n",
    );
}

#[test]
fn method_and_closure_local_destructors_run_when_their_frames_return() {
    let source = r#"<?php
        class NestedFrameDestructor {
            private $label;
            public function __construct($label) { $this->label = $label; }
            public function __destruct() { echo 'destroy:', $this->label, "\n"; }
            public function runInstance() {
                $local = new NestedFrameDestructor('method');
                echo "method-returning\n";
            }
            public function runParent() {
                $local = new NestedFrameDestructor('parent');
                echo "parent-returning\n";
            }
            public static function runStatic() {
                $local = new NestedFrameDestructor('static');
                echo "static-returning\n";
            }
        }
        class NestedFrameDestructorChild extends NestedFrameDestructor {
            public function runParentChain() { parent::runParent(); }
        }
        $owner = new NestedFrameDestructor('owner');
        $owner->runInstance();
        echo "after-method\n";
        call_user_func_array([$owner, 'runInstance'], []);
        echo "after-method-callback\n";
        $childOwner = new NestedFrameDestructorChild('child-owner');
        $childOwner->runParentChain();
        echo "after-parent\n";
        NestedFrameDestructor::runStatic();
        echo "after-static\n";
        $callback = function() {
            $local = new NestedFrameDestructor('closure');
            echo "closure-returning\n";
        };
        $callback();
        echo "after-closure\n";
        unset($childOwner);
        unset($owner);
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "method-returning\ndestroy:method\nafter-method\nmethod-returning\ndestroy:method\nafter-method-callback\nparent-returning\ndestroy:parent\nafter-parent\nstatic-returning\ndestroy:static\nafter-static\nclosure-returning\ndestroy:closure\nafter-closure\ndestroy:child-owner\ndestroy:owner\n",
    );
}

#[test]
fn constructor_and_magic_property_frames_release_local_objects_on_return() {
    let source = r#"<?php
        class HookFrameNotice {
            private $label;
            public function __construct($label) { $this->label = $label; }
            public function __destruct() { echo 'destroy:', $this->label, "\n"; }
        }
        class ConstructorFrameOwner {
            public function __construct() {
                $temporary = new HookFrameNotice('constructor');
                echo "constructor-returning\n";
            }
            public function __get($name) {
                $temporary = new HookFrameNotice('magic-get');
                echo "get-returning\n";
                return 7;
            }
        }
        $owner = new ConstructorFrameOwner();
        echo "constructed\n";
        echo $owner->missing, "\n";
        echo "after-get\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "constructor-returning\ndestroy:constructor\nconstructed\nget-returning\ndestroy:magic-get\n7\nafter-get\n",
    );
}

#[test]
fn private_and_protected_destructors_run_when_objects_are_destroyed() {
    let source = r#"<?php
        class PrivateDestructor { private function __destruct() { echo 'private:', "\n"; } }
        class ProtectedDestructor { protected function __destruct() { echo 'protected:', "\n"; } }
        $private = new PrivateDestructor();
        $protected = new ProtectedDestructor();
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "protected:\nprivate:\n"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "before\ndestroyed\nafter\n"
    );
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
fn destructor_runs_when_compound_array_assignment_replaces_a_stringable_object() {
    let source = r#"<?php
        class CompoundArrayDestructor {
            public function __toString(): string { return ''; }
            public function __destruct() { echo "drop\n"; }
        }
        $items = [new CompoundArrayDestructor()];
        $items[0] .= 'value';
        echo "assigned\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "drop\nassigned\n"
    );
}

#[test]
fn destructor_runs_when_destructuring_replaces_an_array_slot() {
    let source = r#"<?php
        class DestructureSlotDestructor {
            public function __destruct() { echo "drop\n"; }
        }
        $direct = new DestructureSlotDestructor();
        [$direct] = ['replacement'];
        $items = [new DestructureSlotDestructor()];
        [$items[0]] = ['replacement'];
        $owner = new class { public $items = []; };
        $owner->items = [new DestructureSlotDestructor()];
        [$owner->items[0]] = ['replacement'];
        echo "assigned\n";
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "drop\ndrop\ndrop\nassigned\n"
    );
}

#[test]
fn magic_unset_handles_inaccessible_properties_and_direct_unset_skips_hook() {
    let source = r#"<?php
        class MagicUnsetProbe {
            public $calls = [];
            public $visible = 1;
            private $hidden = 2;
            public function __unset($name) { $this->calls[] = $name; }
            public function hasHidden() { return isset($this->hidden); }
        }
        $value = new MagicUnsetProbe();
        unset($value->missing);
        unset($value->hidden);
        unset($value->visible);
        echo implode(',', $value->calls), '|', (int)$value->hasHidden(), '|', (int)isset($value->visible);
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "missing,hidden|1|0"
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "hello world\n"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "2:triggerNativeFailure:1"
    );
}

#[test]
fn builtin_exception_objects_expose_core_fields_and_throwable_hierarchy() {
    let source = r#"<?php
        $base = new Exception('bad input', 12);
        echo $base->getMessage(), ':', $base->getCode(), ':', $base->getFile(), ':', $base, ':', is_a($base, 'Throwable'), "\n";
        $error = new TypeError('wrong type');
        echo get_class($error), ':', get_parent_class($error), ':', ($error instanceof Error), ':', class_exists('Exception'), ':', interface_exists('Throwable'), ':', method_exists('Throwable', 'getMessage') ? 1 : 0, ':', method_exists('Throwable', '__construct') ? 1 : 0, ':', method_exists('Throwable', '__toString') ? 1 : 0, ':', ($error instanceof Stringable), ':', (new ReflectionClass('Throwable'))->implementsInterface('Stringable') ? 1 : 0, "\n";
        try { throw $error; }
        catch (Exception $wrong) { echo "wrong catch\n"; }
        catch (Throwable $caught) { echo $caught->getMessage(), "\n"; }
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert!(
        output
            .starts_with("bad input:12:Command line code:Exception: bad input in Command line code:2\nStack trace:\n#0 {main}\n:1\n"),
        "{output}"
    );
    assert!(
        output.ends_with("TypeError:Error:1:1:1:1:0:1:1:1\nwrong type\n"),
        "{output}"
    );
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
    assert_eq!(
        output,
        "BadFunctionCallException:BadFunctionCallException:1:1\nRuntimeException:1:1\nmissing\n"
    );
}

#[test]
fn exception_subclasses_can_override_stringification() {
    let output = compile_source(r#"<?php
        class DomainFailureWithString extends RuntimeException {
            public function __toString(): string { return 'domain failure'; }
        }
        class ReflectionFailureChild extends ReflectionException {}
        $error = new DomainFailureWithString('original message');
        echo $error, ':', (string) $error, ':', get_parent_class(new ReflectionFailureChild());
    "#)
    .unwrap()
    .execute()
    .unwrap();
    assert_eq!(output, "domain failure:domain failure:ReflectionException");
}

#[test]
fn throwable_objects_cannot_be_cloned() {
    let output = compile_source(r#"<?php
        class CustomCloneFailure extends Exception {}
        try { clone new Exception('base'); }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(), "\n"; }
        try { clone new CustomCloneFailure('custom'); }
        catch (Error $error) { echo get_class($error), ':', $error->getMessage(); }
    "#)
    .unwrap()
    .execute()
    .unwrap();
    assert_eq!(
        output,
        "Error:Trying to clone an uncloneable object of class Exception\nError:Trying to clone an uncloneable object of class CustomCloneFailure"
    );
}

#[test]
fn error_exception_preserves_severity_and_optional_source_location() {
    let inherited_declaration = compile_source("<?php class ErrorExceptionChild extends ErrorException {} $error = new ErrorExceptionChild('child', 0, E_NOTICE, 'child.php', 11); echo get_class($error), ':', $error->getSeverity(), ':', $error->getFile(), ':', $error->getLine(), ':', method_exists($error, 'getSeverity');")
        .unwrap()
        .execute();
    assert_eq!(inherited_declaration.unwrap(), "ErrorExceptionChild:8:child.php:11:1");
    let source = r#"<?php
        $error = new ErrorException(
            message: 'converted warning',
            code: 7,
            severity: E_WARNING,
            filename: 'warning.php',
            line: 29,
        );
        echo get_class($error), ':', get_parent_class($error), ':', $error instanceof Exception, ':';
        echo $error->getMessage(), ':', $error->getCode(), ':', $error->getSeverity(), ':';
        echo $error->getFile(), ':', $error->getLine(), ':';
        echo class_exists('ErrorException'), ':', method_exists($error, 'getSeverity'), "\n";
        $defaults = new ErrorException(filename: 'provided.php');
        echo $defaults->getSeverity(), ':', $defaults->getFile(), ':', $defaults->getLine();
        $coercedFilename = new ErrorException(filename: 42);
        echo ':', $coercedFilename->getFile(), ':', $coercedFilename->getLine();
        $cause = new Exception('cause');
        $chained = new ErrorException(previous: $cause);
        echo ':', $chained->getPrevious()->getMessage();
        try { new ErrorException(filename: []); }
        catch (TypeError $error) { echo ':', get_class($error); }
        try { new ErrorException('bad severity', 0, 'invalid'); }
        catch (TypeError $error) { echo ':', get_class($error); }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "ErrorException:Exception:1:converted warning:7:2:warning.php:29:1:1\n1:provided.php:0:42:0:cause:TypeError:TypeError"
    );
}

#[test]
fn error_exception_severity_is_reflectable_and_shared_with_getter() {
    let source = r#"<?php
        $error = new ErrorException('notice', 0, E_NOTICE);
        $severity = (new ReflectionClass(ErrorException::class))->getProperty('severity');
        echo $severity->getName(), ':', $severity->getType()->getName(), ':', $severity->isProtected(), ':';
        echo $severity->getValue($error), ':';
        $severity->setValue($error, E_WARNING);
        echo $error->getSeverity();
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "severity:int:1:8:2"
    );
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
    let coercions = compile_source(r#"<?php
        echo (new Exception('numeric', '12.8'))->getCode(), ':';
        try { new Exception('invalid', 'nope'); }
        catch (TypeError $error) { echo get_class($error), ':', $error->getMessage(); }
    "#)
    .unwrap()
    .execute()
    .unwrap();
    assert_eq!(
        coercions,
        "12:TypeError:Exception::__construct(): Argument #2 ($code) must be of type int, string given"
    );
    let invalid_message = compile_source(r#"<?php
        try { new Exception(['invalid']); }
        catch (TypeError $error) { echo get_class($error), ':', $error->getMessage(); }
    "#)
    .unwrap()
    .execute()
    .unwrap();
    assert_eq!(
        invalid_message,
        "TypeError:Exception::__construct(): Argument #1 ($message) must be of type string, array given"
    );
    let stringable_message = compile_source(r#"<?php
        class ExceptionMessageText { public function __toString(): string { return 'coerced'; } }
        class WrappedMessageException extends Exception {
            public function __construct($message) { parent::__construct($message); }
        }
        echo (new Exception(new ExceptionMessageText()))->getMessage(), ':';
        echo (new WrappedMessageException(new ExceptionMessageText()))->getMessage();
    "#)
    .unwrap()
    .execute()
    .unwrap();
    assert_eq!(stringable_message, "coerced:coerced");
}

#[test]
fn native_exception_methods_reject_unknown_named_arguments() {
    for method in ["getMessage", "getTraceAsString"] {
        let source = format!("<?php (new Exception('x'))->{method}(unknown: true);");
        let error = compile_source(&source).unwrap().execute().unwrap_err();
        assert!(
            error.contains("unknown named parameter '$unknown'"),
            "{method}: {error}"
        );
    }
}

#[test]
fn native_exception_constructors_reject_unknown_and_duplicate_named_arguments() {
    let unknown = compile_source("<?php new RuntimeException(detail: 'x');")
        .unwrap()
        .execute()
        .unwrap_err();
    assert!(
        unknown.contains("unknown named parameter '$detail'"),
        "{unknown}"
    );
    let duplicate = compile_source("<?php new Exception(message: 'first', message: 'second');")
        .unwrap()
        .execute()
        .unwrap_err();
    assert!(
        duplicate.contains("Named parameter $message overwrites previous argument"),
        "{duplicate}"
    );
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
    assert_eq!(
        output,
        "DomainFailure:domain:23:1:1:1:1:1\ntype:TypeError\n"
    );
}

#[test]
fn exception_subclasses_cannot_override_final_builtin_accessors() {
    for (source, expected) in [
        (
            "<?php class CustomFailure extends Exception { public function getMessage() {} }",
            "Cannot override final method Exception::getMessage()",
        ),
        (
            "<?php class CustomError extends Error { public function getTrace() {} }",
            "Cannot override final method Error::getTrace()",
        ),
        (
            "<?php class CustomErrorFailure extends ErrorException { public function getSeverity() {} }",
            "Cannot override final method ErrorException::getSeverity()",
        ),
    ] {
        let error = match compile_source(source) {
            Ok(_) => panic!("expected compile failure containing {expected:?}"),
            Err(error) => error,
        };
        assert!(error.contains(expected), "expected {expected:?}, got {error:?}");
    }
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
    assert_eq!(output, "Error:Can only throw objects\nError:Cannot throw objects that do not implement Throwable\n");
}

#[test]
fn invalid_throw_operand_error_retains_throw_site_location() {
    let source = r#"<?php
        try {
            throw 'not throwable';
        } catch (Error $error) {
            echo $error->getFile(), ':', $error->getLine();
        }
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "Command line code:3"
    );
}

#[test]
fn throwable_cannot_be_implemented_directly_but_can_be_extended_by_an_interface() {
    let class_error = compile_source("<?php class InvalidThrowable implements Throwable {}")
        .err()
        .expect("direct Throwable implementation must fail");
    assert!(class_error.contains("cannot implement interface Throwable directly"));
    compile_source("<?php interface InvalidThrowable extends Throwable {}")
        .expect("user interfaces may extend Throwable");
}

#[test]
fn exception_previous_argument_must_implement_throwable() {
    let invalid =
        compile_source("<?php class PlainObject {} new Exception('outer', 0, new PlainObject());")
            .unwrap()
            .execute()
            .unwrap_err();
    assert!(invalid.contains("Argument #3 ($previous) must be of type ?Throwable"));
    let caught = compile_source(r#"<?php
        class PlainPrevious {}
        try { new Exception('outer', 0, new PlainPrevious()); }
        catch (TypeError $error) { echo get_class($error), ':', $error->getMessage(); }
    "#)
    .unwrap()
    .execute()
    .expect("invalid previous argument should throw a catchable TypeError");
    assert!(caught.starts_with("TypeError:"), "{caught}");
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
    assert_eq!(
        output,
        "10:10\n10:20\n30:30\nafter\nchanged\nproperty assignment\n30:40\n"
    );
}

#[test]
fn reference_returns_reject_non_reference_results_and_non_reference_callables() {
    let invalid_return =
        compile_source("<?php function &badReference() { return 42; } badReference();").unwrap();
    assert_eq!(
        invalid_return.execute().unwrap_err(),
        "badreference() must return a reference"
    );
    let invalid_assignment =
        compile_source("<?php function valueOnly() { return 42; } $value =& valueOnly();").unwrap();
    assert_eq!(
        invalid_assignment.execute().unwrap_err(),
        "Only a reference-returning function can be assigned by reference"
    );
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
    assert_eq!(
        output,
        "caught second clause\nouter finally\ncaught without binding\n"
    );
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
        echo is_subclass_of($user, 'UserEntity'), ':', is_subclass_of($user, 'Entity'), ':', is_subclass_of($user, 'Savable'), ':', is_subclass_of('UserEntity', 'Entity'), ':', is_subclass_of('UserEntity', 'Entity', true), ':', is_subclass_of(object_or_class: $user, class: 'Entity'), ':', is_subclass_of(object_or_class: 'UserEntity', class: 'Entity'), ':', is_subclass_of(object_or_class: 'UserEntity', class: 'Entity', allow_string: false), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, ":1:1:1:1:1:1:\n");
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "after:changed"
    );
    let invalid = compile_source("<?php class ValueGetter { public function __get($name) { return 1; } } $object = new ValueGetter(); $alias =& $object->missing;").unwrap();
    assert!(invalid
        .execute()
        .unwrap_err()
        .contains("__get() must return by reference"));
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "4:6:6:get:missing|set:missing:5|get:missing|set:missing:6"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "ReflectedService:ReflectedService:2:ExampleMarker:1::safe:3:safe:3:0:1:1"
    );
    let invalid = compile_source("<?php class NotAnAttribute {} #[NotAnAttribute] class Holder {} (new ReflectionClass('Holder'))->getAttributes()[0]->newInstance();").unwrap();
    assert!(invalid
        .execute()
        .unwrap_err()
        .contains("Attempting to use non-attribute class"));
    for method in ["getTarget", "isRepeated"] {
        let source = format!("<?php #[Attribute] class Marker {{}} #[Marker] class Holder {{}} (new ReflectionClass('Holder'))->getAttributes()[0]->{method}(unexpected: true);");
        let error = compile_source(&source).unwrap().execute().unwrap_err();
        assert!(
            error.contains("unknown named parameter '$unexpected'"),
            "{method}: {error}"
        );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        ":::1:1:1:1:1:1"
    );
}

#[test]
fn reflection_class_reports_instantiability_for_declaration_kinds_and_constructors() {
    let source = r#"<?php
        interface InstantiableContract { public function run(); }
        trait InstantiableTrait {}
        abstract class InstantiableBase { abstract public function run(); }
        class InstantiableChild extends InstantiableBase implements InstantiableContract { public function run() {} }
        class PrivateConstructor { private function __construct() {} }
        class ProtectedConstructor { protected function __construct() {} }
        class PublicConstructor { public function __construct() {} }
        class ImplicitConstructor {}
        enum InstantiableEnum { case Ready; }
        $names = ['InstantiableContract', 'InstantiableTrait', 'InstantiableBase', 'InstantiableChild',
            'PrivateConstructor', 'ProtectedConstructor', 'PublicConstructor', 'ImplicitConstructor'];
        $flags = [];
        foreach ($names as $name) { $flags[] = (new ReflectionClass($name))->isInstantiable() ? '1' : '0'; }
        $enum = new ReflectionEnum('InstantiableEnum');
        echo implode('', $flags), ':', ($enum->isInstantiable() ? '1' : '0'), ':',
            method_exists('ReflectionClass', 'isInstantiable'), ':', method_exists('ReflectionEnum', 'isInstantiable');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "00010011:0:1:1"
    );
}

#[test]
fn reflection_class_reports_cloneability_including_inherited_clone_visibility() {
    let source = r#"<?php
        class CloneableByDefault {}
        class PrivateCloneMethod { private function __clone() {} }
        class ProtectedCloneMethod { protected function __clone() {} }
        class PrivateCloneBase { private function __clone() {} }
        class InheritsPrivateClone extends PrivateCloneBase {}
        class PublicCloneChild extends PrivateCloneBase { public function __clone() {} }
        interface CloneInterface {}
        trait CloneTrait {}
        enum CloneEnum { case Only; }
        $names = ['CloneableByDefault', 'PrivateCloneMethod', 'ProtectedCloneMethod',
            'PrivateCloneBase', 'InheritsPrivateClone', 'PublicCloneChild', 'CloneInterface', 'CloneTrait'];
        $flags = [];
        foreach ($names as $name) { $flags[] = (new ReflectionClass($name))->isCloneable() ? '1' : '0'; }
        echo implode('', $flags), ':', (new ReflectionEnum('CloneEnum'))->isCloneable(), ':';
        echo method_exists('ReflectionClass', 'isCloneable'), ':', method_exists('ReflectionEnum', 'isCloneable');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "10100100::1:1"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "ReflectionParent::1"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:1::1:1"
    );
}

#[test]
fn attribute_marker_rejects_abstract_and_non_class_declarations_at_compile_time() {
    for (source, expected) in [
        (
            "<?php #[Attribute] abstract class AbstractTag {}",
            "Cannot apply #[\\Attribute] to abstract class AbstractTag",
        ),
        (
            "<?php #[Attribute] enum TaggedEnum {}",
            "Cannot apply #[\\Attribute] to enum TaggedEnum",
        ),
        (
            "<?php #[Attribute] interface TaggedInterface {}",
            "Cannot apply #[\\Attribute] to interface TaggedInterface",
        ),
        (
            "<?php #[Attribute] trait TaggedTrait {}",
            "Cannot apply #[\\Attribute] to trait TaggedTrait",
        ),
    ] {
        let error = compile_source(source)
            .err()
            .expect("invalid attribute marker should fail compilation");
        assert!(error.contains(expected), "{error}");
    }
    let valid_trait_attribute = r#"<?php
        #[Attribute(Attribute::TARGET_CLASS)] class TraitTag {}
        #[TraitTag] trait TaggedTrait {}
        $attributes = (new ReflectionClass('TaggedTrait'))->getAttributes();
        echo count($attributes), ':', get_class($attributes[0]->newInstance());
    "#;
    assert_eq!(
        compile_source(valid_trait_attribute)
            .unwrap()
            .execute()
            .unwrap(),
        "1:TraitTag"
    );
    compile_source("<?php #[Attribute] class ConcreteTag {}").unwrap();
    compile_source("<?php #[Vendor\\Attribute] abstract class VendorMarkedType {}").unwrap();
}

#[test]
fn php85_global_constant_attributes_reflect_target_repeatability_and_values() {
    let source = r#"<?php
        #[Attribute(Attribute::TARGET_CONSTANT | Attribute::IS_REPEATABLE)]
        class ConstantTag { public $label; public function __construct($label) { $this->label = $label; } }
        #[Attribute(flags: Attribute::TARGET_CONSTANT)] class NamedConstantTag {}
        #[ConstantTag('primary'), ConstantTag('alias')]
        const APP_NAME = 'rust-php';
        #[NamedConstantTag] const BUILD_VERSION = '1.0';
        $reflection = new ReflectionConstant('APP_NAME');
        $attributes = $reflection->getAttributes();
        echo $reflection->getName(), ':', $reflection->getValue(), ':', $reflection->getNamespace(), ':';
        echo $attributes[0]->getTarget(), ':', $attributes[1]->isRepeated(), ':';
        echo $attributes[0]->newInstance()->label, ':', count($reflection->getAttributes('ConstantTag')), ':';
        echo Attribute::TARGET_CONSTANT, ':', Attribute::TARGET_ALL, ':', Attribute::IS_REPEATABLE, ':';
        echo (new ReflectionConstant('BUILD_VERSION'))->getAttributes()[0]->newInstance() instanceof NamedConstantTag, ':';
        echo method_exists('ReflectionConstant', 'getAttributes'), ':', class_exists('ReflectionConstant');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "APP_NAME:rust-php::64:1:primary:2:64:127:128:1:1:1"
    );

    let invalid = compile_source(
        r#"<?php
        #[Attribute(Attribute::TARGET_CLASS)] class ClassOnlyTag {}
        #[ClassOnlyTag] const INVALID_CONSTANT = 1;
        (new ReflectionConstant('INVALID_CONSTANT'))->getAttributes()[0]->newInstance();
    "#,
    )
    .unwrap();
    assert!(invalid
        .execute()
        .unwrap_err()
        .contains("cannot target this declaration"));
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "2:1:RepeatableTag"
    );

    let invalid_target = compile_source("<?php #[Attribute(Attribute::TARGET_METHOD)] class MethodTag {} #[MethodTag] class InvalidTarget {} (new ReflectionClass('InvalidTarget'))->getAttributes()[0]->newInstance();").unwrap();
    assert!(invalid_target
        .execute()
        .unwrap_err()
        .contains("cannot target this declaration"));

    let invalid_repeat = compile_source("<?php #[Attribute(Attribute::TARGET_CLASS)] class SingleTag {} #[SingleTag, SingleTag] class RepeatedInvalid {} (new ReflectionClass('RepeatedInvalid'))->getAttributes()[0]->newInstance();").unwrap();
    assert!(invalid_repeat
        .execute()
        .unwrap_err()
        .contains("must not be repeated"));
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "listItems:RouteTag:4:/items:/items"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "id:ColumnTag:8:record_id:record_id"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "2:value:0:32:payload:payload:1:1"
    );

    let invalid_target = compile_source("<?php #[Attribute(Attribute::TARGET_METHOD)] class MethodOnly {} class Subject { public function run(#[MethodOnly] $value) {} } (new ReflectionClass('Subject'))->getMethod('run')->getParameters()[0]->getAttributes()[0]->newInstance();").unwrap();
    assert!(invalid_target
        .execute()
        .unwrap_err()
        .contains("cannot target this declaration"));
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "processTask:2:worker:worker:1:1"
    );

    let invalid_target = compile_source("<?php #[Attribute(Attribute::TARGET_METHOD)] class MethodOnly {} #[MethodOnly] function utility() {} (new ReflectionFunction('utility'))->getAttributes()[0]->newInstance();").unwrap();
    assert!(invalid_target
        .execute()
        .unwrap_err()
        .contains("cannot target this declaration"));
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:cacheable"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:ephemeral"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "parent!:child!\n"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "shared\n"
    );
}

#[test]
fn nested_array_references_through_object_property_aliases_remain_connected() {
    let source = r#"<?php
        function replaceNested(&$value) { $value = 'updated'; }
        function updateAlias(&$values) { replaceNested($values['nested']['item']); }
        class PropertyAliasBox {
            public $values = ['nested' => ['item' => 'before']];
            public function update() { updateAlias($this->values); }
        }
        $box = new PropertyAliasBox();
        $box->update();
        echo $box->values['nested']['item'];
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "updated");
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "from-variable:from-property\n"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "parent alias:child alias\n"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "Error\n"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "shared:Error\n"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "9\n13\nTyped property statictypedaliastarget::$value must not be referenced\n"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "untyped alias"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:direct!:named!:a!,b!:xa\n"
    );
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "1:1:1:1:1::1:1:1:1:1:1"
    );
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
        echo $repeatedParameter->getAttributes()[0]->isRepeated(), ':', count($closureReflection->getAttributes()), ':', method_exists('ReflectionFunction', 'getParameters');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "value:closure:value:arrow:1:0:1"
    );
}

#[test]
fn reflection_function_get_static_variables_exposes_closure_captures() {
    let source = r#"<?php
        function ordinary_callable() { static $counter = 3; return ++$counter; }
        function dynamic_static_callable() { static $dynamic = 2 + 3; return $dynamic; }
        $copied = 'copy';
        $linked = 'first';
        $closure = function () use ($copied, &$linked) {};
        $reflection = new ReflectionFunction($closure);
        $variables = $reflection->getStaticVariables();
        $linked = 'updated';
        $variables = $reflection->getStaticVariables();
        echo implode(',', array_keys($variables)), ':', $variables['copied'], ':', $variables['linked'], ':';
        $ordinaryReflection = new ReflectionFunction('ordinary_callable');
        echo $ordinaryReflection->getStaticVariables()['counter'], ':';
        ordinary_callable();
        echo $ordinaryReflection->getStaticVariables()['counter'], ':';
        $dynamicReflection = new ReflectionFunction('dynamic_static_callable');
        echo count($dynamicReflection->getStaticVariables()), ':';
        dynamic_static_callable();
        echo $dynamicReflection->getStaticVariables()['dynamic'], ':';
        $staticClosure = function () { static $closureCounter = 8; return ++$closureCounter; };
        $staticClosureReflection = new ReflectionFunction($staticClosure);
        echo $staticClosureReflection->getStaticVariables()['closureCounter'], ':';
        $staticClosure();
        echo $staticClosureReflection->getStaticVariables()['closureCounter'], ':';
        echo method_exists('ReflectionFunction', 'getStaticVariables');
    "#;
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "copied,linked:copy:updated:3:4:0:5:8:9:1"
    );
    let unknown_named = compile_source("<?php function reflected_empty() {} (new ReflectionFunction('reflected_empty'))->getStaticVariables(unknown: true);")
        .unwrap()
        .execute()
        .unwrap_err();
    assert!(unknown_named.contains("unknown named parameter '$unknown'"), "{unknown_named}");
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
    assert_eq!(
        compile_source(source).unwrap().execute().unwrap(),
        "argument:function"
    );
}

#[test]
fn override_attribute_on_properties_requires_a_parent_property() {
    let source = r#"<?php
        class OverridePropertyBase { public string $label = ''; }
        class OverridePropertyChild extends OverridePropertyBase {
            #[\Override]
            public string $label = 'child';
        }
        echo (new OverridePropertyChild())->label;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "child");

    let invalid = r#"<?php class MissingOverrideProperty { #[\Override] public $label; }"#;
    let error = compile_source(invalid)
        .err()
        .expect("unmatched Override property must fail compilation");
    assert!(error.contains("MissingOverrideProperty::$label has #[\\Override] attribute, but no matching parent property exists"), "{error}");
}
