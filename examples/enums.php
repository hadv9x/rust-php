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
?>
