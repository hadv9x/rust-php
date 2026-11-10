use super::parse;
use crate::{Expr, Stmt};

fn source_statement(statement: &Stmt) -> &Stmt {
    match statement {
        Stmt::SourceLine(_, inner) => source_statement(inner),
        statement => statement,
    }
}

#[test]
fn parser_records_reference_return_declarations() {
    let program = parse(
        r#"<?php
            function &referenceFunction() { return $value; }
            interface ReferenceContract { public function &referenceMethod(); }
            class ReferenceImplementation implements ReferenceContract {
                public function &referenceMethod() { return $value; }
            }
        "#,
    )
    .unwrap();
    assert!(program.functions[0].returns_by_reference);
    assert!(program.interfaces[0].methods[0].returns_by_reference);
    assert!(program.classes[0].methods[0].returns_by_reference);
}

#[test]
fn parser_skips_phpdoc_and_block_comments() {
    let program = parse(
        r#"<?php
            /** @var string $value */
            /* ordinary block comment */
            $value = 'ready';
        "#,
    )
    .unwrap();
    assert_eq!(program.statements.len(), 1);
}

#[test]
fn parser_reports_unterminated_block_comments() {
    let error = parse("<?php /* missing terminator").unwrap_err();
    assert!(error.contains("Unterminated block comment"));
}

#[test]
fn parser_applies_class_readonly_to_declared_properties() {
    let program = parse("<?php final readonly class Value { public string $amount; }").unwrap();
    assert!(program.classes[0].is_final);
    assert!(program.classes[0].properties[0].is_readonly);
}

#[test]
fn parser_retains_class_attributes_and_their_arguments() {
    let program = parse(r#"<?php namespace Example; #[Marker, Flags(mask: 3, enabled: true)] final class Service {}"#).unwrap();
    let attributes = &program.classes[0].attributes;
    assert_eq!(attributes.len(), 2);
    assert_eq!(attributes[0].name, "Example\\Marker");
    assert_eq!(attributes[1].name, "Example\\Flags");
    assert!(
        matches!(&attributes[1].arguments[0], Expr::NamedArgument(name, value) if name == "mask" && matches!(value.as_ref(), Expr::Integer(3)))
    );
    assert!(
        matches!(&attributes[1].arguments[1], Expr::NamedArgument(name, value) if name == "enabled" && matches!(value.as_ref(), Expr::Bool(true)))
    );
}

#[test]
fn parser_accepts_conditional_class_declarations_dynamic_instanceof_and_typed_trait_constants() {
    let program = parse(r#"<?php
            if (interface_exists(\Optional\Runner::class)) {
                class Runner implements \Optional\Runner {}
            } else {
                class Runner implements \Fallback\Runner {}
            }
            trait Limits { private const int MAXIMUM = 5; }
            class Validator { public function accepts($value) { return $value instanceof $this->type; } }
        "#).unwrap();
    let Stmt::If {
        then_body,
        else_body,
        ..
    } = source_statement(&program.statements[0])
    else {
        panic!("expected conditional")
    };
    assert!(matches!(
        source_statement(&then_body[0]),
        Stmt::ClassDeclaration(_)
    ));
    assert!(matches!(
        source_statement(&else_body[0]),
        Stmt::ClassDeclaration(_)
    ));
    assert_eq!(program.traits[0].constants[0].name, "MAXIMUM");
    assert!(matches!(
        source_statement(&program.classes[0].methods[0].body[0]),
        Stmt::Return(Some(Expr::InstanceOf(_, _)))
    ));
}

#[test]
fn lexer_keeps_quotes_inside_nested_string_interpolation() {
    let program =
        parse(r#"<?php $this->line(" {$this->bgRed($this->white(" {$line} "))}");"#).unwrap();
    assert_eq!(program.statements.len(), 1);
}

#[test]
fn parser_accepts_trailing_comma_after_closure_capture() {
    assert!(parse("<?php $callback = function () use (&$index,) { return $index; };").is_ok());
}

#[test]
fn parser_accepts_single_statement_if_else_bodies() {
    assert!(parse("<?php if ($value) $result = true; else $result = false;").is_ok());
}

#[test]
fn parser_accepts_append_destinations_in_destructuring() {
    assert!(parse("<?php [$values[], $keys[]] = [1, 2];").is_ok());
}

#[test]
fn parser_accepts_list_destructuring_in_foreach() {
    let program =
        parse("<?php foreach ($spans as list($spanType, $spanText)) { echo $spanText; }").unwrap();
    let Stmt::Foreach { body, .. } = source_statement(&program.statements[0]) else {
        panic!("expected foreach")
    };
    assert!(matches!(
        source_statement(&body[0]),
        Stmt::ListAssign { .. }
    ));
}

#[test]
fn parser_accepts_variable_static_property_access() {
    assert!(parse("<?php foreach ($docblock::$vectors as $vector) {} ").is_ok());
}

#[test]
fn parser_accepts_conditional_trait_declarations() {
    let program = parse("<?php if ($typed) { trait DumperEnterHashShim { public function enterHash() {} } } else { trait DumperEnterHashShim { public function enterHash() {} } }").unwrap();
    let Stmt::If {
        then_body,
        else_body,
        ..
    } = source_statement(&program.statements[0])
    else {
        panic!("expected conditional")
    };
    assert!(matches!(
        source_statement(&then_body[0]),
        Stmt::TraitDeclaration(_)
    ));
    assert!(matches!(
        source_statement(&else_body[0]),
        Stmt::TraitDeclaration(_)
    ));
}

#[test]
fn parser_accepts_attributes_on_trait_methods() {
    let program = parse("<?php trait Marked { #[Initialize] public function boot() {} }").unwrap();
    assert_eq!(
        program.traits[0].methods[0].attributes[0].name,
        "Initialize"
    );
}

#[test]
fn parser_accepts_attributes_on_interface_methods() {
    let program = parse(
        "<?php interface Marked { #[Deprecated(message: 'use other')] public function boot(); }",
    )
    .unwrap();
    assert_eq!(
        program.interfaces[0].methods[0].attributes[0].name,
        "Deprecated"
    );
}

#[test]
fn parser_accepts_attributes_on_conditional_classes() {
    assert!(parse("<?php if (true) { #[Marker] final class Marked {} }").is_ok());
}

#[test]
fn parser_accepts_attributes_on_constants() {
    assert!(parse("<?php #[Deprecated('use the replacement')] const OLD_VALUE = 1;").is_ok());
}

#[test]
fn parser_accepts_anonymous_class_trait_method_adaptations() {
    assert!(parse("<?php $value = new class { use ExampleTrait { run as public; } };").is_ok());
}

#[test]
fn parser_accepts_dynamic_static_property_array_assignments() {
    assert!(parse("<?php self::${$property}[$key] = 1; self::${$property}[$key] += 2;").is_ok());
}

#[test]
fn parser_accepts_trailing_comma_in_interface_parameters() {
    assert!(
        parse("<?php interface Contract { public function run(int $value,): string; }").is_ok()
    );
}
