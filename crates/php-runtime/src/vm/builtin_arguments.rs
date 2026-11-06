//! Bind named and positional arguments for the builtins implemented by the VM.

use crate::{bytecode::ReferenceArgument, ArrayKey, Value};

#[derive(Clone, Copy)]
enum DefaultValue { Null, False, True, EmptyString, EmptyArray, StringDefault, MongoUri, Space, Dot, Comma, Quote, Backslash, Newline, Utf8, WordSeparators, Number(f64) }

impl DefaultValue {
    fn value(self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::False => Value::Bool(false),
            Self::True => Value::Bool(true),
            Self::EmptyString => Value::String(String::new()),
            Self::EmptyArray => Value::array(Vec::new()),
            Self::StringDefault => Value::String("stdClass".into()),
            Self::MongoUri => Value::String("mongodb://127.0.0.1/".into()),
            Self::Space => Value::String(" ".into()),
            Self::Dot => Value::String(".".into()),
            Self::Comma => Value::String(",".into()),
            Self::Quote => Value::String("\"".into()),
            Self::Backslash => Value::String("\\".into()),
            Self::Newline => Value::String("\n".into()),
            Self::Utf8 => Value::String("UTF-8".into()),
            Self::WordSeparators => Value::String(" \t\r\n\x0c\x0b".into()),
            Self::Number(value) => Value::Number(value),
        }
    }
}

struct Signature {
    names: &'static [&'static str],
    defaults: &'static [(usize, DefaultValue)],
    variadic: bool,
}

