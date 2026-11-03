<?php
class TraceFactory {
    public function create() { return new RuntimeException('located'); }
}
$located = (new TraceFactory())->create();
echo $located->getTrace()[0]['class'], ' ', $located->getTraceAsString(), "\n";

// Catch an explicitly thrown exception and expose its string form in the catch variable.
try {
    throw new Exception("invalid input");
} catch (Exception $error) {
    echo "caught: ", $error, "\n";
} finally {
    echo "finally after catch\n";
}

try {
    $locatedError = new RuntimeException("created here");
    throw $locatedError;
} catch (RuntimeException $error) {
    echo "created on line: ", $error->getLine(), "\n";
}

// Exceptions thrown in a function propagate to the caller's active catch block.
function failOperation() {
    throw new Exception("operation failed");
}

try {
    failOperation();
} catch (Throwable $error) {
    echo "caught function: ", $error, "\n";
} finally {
    echo "finally after function catch\n";
}

function createLocatedException() {
    return new RuntimeException("trace captured here");
}
$tracedError = createLocatedException();
$trace = $tracedError->getTrace();
echo "trace frame: ", $trace[0]['function'], " at line ", $trace[0]['line'], "\n";
echo $tracedError->getTraceAsString(), "\n";

class AppException extends Exception {}
class InvalidInputException extends AppException {}
class UnrelatedException extends Exception {}

// A mismatched inner catch is skipped and the outer compatible type catches it.
try {
    try {
        throw new InvalidInputException();
    } catch (UnrelatedException $error) {
        echo "wrong catch\n";
    } finally {
        echo "finally after mismatched catch\n";
    }
} catch (AppException $error) {
    echo "caught typed exception\n";
}

function returnThroughFinally() {
    try {
        return "returned";
    } catch (Exception $error) {
        return "caught";
    } finally {
        echo "finally before return\n";
    }
}
echo returnThroughFinally(), "\n";

try {
    try {
        throw new Exception("finally-only");
    } finally {
        echo "finally without catch\n";
    }
} catch (Exception $error) {
    echo "caught after finally-only\n";
}

try {
    try {
        throw new Exception("original");
    } finally {
        throw new Exception("replacement");
    }
} catch (Exception $error) {
    echo "caught replacement: ", $error, "\n";
}

$iterations = 0;
while ($iterations < 3) {
    try {
        $iterations++;
        if ($iterations < 3) {
            continue;
        }
        break;
    } finally {
        echo "loop finally\n";
    }
}
echo "iterations: ", $iterations, "\n";

while (true) {
    try {
        try {
            break;
        } finally {
            echo "inner finally\n";
        }
    } finally {
        echo "outer finally\n";
    }
}

try { 7 % 0; } catch (DivisionByZeroError $error) {
    echo 'caught modulo: ', $error->getMessage(), "\n";
}
try { 1 << -1; } catch (ArithmeticError $error) {
    echo 'caught shift: ', $error->getMessage(), "\n";
}
try { intdiv(PHP_INT_MIN, -1); } catch (ArithmeticError $error) {
    echo 'caught integer overflow: ', $error->getMessage(), "\n";
}
