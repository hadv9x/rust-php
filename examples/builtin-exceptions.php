<?php
$cause = new Exception("root cause");
$error = new TypeError("expected integer", 0, $cause);
echo $error->getMessage(), ":", $error->getCode(), ":", $error->getPrevious()->getMessage(), "\n";
echo get_class($error), ":", get_parent_class($error), ":", ($error instanceof Throwable), "\n";
$exceptionReflection = new ReflectionClass(BadMethodCallException::class);
echo "reflected exception parents:", $exceptionReflection->getParentClass()->getName(), ":", $exceptionReflection->getParentClass()->getParentClass()->getName(), "\n";
echo "throwable-stringable:", ($error instanceof Stringable), ":", (new ReflectionClass(Throwable::class))->implementsInterface(Stringable::class), ":", method_exists(Throwable::class, "__toString"), "\n";
echo $error;
$warning = new ErrorException("warning", 0, E_WARNING, "app.php", 21);
echo get_class($warning), ":", get_parent_class($warning), ":", $warning->getSeverity(), ":", $warning->getFile(), ":", $warning->getLine(), "\n";
$severityProperty = (new ReflectionClass(ErrorException::class))->getProperty("severity");
echo "reflected severity:", $severityProperty->getValue($warning), "\n";
echo "numeric code:", (new Exception("numeric", "12.8"))->getCode(), "\n";
try {
    new Exception("invalid code", "nope");
} catch (TypeError $typeError) {
    echo "invalid code:", get_class($typeError), ":", $typeError->getMessage(), "\n";
}
try {
    new Exception(["invalid message"]);
} catch (TypeError $typeError) {
    echo "invalid message:", get_class($typeError), "\n";
}

class ApplicationFailure extends Exception {
    public function __construct($message) { parent::__construct($message, 23); }
}
$applicationFailure = new ApplicationFailure("database unavailable");
echo $applicationFailure->getMessage(), ":", $applicationFailure->getCode(), "\n";

class ExceptionMessageText {
    public function __toString(): string { return "coerced exception message"; }
}
echo (new Exception(new ExceptionMessageText()))->getMessage(), "\n";

class DisplayFailure extends RuntimeException {
    public function __toString(): string { return "application failure: " . $this->getMessage(); }
}
echo new DisplayFailure("custom string form"), "\n";
try {
    clone $applicationFailure;
} catch (Error $cloneError) {
    echo "exception clone:", get_class($cloneError), "\n";
}

try {
    throw new Error("runtime failure");
} catch (Exception $unexpected) {
    echo "caught as Exception\n";
} catch (Throwable $caught) {
    echo get_class($caught), ":", $caught->getMessage(), "\n";
}

try {
    missing_runtime_function();
} catch (Error $error) {
    echo get_class($error), ":", $error->getMessage(), "\n";
}

class NotStringableForErrorExample {}
try {
    (string) new NotStringableForErrorExample();
} catch (Error $error) {
    echo 'string cast:', get_class($error), ':', $error->getMessage(), "\n";
}

try {
    str_word_count();
} catch (ArgumentCountError $error) {
    echo get_class($error), ":", $error->getMessage(), "\n";
}

function requiresReferenceExample(&$value) {}
try {
    requiresReferenceExample(1);
} catch (Error $error) {
    echo get_class($error), ":", $error->getMessage(), "\n";
}

function divideInsideExample() {
    return intdiv(5, 0);
}
function callDividerExample() {
    return divideInsideExample();
}
try {
    callDividerExample();
} catch (DivisionByZeroError $error) {
    $trace = $error->getTrace();
    echo get_class($error), ":", $trace[0]["function"], ":", $trace[1]["function"], "\n";
}

function traceLongArgumentExample($value) {
    return new RuntimeException('trace argument');
}
$longTrace = traceLongArgumentExample('01234567890123456789')->getTraceAsString();
echo $longTrace, "\n";
$previousTraceLimit = ini_set('zend.exception_string_param_max_len', 5);
echo "trace-limit:", ini_get('zend.exception_string_param_max_len'), ":", ini_set('zend.exception_string_param_max_len', $previousTraceLimit), "\n";

class MissingMethodExample {}
try {
    (new MissingMethodExample())->missingMethod();
} catch (Error $error) {
    echo get_class($error), ":", $error->getMessage(), "\n";
}

$nonObject = null;
try {
    $nonObject->missingMethod();
} catch (Error $error) {
    echo get_class($error), ":", $error->getMessage(), "\n";
}

class NoStringConversionExample {}
try {
    (string) new NoStringConversionExample();
} catch (Error $error) {
    echo get_class($error), ":", $error->getMessage(), "\n";
}
?>