fn signature(name: &str) -> Option<Signature> {
    use DefaultValue::{False as F, Null as N, Number as Num};
    let (names, defaults, variadic): (&'static [&'static str], &'static [(usize, DefaultValue)], bool) = match name.to_ascii_lowercase().as_str() {
        "exit" | "die" => (&["status"], &[], false),
        "mongo_find" => (&["connection", "database", "collection", "filter", "limit"], &[(3, N), (4, Num(0.0))], false),
        "mongo_aggregate" => (&["connection", "database", "collection", "pipeline"], &[], false),
        "mongo_count_documents" => (&["connection", "database", "collection", "filter"], &[(3, N)], false),
        "mongo_estimated_document_count" => (&["connection", "database", "collection"], &[], false),
        "mongo_begin_transaction" | "mongo_commit" | "mongo_rollback" => (&["connection"], &[], false),
        "mongo_insert_one" => (&["connection", "database", "collection", "document"], &[], false),
        "mongo_insert_many" => (&["connection", "database", "collection", "documents"], &[], false),
        "mongo_replace_one" => (&["connection", "database", "collection", "filter", "replacement"], &[], false),
        "mongo_update_one" | "mongo_update_many" => (&["connection", "database", "collection", "filter", "update"], &[], false),
        "mongo_delete_one" | "mongo_delete_many" => (&["connection", "database", "collection", "filter"], &[(3, N)], false),
        "mongodb\\client::__construct" => (&["uri", "uriOptions", "driverOptions"], &[(0, DefaultValue::MongoUri), (1, DefaultValue::EmptyArray), (2, DefaultValue::EmptyArray)], false),
        "mongodb\\client::selectdatabase" => (&["databaseName", "options"], &[(1, DefaultValue::EmptyArray)], false),
        "mongodb\\client::selectcollection" => (&["databaseName", "collectionName", "options"], &[(2, DefaultValue::EmptyArray)], false),
        "mongodb\\database::selectcollection" => (&["collectionName", "options"], &[(1, DefaultValue::EmptyArray)], false),
        "mongodb\\collection::find" => (&["filter", "options"], &[(0, DefaultValue::EmptyArray), (1, DefaultValue::EmptyArray)], false),
        "mongodb\\collection::findone" => (&["filter", "options"], &[(0, DefaultValue::EmptyArray), (1, DefaultValue::EmptyArray)], false),
        "mongodb\\collection::aggregate" => (&["pipeline", "options"], &[(1, DefaultValue::EmptyArray)], false),
        "mongodb\\collection::countdocuments" => (&["filter", "options"], &[(0, DefaultValue::EmptyArray), (1, DefaultValue::EmptyArray)], false),
        "mongodb\\collection::estimateddocumentcount" => (&["options"], &[(0, DefaultValue::EmptyArray)], false),
        "mongodb\\collection::insertone" => (&["document", "options"], &[(1, DefaultValue::EmptyArray)], false),
        "mongodb\\collection::insertmany" => (&["documents", "options"], &[(1, DefaultValue::EmptyArray)], false),
        "mongodb\\collection::replaceone" => (&["filter", "replacement", "options"], &[(2, DefaultValue::EmptyArray)], false),
        "mongodb\\collection::updateone" | "mongodb\\collection::updatemany" => (&["filter", "update", "options"], &[(2, DefaultValue::EmptyArray)], false),
        "mongodb\\collection::deleteone" | "mongodb\\collection::deletemany" => (&["filter", "options"], &[(1, DefaultValue::EmptyArray)], false),
        "mongodb\\cursor::toarray" | "mongodb\\insertresult::getinsertedcount" | "mongodb\\insertmanyresult::getinsertedcount"
        | "mongodb\\updateresult::getmatchedcount" | "mongodb\\updateresult::getmodifiedcount" | "mongodb\\deleteresult::getdeletedcount" => (&[], &[], false),
        "call_user_func_array" => (&["callback", "args"], &[], false),
        "func_get_args" | "func_num_args" | "get_defined_vars" | "error_get_last" | "error_clear_last" | "gc_collect_cycles" => (&[], &[], false),
        "func_get_arg" => (&["position"], &[], false),
        "reflectionclass::__construct" => (&["objectOrClass"], &[], false),
        "reflectionobject::__construct" => (&["object"], &[], false),
        "reflectionenum::__construct" => (&["objectOrClass"], &[], false),
        "reflectionenum::getattributes" => (&["name", "flags"], &[(0, N), (1, Num(0.0))], false),
        "reflectionenum::getmethod" | "reflectionenum::getproperty" => (&["name"], &[], false),
        "reflectionfunction::__construct" => (&["function"], &[], false),
        "reflectionparameter::__construct" => (&["function", "param"], &[], false),
        "reflectionfunction::invoke" => (&["args"], &[], true),
        "reflectionfunction::invokeargs" => (&["args"], &[], false),
        "reflectionfunction::getclosure" => (&[], &[], false),
        "reflectionmethod::__construct" => (&["objectOrMethod", "method"], &[(1, N)], false),
        "reflectionmethod::createfrommethodname" => (&["method"], &[], false),
        "reflectionmethod::getclosure" => (&["object"], &[(0, N)], false),
        "reflectionmethod::isvariadic" | "reflectionmethod::returnsreference" | "reflectionmethod::isclosure" | "reflectionfunction::isvariadic" | "reflectionfunction::returnsreference" | "reflectionfunction::isclosure" | "reflectionfunction::isstatic" => (&[], &[], false),
        "reflectionclass::getattributes" => (&["name", "flags"], &[(0, N), (1, Num(0.0))], false),
        "reflectionclass::getmethod" => (&["name"], &[], false),
        "reflectionclass::getmethods" => (&["filter"], &[(0, N)], false),
        "reflectionclass::getconstructor" | "reflectionenum::getconstructor" | "reflectionclass::getdefaultproperties" | "reflectionenum::getdefaultproperties" | "reflectionclass::getstaticproperties" | "reflectionenum::getstaticproperties" => (&[], &[], false),
        "reflectionclass::getconstants" | "reflectionenum::getconstants" => (&["filter"], &[(0, N)], false),
        "reflectionclass::getproperty" => (&["name"], &[], false),
        "reflectionclass::getproperties" | "reflectionenum::getproperties" => (&["filter"], &[(0, N)], false),
        "reflectionclass::hasmethod" | "reflectionclass::hasproperty" => (&["name"], &[], false),
        "reflectionclass::getconstant" | "reflectionclass::hasconstant" | "reflectionenum::getconstant" | "reflectionenum::hasconstant" => (&["name"], &[], false),
        "reflectionclass::isinstance" | "reflectionenum::isinstance" => (&["object"], &[], false),
        "reflectionclass::getreflectionconstant" | "reflectionenum::getreflectionconstant" => (&["name"], &[], false),
        "reflectionclassconstant::__construct" => (&["class", "constant"], &[], false),
        "reflectionconstant::__construct" => (&["name"], &[], false),
        "reflectionconstant::getattributes" => (&["name", "flags"], &[(0, N), (1, Num(0.0))], false),
        "reflectionconstant::getname" | "reflectionconstant::getvalue" | "reflectionconstant::getnamespace" | "reflectionconstant::isdeprecated" | "reflectionconstant::isinternal" | "reflectionconstant::isuserdefined" => (&[], &[], false),
        "reflectionclassconstant::getattributes" => (&["name", "flags"], &[(0, N), (1, Num(0.0))], false),
        "reflectionclassconstant::getdeclaringclass" | "reflectionclassconstant::getname" | "reflectionclassconstant::getmodifiers" | "reflectionclassconstant::gettype" | "reflectionclassconstant::getvalue" | "reflectionclassconstant::hastype" | "reflectionclassconstant::isenumcase" | "reflectionclassconstant::isfinal" | "reflectionclassconstant::isprivate" | "reflectionclassconstant::isprotected" | "reflectionclassconstant::ispublic" => (&[], &[], false),
        "reflectionuniontype::gettypes" | "reflectionuniontype::allowsnull" | "reflectionintersectiontype::gettypes" | "reflectionintersectiontype::allowsnull" => (&[], &[], false),
        "reflectionclass::isenum" | "reflectionclass::isinterface" | "reflectionclass::isabstract" | "reflectionclass::isinstantiable" | "reflectionclass::isfinal" | "reflectionclass::isreadonly" | "reflectionclass::isanonymous" | "reflectionclass::istrait" | "reflectionclass::iscloneable" | "reflectionclass::isiterable" | "reflectionclass::isiterateable" | "reflectionclass::getparentclass" | "reflectionclass::getinterfacenames" | "reflectionclass::getinterfaces" | "reflectionclass::gettraitnames" | "reflectionclass::gettraits" | "reflectionclass::gettraitaliases" | "reflectionclass::getshortname" | "reflectionclass::getnamespacename" | "reflectionclass::innamespace" => (&[], &[], false),
        "reflectionenum::iscloneable" => (&[], &[], false),
        "reflectionclass::issubclassof" => (&["class"], &[], false),
        "reflectionclass::implementsinterface" => (&["interface"], &[], false),
        "reflectionenum::getcase" | "reflectionenum::hascase" => (&["name"], &[], false),
        "reflectionenum::getcases" | "reflectionenum::isbacked" | "reflectionenum::getbackingtype" | "reflectionenum::getname" | "reflectionenum::isinstantiable" | "reflectionenum::isreadonly" | "reflectionenum::isanonymous" | "reflectionenum::isiterable" | "reflectionenum::isiterateable" | "reflectionenum::getshortname" | "reflectionenum::getnamespacename" | "reflectionenum::innamespace" => (&[], &[], false),
        "reflectionenumunitcase::getname" | "reflectionenumunitcase::getvalue" | "reflectionenumunitcase::getenum" | "reflectionenumunitcase::getdeclaringclass" | "reflectionenumunitcase::getmodifiers" | "reflectionenumunitcase::gettype" | "reflectionenumunitcase::hastype" | "reflectionenumunitcase::isenumcase" | "reflectionenumunitcase::isfinal" | "reflectionenumunitcase::isprivate" | "reflectionenumunitcase::isprotected" | "reflectionenumunitcase::ispublic" | "reflectionenumbackedcase::getbackingvalue" | "reflectionenumbackedcase::getdeclaringclass" | "reflectionenumbackedcase::getmodifiers" | "reflectionenumbackedcase::gettype" | "reflectionenumbackedcase::hastype" | "reflectionenumbackedcase::isenumcase" | "reflectionenumbackedcase::isfinal" | "reflectionenumbackedcase::isprivate" | "reflectionenumbackedcase::isprotected" | "reflectionenumbackedcase::ispublic" => (&[], &[], false),
        "reflectionenumunitcase::getattributes" | "reflectionenumbackedcase::getattributes" => (&["name", "flags"], &[(0, N), (1, Num(0.0))], false),
        "reflectionnamedtype::getname" | "reflectionnamedtype::isbuiltin" | "reflectionnamedtype::allowsnull" => (&[], &[], false),
        "reflectionfunction::getname" => (&[], &[], false),
        "reflectionfunction::getparameters" => (&[], &[], false),
        "reflectionfunction::getattributes" => (&["name", "flags"], &[(0, N), (1, Num(0.0))], false),
        "reflectionmethod::getname" => (&[], &[], false),
        "reflectionmethod::setaccessible" => (&["accessible"], &[], false),
        "reflectionmethod::getdeclaringclass" => (&[], &[], false),
        "reflectionmethod::getmodifiers" => (&[], &[], false),
        "reflectionmethod::getparameters" => (&[], &[], false),
        "reflectionparameter::getname" | "reflectionparameter::getposition" | "reflectionparameter::allowsnull" | "reflectionparameter::getclass" | "reflectionparameter::isarray" | "reflectionparameter::iscallable" | "reflectionparameter::ispromoted" | "reflectionparameter::canbepassedbyvalue" | "reflectionparameter::isdefaultvalueconstant" | "reflectionparameter::getdefaultvalueconstantname" => (&[], &[], false),
        "reflectionparameter::getdeclaringclass" | "reflectionparameter::getdeclaringfunction" => (&[], &[], false),
        "reflectionparameter::getattributes" => (&["name", "flags"], &[(0, N), (1, Num(0.0))], false),
        "reflectionmethod::getattributes" => (&["name", "flags"], &[(0, N), (1, Num(0.0))], false),
        "reflectionmethod::ispublic" | "reflectionmethod::isprotected" | "reflectionmethod::isprivate" | "reflectionmethod::isstatic" | "reflectionmethod::isabstract" | "reflectionmethod::isfinal" | "reflectionmethod::isconstructor" | "reflectionmethod::isdestructor" => (&[], &[], false),
        "reflectionproperty::getname" | "reflectionproperty::gettype" | "reflectionproperty::hastype" | "reflectionproperty::getmodifiers" | "reflectionproperty::ispublic" | "reflectionproperty::isprotected" | "reflectionproperty::isprivate" | "reflectionproperty::isstatic" | "reflectionproperty::isreadonly" | "reflectionproperty::isprivateset" | "reflectionproperty::isprotectedset" | "reflectionproperty::ispromoted" | "reflectionproperty::hasdefaultvalue" | "reflectionproperty::getdefaultvalue" => (&[], &[], false),
        "reflectionproperty::getvalue" => (&["object"], &[(0, N)], false),
        "reflectionproperty::setvalue" => (&["object", "value"], &[(0, N)], false),
        "reflectionproperty::setaccessible" => (&["accessible"], &[], false),
        "reflectionproperty::getdeclaringclass" => (&[], &[], false),
        "reflectionproperty::isinitialized" => (&["object"], &[(0, N)], false),
        "reflectionproperty::getattributes" => (&["name", "flags"], &[(0, N), (1, Num(0.0))], false),
        "reflectionclass::getname" | "reflectionattribute::getname" | "reflectionattribute::getarguments"
        | "reflectionattribute::gettarget" | "reflectionattribute::isrepeated" | "reflectionattribute::newinstance" => (&[], &[], false),
        "random_int" => (&["min", "max"], &[], false),
        "rand" | "mt_rand" => (&["min", "max"], &[(0, Num(0.0)), (1, Num(2_147_483_647.0))], false),
        "getrandmax" | "mt_getrandmax" => (&[], &[], false),
        "str_shuffle" => (&["string"], &[], false),
        "date" | "gmdate" => (&["format", "timestamp"], &[(1, N)], false),
        "strftime" | "gmstrftime" => (&["format", "timestamp"], &[(1, N)], false),
        "strptime" => (&["timestamp", "format"], &[], false),
        "getdate" => (&["timestamp"], &[(0, N)], false),
        "gettimeofday" => (&["as_float"], &[(0, F)], false),
        "idate" => (&["format", "timestamp"], &[(1, N)], false),
        "checkdate" => (&["month", "day", "year"], &[], false),
        "mktime" | "gmmktime" => (&["hour", "minute", "second", "month", "day", "year"], &[(0, N), (1, N), (2, N), (3, N), (4, N), (5, N)], false),
        "localtime" => (&["timestamp", "associative"], &[(0, N), (1, F)], false),
        "strtotime" => (&["datetime", "baseTimestamp"], &[(1, N)], false),
        "exception" | "error" | "typeerror" | "argumentcounterror" | "compileerror" | "parseerror"
        | "arithmeticerror" | "divisionbyzeroerror" | "assertionerror" | "unhandledmatcherror"
        | "valueerror" | "jsonexception" | "logicexception" | "badfunctioncallexception"
        | "badmethodcallexception" | "domainexception" | "invalidargumentexception" | "lengthexception"
        | "outofrangeexception" | "runtimeexception" | "outofboundsexception" | "overflowexception"
        | "rangeexception" | "underflowexception" | "unexpectedvalueexception" => (&["message", "code", "previous"], &[(0, DefaultValue::EmptyString), (1, Num(0.0)), (2, N)], false),
        "exception::getmessage" | "exception::getcode" | "exception::getprevious" | "exception::getfile"
        | "exception::getline" | "exception::gettrace" | "exception::gettraceasstring" => (&[], &[], false),
        "pdo::__construct" => (&["dsn", "username", "password", "options"], &[(1, N), (2, N), (3, N)], false),
        "pdo::exec" => (&["statement"], &[], false),
        "pdo::prepare" => (&["query", "options"], &[(1, DefaultValue::EmptyArray)], false),
        "pdo::quote" => (&["string", "type"], &[(1, Num(2.0))], false),
        "pdo::setattribute" => (&["attribute", "value"], &[], false),
        "pdo::getattribute" => (&["attribute"], &[], false),
        "pdo::lastinsertid" => (&["name"], &[(0, N)], false),
        "pdo::query" => (&["query", "fetchMode"], &[(1, N)], true),
        "pdo::connect" => (&["dsn", "username", "password", "options"], &[(1, N), (2, N), (3, N)], false),
        "pdostatement::bindparam" => (&["param", "var", "type", "maxLength", "driverOptions"], &[(2, Num(2.0)), (3, N), (4, N)], false),
        "pdostatement::bindvalue" => (&["param", "value", "type"], &[(2, Num(2.0))], false),
        "pdostatement::bindcolumn" => (&["column", "var", "type", "maxLength"], &[(2, Num(2.0)), (3, N)], false),
        "pdostatement::execute" => (&["params"], &[(0, N)], false),
        "pdostatement::fetch" => (&["mode", "cursorOrientation", "cursorOffset"], &[(0, N), (1, Num(0.0)), (2, Num(0.0))], false),
        "pdostatement::fetchall" => (&["mode"], &[(0, N)], true),
        "pdostatement::fetchcolumn" => (&["column"], &[(0, Num(0.0))], false),
        "pdostatement::fetchobject" => (&["class", "constructorArgs"], &[(0, DefaultValue::StringDefault), (1, DefaultValue::EmptyArray)], false),
        "pdostatement::getattribute" => (&["name"], &[], false),
        "pdostatement::setattribute" => (&["attribute", "value"], &[], false),
        "pdostatement::getcolumnmeta" => (&["column"], &[], false),
        "pdostatement::setfetchmode" => (&["mode"], &[], true),
        "pdostatement::closecursor" | "pdostatement::columncount" | "pdostatement::debugdumpparams"
        | "pdostatement::errorcode" | "pdostatement::errorinfo" | "pdostatement::nextrowset"
        | "pdostatement::rowcount" | "pdo::begintransaction" | "pdo::commit" | "pdo::errorcode"
        | "pdo::errorinfo" | "pdo::intransaction" | "pdo::rollback" => (&[], &[], false),
        "pdo::getavailabledrivers" => (&[], &[], false),
        "function_exists" => (&["function"], &[], false),
        "getenv" => (&["name", "local_only"], &[(0, N), (1, F)], false),
        "putenv" => (&["assignment"], &[], false),
        "php_sapi_name" => (&[], &[], false),
        "error_reporting" => (&["error_level"], &[], false),
        "trigger_error" | "user_error" => (&["message", "error_level"], &[(1, Num(1024.0))], false),
        "set_error_handler" => (&["callback", "error_levels"], &[(1, Num(32767.0))], false),
        "restore_error_handler" => (&[], &[], false),
        "generator::current" | "generator::key" | "generator::valid" | "generator::rewind"
        | "generator::next" | "generator::getreturn" => (&[], &[], false),
        "generator::send" => (&["value"], &[], false),
        "generator::throw" => (&["exception"], &[], false),
        "enum::cases" => (&[], &[], false),
        "enum::from" | "enum::tryfrom" => (&["value"], &[], false),
        "abs" | "ceil" | "floor" | "sqrt" | "is_finite" | "is_infinite" | "is_nan" | "exp" | "expm1" | "log10" | "log1p" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "asinh" | "acosh" | "atanh" | "deg2rad" | "rad2deg" => (&["num"], &[], false),
        "round" => (&["num", "precision", "mode"], &[(1, Num(0.0)), (2, Num(1.0))], false),
        "fdiv" => (&["num1", "num2"], &[], false),
        "fmod" => (&["num1", "num2"], &[], false),
        "intdiv" => (&["num1", "num2"], &[], false),
        "hypot" => (&["num1", "num2"], &[], false),
        "base_convert" => (&["num", "from_base", "to_base"], &[], false),
        "decbin" | "dechex" | "decoct" => (&["num"], &[], false),
        "atan2" => (&["y", "x"], &[], false),
        "log" => (&["num", "base"], &[(1, Num(std::f64::consts::E))], false),
        "max" | "min" => (&["value"], &[], true),
        "intval" => (&["value", "base"], &[(1, Num(10.0))], false),
        "chr" => (&["codepoint"], &[], false),
        "count_chars" => (&["string", "mode"], &[(1, Num(0.0))], false),
        "floatval" | "strval" | "boolval" => (&["value"], &[], false),
        "gettype" | "get_debug_type" | "is_int" | "is_integer" | "is_long" | "is_float" | "is_double" | "is_real"
        | "is_string" | "is_array" | "is_countable" | "is_iterable" | "is_bool" | "is_null" | "is_object" | "is_resource" | "is_scalar" | "is_numeric" => (&["value"], &[], false),
        "class_exists" => (&["class", "autoload"], &[(1, DefaultValue::True)], false),
        "interface_exists" => (&["interface", "autoload"], &[(1, DefaultValue::True)], false),
        "trait_exists" => (&["trait", "autoload"], &[(1, DefaultValue::True)], false),
        "spl_autoload" => (&["class", "file_extensions"], &[(1, N)], false),
        "spl_autoload_extensions" => (&["file_extensions"], &[(0, N)], false),
        "spl_autoload_register" => (&["autoload_function", "throw", "prepend"], &[(0, N), (1, DefaultValue::True), (2, F)], false),
        "spl_autoload_unregister" => (&["autoload_function"], &[], false),
        "spl_autoload_functions" => (&[], &[], false),
        "spl_autoload_call" => (&["class"], &[], false),
        "method_exists" => (&["object_or_class", "method"], &[], false),
        "property_exists" => (&["object_or_class", "property"], &[], false),
        "get_class" => (&["object"], &[(0, N)], false),
        "get_called_class" => (&[], &[], false),
        "get_parent_class" => (&["object_or_class"], &[(0, N)], false),
        "is_a" => (&["object_or_class", "class", "allow_string"], &[(2, F)], false),
        "is_subclass_of" => (&["object_or_class", "class", "allow_string"], &[(2, DefaultValue::True)], false),
        "is_callable" => (&["value", "syntax_only", "callable_name"], &[(1, F), (2, N)], false),
        "header" => (&["header", "replace", "response_code"], &[(1, DefaultValue::True), (2, N)], false),
        "http_response_code" => (&["response_code"], &[(0, N)], false),
        "header_remove" => (&["name"], &[(0, N)], false),
        "headers_list" => (&[], &[], false),
        "file_exists" | "is_file" | "is_dir" | "is_link" | "is_readable" | "is_writable" | "is_executable" | "filesize" | "filemtime" | "fileatime" | "filectime" | "fileinode" | "fileowner" | "filegroup" | "fileperms" | "filetype" | "realpath" => (&["filename"], &[], false),
        "stat" | "lstat" => (&["filename"], &[], false),
        "getcwd" => (&[], &[], false),
        "get_include_path" => (&[], &[], false),
        "set_include_path" => (&["include_path"], &[], false),
        "sys_get_temp_dir" => (&[], &[], false),
        "time" => (&[], &[], false),
        "microtime" => (&["as_float"], &[(0, F)], false),
        "hrtime" => (&["as_number"], &[(0, F)], false),
        "file_get_contents" => (&["filename", "use_include_path", "context", "offset", "length"], &[(1, F), (2, N), (3, Num(0.0)), (4, N)], false),
        "file" => (&["filename", "flags", "context"], &[(1, Num(0.0)), (2, N)], false),
        "scandir" => (&["directory", "sorting_order", "context"], &[(1, Num(0.0)), (2, N)], false),
        "opendir" => (&["directory", "context"], &[(1, N)], false),
        "readdir" | "closedir" | "rewinddir" => (&["dir_handle"], &[], false),
        "file_put_contents" => (&["filename", "data", "flags", "context"], &[(2, Num(0.0)), (3, N)], false),
        "copy" => (&["from", "to", "context"], &[(2, N)], false),
        "rename" => (&["from", "to", "context"], &[(2, N)], false),
        "mkdir" => (&["directory", "permissions", "recursive", "context"], &[(1, Num(511.0)), (2, F), (3, N)], false),
        "rmdir" => (&["directory", "context"], &[(1, N)], false),
        "tempnam" => (&["directory", "prefix"], &[], false),
        "unlink" => (&["filename"], &[], false),
        "touch" => (&["filename", "mtime", "atime"], &[(1, N), (2, N)], false),
        "chmod" => (&["filename", "permissions"], &[], false),
        "disk_free_space" | "diskfreespace" | "disk_total_space" => (&["directory"], &[], false),
        "fopen" => (&["filename", "mode", "use_include_path", "context"], &[(2, F), (3, N)], false),
        "fwrite" => (&["stream", "data", "length"], &[(2, N)], false),
        "fread" => (&["stream", "length"], &[], false),
        "stream_get_contents" => (&["stream", "length", "offset"], &[(1, Num(-1.0)), (2, Num(-1.0))], false),
        "rewind" | "ftell" | "feof" | "fclose" => (&["stream"], &[], false),
        "fseek" => (&["stream", "offset", "whence"], &[(2, Num(0.0))], false),
        "json_encode" => (&["value", "flags", "depth"], &[(1, Num(0.0)), (2, Num(512.0))], false),
        "json_decode" => (&["json", "associative", "depth", "flags"], &[(1, N), (2, Num(512.0)), (3, Num(0.0))], false),
        "json_validate" => (&["json", "depth", "flags"], &[(1, Num(512.0)), (2, Num(0.0))], false),
        "json_last_error" | "json_last_error_msg" => (&[], &[], false),
        "strtolower" | "strtoupper" | "strlen" | "ucfirst" | "lcfirst" | "strrev" | "str_rot13" | "addslashes" | "stripslashes" | "bin2hex" | "hex2bin" | "ord" | "str_increment" | "str_decrement" => (&["string"], &[], false),
        "crc32" => (&["string"], &[], false),
        "md5" => (&["string", "binary"], &[(1, F)], false),
        "md5_file" => (&["filename", "binary"], &[(1, F)], false),
        "sha1_file" => (&["filename", "binary"], &[(1, F)], false),
        "sha1" => (&["string", "binary"], &[(1, F)], false),
        "hash_equals" => (&["known_string", "user_string"], &[], false),
        "hash_hmac" => (&["algo", "data", "key", "binary"], &[(3, F)], false),
        "hash" => (&["algo", "data", "binary"], &[(2, F)], false),
        "hash_algos" => (&[], &[], false),
        "hash_hmac_algos" => (&[], &[], false),
        "hash_pbkdf2" => (&["algo", "password", "salt", "iterations", "length", "binary", "options"], &[(4, Num(0.0)), (5, F), (6, DefaultValue::EmptyArray)], false),
        "hash_hkdf" => (&["algo", "key", "length", "info", "salt"], &[(2, Num(0.0)), (3, DefaultValue::EmptyString), (4, DefaultValue::EmptyString)], false),
        "hash_file" => (&["algo", "filename", "binary"], &[(2, F)], false),
        "hash_hmac_file" => (&["algo", "filename", "key", "binary"], &[(3, F)], false),
        "base64_encode" => (&["string"], &[], false),
        "base64_decode" => (&["string", "strict"], &[(1, F)], false),
        "htmlentities" => (&["string", "flags", "encoding", "double_encode"], &[(1, Num(11.0)), (2, DefaultValue::Utf8), (3, DefaultValue::True)], false),
        "html_entity_decode" => (&["string", "flags", "encoding"], &[(1, Num(11.0)), (2, N)], false),
        "get_html_translation_table" => (&["table", "flags", "encoding"], &[(0, Num(0.0)), (1, Num(11.0)), (2, DefaultValue::Utf8)], false),
        "addcslashes" => (&["string", "characters"], &[], false),
        "number_format" => (&["num", "decimals", "decimal_separator", "thousands_separator"], &[(1, Num(0.0)), (2, DefaultValue::Dot), (3, DefaultValue::Comma)], false),
        "strcmp" | "strcasecmp" | "strnatcmp" | "strnatcasecmp" => (&["string1", "string2"], &[], false),
        "strncmp" | "strncasecmp" => (&["string1", "string2", "length"], &[], false),
        "strtr" => (&["string", "from", "to"], &[(2, N)], false),
        "ucwords" => (&["string", "separators"], &[(1, DefaultValue::WordSeparators)], false),
        "str_contains" | "str_starts_with" | "str_ends_with" => (&["haystack", "needle"], &[], false),
        "strstr" | "strchr" | "stristr" => (&["haystack", "needle", "before_needle"], &[(2, F)], false),
        "str_replace" | "str_ireplace" => (&["search", "replace", "subject", "count"], &[(3, N)], false),
        "trim" | "ltrim" | "rtrim" => (&["string", "characters"], &[(1, N)], false),
        "str_repeat" => (&["string", "times"], &[], false),
        "strspn" | "strcspn" => (&["string", "characters", "offset", "length"], &[(2, Num(0.0)), (3, N)], false),
        "strpbrk" => (&["string", "characters"], &[], false),
        "str_word_count" => (&["string", "format", "characters"], &[(1, Num(0.0)), (2, DefaultValue::EmptyString)], false),
        "str_pad" => (&["string", "length", "pad_string", "pad_type"], &[(2, DefaultValue::Space), (3, Num(1.0))], false),
        "str_split" => (&["string", "length"], &[(1, Num(1.0))], false),
        "strpos" | "stripos" | "strrpos" | "strripos" => (&["haystack", "needle", "offset"], &[(2, Num(0.0))], false),
        "substr_count" => (&["haystack", "needle", "offset", "length"], &[(2, Num(0.0)), (3, N)], false),
        "substr_compare" => (&["haystack", "needle", "offset", "length", "case_insensitive"], &[(3, N), (4, F)], false),
        "strtok" => (&["string", "token"], &[], false),
        "levenshtein" => (&["string1", "string2", "insertion_cost", "replacement_cost", "deletion_cost"], &[(2, Num(1.0)), (3, Num(1.0)), (4, Num(1.0))], false),
        "similar_text" => (&["string1", "string2", "percent"], &[(2, N)], false),
        "substr" => (&["string", "offset", "length"], &[(2, N)], false),
        "substr_replace" => (&["string", "replace", "offset", "length"], &[(3, N)], false),
        "basename" => (&["path", "suffix"], &[(1, DefaultValue::EmptyString)], false),
        "dirname" => (&["path", "levels"], &[(1, Num(1.0))], false),
        "pathinfo" => (&["path", "flags"], &[(1, Num(15.0))], false),
        "strip_tags" => (&["string", "allowed_tags"], &[(1, DefaultValue::EmptyString)], false),
        "sprintf" => (&["format"], &[], true),
        "printf" => (&["format"], &[], true),
        "vprintf" | "vsprintf" => (&["format", "values"], &[], false),
        "fprintf" => (&["stream", "format"], &[], true),
        "parse_str" => (&["string", "result"], &[], false),
        "stripcslashes" => (&["string"], &[], false),
        "urlencode" | "urldecode" | "rawurlencode" | "rawurldecode" => (&["string"], &[], false),
        "explode" => (&["separator", "string", "limit"], &[(2, Num(i64::MAX as f64))], false),
        "implode" => (&["separator", "array"], &[(0, DefaultValue::EmptyString)], false),
        "str_getcsv" => (&["string", "separator", "enclosure", "escape"], &[(1, DefaultValue::Comma), (2, DefaultValue::Quote), (3, DefaultValue::Backslash)], false),
        "var_dump" => (&["value"], &[], true),
        "htmlspecialchars" => (&["string", "flags", "encoding", "double_encode"], &[(1, Num(11.0)), (2, DefaultValue::Utf8), (3, DefaultValue::True)], false),
        "htmlspecialchars_decode" => (&["string", "flags"], &[(1, Num(3.0))], false),
        "nl2br" => (&["string", "use_xhtml"], &[(1, DefaultValue::True)], false),
        "wordwrap" => (&["string", "width", "break", "cut_long_words"], &[(1, Num(75.0)), (2, DefaultValue::Newline), (3, F)], false),
        "is_uploaded_file" => (&["filename"], &[], false),
        "move_uploaded_file" => (&["from", "to"], &[], false),
        "session_start" => (&["options"], &[(0, N)], false),
        "session_destroy" | "session_status" => (&[], &[], false),
        "session_id" => (&["id"], &[(0, N)], false),
        "session_unset" => (&[], &[], false),
        "array_change_key_case" => (&["array", "case"], &[(1, Num(0.0))], false),
        "array_chunk" => (&["array", "length", "preserve_keys"], &[(2, F)], false),
        "array_column" => (&["array", "column_key", "index_key"], &[(2, N)], false),
        "array_combine" => (&["keys", "values"], &[], false),
        "array_count_values" => (&["array"], &[], false),
        "array_fill" => (&["start_index", "count", "value"], &[], false),
        "array_fill_keys" => (&["keys", "value"], &[], false),
        "range" => (&["start", "end", "step"], &[(2, Num(1.0))], false),
        "array_flip" => (&["array"], &[], false),
        "array_key_exists" | "key_exists" => (&["key", "array"], &[], false),
        "array_key_first" | "array_key_last" | "array_first" | "array_last" => (&["array"], &[], false),
        "array_is_list" => (&["array"], &[], false),
        "in_array" => (&["needle", "haystack", "strict"], &[(2, F)], false),
        "array_merge" | "array_merge_recursive" => (&["arrays"], &[], true),
        "array_replace" | "array_replace_recursive" | "array_diff" | "array_intersect" | "array_diff_assoc" | "array_intersect_assoc"
        | "array_diff_key" | "array_intersect_key" => (&["array"], &[], true),
        "array_reverse" => (&["array", "preserve_keys"], &[(1, F)], false),
        "array_unique" => (&["array", "flags"], &[(1, Num(2.0))], false),
        "array_filter" => (&["array", "callback", "mode"], &[(1, N), (2, Num(0.0))], false),
        "array_all" | "array_any" | "array_find" | "array_find_key" => (&["array", "callback"], &[], false),
        "array_walk" | "array_walk_recursive" => (&["array", "callback", "arg"], &[(2, N)], false),
        "array_map" => (&["callback", "array"], &[], true),
        "array_reduce" => (&["array", "callback", "initial"], &[(2, N)], false),
        "array_search" => (&["needle", "haystack", "strict"], &[(2, F)], false),
        "array_rand" => (&["array", "num"], &[(1, Num(1.0))], false),
        "array_sum" | "array_product" => (&["array"], &[], false),
        "array_slice" => (&["array", "offset", "length", "preserve_keys"], &[(2, N), (3, F)], false),
        "array_pad" => (&["array", "length", "value"], &[], false),
        "array_pop" | "array_shift" | "shuffle" | "natsort" | "natcasesort" => (&["array"], &[], false),
        "array_push" | "array_unshift" => (&["array"], &[], true),
        "array_splice" => (&["array", "offset", "length", "replacement"], &[(2, N), (3, N)], false),
        "sort" | "rsort" | "asort" | "arsort" | "ksort" | "krsort" => (&["array", "flags"], &[(1, Num(0.0))], false),
        "usort" | "uasort" | "uksort" => (&["array", "callback"], &[], false),
        "array_multisort" => (&["array"], &[], true),
        "current" | "pos" | "key" | "next" | "prev" | "reset" | "end" | "each" => (&["array"], &[], false),
        "compact" => (&["var_name"], &[], true),
        "extract" => (&["array", "flags", "prefix"], &[(1, Num(0.0)), (2, N)], false),
        "define" => (&["constant_name", "value", "case_insensitive"], &[(2, F)], false),
        "defined" => (&["constant_name"], &[], false),
        "constant" => (&["name"], &[], false),
        "count" | "sizeof" => (&["value", "mode"], &[(1, Num(0.0))], false),
        "array_keys" => (&["array", "filter_value", "strict"], &[(1, N), (2, F)], false),
        "array_values" => (&["array"], &[], false),
        "iterator_count" => (&["iterator"], &[], false),
        "iterator_to_array" => (&["iterator", "preserve_keys"], &[(1, DefaultValue::True)], false),
        "" => return None,
        _ => return None,
    };
    Some(Signature { names, defaults, variadic })
}

