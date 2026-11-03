<?php
interface Cacheable {}
class Resource {}
class UserResource extends Resource implements Cacheable {}
class NamedResource extends UserResource {
    public function classNames() { return get_class() . ':' . get_parent_class(); }
}

$resource = new NamedResource();
$parentClass = 'Resource';
echo ($resource instanceof UserResource), ':', ($resource instanceof Resource), ':', ($resource instanceof Cacheable), ':', ($resource instanceof $parentClass), "\n";
echo get_class($resource), ':', get_parent_class($resource), ':', $resource->classNames(), "\n";
echo is_a($resource, 'Resource'), ':', is_a('NamedResource', 'Resource', true), "\n";
echo is_subclass_of($resource, 'UserResource'), ':', is_subclass_of($resource, 'Resource'), "\n";
