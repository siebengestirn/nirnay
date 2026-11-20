//! Real-engine replay of the golden contract. Skips where KDB-X is not
//! installed so the repository's CI stays engine-independent.

use kdb_lab::{
    analyze, //
    find_q,
    golden_dir,
};

#[test]
fn fresh_q_replay_is_byte_identical_and_digests_are_stable() {
    let Ok(q_bin) = find_q() else {
        eprintln!("KDB-X not installed; skipping real-engine golden replay");
        return;
    };
    let dir = golden_dir();
    let input = dir.join("mbp-observation-v1.csv");
    let golden = std::fs::read_to_string(dir.join("mbp-metrics-v1.csv"))
        .expect("golden metrics file must exist");
    let first = analyze(&input, "synthetic-source", &q_bin).expect("fresh q analysis must succeed");
    assert_eq!(
        first.stdout, golden,
        "fresh q output must equal the golden bytes"
    );
    let second =
        analyze(&input, "synthetic-source", &q_bin).expect("second q analysis must succeed");
    assert_eq!(
        first.output_sha256, second.output_sha256,
        "output digest must be stable across processes"
    );
    assert_eq!(
        first.input_sha256, second.input_sha256,
        "input digest must be stable across processes"
    );
}
