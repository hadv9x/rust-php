//! Bytecode generation for top-level PHP functions.

use std::collections::HashMap;

use php_runtime::{bytecode::Op, BytecodeAttribute, BytecodeFunction, MemberVisibility, Value};
use php_syntax::FunctionDecl;

use super::super::CodeGenerator;

/// Lower top-level user functions into isolated bytecode bodies.
pub(super) fn compile(declarations: &[FunctionDecl], program: &php_syntax::Program) -> Result<HashMap<String, BytecodeFunction>, String> {
    let mut functions = HashMap::new();
    for function in declarations {
        let function_name = function.name.rsplit('\\').next().unwrap_or(&function.name).to_string();
        let strict_types = super::strict_mode_for_body(&function.body, program.strict_types);
        let mut generator = CodeGenerator { program: Some(program), instructions: Vec::new(), current_source_file: None, parent_class: None, current_class: None, current_class_display: None, current_method: None, current_function: Some(function_name), strict_types, known_callables: HashMap::new(), variable_types: super::super::declared_parameter_types(&function.parameters, &function.parameter_types), returns_by_reference: function.returns_by_reference, control_stack: Vec::new(), labels: HashMap::new(), pending_gotos: Vec::new(), next_control_id: 0, include_return_stack: Vec::new(), errors: Vec::new() };
        generator.statements(&function.body);
        generator.resolve_gotos();
        if let Some(error) = generator.errors.first() { return Err(error.clone()); }
        generator.instructions.push(Op::Const(Value::Null));
        generator.instructions.push(Op::Return);
        let mut compiled = BytecodeFunction::new(function.name.to_ascii_lowercase(), function.parameters.clone(), function.parameter_types.clone(), function.parameter_by_reference.clone(), function.parameter_variadic.clone(), super::parameter_defaults(&function.parameter_defaults, Some(program), None, None)?, function.return_type.clone(), function.returns_by_reference, generator.instructions, false, false, false, MemberVisibility::Public, None);
        compiled.parameter_default_constant_names = super::parameter_default_constant_names(&function.parameter_defaults);
        compiled.display_name = function.name.clone();
        compiled.strict_types = strict_types;
        compiled.attributes = function.attributes.iter().map(|attribute| {
            let arguments = attribute.arguments.iter().map(|argument| match argument {
                php_syntax::Expr::NamedArgument(name, value) => Ok((Some(name.clone()), super::constant_parameter_value(value, Some(program), None, None)?)),
                value => Ok((None, super::constant_parameter_value(value, Some(program), None, None)?)),
            }).collect::<Result<Vec<_>, String>>()?;
            Ok(BytecodeAttribute { name: attribute.name.clone(), arguments })
        }).collect::<Result<Vec<_>, String>>()?;
        compiled.parameter_attributes = function.parameter_attributes.iter().map(|attributes| attributes.iter().map(|attribute| {
            let arguments = attribute.arguments.iter().map(|argument| match argument {
                php_syntax::Expr::NamedArgument(name, value) => Ok((Some(name.clone()), super::constant_parameter_value(value, Some(program), None, None)?)),
                value => Ok((None, super::constant_parameter_value(value, Some(program), None, None)?)),
            }).collect::<Result<Vec<_>, String>>()?;
            Ok(BytecodeAttribute { name: attribute.name.clone(), arguments })
        }).collect::<Result<Vec<_>, String>>()).collect::<Result<Vec<_>, String>>()?;
        functions.insert(function.name.to_ascii_lowercase(), compiled);
    }
    Ok(functions)
}
