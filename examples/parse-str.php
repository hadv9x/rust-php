<?php
$query = [];
parse_str('page=2&filter[status]=open&filter[tags][]=rust&filter[tags][]=php', result: $query);
echo $query['page'], ':', $query['filter']['status'], ':', implode(',', $query['filter']['tags']), "\n";
