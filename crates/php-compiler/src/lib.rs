//! Public source-to-bytecode compiler interface.

use php_runtime::BytecodeProgram;
use php_syntax::{BinaryOp, Expr, IncludeKind, Program, Stmt};
use std::{cell::RefCell, collections::{hash_map::DefaultHasher, HashMap, HashSet}, fs, hash::{Hash, Hasher}, io::Read, path::{Path, PathBuf}, time::SystemTime};

mod codegen;
mod composer;

thread_local! {
    static FILE_CACHE: RefCell<HashMap<(PathBuf, Option<PathBuf>, Vec<PathBuf>), CachedProgram>> = RefCell::new(HashMap::new());
}

struct SourceSignature {
    path: PathBuf,
    length: u64,
    modified: Option<SystemTime>,
    fingerprint: u64,
}

struct CachedProgram {
    sources: Vec<SourceSignature>,
    program: BytecodeProgram,
}

impl CachedProgram {
    fn is_current(&self) -> bool {
        self.sources.iter().all(|source| source_signature(&source.path).is_ok_and(|current| {
            current.length == source.length && current.modified == source.modified && current.fingerprint == source.fingerprint
        }))
    }
}

fn source_signature(path: &Path) -> Result<SourceSignature, String> {
    let metadata = fs::metadata(path).map_err(|error| format!("cannot inspect PHP dependency {}: {error}", path.display()))?;
    let mut hasher = DefaultHasher::new();
    if metadata.is_dir() {
        let mut entries = fs::read_dir(path).map_err(|error| format!("cannot scan PHP dependency directory {}: {error}", path.display()))?
            .map(|entry| {
                let entry = entry.map_err(|error| format!("cannot read PHP dependency entry in {}: {error}", path.display()))?;
                let file_type = entry.file_type().map_err(|error| format!("cannot inspect PHP dependency {}: {error}", entry.path().display()))?;
                Ok((entry.file_name(), file_type.is_file(), file_type.is_dir(), file_type.is_symlink()))
            }).collect::<Result<Vec<_>, String>>()?;
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        entries.hash(&mut hasher);
    } else if metadata.is_file() {
        let mut file = fs::File::open(path).map_err(|error| format!("cannot read PHP dependency {}: {error}", path.display()))?;
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let count = file.read(&mut buffer).map_err(|error| format!("cannot fingerprint PHP dependency {}: {error}", path.display()))?;
            if count == 0 { break; }
            hasher.write(&buffer[..count]);
        }
    } else {
        return Err(format!("PHP dependency is neither a file nor a directory: {}", path.display()));
    }
    Ok(SourceSignature { path: path.to_path_buf(), length: metadata.len(), modified: metadata.modified().ok(), fingerprint: hasher.finish() })
}

/// Compile PHP source into the internal bytecode executed by php-runtime.
pub fn compile_source(source: &str) -> Result<BytecodeProgram, String> {
    let syntax_tree = php_syntax::parse(source)?;
    codegen::compile(&syntax_tree)
}

/// Compile a PHP file and inline literal or `__DIR__`-relative include/require statements.
pub fn compile_file(path: impl AsRef<Path>) -> Result<BytecodeProgram, String> {
    compile_file_with_root(path.as_ref(), None)
}

/// Compile a PHP file while restricting included files to an allowed source root.
pub fn compile_file_in_root(path: impl AsRef<Path>, allowed_root: impl AsRef<Path>) -> Result<BytecodeProgram, String> {
    let root = allowed_root.as_ref().canonicalize().map_err(|error| format!("cannot resolve PHP source root {}: {error}", allowed_root.as_ref().display()))?;
    compile_file_with_root(path.as_ref(), Some(root))
}

/// Compile and execute a file through a source-validated per-worker cache, useful for the HTTP server.
pub fn execute_file_with_globals_and_limits(
    path: impl AsRef<Path>,
    allowed_root: impl AsRef<Path>,
    globals: std::collections::HashMap<String, php_runtime::Value>,
    limits: php_runtime::ExecutionLimits,
) -> Result<php_runtime::ExecutionResult, String> {
    execute_file_with_globals_limits_and_diagnostics(path, allowed_root, globals, limits, Vec::new())
}

/// Execute a cached file with diagnostics raised while preparing request globals.
pub fn execute_file_with_globals_limits_and_diagnostics(
    path: impl AsRef<Path>,
    allowed_root: impl AsRef<Path>,
    globals: std::collections::HashMap<String, php_runtime::Value>,
    limits: php_runtime::ExecutionLimits,
    initial_diagnostics: Vec<php_runtime::RuntimeDiagnostic>,
) -> Result<php_runtime::ExecutionResult, String> {
    execute_file_with_globals_limits_diagnostics_and_include_path(path, allowed_root, globals, limits, initial_diagnostics, &[])
}

/// Execute a cached file while resolving statically known includes through PHP's configured include path.
pub fn execute_file_with_globals_limits_diagnostics_and_include_path(
    path: impl AsRef<Path>,
    allowed_root: impl AsRef<Path>,
    globals: std::collections::HashMap<String, php_runtime::Value>,
    limits: php_runtime::ExecutionLimits,
    initial_diagnostics: Vec<php_runtime::RuntimeDiagnostic>,
    include_path: &[PathBuf],
) -> Result<php_runtime::ExecutionResult, String> {
    let entry = path.as_ref().canonicalize().map_err(|error| format!("cannot resolve PHP file {}: {error}", path.as_ref().display()))?;
    let root = allowed_root.as_ref().canonicalize().map_err(|error| format!("cannot resolve PHP source root {}: {error}", allowed_root.as_ref().display()))?;
    if !entry.starts_with(&root) { return Err("PHP entry file escapes the configured source root".into()); }
    FILE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = (entry.clone(), Some(root.clone()), include_path.to_vec());
        if cache.get(&key).is_none_or(|cached| !cached.is_current()) {
            let (program, dependencies) = compile_file_with_root_and_dependencies(&entry, Some(root), include_path)?;
            let sources = dependencies.into_iter().map(|path| source_signature(&path)).collect::<Result<Vec<_>, String>>()?;
            cache.insert(key.clone(), CachedProgram { sources, program });
        }
        if cache.len() > 64 {
            if let Some(oldest) = cache.keys().next().cloned() { cache.remove(&oldest); }
        }
        let mut globals = globals;
        if !include_path.is_empty() {
            globals.insert("\0php_include_path".into(), php_runtime::Value::String(include_path.iter().map(|path| path.to_string_lossy()).collect::<Vec<_>>().join(if cfg!(windows) { ";" } else { ":" })));
        }
        cache.get(&key).expect("compiled file is cached").program.execute_with_globals_limits_and_diagnostics(globals, limits, initial_diagnostics)
    })
}

