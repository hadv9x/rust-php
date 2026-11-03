<?php
interface Readable {}
interface Writable {}
class FileHandle implements Readable, Writable {}
class ReadOnlyHandle implements Readable {}

function exchange(Readable&Writable $handle): (Readable&Writable)|ReadOnlyHandle {
    return $handle;
}

$handle = exchange(new FileHandle());
echo get_class($handle), "\n";
