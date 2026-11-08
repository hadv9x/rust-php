<?php
$cause = new Exception("root cause");
$error = new TypeError("expected integer", 0, $cause);
echo $error->getMessage(), ":", $error->getCode(), ":", $error->getPrevious()->getMessage(), "\n";
echo get_class($error), ":", get_parent_class($error), ":", ($error instanceof Throwable), "\n";
echo $error;

class ApplicationFailure extends Exception {
    public function __construct($message) { parent::__construct($message, 23); }
}
$applicationFailure = new ApplicationFailure("database unavailable");
echo $applicationFailure->getMessage(), ":", $applicationFailure->getCode(), "\n";

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
