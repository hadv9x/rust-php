<?php
namespace Demo\Tools;
echo __NAMESPACE__, "\n";

use Demo\Tools\Printer as DisplayPrinter;
use function Demo\Tools\label as DisplayPrinter;
use Demo\Tools\{Printer as GroupPrinter};
use function Demo\Tools\{label as groupLabel};

// Unqualified declarations and references resolve inside the active namespace.
function label($value) {
    return "tool:" . $value;
}

class Printer {
    public function printValue(string $value): string {
        return DisplayPrinter($value);
    }
}

// Unqualified builtin calls fall back to the global builtin namespace.
$printer = new DisplayPrinter();
$grouped = new GroupPrinter();
echo $printer->printValue("ready"), ":", $grouped->printValue(groupLabel("grouped")), ":", count([1, 2, 3]), "\n";

namespace Demo\Extra;
echo __NAMESPACE__, "\n";
use Demo\Tools\{Printer as ExtraPrinter};
$extra = new ExtraPrinter();
echo $extra->printValue("second namespace"), "\n";
