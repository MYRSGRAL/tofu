//! Явная read-only инспекция сети: один ограниченный JSON-запрос через stdin.

#[cfg(target_os = "linux")]
mod linux {
    use serde::Deserialize;
    use serde_json::{Value, json};
    use std::{
        io::{self, Read, Write},
        net::IpAddr,
        num::NonZeroU32,
        time::Duration,
    };
    use tofu_linux::{Cancellation, NetworkReadError, RouteQuery, TransportProtocol};

    const INPUT_LIMIT: usize = 16 * 1024;
    const OUTPUT_LIMIT: usize = 16 * 1024 * 1024;

    fn timeout_default() -> u64 {
        5000
    }
    #[derive(Deserialize)]
    #[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
    enum Request {
        Snapshot {
            #[serde(default = "timeout_default")]
            timeout_ms: u64,
        },
        Lookup {
            destination: IpAddr,
            source: Option<IpAddr>,
            uid: u32,
            #[serde(default)]
            mark: u32,
            #[serde(default)]
            tos: u8,
            input_interface: Option<NonZeroU32>,
            output_interface: Option<NonZeroU32>,
            transport: Option<Protocol>,
            source_port: Option<u16>,
            destination_port: Option<u16>,
            #[serde(default = "timeout_default")]
            timeout_ms: u64,
        },
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Protocol {
        Tcp,
        Udp,
    }