fn compile_file_with_root(path: &Path, allowed_root: Option<PathBuf>) -> Result<BytecodeProgram, String> {
    compile_file_with_root_and_dependencies(path, allowed_root, &[]).map(|(program, _)| program)
}

fn compile_file_with_root_and_dependencies(path: &Path, allowed_root: Option<PathBuf>, include_path: &[PathBuf]) -> Result<(BytecodeProgram, Vec<PathBuf>), String> {
    let entry = path.canonicalize().map_err(|error| format!("cannot resolve PHP file {}: {error}", path.display()))?;
    if allowed_root.as_ref().is_some_and(|root| !entry.starts_with(root)) { return Err("PHP entry file escapes the configured source root".into()); }
    let mut state = IncludeState { entry: entry.clone(), allowed_root: allowed_root.clone(), include_path: include_path.to_vec(), once: HashSet::from([entry.clone()]), dependencies: HashSet::from([entry.clone()]), active: vec![entry.clone()], composer_loaded: HashSet::new(), composer_files_loaded: HashSet::new(), static_paths: HashMap::new(), strict_types_by_file: HashMap::new(), next_anonymous_class_id: 0 };
    for directory in include_path {
        let mut tracked = directory.as_path();
        while !tracked.is_dir() {
            let Some(parent) = tracked.parent() else { break; };
            tracked = parent;
        }
        if tracked.is_dir() {
            if let Ok(canonical) = tracked.canonicalize() { state.dependencies.insert(canonical); }
        }
    }
    let mut combined = Program { strict_types: false, statements: Vec::new(), functions: Vec::new(), classes: Vec::new(), interfaces: Vec::new(), traits: Vec::new(), constants: Vec::new() };
    combined.statements = append_file(&entry, &mut combined, &mut state)?;
    let mut bytecode = codegen::compile(&combined)?;
    bytecode.set_source_file(entry.to_string_lossy().into_owned());
    let loader_root = allowed_root.clone();
    bytecode.set_include_loader(std::rc::Rc::new(move |requested, including_file, include_path| {
        let separator = if cfg!(windows) { ';' } else { ':' };
        let configured = include_path.split(separator).map(|part| PathBuf::from(if part.is_empty() { "." } else { part })).collect::<Vec<_>>();
        let requested = PathBuf::from(requested);
        let mut candidates = Vec::new();
        if requested.is_absolute() {
            candidates.push(requested.clone());
        } else {
            candidates.extend(configured.iter().map(|directory| directory.join(&requested)));
            if let Some(parent) = Path::new(including_file).parent() { candidates.push(parent.join(&requested)); }
            candidates.push(std::env::current_dir().unwrap_or_else(|_| PathBuf::from("." )).join(&requested));
        }
        let resolved = candidates.into_iter().find(|candidate| candidate.is_file())
            .ok_or_else(|| format!("No such file or directory"))?.canonicalize()
            .map_err(|error| error.to_string())?;
        if loader_root.as_ref().is_some_and(|root| !resolved.starts_with(root)) {
            return Err("included file escapes the configured source root".into());
        }
        let (program, _) = compile_file_with_root_and_dependencies(&resolved, loader_root.clone(), &configured)?;
        Ok(program)
    }));
    Ok((bytecode, state.dependencies.into_iter().collect()))
}

struct IncludeState {
    entry: PathBuf,
    allowed_root: Option<PathBuf>,
    include_path: Vec<PathBuf>,
    once: HashSet<PathBuf>,
    dependencies: HashSet<PathBuf>,
    active: Vec<PathBuf>,
    composer_loaded: HashSet<PathBuf>,
    composer_files_loaded: HashSet<PathBuf>,
    static_paths: HashMap<String, PathBuf>,
    strict_types_by_file: HashMap<PathBuf, bool>,
    next_anonymous_class_id: usize,
}

fn file_strict_mode(path: &Path, state: &IncludeState, combined: &Program) -> bool {
    state.strict_types_by_file.get(path).copied().unwrap_or(combined.strict_types)
}

fn mark_property_hook_modes(properties: &mut [php_syntax::PropertyDecl], path: &Path, strict_types: bool) {
    let file = path.to_string_lossy().into_owned();
    for property in properties {
        if let Some(body) = &mut property.get_hook {
            *body = vec![Stmt::SourceFile(file.clone(), strict_types, std::mem::take(body))];
        }
        if let Some(hook) = &mut property.set_hook {
            hook.body = vec![Stmt::SourceFile(file.clone(), strict_types, std::mem::take(&mut hook.body))];
        }
    }
}

