/ Nirnay KDB-X q unit-test runner (k4unit model).
/ Usage: q mbp-test.q <golden.csv> <empty.csv> <tests.csv>
/ Loads the MBP library, publishes fixture paths to globals GOLD and EMPTY,
/ then runs true/fail assertions from a CSV. Each CSV line is
/   name,expression,expect
/ where expression may contain commas; it is split on the first and last comma.
/ expect is "true" or "false" (value must be or not be 1b). Prints one
/ PASS/FAIL line per case and exits non-zero on any failure. No network,
/ no broker access, no writes.

dir:"/" sv -1_ "/" vs string .z.f;
lib:hsym `$dir,"/mbp.q";
value "\\l ",string lib;

args:.z.x;
if[(count args) < 3; -1 "ERROR,expected <golden.csv> <empty.csv> <tests.csv>"; exit 1];
GOLD:first args; EMPTY:args[1];
lines:read0 hsym `$args[2];

/ Run one assertion; return 1b on pass, 0b on fail
case:{[line]
  if[0=count line;:1b];
  parts:"," vs line;
  nm:parts[0];
  expr:"," sv 1_ -1_ parts;
  expect:last parts;
  result:@[value;expr;0b];
  ok:$[expect~"true";result~1b;expect~"false";not result~1b;0b];
  -1 $[ok;"PASS ";"FAIL "],nm;
  ok};

results:case each lines;
fails:sum not results;
-1 "TOTAL ",string[count lines]," FAILED ",string fails;
if[fails>0;exit 1];
exit 0