pub(super) fn is_vm_function(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    !name.contains("::")
        && (name != "strptime" || cfg!(unix))
        && !matches!(name.as_str(),
            "exception" | "error" | "typeerror" | "argumentcounterror" | "compileerror" | "parseerror"
            | "arithmeticerror" | "divisionbyzeroerror" | "assertionerror" | "unhandledmatcherror"
            | "valueerror" | "jsonexception" | "logicexception" | "badfunctioncallexception"
            | "badmethodcallexception" | "domainexception" | "invalidargumentexception" | "lengthexception"
            | "outofrangeexception" | "runtimeexception" | "outofboundsexception" | "overflowexception"
            | "rangeexception" | "underflowexception" | "unexpectedvalueexception")
        && signature(&name).is_some()
}

fn callback_variadic_parameters(name: &str) -> Option<&'static [&'static str]> {
    match name.to_ascii_lowercase().as_str() {
        "array_diff_uassoc" | "array_intersect_uassoc" | "array_diff_ukey" | "array_intersect_ukey" => Some(&["key_compare_func"]),
        "array_udiff" | "array_udiff_assoc" | "array_uintersect" | "array_uintersect_assoc" => Some(&["value_compare_func"]),
        "array_udiff_uassoc" | "array_udiff_ukey" | "array_uintersect_uassoc" | "array_uintersect_ukey" => Some(&["value_compare_func", "key_compare_func"]),
        _ => None,
    }
}

