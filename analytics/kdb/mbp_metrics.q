/ Nirnay KDB-X MBP analytics CLI v1.
/ Nirnay KDB-X analytics contract
/ Usage: q mbp_metrics.q <input.csv> <source_id>
/ Loads the pure library beside this file, validates arguments, emits the
/ deterministic metrics document. No network, no broker access, no writes.

dir:"/" sv -1_ "/" vs string .z.f;
lib:hsym `$dir,"/mbp.q";
/ \l takes a literal path; the value comes only from this script
value "\\l ",string lib;

args:.z.x;
if[(count args) < 2; -1 "ERROR,expected <input.csv> <source_id>"; exit 1];
.nirnay.mbp.run[first args;args[1]];
exit 0
