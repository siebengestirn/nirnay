/ Nirnay KDB-X MBP analytics library v1.
/ Pure analytics: no network, no writes, no process state beyond the input.
/ One book per call; sources are never merged or summed. All price and spread
/ metrics use the input unit (rupees as exported by upstox-decode-export).
/ Public API: observations, metric, run, NAMES.
/ Style: engineering/q-kdb-style.md.

\d .nirnay.mbp

/ Column types in header order (external CSV contract)
TYPES:"JJFJFJFFJ";

/ Ordered metric names; mirrored by the Rust and schema contracts
NAMES:(`schema_version`source_id`rows`duration_s`gap_count),
      (`spread_mean`spread_median`spread_max`imbalance_mean),
      (`imbalance_min`imbalance_max`levels_min`bid1_mean),
      (`ask1_mean`bid1_max_qty`ask1_max_qty);

/ Load a header-prefixed observation CSV into a typed table
observations:{[p]
  t:(TYPES;enlist",") 0:hsym `$p;
  if[0=count t; 'empty_observation_set];
  t};

/ Render a scalar as q-printed text; strings pass through unchanged
str:{[v]
  $[10h=type v;v;string v]};

/ Emit one metric,value line and discard the result
say:{[k;v]
  -1 string[k],",",str[v];};

/ Compute one metric by name from table t and source s
/ Inside a function, column access uses t[`col]: t.col is parsed as a dotted
/ global name. deltas keeps the first seq; drop it so a capture starting above
/ 1 is not falsely reported as a gap.
metric:{[t;s;k]
  $[k=`schema_version;1;
    k=`source_id;s;
    k=`rows;count t;
    k=`duration_s;(last t[`recv_ns] - first t[`recv_ns])%1000000000;
    k=`gap_count;count where 1<>1_(deltas t[`seq]);
    k=`spread_mean;avg t[`spread];
    k=`spread_median;med t[`spread];
    k=`spread_max;max t[`spread];
    k=`imbalance_mean;avg t[`imbalance];
    k=`imbalance_min;min t[`imbalance];
    k=`imbalance_max;max t[`imbalance];
    k=`levels_min;min t[`levels];
    k=`bid1_mean;avg t[`bid1_p];
    k=`ask1_mean;avg t[`ask1_p];
    k=`bid1_max_qty;max t[`bid1_q];
    k=`ask1_max_qty;max t[`ask1_q];
    'unknown_metric]};

/ Emit the complete deterministic metrics document for one source
run:{[p;s]
  t:observations p;
  -1 "metric,value";
  n:count NAMES; j:0;
  while[j<n;
    k:NAMES j;
    say[k;metric[t;s;k]];
    j:j+1];
  };

\d .