fn bind_callback_variadic(
    function_name: &str,
    callback_names: &[&str],
    arguments: Vec<Value>,
    reference_names: &[Option<ReferenceArgument>],
    argument_names: &[Option<String>],
) -> Result<(Vec<Value>, Vec<Option<ReferenceArgument>>), String> {
    let mut primary = None;
    let mut positional = Vec::<(usize, Value, Option<ReferenceArgument>)>::new();
    let mut extras = Vec::<(usize, ArrayKey, Value, Option<ReferenceArgument>)>::new();
    let mut callbacks = vec![None; callback_names.len()];
    let mut named_seen = false;
    let mut next_index = 0i64;
    for (index, value) in arguments.into_iter().enumerate() {
        let reference = reference_names.get(index).cloned().flatten();
        match argument_names.get(index).and_then(Option::as_deref) {
            Some("array") => {
                named_seen = true;
                if primary.replace((value, reference)).is_some() {
                    return Err("Named parameter $array overwrites previous argument".into());
                }
            }
            Some(label) if callback_names.iter().any(|name| *name == label) => {
                named_seen = true;
                let callback_index = callback_names.iter().position(|name| *name == label).expect("matched callback name");
                if callbacks[callback_index].replace((value, reference)).is_some() {
                    return Err(format!("Named parameter ${label} overwrites previous argument"));
                }
            }
            Some(label) => {
                named_seen = true;
                if extras.iter().any(|(_, key, _, _)| key == &ArrayKey::String(label.to_string())) {
                    return Err(format!("{function_name}(): named argument '${label}' is passed more than once"));
                }
                extras.push((index, ArrayKey::String(label.to_string()), value, reference));
            }
            None => {
                if named_seen {
                    return Err(format!("{function_name}(): positional argument follows a named argument"));
                }
                positional.push((index, value, reference));
            }
        }
    }

    if primary.is_some() && !positional.is_empty() {
        return Err(format!("{function_name}(): argument '$array' is passed more than once"));
    }
    if primary.is_none() && !positional.is_empty() {
        let (_, value, reference) = positional.remove(0);
        primary = Some((value, reference));
    }
    let primary = primary.ok_or_else(|| format!("{function_name}(): argument '$array' is required"))?;
    let missing_indexes = (0..callbacks.len()).filter(|index| callbacks[*index].is_none()).collect::<Vec<_>>();
    if positional.len() < missing_indexes.len() {
        return Err(format!("{function_name}(): expected at least {} arguments", 1 + callback_names.len()));
    }
    for callback_index in missing_indexes.into_iter().rev() {
        let (_, value, reference) = positional.pop().expect("enough positional callback arguments checked");
        callbacks[callback_index] = Some((value, reference));
    }
    for (index, value, reference) in positional {
        extras.push((index, ArrayKey::Integer(next_index), value, reference));
        next_index += 1;
    }

    let mut bound = vec![primary.0];
    let mut references = vec![primary.1];
    extras.sort_by_key(|(index, _, _, _)| *index);
    for (_, _, value, reference) in extras {
        bound.push(value);
        references.push(reference);
    }
    for (index, callback) in callbacks.into_iter().enumerate() {
        let (value, reference) = callback.ok_or_else(|| format!("{function_name}(): argument '${}' is required", callback_names[index]))?;
        bound.push(value);
        references.push(reference);
    }
    Ok((bound, references))
}

