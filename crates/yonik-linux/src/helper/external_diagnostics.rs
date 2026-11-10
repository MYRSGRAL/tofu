//! Только synthetic VM после guestguard. Никогда не выводит opaque bytes/имена.
use crate::{
    LinuxTopology, NetworkAttribute,
    managed_network::{Journal, tun::TunError},
    network::TopologyCapture,
};
use std::{
    fmt::Arguments,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::time::Instant;
const LIMIT: usize = 32 * 1024;
const RECORDS: usize = 16;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static LEFT: AtomicUsize = AtomicUsize::new(LIMIT);
pub(in crate::helper) struct Scope;
impl Scope {
    // Единственный caller — guest_guard после marker/DMI/systemd/cgroup/flock.
    pub(in crate::helper) fn after_guest_guard() -> Self {
        LEFT.store(LIMIT, Ordering::SeqCst);
        ACTIVE.store(true, Ordering::SeqCst);
        Self
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::SeqCst);
    }
}
fn requested() -> bool {
    std::env::var_os("YONIK_SESSION_DIAGNOSTICS").as_deref() == Some(std::ffi::OsStr::new("1"))
}
fn enabled() -> bool {
    ACTIVE.load(Ordering::SeqCst) && requested()
}
pub(super) fn snapshot(value: &LinuxTopology) -> Option<LinuxTopology> {
    (enabled() && LEFT.load(Ordering::SeqCst) > 0).then(|| value.clone())
}
fn emit(line: &str) {
    if LEFT
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
            left.checked_sub(line.len() + 1)
        })
        .is_ok()
    {
        eprintln!("{line}");
    }
}
pub(super) fn stage(stage: &str, error: TunError) {
    if requested() {
        emit(&format!(
            "external diagnostic failed_stage={stage} error={error:?}"
        ));
    }
}
struct Sink {
    lines: Vec<String>,
    left: usize,
    truncated: bool,
}
impl Sink {
    fn new(limit: usize) -> Self {
        Self {
            lines: vec![],
            left: limit,
            truncated: false,
        }
    }
    fn line(&mut self, args: Arguments<'_>) {
        let line = args.to_string();
        if line.len() < self.left {
            self.left -= line.len() + 1;
            self.lines.push(line);
        } else {
            self.truncated = true;
        }
    }
}
macro_rules! line { ($s:expr,$($arg:tt)*)=>{$s.line(format_args!($($arg)*))}; }
macro_rules! fields {
    ($s:expr,$class:expr,$a:expr,$b:expr;$($field:ident),+) => {
        if let (Some(a),Some(b))=($a,$b) {
            let mut names=Vec::new(); $(if a.$field!=b.$field {names.push(stringify!($field));})+
            line!($s,"{} changed_fields={:?}",$class,names);
        }
    };
}
fn nanos(value: Instant, origin: Instant) -> i128 {
    if value >= origin {
        (value - origin).as_nanos() as i128
    } else {
        -((origin - value).as_nanos() as i128)
    }
}
fn offsets(a: &[u8], b: &[u8]) -> Vec<usize> {
    a.iter()
        .zip(b)
        .enumerate()
        .filter_map(|(i, (a, b))| (a != b).then_some(i))
        .take(16)
        .collect()
}
fn nla(mut data: &[u8]) -> Option<std::collections::BTreeMap<u16, &[u8]>> {
    let mut result = std::collections::BTreeMap::new();
    while !data.is_empty() {
        if data.len() < 4 || result.len() >= 128 {
            return None;
        }
        let len = usize::from(u16::from_ne_bytes([data[0], data[1]]));
        let kind = u16::from_ne_bytes([data[2], data[3]]);
        let end = (len + 3) & !3;
        if len < 4 || end > data.len() || result.insert(kind, &data[4..len]).is_some() {
            return None;
        }
        data = &data[end..];
    }
    Some(result)
}
fn words(data: &[u8]) -> Vec<u32> {
    data.chunks_exact(4)
        .take(8)
        .map(|v| u32::from_ne_bytes([v[0], v[1], v[2], v[3]]))
        .collect()
}
fn leaf(s: &mut Sink, class: &str, path: &[u16], a: &[u8], b: &[u8], tun: bool) {
    line!(
        s,
        "{class} attr_path={path:?} len={}=>{} changed_offsets={:?}",
        a.len(),
        b.len(),
        offsets(a, b)
    );
    if (class == "address" && path == [6] && a.len() == 16 && b.len() == 16)
        || (class == "route" && path == [12] && a.len() == 32 && b.len() == 32)
        || (class == "route" && path == [23] && a.len() == 4 && b.len() == 4)
        || (class == "link" && path == [26, 10, 5] && a.len() == 16 && b.len() == 16)
        || (class == "link" && path == [26, 10, 1] && a.len() == 4 && b.len() == 4)
    {
        line!(
            s,
            "{class} attr_path={path:?} numeric_words={:?}=>{:?}",
            words(a),
            words(b)
        );
        if class == "route" && path == [12] {
            line!(
                s,
                "route signed_cache_expires={}=>{} units=USER_HZ",
                i32::from_ne_bytes([a[8], a[9], a[10], a[11]]),
                i32::from_ne_bytes([b[8], b[9], b[10], b[11]])
            );
        }
    }
    if class == "link" && path == [26, 10, 8] && a.len() == 1 && b.len() == 1 {
        line!(s, "addr_gen_mode={}=>{}", a[0], b[0]);
    }
    if class == "link" && path == [26, 10, 2] && a.len() >= 192 && b.len() >= 192 {
        line!(
            s,
            "devconf_addr_gen_mode={:?}=>{:?}",
            words(&a[188..192]),
            words(&b[188..192])
        );
    }
    if class == "link"
        && tun
        && path.len() == 3
        && path[..2] == [18, 2]
        && (1..=9).contains(&path[2])
    {
        if a.len() == 1 && b.len() == 1 {
            line!(s, "tun attr_path={path:?} numeric={}=>{}", a[0], b[0]);
        }
        if a.len() == 4 && b.len() == 4 {
            line!(
                s,
                "tun attr_path={path:?} numeric={:?}=>{:?}",
                words(a),
                words(b)
            );
        }
    }
    if class == "link"
        && path.len() < 3
        && matches!(path, [26] | [26, 10] | [26, 2] | [26, 45] | [18] | [18, 2])
    {
        if let (Some(left), Some(right)) = (nla(a), nla(b)) {
            for key in left
                .keys()
                .chain(right.keys())
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .take(16)
            {
                let x = left.get(&key).copied().unwrap_or(&[]);
                let y = right.get(&key).copied().unwrap_or(&[]);
                if x != y {
                    let mut nested = path.to_vec();
                    nested.push(key);
                    leaf(s, class, &nested, x, y, tun);
                }
            }
        } else {
            line!(s, "{class} attr_path={path:?} nla_shape=invalid");
        }
    }
}
fn attrs(s: &mut Sink, class: &str, a: &[NetworkAttribute], b: &[NetworkAttribute], tun: bool) {
    let a: std::collections::BTreeMap<_, _> =
        a.iter().map(|v| (v.kind, v.value.as_slice())).collect();
    let b: std::collections::BTreeMap<_, _> =
        b.iter().map(|v| (v.kind, v.value.as_slice())).collect();
    for kind in a
        .keys()
        .chain(b.keys())
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|kind| {
            a.get(kind) != b.get(kind)
                || (class == "address" && *kind == 6)
                || (class == "route" && matches!(*kind, 12 | 23))
        })
        .take(16)
    {
        leaf(
            s,
            class,
            &[kind],
            a.get(&kind).copied().unwrap_or(&[]),
            b.get(&kind).copied().unwrap_or(&[]),
            tun,
        );
    }
}
fn dump(s: &mut Sink, label: &str, a: &LinuxTopology, b: &LinuxTopology) {
    line!(
        s,
        "topology roles=baseline/{label} namespace_equal={} counts link={}/{} address={}/{} route={}/{} rule={}/{} nexthop={}/{} shown_per_class={RECORDS}",
        a.namespace == b.namespace,
        a.links.len(),
        b.links.len(),
        a.addresses.len(),
        b.addresses.len(),
        a.routes.len(),
        b.routes.len(),
        a.rules.len(),
        b.rules.len(),
        a.nexthops.len(),
        b.nexthops.len()
    );
    for i in 0..a.links.len().max(b.links.len()).min(RECORDS) {
        let x = a.links.get(i);
        let y = b.links.get(i);
        fields!(s,"link",x,y;index,link_type,flags,name,alias,mtu,master,parent,kind,operational_state,unknown);
        for (role, l) in [("baseline", x), (label, y)] {
            if let Some(l) = l {
                line!(
                    s,
                    "{role} link#{i} index={} type={} flags={:#x} mtu={:?} master={:?} parent={:?} operstate={:?} name_len={} alias_len={:?}",
                    l.index,
                    l.link_type,
                    l.flags,
                    l.mtu,
                    l.master,
                    l.parent,
                    l.operational_state,
                    l.name.len(),
                    l.alias.as_ref().map(Vec::len)
                );
            }
        }
        attrs(
            s,
            "link",
            x.map_or(&[], |v| v.unknown.as_slice()),
            y.map_or(&[], |v| v.unknown.as_slice()),
            y.or(x).is_some_and(|v| v.kind.as_deref() == Some(b"tun")),
        );
    }
    for i in 0..a.addresses.len().max(b.addresses.len()).min(RECORDS) {
        let x = a.addresses.get(i);
        let y = b.addresses.get(i);
        fields!(s,"address",x,y;family,index,address,local,prefix_len,scope,flags,unknown);
        for (role, v) in [("baseline", x), (label, y)] {
            if let Some(v) = v {
                line!(
                    s,
                    "{role} address#{i} family={} index={} address={}/{} local={:?} scope={} flags={:#x}",
                    v.family,
                    v.index,
                    v.address,
                    v.prefix_len,
                    v.local,
                    v.scope,
                    v.flags
                );
            }
        }
        attrs(
            s,
            "address",
            x.map_or(&[], |v| v.unknown.as_slice()),
            y.map_or(&[], |v| v.unknown.as_slice()),
            false,
        );
    }
    for i in 0..a.routes.len().max(b.routes.len()).min(RECORDS) {
        let x = a.routes.get(i);
        let y = b.routes.get(i);
        fields!(s,"route",x,y;family,destination,source,table,preferred_source,gateway,output_interface,input_interface,priority,protocol,kind,scope,tos,flags,multipath,nexthop_id,unknown);
        for (role, v) in [("baseline", x), (label, y)] {
            if let Some(v) = v {
                line!(
                    s,
                    "{role} route#{i} family={} dst={}/{} src={}/{} table={} gateway={:?} prefsrc={:?} oif={:?} iif={:?} metric={:?} protocol={} kind={} scope={} tos={} flags={:#x} nh={:?} multipath_count={}",
                    v.family,
                    v.destination.address,
                    v.destination.length,
                    v.source.address,
                    v.source.length,
                    v.table,
                    v.gateway,
                    v.preferred_source,
                    v.output_interface,
                    v.input_interface,
                    v.priority,
                    v.protocol,
                    v.kind,
                    v.scope,
                    v.tos,
                    v.flags,
                    v.nexthop_id,
                    v.multipath.len()
                );
            }
        }
        attrs(
            s,
            "route",
            x.map_or(&[], |v| v.unknown.as_slice()),
            y.map_or(&[], |v| v.unknown.as_slice()),
            false,
        );
    }
    for i in 0..a.rules.len().max(b.rules.len()).min(RECORDS) {
        let x = a.rules.get(i);
        let y = b.rules.get(i);
        fields!(s,"rule",x,y;family,destination,source,tos,table,action,flags,priority,goto,fwmark,fwmask,uid_range,input_interface,output_interface,protocol,ip_protocol,source_ports,destination_ports,suppress_prefix_len,suppress_interface_group,l3mdev,unknown);
        for (role, v) in [("baseline", x), (label, y)] {
            if let Some(v) = v {
                line!(
                    s,
                    "{role} rule#{i} family={} table={} priority={:?} action={} mark={:?} mask={:?} flags={:#x} goto={:?} uid={:?} protocol={:?}/{:?} suppress={:?}/{:?} l3mdev={:?}",
                    v.family,
                    v.table,
                    v.priority,
                    v.action,
                    v.fwmark,
                    v.fwmask,
                    v.flags,
                    v.goto,
                    v.uid_range,
                    v.protocol,
                    v.ip_protocol,
                    v.suppress_prefix_len,
                    v.suppress_interface_group,
                    v.l3mdev
                );
            }
        }
        attrs(
            s,
            "rule",
            x.map_or(&[], |v| v.unknown.as_slice()),
            y.map_or(&[], |v| v.unknown.as_slice()),
            false,
        );
    }
    for i in 0..a.nexthops.len().max(b.nexthops.len()).min(RECORDS) {
        let x = a.nexthops.get(i);
        let y = b.nexthops.get(i);
        fields!(s,"nexthop",x,y;id,family,scope,protocol,flags,blackhole,output_interface,gateway,group_type,group,unknown);
        for (role, v) in [("baseline", x), (label, y)] {
            if let Some(v) = v {
                line!(
                    s,
                    "{role} nexthop#{i} id={} family={} oif={:?} gateway={:?} flags={:#x} group_type={:?} group_count={}",
                    v.id,
                    v.family,
                    v.output_interface,
                    v.gateway,
                    v.flags,
                    v.group_type,
                    v.group.len()
                );
            }
        }
        attrs(
            s,
            "nexthop",
            x.map_or(&[], |v| v.unknown.as_slice()),
            y.map_or(&[], |v| v.unknown.as_slice()),
            false,
        );
    }
}
pub(super) fn failure(
    stage_name: &str,
    error: TunError,
    before: (&LinuxTopology, TopologyCapture),
    after: (&LinuxTopology, TopologyCapture),
    journal: &Journal,
    raw: Option<&LinuxTopology>,
    expected: (&super::SessionNetworkPlan, u32),
) {
    let (baseline, bt) = before;
    let (target, tt) = after;
    stage(stage_name, error);
    if !enabled() || LEFT.load(Ordering::SeqCst) == 0 {
        return;
    }
    let classes = crate::network::diagnostic_class_matches(baseline, bt, target, tt);
    let now = Instant::now();
    let origin = bt.family(2).start;
    let mut sink = Sink::new(LIMIT.saturating_sub(128));
    line!(
        sink,
        "external synthetic VM failed_stage={stage_name} comparison_recomputed={classes:?} diagnostic_now_ns={} byte_limit={LIMIT}",
        nanos(now, origin)
    );
    line!(
        sink,
        "raw_equal link={} address={} route={} rule={} nexthop={}",
        baseline.links == target.links,
        baseline.addresses == target.addresses,
        baseline.routes == target.routes,
        baseline.rules == target.rules,
        baseline.nexthops == target.nexthops
    );
    for family in [2, 10] {
        for (label, t) in [("baseline", bt), ("current/projected", tt)] {
            let a = t.family(family);
            let r = t.route(family);
            line!(
                sink,
                "{label} family={family} address_capture_ns={}..{} route_capture_ns={}..{} clock_ticks={:?}",
                nanos(a.start, origin),
                nanos(a.end, origin),
                nanos(r.window.start, origin),
                nanos(r.window.end, origin),
                r.clock_ticks
            );
        }
    }
    let (plan, index) = expected;
    let resources = plan.resources(index);
    line!(
        sink,
        "records expected_session_count={} ownership_match={} expected_vnet_hdr={}",
        resources.len(),
        journal.ownership() == plan.owner,
        plan.vnet_hdr
    );
    for record in journal.records().iter().take(16) {
        line!(
            sink,
            "record id={} scope={:?} state={:?} needs_reconciliation={} expected_session_resource_match={:?}",
            record.id(),
            record.scope(),
            record.state(),
            journal.needs_reconciliation(record.id()),
            (record.scope() == crate::managed_network::Scope::Session).then(|| resources
                .iter()
                .any(|(id, value)| *id == record.id() && value == record.resource()))
        );
    }
    dump(
        &mut sink,
        if stage_name == "equivalence" {
            "projected"
        } else {
            "current_unprojected"
        },
        baseline,
        target,
    );
    if let Some(raw) = raw {
        dump(&mut sink, "current_unprojected", baseline, raw);
    }
    let truncated = sink.truncated;
    for line in sink.lines {
        if !enabled() {
            break;
        }
        emit(&line);
    }
    if truncated && enabled() {
        emit("external diagnostic truncated=true");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_output_is_bounded_and_opaque_strings_are_not_rendered() {
        let mut sink = Sink::new(128);
        for _ in 0..100 {
            line!(sink, "only bounded numeric metadata {}", 123);
        }
        assert!(sink.lines.iter().map(|v| v.len() + 1).sum::<usize>() <= 128);
        assert!(sink.truncated);
        let secret = b"credentials-or-hostname-must-not-appear";
        let mut sink = Sink::new(LIMIT);
        leaf(&mut sink, "link", &[3], secret, b"another-secret", false);
        leaf(&mut sink, "link", &[999], secret, secret, false);
        let topology = LinuxTopology {
            namespace: crate::NetworkNamespace {
                device: 1,
                inode: 2,
            },
            links: vec![crate::LinuxLink {
                index: 1,
                link_type: 1,
                flags: 0,
                name: secret.to_vec(),
                alias: Some(secret.to_vec()),
                mtu: Some(1500),
                master: None,
                parent: None,
                kind: None,
                operational_state: None,
                unknown: vec![NetworkAttribute {
                    kind: 999,
                    value: secret.to_vec(),
                }],
            }],
            addresses: vec![],
            routes: vec![],
            rules: vec![],
            nexthops: vec![],
        };
        dump(&mut sink, "synthetic", &topology, &topology);
        let output = sink.lines.join("\n");
        assert!(!output.contains("credentials"));
        assert!(!output.contains("another-secret"));
        assert!(output.contains("changed_offsets"));
    }
    #[test]
    fn nla_decoder_rejects_malformed_and_duplicate_fields() {
        assert!(nla(&[1, 0, 2, 0]).is_none());
        assert!(nla(&[4, 0, 1, 0, 4, 0, 1, 0]).is_none());
        assert!(nla(&[5, 0, 1, 0, 0]).is_none());
    }
}
