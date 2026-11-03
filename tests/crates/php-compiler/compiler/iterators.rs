use php_compiler::compile_source;

#[test]
fn generators_suspend_lazily_resume_loops_and_run_finally() {
    let source = r#"<?php
        function sequence() {
            echo 'start;';
            yield 10;
            echo 'middle;';
            yield 20;
            echo 'end;';
        }
        function keyedSequence() {
            yield 'named' => 'A';
            yield 4 => 'B';
            yield 'C';
        }
        function protectedSequence() {
            try { yield 'safe'; }
            finally { echo 'finally;'; }
        }
        function nestedSequence() {
            foreach ([3, 4] as $number) { yield $number; }
        }
        function referenceSequence(&$number) {
            yield $number;
            $number = 99;
            yield $number;
        }
        function manualSequence() {
            $sent = yield 1;
            yield $sent;
            return 9;
        }
        function typedSequence(): Generator { yield 2; }
        echo 'created;';
        foreach (sequence() as $key => $value) { echo $key, ':', $value, ';'; }
        foreach (keyedSequence() as $key => $value) { echo $key, ':', $value, ';'; }
        foreach (protectedSequence() as $value) { echo $value, ';'; }
        foreach (nestedSequence() as $value) { echo $value, ';'; }
        $closureGenerator = function () { yield 'closure'; };
        foreach ($closureGenerator() as $value) { echo $value, ';'; }
        $referenceValue = 7;
        foreach (referenceSequence($referenceValue) as $value) { echo $value, ';'; }
        echo $referenceValue, ';';
        $manual = manualSequence();
        echo get_class($manual), ':', class_exists('Generator') ? 'class' : 'missing', ':';
        echo $manual->valid() ? 'valid' : 'invalid', ':', $manual->current(), ':', $manual->key(), ';';
        echo $manual->send(value: 42), ';';
        $manual->next();
        echo $manual->valid() ? 'valid' : 'closed', ':', $manual->getReturn(), ';';
        $primed = manualSequence();
        echo $primed->current(), ':';
        $primed->rewind();
        foreach ($primed as $primedValue) { echo $primedValue, ';'; }
        echo $primed instanceof Generator ? 'instance' : 'not-instance', ':', is_a($primed, 'Generator') ? 'is-a' : 'not-is-a', ':', method_exists($primed, 'send') ? 'method' : 'missing', ';';
        foreach (typedSequence() as $typedValue) { echo $typedValue, ';'; }
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "created;start;0:10;middle;1:20;end;named:A;4:B;5:C;safe;finally;3;4;closure;7;99;99;Generator:class:valid:1:0;42;closed:9;1:1;;instance:is-a:method;2;");
    let rewind_error = compile_source("<?php function values() { yield 1; yield 2; } $g = values(); $g->next(); $g->rewind();")
        .unwrap().execute().unwrap_err();
    assert!(rewind_error.contains("Cannot rewind a generator that was already run"));
}

#[test]
fn generator_throw_enters_suspended_yield_and_routes_through_catches() {
    let source = r#"<?php
        class InjectedFailure extends Exception {}
        function catchesInjectedFailure() {
            try {
                yield 'ready';
                echo 'skipped';
            } catch (InjectedFailure $failure) {
                yield 'caught';
            } finally {
                echo "finally\n";
            }
            yield 'after';
        }
        function letsInjectedFailureEscape() { yield 'waiting'; }
        $generator = catchesInjectedFailure();
        echo $generator->current(), "\n";
        echo $generator->throw(exception: new InjectedFailure()), "\n";
        $generator->next();
        echo $generator->current(), "\n";
        $escaped = letsInjectedFailureEscape();
        try {
            $escaped->throw(new InjectedFailure());
        } catch (InjectedFailure $failure) {
            echo "outer catch\n";
        }
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "ready\ncaught\nfinally\nafter\nouter catch\n");

    let invalid = compile_source("<?php function values() { yield 1; } $invalid = values(); $invalid->throw('not an exception');").unwrap().execute().unwrap_err();
    assert!(invalid.contains("must be an object implementing Throwable"));
    let invalid_object = compile_source("<?php class NotThrowable {} function values() { yield 1; } $invalid = values(); $invalid->throw(new NotThrowable());")
        .unwrap().execute().unwrap_err();
    assert!(invalid_object.contains("does not implement Throwable"));

    let unknown_name = compile_source("<?php function values() { yield 1; } $generator = values(); $generator->send(other: 42);")
        .unwrap().execute().unwrap_err();
    assert!(unknown_name.contains("unknown named parameter '$other'"));
}