/// Bind arguments for supported builtins while keeping by-reference variable names aligned.
pub(super) fn bind(
    name: &str,
    arguments: Vec<Value>,
    reference_names: &[Option<ReferenceArgument>],
    argument_names: &[Option<String>],
) -> Result<(Vec<Value>, Vec<Option<ReferenceArgument>>), String> {
    if !argument_names.iter().any(Option::is_some) {
        return Ok((arguments, reference_names.to_vec()));
    }
    let function_name = name.to_ascii_lowercase();
    if let Some(callback_names) = callback_variadic_parameters(&function_name) {
        return bind_callback_variadic(&function_name, callback_names, arguments, reference_names, argument_names);
    }
    let signature = signature(&function_name)
        .ok_or_else(|| format!("{function_name}(): named arguments are not supported for this built-in"))?;
    let mut slots = vec![None::<(Value, Option<ReferenceArgument>)>; signature.names.len()];
    let mut extras = Vec::<(ArrayKey, Value, Option<ReferenceArgument>)>::new();
    let mut next_position = 0;
    let mut next_extra_index = 0i64;
    let mut named_seen = false;
    for (index, value) in arguments.into_iter().enumerate() {
        let reference = reference_names.get(index).cloned().flatten();
        if let Some(Some(label)) = argument_names.get(index) {
            named_seen = true;
            let parameter = signature.names.iter().position(|parameter| parameter == label);
            if let Some(parameter) = parameter {
                if slots[parameter].is_some() {
                    return Err(format!("Named parameter ${label} overwrites previous argument"));
                }
                slots[parameter] = Some((value, reference));
            } else if signature.variadic {
                if extras.iter().any(|(key, _, _)| key == &ArrayKey::String(label.clone())) {
                    return Err(format!("Named parameter ${label} overwrites previous argument"));
                }
                extras.push((ArrayKey::String(label.clone()), value, reference));
            } else {
                return Err(format!("{function_name}(): unknown named parameter '${label}'"));
            }
        } else {
            if named_seen {
                return Err(format!("{function_name}(): positional argument follows a named argument"));
            }
            if next_position < slots.len() {
                slots[next_position] = Some((value, reference));
                next_position += 1;
            } else if signature.variadic {
                extras.push((ArrayKey::Integer(next_extra_index), value, reference));
                next_extra_index += 1;
            } else {
                // Keep legacy arity diagnostics and extra-argument handling in the builtin itself.
                extras.push((ArrayKey::Integer(next_extra_index), value, reference));
                next_extra_index += 1;
            }
        }
    }
    let mut bound = Vec::new();
    let mut references = Vec::new();
    for (index, slot) in slots.into_iter().enumerate() {
        if let Some((value, reference)) = slot {
            bound.push(value);
            references.push(reference);
        } else if let Some((_, default)) = signature.defaults.iter().find(|(position, _)| *position == index) {
            if matches!(function_name.as_str(), "array_walk" | "array_walk_recursive") && index == 2 { break; }
            bound.push(default.value());
            references.push(None);
        } else {
            if !matches!(function_name.as_str(), "array_merge" | "array_merge_recursive") {
                return Err(format!("{function_name}(): required argument '${}' was not passed", signature.names[index]));
            }
        }
    }
    if signature.variadic && !extras.is_empty() {
        for (_, value, reference) in extras.drain(..) {
            bound.push(value);
            references.push(reference);
        }
    } else {
        for (_, value, reference) in extras {
            bound.push(value);
            references.push(reference);
        }
    }
    Ok((bound, references))
}