fn append_file(path: &Path, combined: &mut Program, state: &mut IncludeState) -> Result<Vec<Stmt>, String> {
    state.dependencies.insert(path.to_path_buf());
    let source = fs::read_to_string(path).map_err(|error| format!("cannot read included PHP file {}: {error}", path.display()))?;
    let mut program = php_syntax::parse_with_anonymous_class_id(&source, state.next_anonymous_class_id).map_err(|error| format!("{}: {error}", path.display()))?;
    state.next_anonymous_class_id += program.classes.iter().filter(|class| class.anonymous_source.is_some()).count();
    if path == state.entry { combined.strict_types = program.strict_types; }
    let file_strict_types = program.strict_types;
    state.strict_types_by_file.insert(path.to_path_buf(), file_strict_types);
    for constant in &mut program.constants { resolve_magic_constants(&mut constant.value, path); }
    for function in &mut program.functions { resolve_function_magic_constants(function, path); }
    for class in &mut program.classes {
        if let Some((source_file, _)) = &mut class.anonymous_source {
            *source_file = Some(path.to_string_lossy().into_owned());
        }
        for constant in &mut class.constants {
            resolve_magic_constants(&mut constant.value, path);
            for attribute in &mut constant.attributes {
                for argument in &mut attribute.arguments {
                    match argument { Expr::NamedArgument(_, value) => resolve_magic_constants(value, path), value => resolve_magic_constants(value, path) }
                }
            }
        }
        for property in &mut class.properties { if let Some(value) = &mut property.default { resolve_magic_constants(value, path); } }
        for method in &mut class.methods { resolve_function_magic_constants(method, path); }
    }
    for interface in &mut program.interfaces {
        for constant in &mut interface.constants {
            resolve_magic_constants(&mut constant.value, path);
            for attribute in &mut constant.attributes {
                for argument in &mut attribute.arguments {
                    match argument { Expr::NamedArgument(_, value) => resolve_magic_constants(value, path), value => resolve_magic_constants(value, path) }
                }
            }
        }
        for method in &mut interface.methods { resolve_function_magic_constants(method, path); }
    }
    for trait_decl in &mut program.traits {
        for constant in &mut trait_decl.constants {
            resolve_magic_constants(&mut constant.value, path);
            for attribute in &mut constant.attributes {
                for argument in &mut attribute.arguments {
                    match argument { Expr::NamedArgument(_, value) => resolve_magic_constants(value, path), value => resolve_magic_constants(value, path) }
                }
            }
        }
        for property in &mut trait_decl.properties { if let Some(value) = &mut property.default { resolve_magic_constants(value, path); } }
        for method in &mut trait_decl.methods { resolve_function_magic_constants(method, path); }
    }
    let has_return = path != state.entry && has_top_level_return(&program.statements);
    for mut function in program.functions.drain(..) {
        let enclosing_paths = std::mem::replace(&mut state.static_paths, HashMap::new());
        expand_statements(&mut function.body, path, combined, state)?;
        state.static_paths = enclosing_paths;
        function.body = vec![Stmt::SourceFile(path.to_string_lossy().into_owned(), file_strict_types, std::mem::take(&mut function.body))];
        combined.functions.push(function);
    }
    for mut class in program.classes.drain(..) {
        if skip_polyfill_builtin_class(path, &class.name) { continue; }
        for method in &mut class.methods {
            let enclosing_paths = std::mem::replace(&mut state.static_paths, HashMap::new());
            expand_statements(&mut method.body, path, combined, state)?;
            state.static_paths = enclosing_paths;
            method.body = vec![Stmt::SourceFile(path.to_string_lossy().into_owned(), file_strict_types, std::mem::take(&mut method.body))];
        }
        mark_property_hook_modes(&mut class.properties, path, file_strict_types);
        combined.classes.push(class);
    }
    for constant in program.constants.drain(..) {
        let duplicate = combined.constants.iter().find(|existing| existing.name == constant.name);
        if let Some(existing) = duplicate {
            if format!("{:?}", existing.value) == format!("{:?}", constant.value) {
                continue;
            }
        }
        combined.constants.push(constant);
    }
    combined.interfaces.append(&mut program.interfaces);
    for mut trait_decl in program.traits.drain(..) {
        for method in &mut trait_decl.methods {
            let enclosing_paths = std::mem::replace(&mut state.static_paths, HashMap::new());
            expand_statements(&mut method.body, path, combined, state)?;
            state.static_paths = enclosing_paths;
            method.body = vec![Stmt::SourceFile(path.to_string_lossy().into_owned(), file_strict_types, std::mem::take(&mut method.body))];
        }
        mark_property_hook_modes(&mut trait_decl.properties, path, file_strict_types);
        combined.traits.push(trait_decl);
    }
    let mut statements = Vec::new();
    for mut statement in program.statements {
        expand_statement(&mut statement, path, &mut statements, combined, state)?;
    }
    if has_return {
        replace_include_returns(&mut statements);
        return Ok(vec![Stmt::SourceFile(path.to_string_lossy().into_owned(), file_strict_types, vec![Stmt::IncludeScope(statements)])]);
    }
    Ok(vec![Stmt::SourceFile(path.to_string_lossy().into_owned(), file_strict_types, statements)])
}

fn resolve_function_magic_constants(function: &mut php_syntax::FunctionDecl, current_file: &Path) {
    for default in function.parameter_defaults.iter_mut().flatten() { resolve_magic_constants(default, current_file); }
    for property in &mut function.promoted_properties {
        if let Some(default) = &mut property.default { resolve_magic_constants(default, current_file); }
    }
}

fn resolve_magic_constants(expression: &mut Expr, current_file: &Path) {
    match expression {
        Expr::Constant(name) if name.ends_with("__DIR__") => {
            *expression = Expr::String(current_file.parent().unwrap_or(Path::new(".")).to_string_lossy().into_owned());
        }
        Expr::Constant(name) if name.ends_with("__FILE__") => {
            *expression = Expr::String(current_file.to_string_lossy().into_owned());
        }
        Expr::Binary(_, left, right) => { resolve_magic_constants(left, current_file); resolve_magic_constants(right, current_file); }
        Expr::Array(entries) => for entry in entries {
            if let Some(key) = &mut entry.key { resolve_magic_constants(key, current_file); }
            resolve_magic_constants(&mut entry.value, current_file);
        },
        Expr::Negate(value) | Expr::Positive(value) => resolve_magic_constants(value, current_file),
        _ => {},
    }
}

fn replace_include_returns(statements: &mut [Stmt]) {
    for statement in statements {
        match statement {
            Stmt::SourceLine(_, inner) => replace_include_returns(std::slice::from_mut(inner.as_mut())),
            Stmt::SourceFile(_, _, body) => replace_include_returns(body),
            Stmt::Return(None) => *statement = Stmt::IncludeReturn,
            Stmt::Return(Some(value)) => *statement = Stmt::IncludeReturnValue(value.clone()),
            Stmt::If { then_body, else_body, .. } => { replace_include_returns(then_body); replace_include_returns(else_body); }
            Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::Foreach { body, .. } => replace_include_returns(body),
            Stmt::For { initializers, updates, body, .. } => { replace_include_returns(initializers); replace_include_returns(updates); replace_include_returns(body); }
            Stmt::Switch { cases, .. } => for case in cases { replace_include_returns(&mut case.body); },
            Stmt::TryCatch { try_body, catches, finally_body } => {
                replace_include_returns(try_body);
                for catch in catches { replace_include_returns(&mut catch.body); }
                if let Some(body) = finally_body { replace_include_returns(body); }
            }
            Stmt::TryFinally { try_body, finally_body } => { replace_include_returns(try_body); replace_include_returns(finally_body); }
            _ => {}
        }
    }
}

