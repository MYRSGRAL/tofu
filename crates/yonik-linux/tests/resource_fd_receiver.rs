use receiver::{CanonicalDescriptor, verify};
use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Write};
use yonik_engine::resource_bundle::ResourceKind;
use yonik_linux::resource_fd_receiver as receiver;
fn fixture(bytes: &[u8], sealed: bool, readonly: bool) -> File {
    let mut fd = File::from(
        memfd_create(
            "receiver-test",
            MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
        )
        .unwrap(),
    );
    fd.write_all(bytes).unwrap();
    if sealed {
        fcntl_add_seals(
            &fd,
            SealFlags::WRITE | SealFlags::GROW | SealFlags::SHRINK | SealFlags::SEAL,
        )
        .unwrap();
    }
    if readonly {
        use std::os::fd::AsRawFd;
        File::open(format!("/proc/self/fd/{}", fd.as_raw_fd())).unwrap()
    } else {
        fd
    }
}
fn descriptor(bytes: &[u8]) -> CanonicalDescriptor {
    CanonicalDescriptor {
        digest: Sha256::digest(bytes).into(),
        bytes: bytes.len() as u64,
        records: 1,
        kind: ResourceKind::Cidrs,
        format_version: 1,
    }
}
#[test]
fn closed_shape_and_fd_authority() {
    for bytes in [
        b"10.0.0.0/8\n".as_slice(),
        b"{\"rules\":[]}\n",
        b"regexp:.*\n",
        b"10.0.0.1/8\n",
        b"10.0.0.0/8\nextra\n",
    ] {
        let d = descriptor(bytes);
        let good = bytes == b"10.0.0.0/8\n";
        assert_eq!(
            verify(&[d], &[fixture(bytes, true, true)], 0, &|| false).is_ok(),
            good
        );
    }
    let bytes = b"10.0.0.0/8\n";
    for (sealed, readonly) in [(false, true), (true, false)] {
        assert!(
            verify(
                &[descriptor(bytes)],
                &[fixture(bytes, sealed, readonly)],
                0,
                &|| false
            )
            .is_err()
        );
    }
}
#[test]
fn metadata_budget_order_and_cancel() {
    let bytes = b"10.0.0.0/8\n";
    for field in 0..5 {
        let mut d = descriptor(bytes);
        match field {
            0 => d.digest[0] ^= 1,
            1 => d.bytes += 1,
            2 => d.records += 1,
            3 => d.format_version += 1,
            _ => d.bytes = u64::MAX,
        }
        assert!(verify(&[d], &[fixture(bytes, true, true)], 0, &|| false).is_err());
    }
    assert!(verify(&[descriptor(bytes)], &[], 0, &|| false).is_err());
    assert!(
        verify(
            &[descriptor(bytes)],
            &[fixture(bytes, true, true)],
            64 * 1024 * 1024,
            &|| false
        )
        .is_err()
    );
    assert!(
        verify(
            &[descriptor(bytes)],
            &[fixture(bytes, true, true)],
            0,
            &|| true
        )
        .is_err()
    );
    let other = b"192.168.0.0/16\n";
    assert!(
        verify(
            &[descriptor(bytes), descriptor(other)],
            &[fixture(other, true, true), fixture(bytes, true, true)],
            0,
            &|| false
        )
        .is_err()
    );
}

#[test]
fn rebuild_both_formats_and_preserve_duplicates() {
    use yonik_engine::{
        Engine,
        external_rules::{Format, Limits, compile_records},
        resource_bundle::ExternalRef,
    };
    let bytes = b"10.0.0.0/8\n10.0.0.0/8\n192.168.0.0/16\n";
    for (engine, format, version) in [
        (Engine::Xray, Format::XrayCidrs, 1),
        (Engine::SingBox, Format::SingCidrs, 3),
    ] {
        let mut native = Vec::new();
        compile_records(
            std::io::Cursor::new(bytes),
            &mut native,
            format,
            3,
            Limits::default(),
        )
        .unwrap();
        let expected = ExternalRef {
            engine,
            digest: Sha256::digest(&native).into(),
            bytes: native.len() as u64,
            records: 3,
            kind: ResourceKind::Cidrs,
            format_version: version,
        };
        let mut d = descriptor(bytes);
        d.records = 3;
        let batch = receiver::rebuild(
            vec![fixture(bytes, true, true)],
            &[d.clone()],
            &[expected.clone()],
            &|| false,
        )
        .unwrap();
        assert_eq!(
            batch.artifacts[0].metadata().unwrap().len(),
            native.len() as u64
        );
        let mut wrong = expected;
        wrong.digest[0] ^= 1;
        assert!(
            receiver::rebuild(vec![fixture(bytes, true, true)], &[d], &[wrong], &|| false).is_err()
        );
    }
}

