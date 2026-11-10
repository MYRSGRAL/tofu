use std::num::NonZeroU16;
use yonik_domain::{DnsPolicy, NativeDnsMode, Policy};
use yonik_engine::{Engine, PolicyMode, compile_native_dns};
use yonik_import::import_bytes;

#[test]
fn generate_native_modes() {
    let source = import_bytes(b"socks5://127.0.0.1:24082".to_vec()).unwrap();
    for (engine, name) in [(Engine::Xray, "xray"), (Engine::SingBox, "sing-box")] {
        for mode in [NativeDnsMode::Real, NativeDnsMode::FakeIp] {
            let dns = DnsPolicy {
                mode,
                upstream: "127.0.0.1:1053".parse().unwrap(),
                listen_port: NonZeroU16::new(1054).unwrap(),
            };
            let config = compile_native_dns(
                &source.servers[0],
                engine,
                NonZeroU16::new(10808).unwrap(),
                NonZeroU16::new(10809).unwrap(),
                &Policy::default(),
                PolicyMode::Names,
                &dns,
            )
            .unwrap();
            let mut bytes = Vec::new();
            config.write_sensitive(&mut bytes).unwrap();
            let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(json.get("dns").is_some());
            if let Ok(path) = std::env::var("YONIK_DNS_EVIDENCE") {
                std::fs::write(format!("{path}/{name}-{mode:?}.json"), bytes).unwrap();
            }
            assert!(
                compile_native_dns(
                    &source.servers[0],
                    engine,
                    NonZeroU16::new(10808).unwrap(),
                    NonZeroU16::new(10809).unwrap(),
                    &Policy::default(),
                    PolicyMode::IpOnly,
                    &dns
                )
                .is_err()
            );
        }
    }
}

#[test]
fn reject_recursive_upstream_bootstrap_and_routing_before_activation() {
    use yonik_domain::{Action, Cidr};
    use yonik_engine::{PolicyCapability, PolicyCompileError};
    for engine in [Engine::Xray, Engine::SingBox] {
        for uri in ["socks5://proxy.test:1080", "socks5://127.0.0.1:24082"] {
            let source = import_bytes(uri.as_bytes().to_vec()).unwrap();
            let mut dns = DnsPolicy {
                mode: NativeDnsMode::Real,
                upstream: "127.0.0.1:1053".parse().unwrap(),
                listen_port: NonZeroU16::new(1054).unwrap(),
            };
            for case in 0..4 {
                let mut policy = Policy::default();
                match case {
                    0 => dns.upstream.set_port(1054),
                    1 => {
                        dns.upstream.set_port(1053);
                        policy.default_action = Action::Direct;
                    }
                    2 => policy.lan.push("192.168.0.0/16".parse::<Cidr>().unwrap()),
                    _ if uri.contains("proxy.test") => {}
                    _ => continue,
                }
                assert_eq!(
                    compile_native_dns(
                        &source.servers[0],
                        engine,
                        NonZeroU16::new(10808).unwrap(),
                        NonZeroU16::new(10809).unwrap(),
                        &policy,
                        PolicyMode::Names,
                        &dns
                    )
                    .unwrap_err(),
                    PolicyCompileError::Unsupported(PolicyCapability::DnsContract)
                );
            }
        }
    }
}

/// Требует pinned binaries и unprivileged network namespaces; DNS ОС не меняет.
#[cfg(unix)]
#[test]
#[ignore = "явный изолированный native gate"]
fn live_dns_and_fake_mapping() {
    assert!(
        std::env::var("YONIK_DNS_EVIDENCE").is_ok(),
        "Нужен постоянный каталог evidence"
    );
    generate_native_modes();
    let namespace = std::fs::metadata("/proc/self/ns/net").unwrap();
    use std::os::unix::fs::MetadataExt;
    let status = std::process::Command::new("unshare")
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .args([
            "--user",
            "--map-root-user",
            "--net",
            "python",
            "-c",
            LIVE_FIXTURE,
            &namespace.ino().to_string(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(unix)]
const LIVE_FIXTURE: &str = r#"import sys,os,socket,struct,threading,time,json,ipaddress
from pathlib import Path
sys.path.insert(0,str(Path.cwd()/'tools'))
from dns_fixture import DnsFixture,Reply,dns_query,verify_namespace,validated_binary,engine_process
from traffic_echo import receive_exact
base=Path(os.environ['YONIK_DNS_EVIDENCE'])
verify_namespace(int(sys.argv[1]))
seen=[]
stop=threading.Event()
def serve():
 with socket.socket() as s:
  s.bind(('127.0.0.1',24082));s.listen();s.settimeout(.2)
  while not stop.is_set():
   try:c,_=s.accept()
   except socket.timeout:continue
   with c:
    c.settimeout(3); v,n=receive_exact(c,2);receive_exact(c,n);c.sendall(b'\x05\x00')
    h=receive_exact(c,4);typ=h[3]
    if typ==3: host=receive_exact(c,receive_exact(c,1)[0]).decode()
    elif typ==1:host=socket.inet_ntoa(receive_exact(c,4))
    else:raise RuntimeError('unexpected family')
    port=struct.unpack('!H',receive_exact(c,2))[0];seen.append([host,port]);c.sendall(b'\x05\x00\x00\x01'+bytes(6));
    with socket.create_connection(('127.0.0.1',24081),timeout=2) as endpoint: c.sendall(receive_exact(endpoint,14))
def endpoint():
 with socket.socket() as s:
  s.bind(('127.0.0.1',24081));s.listen();s.settimeout(.2)
  while not stop.is_set():
   try:c,_=s.accept()
   except socket.timeout:continue
   with c:c.sendall(b'LOCAL_ENDPOINT')
e=threading.Thread(target=endpoint);e.start()
t=threading.Thread(target=serve);t.start()
results=[]
try:
 for engine in ['xray','sing-box']:
  binary=validated_binary(Path('/home/myrs/projects/yonik/local/engines')/('xray-26.10.31' if engine=='xray' else 'sing-box-1.14.4')/engine,engine)
  for mode in ['Real','FakeIp']:
   config=json.loads((base/f'{engine}-{mode}.json').read_text())
   with DnsFixture(lambda *args:Reply(addresses=('192.0.2.20',),ttl=30)) as fixture:
    with engine_process(binary,engine,config,base/f'run-{engine}-{mode}-{time.time_ns()}'):
     time.sleep(.3)
     addresses=dns_query('public.test',port=1054)
     assert len(addresses)==1,addresses
     fake=ipaddress.ip_address(addresses[0]) in ipaddress.ip_network('198.18.0.0/15')
     assert fake==(mode=='FakeIp'),addresses
     with socket.create_connection(('127.0.0.1',10808),timeout=3) as c:
      c.settimeout(3);c.sendall(b'\x05\x01\x00');assert receive_exact(c,2)==b'\x05\x00'
      c.sendall(b'\x05\x01\x00\x01'+socket.inet_aton(addresses[0])+struct.pack('!H',24081))
      reply=receive_exact(c,10);assert reply[1]==0,reply
      assert receive_exact(c,14)==b'LOCAL_ENDPOINT'
     assert seen[-1][0]==('public.test' if fake else addresses[0]),seen
     results.append(dict(engine=engine,mode=mode,answers=addresses,upstream=fixture.records,socks=seen[-1]))
finally:stop.set();t.join(3);e.join(3)
(base/'live-results.json').write_text(json.dumps(results,indent=2))
"#;
