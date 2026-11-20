/ Nirnay KDB-X analytics v1: per-source market-by-price book metrics.
/ Usage: q mbp_metrics.q <input.csv> <source_id>
/ Input schema (comma-separated, header row required):
/   seq,recv_ns,bid1_p,bid1_q,ask1_p,ask1_q,spread,imbalance,levels
/   seq:J, recv_ns:J, bid1_p:F, bid1_q:J, ask1_p:F, ask1_q:J,
/   spread:F, imbalance:F, levels:J
/ Output: deterministic `metric,value` lines on stdout. No network, no state
/ outside the input file. One book per source; sources are never combined.
/ All price and spread metrics use the input price unit (rupees as exported by
/ upstox-decode-export); no unit suffix is implied by a metric name.

args:.z.x;
if[(count args) < 2; -1 "ERROR,expected <input.csv> <source_id>"; exit 1];
path:first args;
source:args[1];

t:("JJFJFJFFJ";enlist",") 0: hsym `$path;

tostr:{[v] $[10h=type v; v; string v]};
emit:{[k;v] -1 string[k],",",tostr[v];};

-1 "metric,value";
emit[`schema_version;1];
emit[`source_id;source];
emit[`rows;count t];
emit[`duration_s;(last t.recv_ns - first t.recv_ns)%1000000000];
/ deltas keeps the first sequence value unchanged; drop it before counting so
/ a capture whose first seq is not 1 is not reported as a gap.
emit[`gap_count;count where 1<>1_(deltas t.seq)];
emit[`spread_mean;avg t.spread];
emit[`spread_median;med t.spread];
emit[`spread_max;max t.spread];
emit[`imbalance_mean;avg t.imbalance];
emit[`imbalance_min;min t.imbalance];
emit[`imbalance_max;max t.imbalance];
emit[`levels_min;min t.levels];
emit[`bid1_mean;avg t.bid1_p];
emit[`ask1_mean;avg t.ask1_p];
emit[`bid1_max_qty;max t.bid1_q];
emit[`ask1_max_qty;max t.ask1_q];
exit 0