#[test]
fn encrypted_original_source_roundtrip_and_legacy_gate() {
    use std::os::unix::fs::DirBuilderExt;
    use yonik_engine::external_rules::{Format, Limits, compile_records};
    use yonik_linux::{
        rule_set_custody::{RuleSetFormat, RuleSetRef},
        rule_set_store::EncryptedRuleSetStore,
    };
    let path = std::env::temp_dir().join(format!("yonik-canonical-test-{}", std::process::id()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    let source = b"10.0.0.0/8\n10.0.0.0/8\n";
    let mut native = Vec::new();
    compile_records(
        std::io::Cursor::new(source),
        &mut native,
        Format::XrayCidrs,
        2,
        Limits::default(),
    )
    .unwrap();
    let reference = RuleSetRef::new(
        Sha256::digest(&native).into(),
        native.len() as u64,
        2,
        RuleSetFormat::XrayCidrsV1,
    )
    .unwrap();
    {
        let mut store = EncryptedRuleSetStore::open(File::open(&path).unwrap(), [73; 32]).unwrap();
        store.publish(&reference, &native).unwrap();
        store.commit(0, std::slice::from_ref(&reference)).unwrap();
        let lease = store.lease_current().unwrap();
        assert_eq!(
            lease.canonical_sources().err().unwrap().kind(),
            std::io::ErrorKind::Unsupported
        );
        assert!(
            store
                .publish_canonical(
                    &reference,
                    b"10.0.0.0/8\n192.168.0.0/16\n",
                    ResourceKind::Cidrs
                )
                .is_err()
        );
        store
            .publish_canonical(&reference, source, ResourceKind::Cidrs)
            .unwrap();
        store
            .publish_canonical(&reference, source, ResourceKind::Cidrs)
            .unwrap();
        let exported = lease.canonical_sources().unwrap();
        verify(
            &[exported.resources[0].0.clone()],
            &[exported.resources[0].1.try_clone().unwrap()],
            0,
            &|| false,
        )
        .unwrap();
        assert!(store.reservations().unwrap().active_quarantine > native.len() as u64 + 40);
    }
    {
        let mut store = EncryptedRuleSetStore::open(File::open(&path).unwrap(), [73; 32]).unwrap();
        let lease = store.lease_current().unwrap();
        let exported = lease.canonical_sources().unwrap();
        assert_eq!(exported.resources[0].0.records, 2);
        assert_eq!(
            exported.resources[0].0.digest,
            Sha256::digest(source).as_slice()
        );
    }
    for entry in std::fs::read_dir(&path).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    std::fs::remove_dir(path).unwrap();
}

#[test]
fn canonical_150k_bounded_cancel_and_descriptor_cleanup() {
    use std::cell::Cell;
    let mut source = Vec::new();
    for i in 0..150_000u32 {
        writeln!(&mut source, "exact:d{i}.example").unwrap();
    }
    let d = CanonicalDescriptor {
        digest: Sha256::digest(&source).into(),
        bytes: source.len() as u64,
        records: 150_000,
        kind: ResourceKind::Domains,
        format_version: 1,
    };
    let fd = fixture(&source, true, true);
    verify(
        std::slice::from_ref(&d),
        std::slice::from_ref(&fd),
        0,
        &|| false,
    )
    .unwrap();
    let calls = Cell::new(0);
    let error = verify(&[d], &[fd], 0, &|| {
        calls.set(calls.get() + 1);
        calls.get() == 5
    })
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(calls.get(), 5);
    eprintln!(
        "canonical_records=150000 canonical_bytes={} block_bytes=8192 cancel_check=5",
        source.len()
    );
}

#[test]
fn catalog_150k_original_proofs_for_both_engines() {
    use std::os::unix::fs::DirBuilderExt;
    use yonik_engine::Engine;
    use yonik_linux::{
        resource_catalog::{ResourceCatalog, SourceResource},
        rule_set_store::EncryptedRuleSetStore,
    };
    let mut domains = Vec::new();
    let mut cidrs = Vec::new();
    for i in 0..100_000 {
        writeln!(&mut domains, "exact:d{i}.example").unwrap();
    }
    for i in 0..50_000u32 {
        writeln!(&mut cidrs, "10.0.{}.{}/32", (i >> 8) & 255, i & 255).unwrap();
    }
    for engine in [Engine::Xray, Engine::SingBox] {
        let path = std::env::temp_dir().join(format!(
            "yonik-canonical-large-{}-{engine:?}",
            std::process::id()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        {
            let mut store =
                EncryptedRuleSetStore::open(File::open(&path).unwrap(), [41; 32]).unwrap();
            let sources = [
                SourceResource {
                    source: &domains,
                    digest: Sha256::digest(&domains).into(),
                    count: 100_000,
                    kind: ResourceKind::Domains,
                },
                SourceResource {
                    source: &cidrs,
                    digest: Sha256::digest(&cidrs).into(),
                    count: 50_000,
                    kind: ResourceKind::Cidrs,
                },
            ];
            let catalog = ResourceCatalog::build(engine, &sources, &mut store).unwrap();
            store.commit(0, catalog.references()).unwrap();
            use yonik_engine::resource_bundle::VerifiedCatalog;
            let expected_native = catalog.manifest().to_vec();
            drop(catalog);
            let lease = store.lease_current().unwrap();
            let exported = lease.canonical_sources().unwrap();
            assert_eq!(exported.resources.len(), 2);
            let descriptors = exported
                .resources
                .iter()
                .map(|(d, _)| d.clone())
                .collect::<Vec<_>>();
            let files = exported
                .resources
                .iter()
                .map(|(_, f)| f.try_clone().unwrap())
                .collect();
            let rebuilt =
                receiver::rebuild(files, &descriptors, &expected_native, &|| false).unwrap();
            assert_eq!(rebuilt.artifacts.len(), 2);
            assert_eq!(
                exported.resources[0].0.digest,
                Sha256::digest(&domains).as_slice()
            );
            assert_eq!(
                exported.resources[1].0.digest,
                Sha256::digest(&cidrs).as_slice()
            );
            eprintln!(
                "engine={engine:?} canonical_records=150000 source_bytes={} exported_fds=2 disk_bytes={}",
                domains.len() + cidrs.len(),
                store.reservations().unwrap().physical
            );
        }
        for entry in std::fs::read_dir(&path).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(path).unwrap();
    }
}

#[test]
fn semantic_identity_is_separate_from_helper_runtime() {
    use yonik_engine::{
        Engine,
        resource_bundle::{ExternalRef, ResourceBinding, ResourceRule},
    };
    use yonik_linux::bundle_identity::{CompileRequirements, IntentBundle, runtime_bundle_digest};
    let report = yonik_import::import_bytes(
        b"vless://11111111-1111-1111-1111-111111111111@example.org:443?security=tls".to_vec(),
    )
    .unwrap();
    let server = yonik_import::ServerIntent::from_server(&report.servers[0]).unwrap();
    let policy = yonik_engine::policy_wire::SystemPolicy::parse(r#"{"version":1,"default_action":"direct","user_rules":[{"id":1,"enabled":true,"action":"block","domain":[],"domain_suffix":[],"ip_cidr":[],"executable_paths":[]}],"preset_rules":[],"lan":[],"private_networks":[],"private_domains":[]}"#).unwrap();
    let canonical = [descriptor(b"10.0.0.0/8\n")];
    let expected = [ExternalRef {
        engine: Engine::Xray,
        digest: [42; 32],
        bytes: 32,
        records: 1,
        kind: ResourceKind::Cidrs,
        format_version: 1,
    }];
    let binding = [ResourceBinding {
        rule: ResourceRule::User(1),
        resource: 0,
    }];
    let mut intent = IntentBundle {
        server: &server,
        policy: &policy,
        bindings: &binding,
        revision: 1,
        engine: Engine::Xray,
        native_pin: [27; 32],
        requirements: CompileRequirements {
            kill_switch: true,
            system_tun: true,
        },
        canonical: &canonical,
        expected_native: &expected,
    };
    let digest = intent.digest().unwrap();
    assert_eq!(digest, intent.digest().unwrap());
    let first = runtime_bundle_digest(digest, [1; 32], &expected, [3; 32]).unwrap();
    let second = runtime_bundle_digest(digest, [2; 32], &expected, [4; 32]).unwrap();
    assert_ne!(first, second);
    assert_ne!(first, digest);
    intent.revision = 2;
    assert_ne!(digest, intent.digest().unwrap());
    intent.revision = 1;
    intent.native_pin[0] ^= 1;
    assert_ne!(digest, intent.digest().unwrap());
}
