<?php
enum Direction { case North; case South; }
enum Publication: string { case Draft = 'draft'; case Live = 'live'; }
enum HttpCode: int { case Ok = 200; case NotFound = 404; }

$directions = Direction::cases();
echo $directions[0]->name, ':', $directions[1]->name, ':', count($directions), "\n";
echo Publication::Draft->name, ':', Publication::Draft->value, ':', count(Publication::cases()), "\n";
echo (Publication::from('live') === Publication::Live), ':', (Publication::tryFrom('missing') === null), "\n";
echo (Direction::North === Direction::North), ':', get_class(Direction::North), "\n";
echo (HttpCode::from('200') === HttpCode::Ok), ':', (HttpCode::tryFrom(500) === null), "\n";
$httpReflection = new ReflectionEnum('HttpCode');
$reflectedCases = $httpReflection->getCases();
echo $httpReflection->isBacked(), ':', $reflectedCases[1]->getName(), ':', $reflectedCases[1]->getBackingValue(), ':', $httpReflection->getBackingType()->getName(), "\n";
echo $httpReflection->getShortName(), ':', $httpReflection->getNamespaceName(), ':', $httpReflection->inNamespace(), "\n";
echo $httpReflection->hasCase(name: 'Ok'), ':', $httpReflection->hasCase('Missing'), ':', $httpReflection->getCase('NotFound')->getBackingValue(), "\n";
echo $httpReflection->hasConstant('Ok'), ':', $httpReflection->getConstant(name: 'NotFound') === HttpCode::NotFound, "\n";
echo implode(',', array_keys($httpReflection->getConstants())), "\n";
class EnumCaseName implements Stringable { public function __toString(): string { return 'Ok'; } }
echo $httpReflection->hasCase(new EnumCaseName()), ':', $httpReflection->getCase(new EnumCaseName())->getName(), "\n";
try { $httpReflection->getCase([]); } catch (TypeError $error) { echo get_class($error), "\n"; }

#[Attribute(Attribute::TARGET_CLASS_CONSTANT | Attribute::IS_REPEATABLE)]
class EnumCaseLabel { public function __construct(public string $label) {} }
enum Color { #[EnumCaseLabel('rouge'), EnumCaseLabel(label: 'red')] case Red; }
$reflectedColor = (new ReflectionEnum('Color'))->getCase('Red');
$labels = $reflectedColor->getAttributes(EnumCaseLabel::class);
echo $labels[0]->newInstance()->label, ':', $labels[1]->getArguments()['label'], ':', $labels[0]->getTarget(), "\n";
echo $reflectedColor->getEnum()->getName(), ':', $reflectedColor->getDeclaringClass()->getName(), ':', $reflectedColor->getModifiers(), ':', $reflectedColor->isEnumCase(), "\n";
?>
