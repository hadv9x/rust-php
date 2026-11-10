//! Command-line entry point for compiling and running one PHP source file.

use std::{collections::HashMap, env, process};

use php_compiler::compile_file;
use php_runtime::{ArrayKey, Value};

/// Print command-line usage for invalid or incomplete invocations.
fn usage() {
    println!(
        "php-rust {} — Rust PHP runtime prototype",
        env!("CARGO_PKG_VERSION")
    );
    println!("Usage: php-rust <file.php> [script-arguments...]");
    println!("       php-rust --help");
    println!("       php-rust --version");
}

/// CLI entry point: read one PHP file, compile it, execute it, and forward output/errors.
fn main() {
    let mut args = env::args().skip(1);
    let Some(input) = args.next() else {
        usage();
        process::exit(2);
    };

    match input.as_str() {
        "--help" | "-h" => {
            usage();
            return;
        }
        "--version" | "-V" => {
            println!("php-rust {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        _ => {}
    }

    let script_arguments = args.collect::<Vec<_>>();

    let globals = cli_globals(&input, &script_arguments);
    match compile_file(&input).and_then(|program| program.execute_with_globals(globals)) {
        Ok(result) => {
            print!("{}", result.output);
            for diagnostic in result.diagnostics {
                let level = match diagnostic.level {
                    php_runtime::DiagnosticLevel::Warning => "Warning",
                    php_runtime::DiagnosticLevel::Deprecated => "Deprecated",
                    php_runtime::DiagnosticLevel::Notice
                    | php_runtime::DiagnosticLevel::UserNotice => "Notice",
                    php_runtime::DiagnosticLevel::UserWarning => "Warning",
                    php_runtime::DiagnosticLevel::UserDeprecated => "Deprecated",
                    php_runtime::DiagnosticLevel::Fatal => "Fatal error",
                };
                if let Some(line) = diagnostic.source_line {
                    if let Some(file) = diagnostic.source_file {
                        eprintln!("{level}: {} in {file} on line {line}", diagnostic.message);
                    } else {
                        eprintln!("{level}: {} on line {line}", diagnostic.message);
                    }
                } else {
                    eprintln!("{level}: {}", diagnostic.message);
                }
            }
            if result.exit_code != 0 {
                process::exit(result.exit_code);
            }
        }
        Err(error) => {
            eprintln!("php-rust: {error}");
            process::exit(1);
        }
    }
}

/// Build the PHP CLI argument globals. `$argv[0]` is the script path, not this executable.
fn cli_globals(script: &str, script_arguments: &[String]) -> HashMap<String, Value> {
    let mut argv = Vec::with_capacity(script_arguments.len() + 1);
    argv.push((ArrayKey::Integer(0), Value::String(script.to_string())));
    argv.extend(
        script_arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                (
                    ArrayKey::Integer(index as i64 + 1),
                    Value::String(argument.clone()),
                )
            }),
    );
    HashMap::from([
        ("argc".to_string(), Value::Integer(argv.len() as i64)),
        ("argv".to_string(), Value::from_array_entries(argv)),
    ])
}

#[cfg(test)]
#[path = "../../../tests/crates/php-cli/cli/main_unit.rs"]
mod tests;
