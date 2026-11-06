use php_compiler::compile_source;

#[test]
fn foreach_by_reference_uses_reference_returning_iterator_current() {
    let source = r#"<?php
        class ReferenceIterator implements Iterator {
            public $position = 0;
            public $values = [3, 5];
            public function rewind() { $this->position = 0; }
            public function valid() { return $this->position < count($this->values); }
            public function &current() { return $this->values[$this->position]; }
            public function key() { return $this->position; }
            public function next() { $this->position++; }
        }
        class ReferenceCollection implements IteratorAggregate {
            public $inner;
            public function __construct($inner) { $this->inner = $inner; }
            public function getIterator() { return $this->inner; }
        }
        $iterator = new ReferenceIterator();
        foreach ($iterator as &$value) { $value *= 10; }
        unset($value);
        echo implode(',', $iterator->values), '|';
        $wrappedIterator = new ReferenceIterator();
        $collection = new ReferenceCollection(new ReferenceCollection($wrappedIterator));
        foreach ($collection as &$value) { $value += 1; }
        unset($value);
        echo implode(',', $wrappedIterator->values);
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "30,50|4,6");
}

#[test]
fn foreach_by_reference_rejects_iterators_that_return_values_not_references() {
    let source = r#"<?php
        class ValueIterator implements Iterator {
            public $position = 0;
            public function rewind() {}
            public function valid() { return $this->position < 1; }
            public function current() { return 1; }
            public function key() { return 0; }
            public function next() { $this->position++; }
        }
        $iterator = new ValueIterator();
        foreach ($iterator as &$value) {}
    "#;
    let error = compile_source(source).unwrap().execute().unwrap_err();
    assert!(error.contains("An iterator cannot be used with foreach by reference"), "{error}");
}

#[test]
fn foreach_by_reference_mutates_values_yielded_by_reference_generators() {
    let source = r#"<?php
        function &referenceGenerator() {
            $values = [2, 4];
            foreach ($values as &$value) { yield $value; }
        }
        $generator = referenceGenerator();
        foreach ($generator as &$value) { $value *= 10; echo $value, ','; }
        unset($value);
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "20,40,");
}

#[test]
fn by_reference_generators_report_notices_for_non_reference_yields() {
    let source = r#"<?php
        function &noticeGenerator() {
            yield 1 + 2;
            yield from [4];
        }
        foreach (noticeGenerator() as $value) { echo $value, ','; }
    "#;
    let result = compile_source(source).unwrap().execute_with_globals(Default::default()).unwrap();
    assert_eq!(result.output, "3,4,");
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result.diagnostics.iter().all(|diagnostic| {
        diagnostic.level == php_runtime::DiagnosticLevel::Notice
            && diagnostic.message == "Only variable references should be yielded by reference"
    }));
    assert_eq!(result.diagnostics.iter().map(|diagnostic| diagnostic.source_line).collect::<Vec<_>>(), vec![Some(3), Some(4)]);
}

#[test]
fn by_reference_generator_scalar_local_yield_mutates_suspended_frame() {
    let source = r#"<?php function &g() { $value=3; yield $value; echo $value; } foreach (g() as &$value) { $value=42; }"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "42");
}

#[test]
fn foreach_by_reference_accepts_temporary_array_expressions() {
    let source = r#"<?php foreach ([1, 2] as &$value) { $value *= 10; echo $value, ','; }"#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "10,20,");
}

#[test]
fn foreach_by_reference_rejects_non_reference_generators() {
    let source = r#"<?php
        function valueGenerator() { $value = 1; yield $value; }
        $generator = valueGenerator();
        foreach ($generator as &$value) {}
    "#;
    let error = compile_source(source).unwrap().execute().unwrap_err();
    assert!(error.contains("non-referenceable generator"), "{error}");
}

