//! Статические golden/reference проверки без native producer и kernel queries.
use sha2::{Digest, Sha256};

#[test]
fn ynkrole1_exact_static_vector() {
    let binding = crate::worker::role_handoff::Binding {
        nonce: [1; 32],
        digest: [2; 32],
        epoch: 3,
        session: [4; 16],
        generation: 5,
        invocation: [6; 16],
        server_pid: 7,
        server_start: 8,
    };
    assert_eq!(
        binding.record().expect("pure record"),
        *include_bytes!("fixtures/ynkrole1.bin")
    );
}

#[test]
fn abi260_byte_exact_reference_and_existing_vectors() {
    let references = include_str!("fixtures/abi260.sha256");
    let inputs: [&[u8]; 3] = [
        include_bytes!("../../attribution/native_wire.rs"),
        include_bytes!("../../../../../tests/fixtures/native-broker-v1.json"),
        include_bytes!("../../../../../docs/specs/native-broker-bootstrap-v1.md"),
    ];
    for (line, bytes) in references.lines().zip(inputs) {
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            line.split_whitespace().next().expect("digest")
        );
    }
    assert_eq!(references.lines().count(), inputs.len());
    let vectors: serde_json::Value = serde_json::from_slice(inputs[1]).expect("accepted JSON");
    assert_eq!(
        inputs[1],
        include_bytes!("../../../../../tests/isolatedfixture/nativehooks/frozen-vectors.json")
    );
    for row in vectors["vectors"].as_array().expect("vectors") {
        let hex = row["hex"].as_str().expect("hex");
        assert_eq!(hex.len() % 2, 0);
        let bytes: Vec<u8> = hex
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII"), 16).expect("hex byte")
            })
            .collect();
        assert_eq!(
            crate::attribution::native_wire::Header::decode(
                &bytes,
                row["fd_count"].as_u64().expect("FD count") as usize
            )
            .is_ok(),
            row["header_ok"].as_bool().expect("expected result"),
            "{}",
            row["name"]
        );
        if let Some(required) = row["app_required"].as_bool() {
            assert_eq!(
                crate::attribution::native_wire::selected_features(&bytes, required).is_ok(),
                row["features_ok"].as_bool().expect("features result"),
                "{}",
                row["name"]
            );
        }
    }
}