fn expand_statements(statements: &mut Vec<Stmt>, current_file: &Path, combined: &mut Program, state: &mut IncludeState) -> Result<(), String> {
    let mut expanded = Vec::new();
    for mut statement in std::mem::take(statements) {
        expand_statement(&mut statement, current_file, &mut expanded, combined, state)?;
    }
    *statements = expanded;
    Ok(())
}

fn merge_static_paths(paths: &[HashMap<String, PathBuf>]) -> HashMap<String, PathBuf> {
    let Some(first) = paths.first() else { return HashMap::new(); };
    first.iter().filter(|(name, path)| paths.iter().skip(1).all(|candidate| candidate.get(*name) == Some(*path)))
        .map(|(name, path)| (name.clone(), path.clone())).collect()
}

fn expand_statement(statement: &mut Stmt, current_file: &Path, output: &mut Vec<Stmt>, combined: &mut Program, state: &mut IncludeState) -> Result<(), String> {
    match statement {
        Stmt::SourceLine(line, inner) => {
            let line = *line;
            let mut expanded = Vec::new();
            expand_statement(inner, current_file, &mut expanded, combined, state)?;
            if expanded.len() == 1 {
                output.push(Stmt::SourceLine(line, Box::new(expanded.pop().expect("one expanded statement"))));
            } else {
                output.extend(expanded);
            }
        }
        Stmt::FunctionDeclaration(function) => {
            resolve_function_magic_constants(function, current_file);
            let enclosing_paths = std::mem::replace(&mut state.static_paths, HashMap::new());
            expand_statements(&mut function.body, current_file, combined, state)?;
            state.static_paths = enclosing_paths;
            let mut function = function.clone();
            function.body = vec![Stmt::SourceFile(current_file.to_string_lossy().into_owned(), file_strict_mode(current_file, state, combined), std::mem::take(&mut function.body))];
            combined.functions.push(function);
        }
        Stmt::ClassDeclaration(class) => {
            if skip_polyfill_builtin_class(current_file, &class.name) { return Ok(()); }
            let mut class = class.clone();
            for constant in &mut class.constants { resolve_magic_constants(&mut constant.value, current_file); }
            for property in &mut class.properties { if let Some(value) = &mut property.default { resolve_magic_constants(value, current_file); } }
            let strict_types = file_strict_mode(current_file, state, combined);
            for method in &mut class.methods {
                let enclosing_paths = std::mem::replace(&mut state.static_paths, HashMap::new());
                expand_statements(&mut method.body, current_file, combined, state)?;
                state.static_paths = enclosing_paths;
                method.body = vec![Stmt::SourceFile(current_file.to_string_lossy().into_owned(), strict_types, std::mem::take(&mut method.body))];
            }
            mark_property_hook_modes(&mut class.properties, current_file, strict_types);
            combined.classes.push(class);
        }
        Stmt::InterfaceDeclaration(interface) => {
            let mut interface = interface.clone();
            for constant in &mut interface.constants { resolve_magic_constants(&mut constant.value, current_file); }
            for method in &mut interface.methods { resolve_function_magic_constants(method, current_file); }
            combined.interfaces.push(interface);
        }
        Stmt::TraitDeclaration(trait_decl) => {
            let mut trait_decl = trait_decl.clone();
            for constant in &mut trait_decl.constants { resolve_magic_constants(&mut constant.value, current_file); }
            for property in &mut trait_decl.properties { if let Some(value) = &mut property.default { resolve_magic_constants(value, current_file); } }
            let strict_types = file_strict_mode(current_file, state, combined);
            for method in &mut trait_decl.methods {
                resolve_function_magic_constants(method, current_file);
                let enclosing_paths = std::mem::replace(&mut state.static_paths, HashMap::new());
                expand_statements(&mut method.body, current_file, combined, state)?;
                state.static_paths = enclosing_paths;
                method.body = vec![Stmt::SourceFile(current_file.to_string_lossy().into_owned(), strict_types, std::mem::take(&mut method.body))];
            }
            mark_property_hook_modes(&mut trait_decl.properties, current_file, strict_types);
            combined.traits.push(trait_decl);
        }
        Stmt::Expression(expression) => {
            if let Expr::Assignment { variable, operation, value } = expression {
                if operation.is_none() {
                    if let Ok(path) = static_include_path(value, current_file, &state.static_paths) {
                        let path = if path.is_absolute() { path } else { current_file.parent().unwrap_or(Path::new(".")).join(path) };
                        state.static_paths.insert(variable.clone(), path);
                        resolve_magic_constants(value, current_file);
                    } else {
                        state.static_paths.remove(variable);
                    }
                } else {
                    state.static_paths.remove(variable);
                }
            }
            *expression = expand_include_expression(std::mem::replace(expression, Expr::Null), current_file, combined, state)?;
            output.push(statement.clone());
        }
        Stmt::Unset { variable, keys } if keys.is_empty() => {
            state.static_paths.remove(variable);
            output.push(statement.clone());
        }
        Stmt::UnsetMany(statements) => {
            for statement in statements {
                if let Stmt::Unset { variable, keys } = statement {
                    if keys.is_empty() { state.static_paths.remove(variable); }
                }
            }
            output.push(statement.clone());
        }
        Stmt::Return(Some(Expr::Call(name, arguments))) if name.starts_with("\0php_include:") => {
            if arguments.len() != 1 { return Err("internal include expression has an invalid argument count".into()); }
            let kind = match &name["\0php_include:".len()..] {
                "include" => IncludeKind::Include,
                "include_once" => IncludeKind::IncludeOnce,
                "require" => IncludeKind::Require,
                "require_once" => IncludeKind::RequireOnce,
                _ => return Err("unknown include expression kind".into()),
            };
            let requested = match static_include_path(&arguments[0], current_file, &state.static_paths) {
                Ok(path) => path,
                _ => {
                    output.push(Stmt::Return(Some(Expr::Call(name.clone(), arguments.clone()))));
                    return Ok(());
                }
            };
            let requested = resolve_include_path(requested, current_file, &state.include_path);
            let canonical = requested.canonicalize().map_err(|error| format!("{}: cannot resolve included file {}: {error}", current_file.display(), requested.display()))?;
            if state.allowed_root.as_ref().is_some_and(|root| !canonical.starts_with(root)) {
                return Err(format!("{}: included file escapes the configured source root", current_file.display()));
            }
            let once = matches!(kind, IncludeKind::IncludeOnce | IncludeKind::RequireOnce);
            state.dependencies.insert(canonical.clone());
            if !once || state.once.insert(canonical.clone()) {
                if state.active.contains(&canonical) { return Err(format!("recursive include cycle involving {}", canonical.display())); }
                state.active.push(canonical.clone());
                output.extend(append_file(&canonical, combined, state)?);
                state.active.pop();
            }
            output.push(Stmt::IncludeReturn);
        }
        Stmt::Return(Some(value)) => {
            *value = expand_include_expression(std::mem::replace(value, Expr::Null), current_file, combined, state)?;
            output.push(statement.clone());
        }
        Stmt::Include { kind, path } => {
            let requested = match static_include_path(path, current_file, &state.static_paths) {
                Ok(path) => path,
                Err(error) if error.contains("include path must be a string literal") => {
                    output.push(statement.clone());
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            let requested = resolve_include_path(requested, current_file, &state.include_path);
            let canonical = requested.canonicalize().map_err(|error| format!("{}: cannot resolve included file {}: {error}", current_file.display(), requested.display()))?;
            if state.allowed_root.as_ref().is_some_and(|root| !canonical.starts_with(root)) {
                return Err(format!("{}: included file escapes the configured source root", current_file.display()));
            }
            let once = matches!(kind, IncludeKind::IncludeOnce | IncludeKind::RequireOnce);
            state.dependencies.insert(canonical.clone());
            if once && !state.once.insert(canonical.clone()) { return Ok(()); }
            if canonical.file_name().is_some_and(|name| name == "autoload.php")
                && canonical.parent().and_then(Path::file_name).is_some_and(|name| name == "vendor")
            {
                let project_root = canonical.parent().and_then(Path::parent).ok_or("invalid Composer vendor path")?.to_path_buf();
                if state.composer_loaded.insert(project_root.clone()) {
                    output.extend(load_composer_autoload(&project_root, combined, state)?);
                }
                return Ok(());
            }
            if state.active.contains(&canonical) { return Err(format!("recursive include cycle involving {}", canonical.display())); }
            state.active.push(canonical.clone());
            output.extend(append_file(&canonical, combined, state)?);
            state.active.pop();
        }
        Stmt::If { condition, then_body, else_body } => {
            if let Some((variable, path_expr)) = file_exists_assignment(condition) {
                if let Ok(requested) = static_include_path(path_expr, current_file, &state.static_paths) {
                    let requested = if requested.is_absolute() { requested } else { current_file.parent().unwrap_or(Path::new(".")).join(requested) };
                    state.static_paths.insert(variable.clone(), requested.clone());
                    let exists = requested.exists();
                    output.push(Stmt::Expression(Expr::Assignment {
                        variable,
                        operation: None,
                        value: Box::new(Expr::String(requested.to_string_lossy().into_owned())),
                    }));
                    expand_statements(if exists { then_body } else { else_body }, current_file, combined, state)?;
                    output.extend(if exists { then_body.clone() } else { else_body.clone() });
                    return Ok(());
                }
            }
            let before = state.static_paths.clone();
            expand_statements(then_body, current_file, combined, state)?;
            let then_paths = state.static_paths.clone();
            state.static_paths = before.clone();
            expand_statements(else_body, current_file, combined, state)?;
            let else_paths = state.static_paths.clone();
            state.static_paths = merge_static_paths(&[then_paths, else_paths]);
            output.push(statement.clone());
        }
        Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::Foreach { body, .. } => {
            let before = state.static_paths.clone();
            expand_statements(body, current_file, combined, state)?;
            state.static_paths = before;
            output.push(statement.clone());
        }
        Stmt::For { initializers, updates, body, .. } => {
            expand_statements(initializers, current_file, combined, state)?;
            let after_initializers = state.static_paths.clone();
            expand_statements(body, current_file, combined, state)?;
            state.static_paths = after_initializers.clone();
            expand_statements(updates, current_file, combined, state)?;
            state.static_paths = after_initializers;
            output.push(statement.clone());
        }
        Stmt::Switch { cases, .. } => {
            for case in cases { expand_statements(&mut case.body, current_file, combined, state)?; }
            output.push(statement.clone());
        }
        Stmt::TryCatch { try_body, catches, finally_body } => {
            expand_statements(try_body, current_file, combined, state)?;
            for catch in catches { expand_statements(&mut catch.body, current_file, combined, state)?; }
            if let Some(body) = finally_body { expand_statements(body, current_file, combined, state)?; }
            output.push(statement.clone());
        }
        Stmt::TryFinally { try_body, finally_body } => {
            expand_statements(try_body, current_file, combined, state)?;
            expand_statements(finally_body, current_file, combined, state)?;
            output.push(statement.clone());
        }
        _ => output.push(statement.clone()),
    }
    Ok(())
}

fn expand_include_expression(expression: Expr, current_file: &Path, combined: &mut Program, state: &mut IncludeState) -> Result<Expr, String> {
    match expression {
        Expr::Call(name, mut arguments) if name.starts_with("\0php_include:") => {
            if arguments.len() != 1 { return Err("internal include expression has an invalid argument count".into()); }
            let kind = match &name["\0php_include:".len()..] {
                "include" => IncludeKind::Include,
                "include_once" => IncludeKind::IncludeOnce,
                "require" => IncludeKind::Require,
                "require_once" => IncludeKind::RequireOnce,
                _ => return Err("unknown include expression kind".into()),
            };
            let path_expression = arguments.remove(0);
            let requested = match static_include_path(&path_expression, current_file, &state.static_paths) {
                Ok(path) => path,
                _ => return Ok(Expr::Call(name, vec![path_expression])),
            };
            let requested = resolve_include_path(requested, current_file, &state.include_path);
            let canonical = requested.canonicalize().map_err(|error| format!("{}: cannot resolve included file {}: {error}", current_file.display(), requested.display()))?;
            if state.allowed_root.as_ref().is_some_and(|root| !canonical.starts_with(root)) {
                return Err(format!("{}: included file escapes the configured source root", current_file.display()));
            }
            let once = matches!(kind, IncludeKind::IncludeOnce | IncludeKind::RequireOnce);
            state.dependencies.insert(canonical.clone());
            if once && !state.once.insert(canonical.clone()) { return Ok(Expr::Bool(true)); }
            if state.active.contains(&canonical) { return Err(format!("recursive include cycle involving {}", canonical.display())); }
            let source = fs::read_to_string(&canonical).map_err(|error| format!("cannot read included PHP file {}: {error}", canonical.display()))?;
            let mut included = php_syntax::parse(&source).map_err(|error| format!("{}: {error}", canonical.display()))?;
            included.statements.retain(|statement| match statement {
                Stmt::SourceLine(_, inner) => !matches!(inner.as_ref(), Stmt::Expression(Expr::Null)),
            Stmt::SourceFile(_, _, body) => body.iter().any(|statement| !matches!(statement, Stmt::Expression(Expr::Null))),
                statement => !matches!(statement, Stmt::Expression(Expr::Null)),
            });
            if !included.functions.is_empty() || !included.classes.is_empty() || !included.interfaces.is_empty()
                || !included.traits.is_empty() || !included.constants.is_empty()
            {
                return Err(format!("{}: declarations in a value-returning include expression are not supported", canonical.display()));
            }
            if included.statements.len() != 1 {
                return Err(format!("{}: include expressions currently require one top-level return statement", canonical.display()));
            }
            let mut return_statement = included.statements.remove(0);
            while let Stmt::SourceLine(_, inner) = return_statement { return_statement = *inner; }
            let Stmt::Return(Some(value)) = return_statement else {
                return Err(format!("{}: include expression must return a value", canonical.display()));
            };
            state.active.push(canonical.clone());
            let value = expand_include_expression(value, &canonical, combined, state)?;
            state.active.pop();
            Ok(value)
        }
        Expr::Assignment { variable, operation, value } => Ok(Expr::Assignment {
            variable, operation, value: Box::new(expand_include_expression(*value, current_file, combined, state)?),
        }),
        Expr::Closure { mut declaration, captures } => {
            expand_statements(&mut declaration.body, current_file, combined, state)?;
            Ok(Expr::Closure { declaration, captures })
        }
        Expr::Binary(operator, left, right) => Ok(Expr::Binary(
            operator,
            Box::new(expand_include_expression(*left, current_file, combined, state)?),
            Box::new(expand_include_expression(*right, current_file, combined, state)?),
        )),
        Expr::MethodCall(receiver, method, arguments) => Ok(Expr::MethodCall(
            Box::new(expand_include_expression(*receiver, current_file, combined, state)?),
            method,
            arguments.into_iter().map(|argument| expand_include_expression(argument, current_file, combined, state)).collect::<Result<_, _>>()?,
        )),
        Expr::Property(receiver, property) => Ok(Expr::Property(Box::new(expand_include_expression(*receiver, current_file, combined, state)?), property)),
        Expr::Call(name, arguments) => Ok(Expr::Call(name, arguments.into_iter().map(|argument| expand_include_expression(argument, current_file, combined, state)).collect::<Result<_, _>>()?)),
        Expr::NewObject(class_name, arguments, line) => Ok(Expr::NewObject(class_name, arguments.into_iter().map(|argument| expand_include_expression(argument, current_file, combined, state)).collect::<Result<_, _>>()?, line)),
        Expr::NamedArgument(name, value) => Ok(Expr::NamedArgument(name, Box::new(expand_include_expression(*value, current_file, combined, state)?))),
        Expr::Ternary { condition, then_value, else_value } => Ok(Expr::Ternary {
            condition: Box::new(expand_include_expression(*condition, current_file, combined, state)?),
            then_value: Box::new(expand_include_expression(*then_value, current_file, combined, state)?),
            else_value: Box::new(expand_include_expression(*else_value, current_file, combined, state)?),
        }),
        Expr::Coalesce(left, right) => Ok(Expr::Coalesce(
            Box::new(expand_include_expression(*left, current_file, combined, state)?),
            Box::new(expand_include_expression(*right, current_file, combined, state)?),
        )),
        expression => Ok(expression),
    }
}

fn order_composer_manifests(manifests: Vec<PathBuf>, root_manifest: &Path) -> Vec<PathBuf> {
    let mut package_data = HashMap::<PathBuf, (Option<String>, Vec<String>)>::new();
    let mut package_paths = HashMap::<String, PathBuf>::new();
    for path in manifests.iter().filter(|path| path.as_path() != root_manifest) {
        let Ok(manifest) = composer::read_manifest(path) else { continue; };
        if let Some(name) = &manifest.name {
            package_paths.entry(name.to_ascii_lowercase()).or_insert_with(|| path.clone());
        }
        package_data.insert(path.clone(), (manifest.name, manifest.requires));
    }

    fn visit_manifest(
        path: &Path,
        package_data: &HashMap<PathBuf, (Option<String>, Vec<String>)>,
        package_paths: &HashMap<String, PathBuf>,
        visiting: &mut HashSet<PathBuf>,
        visited: &mut HashSet<PathBuf>,
        ordered: &mut Vec<PathBuf>,
    ) {
        if visited.contains(path) || !visiting.insert(path.to_path_buf()) { return; }
        if let Some((_, dependencies)) = package_data.get(path) {
            for dependency in dependencies {
                if let Some(dependency_path) = package_paths.get(&dependency.to_ascii_lowercase()) {
                    visit_manifest(dependency_path, package_data, package_paths, visiting, visited, ordered);
                }
            }
        }
        visiting.remove(path);
        if visited.insert(path.to_path_buf()) { ordered.push(path.to_path_buf()); }
    }

    let mut ordered = Vec::with_capacity(manifests.len());
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    for path in manifests.iter().filter(|path| path.as_path() != root_manifest) {
        visit_manifest(path, &package_data, &package_paths, &mut visiting, &mut visited, &mut ordered);
    }
    for path in manifests.iter().filter(|path| path.as_path() != root_manifest) {
        if !package_data.contains_key(path) && visited.insert(path.clone()) { ordered.push(path.clone()); }
    }
    if manifests.iter().any(|path| path.as_path() == root_manifest) { ordered.push(root_manifest.to_path_buf()); }
    ordered
}

fn load_composer_autoload(project_root: &Path, combined: &mut Program, state: &mut IncludeState) -> Result<Vec<Stmt>, String> {
    let manifest_path = project_root.join("composer.json");
    let mut manifests = vec![manifest_path.clone()];
    let vendor = project_root.join("vendor");
    let mut manifest_directories = HashSet::from([project_root.to_path_buf()]);
    if vendor.is_dir() { collect_composer_manifests(&vendor, &mut manifests, &mut manifest_directories, 0)?; }
    manifests.sort();
    manifests.dedup();
    manifests.retain(|path| path != &manifest_path);
    manifests.push(manifest_path.clone()); // Root autoload.files run after dependency files.
    let manifests = order_composer_manifests(manifests, &manifest_path);
    let mut class_files = Vec::new();
    let mut class_directories = HashSet::new();
    let mut autoload_files = Vec::new();
    for current_manifest in &manifests {
        state.dependencies.insert(current_manifest.clone());
        let is_root_manifest = current_manifest == &manifest_path;
        let manifest = match composer::read_manifest(current_manifest) {
            Ok(manifest) => manifest,
            Err(_) if !is_root_manifest => continue,
            Err(error) => return Err(error),
        };
        for (_, directories) in manifest.psr4.into_iter().chain(manifest.psr0.into_iter()) {
            for directory in directories {
                collect_composer_path(&directory, &mut class_files, &mut class_directories)?;
            }
        }
        for path in manifest.classmap { collect_composer_path(&path, &mut class_files, &mut class_directories)?; }
        autoload_files.extend(manifest.files);
    }
    state.dependencies.extend(manifest_directories);
    state.dependencies.extend(class_directories);
    class_files.sort();
    class_files.dedup();
    if class_files.len() > 10_000 { return Err("Composer class scan exceeds 10000 PHP files".into()); }
    for file in class_files {
        let file = file.canonicalize().map_err(|error| format!("cannot resolve Composer class file {}: {error}", file.display()))?;
        if state.allowed_root.as_ref().is_some_and(|root| !file.starts_with(root)) {
            return Err(format!("Composer class file escapes the configured source root: {}", file.display()));
        }
        append_composer_declarations(&file, combined, state)?;
    }

    let mut statements = Vec::new();
    for file in autoload_files {
        let file = file.canonicalize().map_err(|error| format!("cannot resolve Composer autoload file {}: {error}", file.display()))?;
        if state.allowed_root.as_ref().is_some_and(|root| !file.starts_with(root)) {
            return Err(format!("Composer autoload file escapes the configured source root: {}", file.display()));
        }
        if state.composer_files_loaded.insert(file.clone()) {
            if state.active.contains(&file) { return Err(format!("recursive Composer autoload file {}", file.display())); }
            state.active.push(file.clone());
            statements.extend(append_file(&file, combined, state)?);
            state.active.pop();
        }
    }
    Ok(statements)
}

fn collect_composer_manifests(directory: &Path, manifests: &mut Vec<PathBuf>, tracked_directories: &mut HashSet<PathBuf>, depth: usize) -> Result<(), String> {
    if depth > 24 { return Err(format!("Composer vendor directory nesting too deep under {}", directory.display())); }
    tracked_directories.insert(directory.to_path_buf());
    for entry in fs::read_dir(directory).map_err(|error| format!("cannot scan Composer vendor directory {}: {error}", directory.display()))? {
        let entry = entry.map_err(|error| format!("cannot read Composer vendor entry: {error}"))?;
        let kind = entry.file_type().map_err(|error| format!("cannot inspect {}: {error}", entry.path().display()))?;
        if kind.is_dir() { collect_composer_manifests(&entry.path(), manifests, tracked_directories, depth + 1)?; }
        else if kind.is_file() && entry.file_name() == "composer.json" { manifests.push(entry.path()); }
        if manifests.len() > 10_000 { return Err("Composer vendor scan exceeds 10000 manifests".into()); }
    }
    Ok(())
}

fn append_composer_declarations(path: &Path, combined: &mut Program, state: &mut IncludeState) -> Result<(), String> {
    state.dependencies.insert(path.to_path_buf());
    let source = fs::read_to_string(path).map_err(|error| format!("cannot read Composer class file {}: {error}", path.display()))?;
    let mut program = php_syntax::parse_with_anonymous_class_id(&source, state.next_anonymous_class_id).map_err(|error| format!("{}: {error}", path.display()))?;
    state.next_anonymous_class_id += program.classes.iter().filter(|class| class.anonymous_source.is_some()).count();
    for constant in &mut program.constants { resolve_magic_constants(&mut constant.value, path); }
    for function in &mut program.functions { resolve_function_magic_constants(function, path); }
    for class in &mut program.classes {
        if let Some((source_file, _)) = &mut class.anonymous_source {
            *source_file = Some(path.to_string_lossy().into_owned());
        }
        for constant in &mut class.constants { resolve_magic_constants(&mut constant.value, path); }
        for property in &mut class.properties { if let Some(value) = &mut property.default { resolve_magic_constants(value, path); } }
        for method in &mut class.methods { resolve_function_magic_constants(method, path); }
    }
    for interface in &mut program.interfaces {
        for constant in &mut interface.constants { resolve_magic_constants(&mut constant.value, path); }
        for method in &mut interface.methods { resolve_function_magic_constants(method, path); }
    }
    for trait_decl in &mut program.traits {
        for constant in &mut trait_decl.constants { resolve_magic_constants(&mut constant.value, path); }
        for property in &mut trait_decl.properties { if let Some(value) = &mut property.default { resolve_magic_constants(value, path); } }
        for method in &mut trait_decl.methods { resolve_function_magic_constants(method, path); }
    }
    combined.constants.append(&mut program.constants);
    for mut function in program.functions.drain(..) {
        expand_statements(&mut function.body, path, combined, state)?;
        combined.functions.push(function);
    }
    for mut class in program.classes.drain(..) {
        if skip_polyfill_builtin_class(path, &class.name) { continue; }
        for method in &mut class.methods { expand_statements(&mut method.body, path, combined, state)?; }
        combined.classes.push(class);
    }
    combined.interfaces.append(&mut program.interfaces);
    for mut trait_decl in program.traits.drain(..) {
        for method in &mut trait_decl.methods { expand_statements(&mut method.body, path, combined, state)?; }
        combined.traits.push(trait_decl);
    }
    Ok(())
}

fn skip_polyfill_builtin_class(path: &Path, name: &str) -> bool {
    let path = path.to_string_lossy().replace('\\', "/");
    let native_builtin = matches!(name.to_ascii_lowercase().as_str(),
        "error" | "exception" | "typeerror" | "valueerror" | "argumentcounterror" | "compileerror"
        | "parseerror" | "arithmeticerror" | "divisionbyzeroerror" | "assertionerror" | "unhandledmatcherror"
        | "jsonexception" | "roundingmode" | "sortdirection");
    native_builtin && path.contains("/polyfill-")
}

fn collect_php_files(directory: &Path, files: &mut Vec<PathBuf>, directories: &mut HashSet<PathBuf>, depth: usize) -> Result<(), String> {
    if depth > 64 { return Err(format!("Composer PSR-4 directory nesting too deep under {}", directory.display())); }
    directories.insert(directory.to_path_buf());
    for entry in fs::read_dir(directory).map_err(|error| format!("cannot scan Composer directory {}: {error}", directory.display()))? {
        let entry = entry.map_err(|error| format!("cannot read Composer directory entry: {error}"))?;
        let kind = entry.file_type().map_err(|error| format!("cannot inspect {}: {error}", entry.path().display()))?;
        if kind.is_dir() { collect_php_files(&entry.path(), files, directories, depth + 1)?; }
        else if kind.is_file() && entry.path().extension().is_some_and(|extension| extension.eq_ignore_ascii_case("php")) { files.push(entry.path()); }
        if files.len() > 10_000 { return Err("Composer PSR-4 scan exceeds 10000 PHP files".into()); }
    }
    Ok(())
}

fn collect_composer_path(path: &Path, files: &mut Vec<PathBuf>, directories: &mut HashSet<PathBuf>) -> Result<(), String> {
    if path.is_dir() { return collect_php_files(path, files, directories, 0); }
    if path.is_file() && path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("php")) {
        files.push(path.to_path_buf());
        if files.len() > 10_000 { return Err("Composer class scan exceeds 10000 PHP files".into()); }
    }
    Ok(())
}

fn has_top_level_return(statements: &[Stmt]) -> bool {
    statements.iter().any(|statement| match statement {
        Stmt::SourceLine(_, inner) => has_top_level_return(std::slice::from_ref(inner.as_ref())),
        Stmt::SourceFile(_, _, body) => has_top_level_return(body),
        Stmt::Return(_) => true,
        Stmt::If { then_body, else_body, .. } => has_top_level_return(then_body) || has_top_level_return(else_body),
        Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::Foreach { body, .. } => has_top_level_return(body),
        Stmt::For { initializers, updates, body, .. } => has_top_level_return(initializers) || has_top_level_return(updates) || has_top_level_return(body),
        Stmt::Switch { cases, .. } => cases.iter().any(|case| has_top_level_return(&case.body)),
        Stmt::TryCatch { try_body, catches, finally_body } => has_top_level_return(try_body) || catches.iter().any(|catch| has_top_level_return(&catch.body)) || finally_body.as_ref().is_some_and(|body| has_top_level_return(body)),
        Stmt::TryFinally { try_body, finally_body } => has_top_level_return(try_body) || has_top_level_return(finally_body),
        _ => false,
    })
}

fn file_exists_assignment(expression: &Expr) -> Option<(String, &Expr)> {
    let Expr::Call(name, arguments) = expression else { return None; };
    if !name.rsplit('\\').next()?.eq_ignore_ascii_case("file_exists") || arguments.len() != 1 { return None; }
    let Expr::Assignment { variable, operation: None, value } = &arguments[0] else { return None; };
    Some((variable.clone(), value))
}

fn static_include_path(expression: &Expr, current_file: &Path, variables: &HashMap<String, PathBuf>) -> Result<PathBuf, String> {
    fn evaluate(expression: &Expr, current_file: &Path, variables: &HashMap<String, PathBuf>) -> Option<String> {
        match expression {
            Expr::String(value) => Some(value.clone()),
            Expr::Variable(name) => variables.get(name).map(|path| path.to_string_lossy().into_owned()),
            Expr::Constant(name) if name.ends_with("__DIR__") => Some(current_file.parent()?.to_string_lossy().into_owned()),
            Expr::Constant(name) if name.ends_with("__FILE__") => Some(current_file.to_string_lossy().into_owned()),
            Expr::Binary(BinaryOp::Concatenate, left, right) => Some(format!("{}{}", evaluate(left, current_file, variables)?, evaluate(right, current_file, variables)?)),
            _ => None,
        }
    }
    evaluate(expression, current_file, variables).map(PathBuf::from).ok_or_else(|| format!("{}: include path must be a string literal, a __DIR__/__FILE__ concatenation, or a statically assigned file_exists path", current_file.display()))
}

fn resolve_include_path(requested: PathBuf, current_file: &Path, include_path: &[PathBuf]) -> PathBuf {
    if requested.is_absolute() { return requested; }
    for directory in include_path {
        let candidate = directory.join(&requested);
        if candidate.is_file() { return candidate; }
    }
    let relative_to_including_file = current_file.parent().unwrap_or(Path::new(".")).join(&requested);
    relative_to_including_file
}
