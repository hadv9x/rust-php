<?php
class ConnectionFailure extends Exception {}
class StorageFailure extends Exception {}
class TimeoutFailure extends Exception {}

try {
    throw new StorageFailure();
} catch (ConnectionFailure $error) {
    echo "handled connection failure\n";
} catch (StorageFailure|TimeoutFailure $error) {
    echo "handled by later union catch\n";
} finally {
    echo "cleanup\n";
}

try {
    throw new ConnectionFailure();
} catch (ConnectionFailure) {
    echo "caught without binding\n";
}
