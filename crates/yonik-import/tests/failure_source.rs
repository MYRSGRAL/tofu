use yonik_import::{MAX_INPUT_BYTES, import_bytes, import_reader};

#[test]
fn fatal_errors_keep_original_bytes_without_debug_leakage() {
    for source in [vec![0xff, 0x00], b"{CANARY_BROKEN_JSON".to_vec()] {
        let failure = import_bytes(source.clone())
            .err()
            .unwrap_or_else(|| panic!("Ожидался отказ"));
        assert_eq!(failure.input(), source);
        assert!(failure.input_is_complete());
        assert!(!format!("{failure:?} {failure}").contains("CANARY"));
    }
}

#[test]
fn oversized_stream_retains_only_bounded_prefix_and_marks_it_incomplete() {
    let failure = import_reader(std::io::repeat(b'a'))
        .err()
        .unwrap_or_else(|| panic!("Ожидался отказ"));
    assert_eq!(failure.input().len(), MAX_INPUT_BYTES + 1);
    assert!(!failure.input_is_complete());
}