#[test]
fn yield_from_delegates_arrays_generators_send_return_and_exceptions() {
    let source = r#"<?php
        class DelegateFailure extends Exception {}
        function child() {
            try {
                yield 'child' => 10;
                $sent = yield 20;
                yield $sent;
                return 'child-return';
            } catch (DelegateFailure $failure) {
                yield 'caught';
                return 'recovered';
            }
        }
        function parentGenerator() {
            $result = yield from child();
            yield $result;
            return $result;
        }
        function arrayParent() {
            try { yield from ['a' => 1, 3 => 4, 5]; }
            catch (DelegateFailure $failure) { yield 99; }
        }
        $parent = parentGenerator();
        echo $parent->current(), ':', $parent->key(), ';';
        $parent->next();
        echo $parent->current(), ';';
        echo $parent->send('sent-value'), ';';
        $parent->next();
        echo $parent->current(), ';';
        $parent->next();
        echo $parent->getReturn(), ';';
        $recovered = parentGenerator();
        $recovered->current();
        echo $recovered->throw(new DelegateFailure()), ';';
        $recovered->next();
        echo $recovered->current(), ';';
        $array = arrayParent();
        foreach ($array as $key => $value) { echo $key, ':', $value, ';'; }
        $interrupted = arrayParent();
        echo $interrupted->current(), ';';
        echo $interrupted->throw(new DelegateFailure()), ';';
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "10:child;20;sent-value;child-return;child-return;caught;recovered;a:1;3:4;4:5;1;99;");

    let invalid = compile_source("<?php function bad() { yield from 5; } foreach (bad() as $value) {} ").unwrap().execute().unwrap_err();
    assert!(invalid.contains("yield from expects an array or Traversable object"));
}

#[test]
fn yield_from_delegates_user_iterators_and_iterator_aggregates() {
    let source = r#"<?php
        class YieldFromIterator implements Iterator {
            private $position = 0;
            public function rewind() { $this->position = 0; }
            public function valid() { return $this->position < 2; }
            public function current() { return $this->position + 10; }
            public function key() { return 'item-' . $this->position; }
            public function next() { $this->position++; }
        }
        class YieldFromAggregate implements IteratorAggregate {
            public function getIterator(): Traversable { return new YieldFromIterator(); }
        }
        function delegateValues($values) { $result = yield from $values; return $result; }
        foreach (delegateValues(new YieldFromIterator()) as $key => $value) { echo $key, ':', $value, ';'; }
        foreach (delegateValues(new YieldFromAggregate()) as $key => $value) { echo $key, ':', $value, ';'; }
        echo 'done';
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "item-0:10;item-1:11;item-0:10;item-1:11;done");
}

#[test]
fn iterator_to_array_copies_arrays_and_consumes_generators_with_key_modes() {
    let source = r#"<?php
        function duplicateKeys() {
            yield 'same' => 'first';
            yield 'same' => 'last';
            yield 5 => 'numbered';
        }
        $preserved = iterator_to_array(duplicateKeys());
        echo count($preserved), ':', $preserved['same'], ':', $preserved[5], "\n";
        $reindexed = iterator_to_array(iterator: duplicateKeys(), preserve_keys: false);
        echo count($reindexed), ':', $reindexed[0], ':', $reindexed[1], ':', $reindexed[2], "\n";
        $array = iterator_to_array(['label' => 'array', 4 => 'value']);
        echo $array['label'], ':', $array[4], ':', function_exists('iterator_to_array'), "\n";
        $primed = duplicateKeys();
        $primed->current();
        $copy = iterator_to_array($primed);
        echo count($copy), ':', $copy['same'], ':', $copy[5], "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "2:last:numbered\n3:first:last:numbered\narray:value:1\n2:last:numbered\n");

    let invalid = compile_source("<?php iterator_to_array(42);").unwrap().execute().unwrap_err();
    assert!(invalid.contains("Traversable|array"));
}

#[test]
fn iterator_count_counts_arrays_and_exhausts_generator_inputs() {
    let source = r#"<?php
        function countedValues() { yield 'first'; yield 'named' => 'second'; yield 'last'; }
        $generator = countedValues();
        echo iterator_count(iterator: $generator), ':', $generator->valid(), "\n";
        echo iterator_count(['a', 'b']), ':', function_exists('iterator_count'), "\n";
        $primed = countedValues();
        $primed->current();
        echo iterator_count($primed), ':', $primed->valid(), "\n";
    "#;
    let output = compile_source(source).unwrap().execute().unwrap();
    assert_eq!(output, "3:\n2:1\n3:\n");

    let invalid = compile_source("<?php iterator_count(42);").unwrap().execute().unwrap_err();
    assert!(invalid.contains("Traversable|array"));
}

#[test]
fn generators_preserve_non_scalar_keys_and_array_conversion_reports_warnings() {
    let source = r#"<?php
        class GeneratorKeyObject {}
        function nonScalarKeys() {
            yield new GeneratorKeyObject() => 1;
            yield ['array'] => 2;
        }
        function relayKeys() { yield from nonScalarKeys(); }
        foreach (nonScalarKeys() as $key => $value) {
            echo is_object($key) ? 'object' : 'array', ':', $value, ';';
        }
        foreach (relayKeys() as $key => $value) {
            echo is_object($key) ? 'object' : 'array', ':', $value, ';';
        }
        echo iterator_count(nonScalarKeys()), ':', count(iterator_to_array(nonScalarKeys())), ':';
        $values = iterator_to_array(nonScalarKeys(), false);
        echo $values[0], ':', $values[1];
    "#;
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "object:1;array:2;object:1;array:2;2:0:1:2");
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result.diagnostics.iter().all(|diagnostic| diagnostic.message == "Illegal offset type"));
}