#[cfg(test)]
mod tests {
    use super::{bind, callback_variadic_parameters, signature};
    use crate::Value;

    #[test]
    fn every_supported_array_builtin_has_named_parameter_metadata() {
        for name in crate::array_builtins::supported_names() {
            assert!(signature(name).is_some() || callback_variadic_parameters(name).is_some(),
                "missing named-argument signature for {name}");
        }
    }

    #[test]
    fn every_supported_math_builtin_has_named_parameter_metadata() {
        for name in crate::math_builtins::supported_names() {
            assert!(signature(name).is_some(), "missing named-argument signature for {name}");
        }
    }

    #[test]
    fn every_supported_string_builtin_has_named_parameter_metadata() {
        for name in crate::string_builtins::supported_names() {
            assert!(signature(name).is_some(), "missing named-argument signature for {name}");
            assert!(crate::string_builtins::is_supported(name), "string dispatcher does not expose {name}");
        }
    }

    #[test]
    fn every_explicit_vm_builtin_has_named_parameter_metadata_or_custom_binding() {
        // call_user_func has a dedicated binder because it must preserve named keys
        // while forwarding arguments to another callable.
        let custom_binding = ["call_user_func"];
        let names = [
            "addcslashes", "addslashes", "basename", "boolval", "call_user_func_array", "checkdate", "chr", "class_exists", "compact", "constant", "count_chars", "date", "define", "defined", "dirname", "error_clear_last", "error_get_last", "error_reporting", "trigger_error", "user_error", "set_error_handler", "restore_error_handler", "exit", "die", "explode", "extract", "fclose", "feof", "file", "file_exists", "file_get_contents", "file_put_contents", "filesize", "floatval", "fopen", "fread", "fseek", "ftell", "func_get_arg", "func_get_args", "func_num_args", "function_exists", "fwrite", "gc_collect_cycles", "get_called_class", "get_class", "get_debug_type", "get_defined_vars", "get_parent_class", "getcwd", "get_include_path", "getdate", "getenv", "gettimeofday", "putenv", "getrandmax", "gettype", "gmdate", "gmmktime", "header", "header_remove", "headers_list", "hrtime", "hex2bin", "htmlspecialchars", "htmlspecialchars_decode", "http_response_code", "idate", "implode", "interface_exists", "trait_exists", "intval", "is_a", "is_array", "is_bool", "is_callable", "is_countable", "is_iterable", "is_dir", "is_double", "is_executable", "is_file", "is_float", "is_int", "is_integer", "is_link", "is_null", "is_numeric", "is_object", "is_readable", "is_real", "is_resource", "is_scalar", "is_string", "is_subclass_of", "is_uploaded_file", "is_writable", "json_decode", "json_encode", "json_last_error", "json_last_error_msg", "json_validate", "lcfirst", "localtime", "ltrim", "method_exists", "microtime", "mktime", "mongo_aggregate", "mongo_begin_transaction", "mongo_commit", "mongo_count_documents", "mongo_delete_many", "mongo_delete_one", "mongo_estimated_document_count", "mongo_find", "mongo_insert_many", "mongo_insert_one", "mongo_replace_one", "mongo_rollback", "mongo_update_many", "mongo_update_one", "move_uploaded_file", "mt_getrandmax", "mt_rand", "nl2br", "number_format", "ord", "pathinfo", "php_sapi_name", "property_exists", "rand", "random_int", "realpath", "rewind", "rtrim", "session_destroy", "session_id", "session_start", "session_status", "session_unset", "set_include_path", "spl_autoload", "spl_autoload_call", "spl_autoload_extensions", "spl_autoload_functions", "spl_autoload_register", "spl_autoload_unregister", "str_contains", "str_decrement", "str_ends_with", "str_increment", "str_ireplace", "str_pad", "str_repeat", "str_replace", "str_shuffle", "str_split", "str_starts_with", "str_word_count", "strcasecmp", "strchr", "strcmp", "strcspn", "stream_get_contents", "stripos", "stripslashes", "stristr", "strlen", "strnatcasecmp", "strnatcmp", "strncasecmp", "strncmp", "strpbrk", "strpos", "strrev", "strripos", "strrpos", "strspn", "strstr", "strtolower", "strtotime", "strtoupper", "strtr", "strval", "substr", "substr_count", "substr_replace", "sys_get_temp_dir", "tempnam", "time", "trim", "ucfirst", "ucwords", "unlink", "var_dump", "wordwrap", "md5_file", "sha1_file", "hash_file",
        ];
        for name in names {
            assert!(signature(name).is_some() || custom_binding.contains(&name),
                "missing named-argument signature or custom binder for {name}");
            assert!(super::is_vm_function(name) || custom_binding.contains(&name),
                "VM builtin inventory does not expose {name} to function_exists()");
        }
    }

