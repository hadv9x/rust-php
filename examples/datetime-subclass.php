<?php

class AppDate extends DateTime
{
    public function __construct($time = 'now')
    {
        parent::__construct($time);
    }

    public function display($format)
    {
        return parent::format($format);
    }

    public function hasNoParseErrors()
    {
        return parent::getLastErrors() === false;
    }
}

$date = new AppDate('@0');
echo $date->display('Y-m-d'), ':', $date->hasNoParseErrors() ? 'clean' : 'errors', "\n";