    fn prefix(value: &tofu_linux::IpPrefix) -> String {
        format!("{}/{}", value.address, value.length)
    }
    fn attributes(values: &[tofu_linux::NetworkAttribute]) -> Value {
        json!(
            values
                .iter()
                .map(|v| json!({"kind":v.kind,"bytes":v.value.len()}))
                .collect::<Vec<_>>()
        )
    }
    fn namespace(value: tofu_linux::NetworkNamespace) -> Value {
        json!({"device":value.device,"inode":value.inode})
    }
    fn error(value: NetworkReadError) -> Value {
        json!({"ok":false,"error":format!("{value:?}"),"errno":match value {NetworkReadError::Kernel(n)=>Some(n),_=>None}})
    }
    async fn inspect(request: Request, cancel: &mut Cancellation) -> Value {
        match request {
            Request::Snapshot { timeout_ms } => {
                match tofu_linux::read_topology(cancel, Duration::from_millis(timeout_ms)).await {
                    Err(e) => error(e),
                    Ok(v) => json!({
                        "ok":true,"namespace":namespace(v.namespace),"incomplete":v.incomplete(),"unsupported_count":v.unsupported_count(),
                        "links":v.links.iter().map(|l| json!({"index":l.index,"name":String::from_utf8_lossy(&l.name),"name_bytes":l.name,"mtu":l.mtu,"flags":l.flags,"master":l.master,"parent":l.parent,"kind":l.kind.as_ref().map(|k|String::from_utf8_lossy(k)),"unknown":attributes(&l.unknown)})).collect::<Vec<_>>(),
                        "addresses":v.addresses.iter().map(|a|json!({"index":a.index,"family":a.family,"address":a.address,"local":a.local,"prefix_len":a.prefix_len,"unknown":attributes(&a.unknown)})).collect::<Vec<_>>(),
                        "routes":v.routes.iter().map(|r|json!({"family":r.family,"destination":prefix(&r.destination),"source":prefix(&r.source),"table":r.table,"gateway":r.gateway,"preferred_source":r.preferred_source,"output_interface":r.output_interface,"priority":r.priority,"kind":r.kind,"tos":r.tos,"nexthop_id":r.nexthop_id,"multipath":r.multipath.iter().map(|n|json!({"output_interface":n.output_interface,"gateway":n.gateway,"weight":n.weight,"unknown":attributes(&n.unknown)})).collect::<Vec<_>>(),"unknown":attributes(&r.unknown)})).collect::<Vec<_>>(),
                        "rules":v.rules.iter().map(|r|json!({"family":r.family,"table":r.table,"priority":r.priority,"source":prefix(&r.source),"destination":prefix(&r.destination),"fwmark":r.fwmark,"fwmask":r.fwmask,"uid_range":r.uid_range,"ip_protocol":r.ip_protocol,"source_ports":r.source_ports,"destination_ports":r.destination_ports,"unknown":attributes(&r.unknown)})).collect::<Vec<_>>(),
                        "nexthops":v.nexthops.iter().map(|n|json!({"id":n.id,"output_interface":n.output_interface,"gateway":n.gateway,"group":n.group.iter().map(|g|json!({"id":g.id,"weight":g.weight})).collect::<Vec<_>>(),"unknown":attributes(&n.unknown)})).collect::<Vec<_>>()
                    }),
                }
            }
            Request::Lookup {
                destination,
                source,
                uid,
                mark,
                tos,
                input_interface,
                output_interface,
                transport,
                source_port,
                destination_port,
                timeout_ms,
            } => {
                let mut query = RouteQuery::new(destination, uid)
                    .with_mark(mark)
                    .with_tos(tos)
                    .with_interfaces(input_interface, output_interface);
                if let Some(source) = source {
                    query = match query.with_source(source) {
                        Ok(q) => q,
                        Err(e) => return error(e),
                    };
                }
                match transport {
                    Some(protocol) => {
                        query = query.with_transport(
                            match protocol {
                                Protocol::Tcp => TransportProtocol::Tcp,
                                Protocol::Udp => TransportProtocol::Udp,
                            },
                            source_port,
                            destination_port,
                        )
                    }
                    None if source_port.is_some() || destination_port.is_some() => {
                        return error(NetworkReadError::Malformed);
                    }
                    None => {}
                }
                match tofu_linux::lookup_route(query, cancel, Duration::from_millis(timeout_ms))
                    .await
                {
                    Err(e) => error(e),
                    Ok(v) => {
                        json!({"ok":true,"namespace":namespace(v.namespace),"table":v.table,"output_interface":v.output_interface,"gateway":v.gateway,"preferred_source":v.preferred_source,"kind":format!("{:?}",v.kind),"incomplete":v.incomplete.iter().map(|i|format!("{i:?}")).collect::<Vec<_>>(),"attributes":attributes(&v.attributes),"nexthop_id":v.nexthop_id,"query":{"destination":v.query.destination(),"source":v.query.source(),"uid":v.query.uid(),"mark":v.query.mark(),"tos":v.query.tos(),"transport":v.query.transport().map(|p|format!("{p:?}")),"source_port":v.query.source_port(),"destination_port":v.query.destination_port()}})
                    }
                }
            }
        }
    }
    async fn run() -> Value {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut interrupt), Ok(mut terminate)) = (
            signal(SignalKind::interrupt()),
            signal(SignalKind::terminate()),
        ) else {
            return error(NetworkReadError::Unavailable);
        };
        let (sender, mut cancel) = Cancellation::new();
        let (tx, rx) = tokio::sync::oneshot::channel();
        // Незавершённый stdin не удерживает runtime или завершение по сигналу.
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let read = io::stdin()
                .take((INPUT_LIMIT + 1) as u64)
                .read_to_end(&mut bytes);
            let result = if read.is_err() || bytes.len() > INPUT_LIMIT {
                Err(())
            } else {
                serde_json::from_slice::<Request>(&bytes).map_err(|_| ())
            };
            let _ = tx.send(result);
        });
        let operation = async {
            let request = match tokio::time::timeout(Duration::from_secs(5), rx).await {
                Ok(Ok(Ok(request))) => request,
                Ok(_) => return error(NetworkReadError::Malformed),
                Err(_) => return error(NetworkReadError::Timeout),
            };
            inspect(request, &mut cancel).await
        };
        tokio::pin!(operation);
        tokio::select! {
            value = &mut operation => value,
            _ = interrupt.recv() => { sender.send_replace(true); error(NetworkReadError::Cancelled) },
            _ = terminate.recv() => { sender.send_replace(true); error(NetworkReadError::Cancelled) },
        }
    }
    pub fn main() {
        if std::env::args_os().nth(1).is_some() {
            let _ =
                io::stdout().write_all(b"{\"ok\":false,\"error\":\"Malformed\",\"errno\":null}\n");
            return;
        }
        let value = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(run()),
            Err(_) => error(NetworkReadError::Unavailable),
        };
        let bytes = serde_json::to_vec(&value)
            .ok()
            .filter(|v| v.len() <= OUTPUT_LIMIT)
            .unwrap_or_else(|| b"{\"ok\":false,\"error\":\"Limit\",\"errno\":null}".to_vec());
        let mut output = io::stdout().lock();
        if output
            .write_all(&bytes)
            .and_then(|()| output.write_all(b"\n"))
            .is_err()
        {
            std::process::exit(1);
        }
    }
}
#[cfg(target_os = "linux")]
fn main() {
    linux::main();
}
#[cfg(not(target_os = "linux"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("Инспекция сети доступна только в Linux".into())
}