    #[test]
    fn include_path_builtins_bind_named_arguments_and_reject_unknown_names() {
        let (arguments, _) = bind(
            "set_include_path",
            vec![Value::String("/custom/php".into())],
            &[None],
            &[Some("include_path".into())],
        ).unwrap();
        assert!(matches!(&arguments[0], Value::String(path) if path == "/custom/php"));

        let (arguments, _) = bind("get_include_path", Vec::new(), &[], &[]).unwrap();
        assert!(arguments.is_empty());
        assert_eq!(
            bind("get_include_path", vec![Value::Null], &[None], &[Some("include_path".into())]).unwrap_err(),
            "get_include_path(): unknown named parameter '$include_path'",
        );
    }

    #[test]
    fn pdo_builtin_methods_bind_named_arguments_and_defaults() {
        let (arguments, _) = bind(
            "pdo::setAttribute",
            vec![Value::String("value".into()), Value::Integer(99)],
            &[None, None],
            &[Some("value".into()), Some("attribute".into())],
        ).unwrap();
        assert!(matches!(arguments[0], Value::Integer(99)));
        assert!(matches!(&arguments[1], Value::String(value) if value == "value"));

        let (arguments, _) = bind(
            "pdo::prepare",
            vec![Value::String("select 1".into())],
            &[None],
            &[Some("query".into())],
        ).unwrap();
        assert_eq!(arguments.len(), 2);
        assert!(matches!(arguments[1], Value::Array(ref options) if options.is_empty()));
    }

