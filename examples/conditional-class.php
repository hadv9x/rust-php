<?php
if (true) {
    class ConditionalService {
        public static function register(): string
        {
            return 'registered';
        }
    }
}

echo ConditionalService::register(), "\n";