#[test]
fn foreach_object_visibility_and_reference_writes_follow_declaring_class_scope() {
    let source = r#"<?php
        class ParentVisibility {
            protected $parentProtected = 'parent';
            private $parentSecret = 'parent-secret';
            public function parentView() {
                $keys = [];
                foreach ($this as $key => $value) { $keys[$key] = true; }
                return isset($keys['parentSecret']) ? 'parent-private' : 'missing-parent-private';
            }
        }
        class ChildVisibility extends ParentVisibility {
            private $childSecret = 'child';
            public function childView() {
                $keys = [];
                foreach ($this as $key => $value) { $keys[$key] = true; }
                return (isset($keys['parentProtected']) ? 'protected' : 'missing-protected') . ':'
                    . (isset($keys['childSecret']) ? 'child-private' : 'missing-child-private') . ':'
                    . (isset($keys['parentSecret']) ? 'leaked-parent-private' : 'hidden-parent-private');
            }
            public function updateVisibleProperties() {
                foreach ($this as &$value) { if ($value === 'child') { $value = 'updated'; } }
                unset($value);
            }
            public function childSecret() { return $this->childSecret; }
        }
        $object = new ChildVisibility();
        echo $object->parentView(), '|', $object->childView(), '|';
        $object->updateVisibleProperties();
        echo $object->childSecret();
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "parent-private|protected:child-private:hidden-parent-private|updated");
}

#[test]
fn by_reference_yield_from_preserves_delegated_generator_references() {
    let source = r#"<?php
        function &innerReferenceGenerator() {
            $value = 3;
            yield $value;
            echo $value;
        }
        function &delegatingReferenceGenerator() {
            yield from innerReferenceGenerator();
        }
        $generator = delegatingReferenceGenerator();
        foreach ($generator as &$value) { $value = 42; }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "42");
}

#[test]
fn by_reference_yield_from_preserves_reference_cells_from_arrays() {
    let source = r#"<?php
        function &arrayReferenceGenerator($values) {
            yield from $values;
        }
        $cell = 3;
        $values = [&$cell];
        $generator = arrayReferenceGenerator($values);
        foreach ($generator as &$value) { $value *= 10; break; }
        unset($value);
        echo $cell;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "30");
}

#[test]
fn by_reference_yield_from_preserves_user_iterator_current_references() {
    let source = r#"<?php
        class YieldFromReferenceIterator implements Iterator {
            public $position = 0;
            public $values = [4, 7];
            public function rewind() { $this->position = 0; }
            public function valid() { return $this->position < count($this->values); }
            public function &current() { return $this->values[$this->position]; }
            public function key() { return $this->position; }
            public function next() { $this->position++; }
        }
        class YieldFromReferenceCollection implements IteratorAggregate {
            public $iterator;
            public function __construct($iterator) { $this->iterator = $iterator; }
            public function getIterator() { return $this->iterator; }
        }
        function &delegateUserIterator($iterator) { yield from $iterator; }
        $iterator = new YieldFromReferenceIterator();
        $collection = new YieldFromReferenceCollection(new YieldFromReferenceCollection($iterator));
        $generator = delegateUserIterator($collection);
        foreach ($generator as &$value) { $value *= 10; }
        unset($value);
        echo implode(',', $iterator->values);
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "40,70");
}

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
fn generator_locals_are_destroyed_on_completion_and_exceptional_exit() {
    let source = r#"<?php
        class GeneratorLocal {
            public $name;
            public function __construct($name) { $this->name = $name; }
            public function __destruct() { echo $this->name, ';'; }
        }
        function completes() { $local = new GeneratorLocal('complete'); yield 'ready'; echo 'resumed;'; }
        function fails() { $local = new GeneratorLocal('exception'); yield 'waiting'; throw new Exception('stop'); }
        $complete = completes();
        echo $complete->current(), ';';
        $complete->next();
        echo 'done;';
        $failure = fails();
        echo $failure->current(), ';';
        try { $failure->next(); } catch (Exception $error) { echo 'caught;'; }
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "ready;resumed;complete;done;waiting;exception;caught;");
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
fn iterator_to_array_preserves_references_from_arrays_and_reference_generators() {
    let source = r#"<?php
        function &yieldShared(&$value) { yield $value; }
        function &duplicateShared(&$first, &$last) { yield 'duplicate' => $first; yield 'duplicate' => $last; }
        $shared = 1;
        $array = ['shared' => &$shared];
        $copy = iterator_to_array($array);
        $copy['shared'] = 2;
        echo $shared, ':';
        $shared = 3;
        $fromGenerator = iterator_to_array(yieldShared($shared));
        $fromGenerator[0] = 4;
        echo $shared, ':', $fromGenerator[0], ':';
        $first = 5; $last = 6;
        $overwritten = iterator_to_array(duplicateShared($first, $last));
        $overwritten['duplicate'] = 7;
        echo $first, ':', $last;
    "#;
    assert_eq!(compile_source(source).unwrap().execute().unwrap(), "2:4:4:5:7");
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
