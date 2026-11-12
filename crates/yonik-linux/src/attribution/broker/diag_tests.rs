use super::{DiagDump, DumpProgress, diag_error};
use crate::attribution::broker::{Flow, Unavailable};
use rustix::net::ReturnFlags;
fn flow() -> Flow {
    Flow {
        family: 2,
        protocol: 17,
        local_port: 1234,
        remote_port: 443,
        local: [0; 16],
        remote: [0; 16],
        cookie: None,
    }
}
fn message(kind: u16, flags: u16, body: &[u8]) -> Vec<u8> {
    let mut b = vec![0; 16];
    b[..4].copy_from_slice(&((16 + body.len()) as u32).to_ne_bytes());
    b[4..6].copy_from_slice(&kind.to_ne_bytes());
    b[6..8].copy_from_slice(&flags.to_ne_bytes());
    b[8..12].copy_from_slice(&1u32.to_ne_bytes());
    b.extend_from_slice(body);
    b
}
fn matched() -> Vec<u8> {
    let f = flow();
    let mut body = [0; 72];
    body[0] = 2;
    body[4..6].copy_from_slice(&f.local_port.to_be_bytes());
    body[6..8].copy_from_slice(&f.remote_port.to_be_bytes());
    body[44..48].copy_from_slice(&9u32.to_ne_bytes());
    body[68..72].copy_from_slice(&42u32.to_ne_bytes());
    message(20, 0, &body)
}
fn dump() -> DiagDump {
    DiagDump {
        flow: flow(),
        found: None,
        count: 0,
    }
}
#[test]
fn partial_match_never_completes_on_error_or_truncation() {
    for terminal in [
        message(2, 0, &(-105i32).to_ne_bytes()),
        message(3, 0x10, &0u32.to_ne_bytes()),
        message(4, 0, &[]),
    ] {
        let mut d = dump();
        assert!(matches!(
            d.consume(&matched(), ReturnFlags::empty()),
            Ok(DumpProgress::Pending)
        ));
        assert!(d.consume(&terminal, ReturnFlags::empty()).is_err());
    }
    for flags in [ReturnFlags::TRUNC, ReturnFlags::CTRUNC] {
        assert!(matches!(
            dump().consume(&matched(), flags),
            Err(Unavailable::Coverage)
        ));
    }
    assert_eq!(
        diag_error(rustix::io::Errno::NOBUFS),
        Unavailable::ReceiveOverflow
    );
    assert_eq!(diag_error(rustix::io::Errno::ACCESS), Unavailable::Coverage);
}
#[test]
fn empty_complete_unknown_duplicate_ambiguous_and_cap_exceeded() {
    assert!(matches!(
        dump().consume(&message(3, 0, &0u32.to_ne_bytes()), ReturnFlags::empty()),
        Ok(DumpProgress::Complete(None))
    ));
    let mut d = dump();
    d.consume(&matched(), ReturnFlags::empty()).unwrap();
    assert!(matches!(
        d.consume(&matched(), ReturnFlags::empty()),
        Err(Unavailable::AmbiguousSocket)
    ));
    let mut d = dump();
    d.count = 65536;
    assert!(matches!(
        d.consume(&matched(), ReturnFlags::empty()),
        Err(Unavailable::Limit)
    ));
    for bad in [
        message(3, 0, &[]),
        message(3, 0, &1u32.to_ne_bytes()),
        vec![0; 15],
    ] {
        assert!(dump().consume(&bad, ReturnFlags::empty()).is_err());
    }
    let mut trailing = message(3, 0, &0u32.to_ne_bytes());
    trailing.extend_from_slice(&matched());
    assert!(dump().consume(&trailing, ReturnFlags::empty()).is_err());
}