    #[test]
    fn implemented_pdo_methods_have_parameter_metadata_for_named_calls() {
        for method in [
            "pdo::__construct", "pdo::beginTransaction", "pdo::commit", "pdo::errorCode", "pdo::errorInfo",
            "pdo::exec", "pdo::getAttribute", "pdo::getAvailableDrivers", "pdo::inTransaction", "pdo::lastInsertId",
            "pdo::prepare", "pdo::query", "pdo::quote", "pdo::rollBack", "pdo::setAttribute", "pdo::connect",
            "pdostatement::bindColumn", "pdostatement::bindParam", "pdostatement::bindValue", "pdostatement::closeCursor",
            "pdostatement::columnCount", "pdostatement::debugDumpParams", "pdostatement::errorCode", "pdostatement::errorInfo",
            "pdostatement::execute", "pdostatement::fetch", "pdostatement::fetchAll", "pdostatement::fetchColumn",
            "pdostatement::fetchObject", "pdostatement::getAttribute", "pdostatement::getColumnMeta", "pdostatement::nextRowset",
            "pdostatement::rowCount", "pdostatement::setAttribute", "pdostatement::setFetchMode",
        ] {
            assert!(signature(method).is_some(), "missing named-argument metadata for {method}");
        }
    }

    #[test]
    fn strftime_family_has_php_named_argument_metadata() {
        for function in ["strftime", "gmstrftime"] {
            assert!(signature(function).is_some(), "missing named-argument metadata for {function}");
        }
    }

    #[test]
    fn strptime_has_php_named_argument_metadata() {
        assert!(signature("strptime").is_some());
    }

    #[test]
    fn implemented_mongodb_methods_have_named_parameter_metadata() {
        for method in [
            "mongodb\\client::__construct", "mongodb\\client::selectDatabase", "mongodb\\client::selectCollection",
            "mongodb\\database::selectCollection", "mongodb\\collection::find", "mongodb\\collection::findOne",
            "mongodb\\collection::aggregate", "mongodb\\collection::countDocuments", "mongodb\\collection::estimatedDocumentCount",
            "mongodb\\collection::insertOne", "mongodb\\collection::insertMany", "mongodb\\collection::replaceOne",
            "mongodb\\collection::updateOne", "mongodb\\collection::updateMany", "mongodb\\collection::deleteOne",
            "mongodb\\collection::deleteMany", "mongodb\\cursor::toArray", "mongodb\\insertresult::getInsertedCount",
            "mongodb\\insertmanyresult::getInsertedCount", "mongodb\\updateresult::getMatchedCount",
            "mongodb\\updateresult::getModifiedCount", "mongodb\\deleteresult::getDeletedCount",
        ] {
            assert!(signature(method).is_some(), "missing named-argument metadata for {method}");
        }
        let (arguments, _) = bind(
            "mongodb\\client::__construct",
            vec![Value::array(Vec::new())],
            &[],
            &[Some("uriOptions".into())],
        ).unwrap();
        assert!(matches!(&arguments[0], Value::String(uri) if uri == "mongodb://127.0.0.1/"));
        assert!(matches!(&arguments[1], Value::Array(options) if options.is_empty()));
        assert!(matches!(&arguments[2], Value::Array(options) if options.is_empty()));
    }

    #[test]
    fn implemented_exception_methods_have_named_parameter_metadata() {
        for method in [
            "exception::getMessage", "exception::getCode", "exception::getPrevious", "exception::getFile",
            "exception::getLine", "exception::getTrace", "exception::getTraceAsString",
        ] {
            assert!(signature(method).is_some(), "missing named-argument metadata for {method}");
        }
    }

    #[test]
    fn implemented_reflection_methods_have_named_parameter_metadata() {
        for method in [
            "reflectionclass::getName", "reflectionclass::getAttributes", "reflectionclass::getMethod", "reflectionclass::getMethods", "reflectionclass::getProperty", "reflectionclass::getProperties", "reflectionenum::getProperties", "reflectionclass::getDefaultProperties", "reflectionenum::getDefaultProperties", "reflectionclass::getStaticProperties", "reflectionenum::getStaticProperties",
            "reflectionclass::isEnum", "reflectionclass::isInterface", "reflectionclass::isAbstract", "reflectionclass::isFinal", "reflectionclass::isReadOnly", "reflectionclass::isAnonymous", "reflectionclass::isIterable", "reflectionclass::isIterateable", "reflectionclass::getParentClass", "reflectionclass::getInterfaceNames", "reflectionclass::getInterfaces", "reflectionclass::getTraitNames", "reflectionclass::getTraits", "reflectionclass::getTraitAliases", "reflectionclass::hasMethod", "reflectionclass::hasProperty",
            "reflectionclass::isSubclassOf", "reflectionclass::implementsInterface",
            "reflectionfunction::getName", "reflectionfunction::getAttributes", "reflectionfunction::getParameters", "reflectionmethod::getName", "reflectionmethod::getModifiers",
            "reflectionmethod::getAttributes", "reflectionmethod::getParameters", "reflectionmethod::setAccessible", "reflectionmethod::isPublic", "reflectionmethod::isProtected", "reflectionmethod::isPrivate", "reflectionmethod::isStatic", "reflectionmethod::isAbstract", "reflectionmethod::isFinal", "reflectionmethod::isConstructor", "reflectionmethod::isDestructor", "reflectionproperty::getName", "reflectionproperty::setAccessible",
            "reflectionproperty::getAttributes", "reflectionproperty::getModifiers", "reflectionproperty::isPublic", "reflectionproperty::isProtected", "reflectionproperty::isPrivate", "reflectionproperty::isStatic", "reflectionproperty::isReadOnly", "reflectionproperty::isPrivateSet", "reflectionproperty::isProtectedSet", "reflectionproperty::isPromoted", "reflectionparameter::getName", "reflectionparameter::getPosition",
            "reflectionparameter::getAttributes", "reflectionattribute::getName", "reflectionattribute::getArguments",
            "reflectionattribute::getTarget", "reflectionattribute::isRepeated", "reflectionattribute::newInstance",
        ] {
            assert!(signature(method).is_some(), "missing named-argument metadata for {method}");
        }
    }

    #[test]
    fn generator_methods_have_php_named_parameter_metadata() {
        for method in ["current", "key", "valid", "rewind", "next", "getReturn", "send", "throw"] {
            assert!(signature(&format!("generator::{method}")).is_some(), "missing named-argument metadata for Generator::{method}");
        }
    }

    #[test]
    fn enum_methods_have_php_named_parameter_metadata() {
        for method in ["enum::cases", "enum::from", "enum::tryFrom"] {
            assert!(signature(method).is_some(), "missing named-argument metadata for {method}");
        }
    }
}
