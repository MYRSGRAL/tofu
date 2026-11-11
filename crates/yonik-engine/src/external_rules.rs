//! Потоковые нативные ресурсы. Не запускает ядра, не выбирает пути и не меняет policy.
use std::io::{self, BufRead, BufReader, Read, Seek, Write};
use yonik_domain::{Cidr, DomainName};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    XrayDomains,
    XrayCidrs,
    SingDomains,
    SingCidrs,
}
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub records: u64,
    pub source_bytes: u64,
    pub artifact_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            records: 1_500_000,
            source_bytes: 128 * 1024 * 1024,
            artifact_bytes: 128 * 1024 * 1024,
        }
    }
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Некорректный набор или превышен бюджет",
    )
}
fn varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 128 {
        out.push(value as u8 | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
fn field(tag: u8, data: &[u8], out: &mut Vec<u8>) {
    out.push(tag);
    varint(data.len() as u64, out);
    out.extend_from_slice(data);
}
fn record(line: &str, format: Format) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(300);
    match format {
        Format::XrayDomains | Format::SingDomains => {
            let (kind, name) = line.split_once(':').ok_or_else(invalid)?;
            if !matches!(kind, "exact" | "suffix") {
                return Err(invalid());
            }
            let parsed: DomainName = name.parse().map_err(|_| invalid())?;
            if parsed.as_str() != name {
                return Err(invalid());
            }
            if format == Format::XrayDomains {
                out.extend_from_slice(&[8, if kind == "exact" { 3 } else { 2 }]);
                field(18, name.as_bytes(), &mut out);
                let mut wrapped = Vec::with_capacity(300);
                field(18, &out, &mut wrapped);
                out = wrapped;
            } else {
                serde_json::to_writer(
                    &mut out,
                    &serde_json::json!({if kind == "exact" { "domain" } else { "domain_suffix" }: [name]}),
                )?;
            }
        }
        Format::XrayCidrs | Format::SingCidrs => {
            let parsed: Cidr = line.parse().map_err(|_| invalid())?;
            if parsed.to_string() != line {
                return Err(invalid());
            }
            if format == Format::XrayCidrs {
                let (ip, prefix) = line.split_once('/').ok_or_else(invalid)?;
                let ip: std::net::IpAddr = ip.parse().map_err(|_| invalid())?;
                match ip {
                    std::net::IpAddr::V4(ip) => field(10, &ip.octets(), &mut out),
                    std::net::IpAddr::V6(ip) => field(10, &ip.octets(), &mut out),
                }
                out.push(16);
                varint(prefix.parse().map_err(|_| invalid())?, &mut out);
                let mut wrapped = Vec::with_capacity(32);
                field(18, &out, &mut wrapped);
                out = wrapped;
            } else {
                serde_json::to_writer(&mut out, &serde_json::json!({"ip_cidr":[line]}))?;
            }
        }
    }
    Ok(out)
}
fn visit(
    source: &mut (impl Read + Seek),
    format: Format,
    expected: u64,
    limits: Limits,
    mut emit: impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<u64> {
    let mut reader = BufReader::new(source);
    let mut count = 0u64;
    let mut bytes = 0u64;
    let mut encoded = 0u64;
    loop {
        let mut line = Vec::with_capacity(300);
        let n = reader.by_ref().take(300).read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        count += 1;
        if bytes > limits.source_bytes || count > expected || line.last() != Some(&b'\n') {
            return Err(invalid());
        }
        line.pop();
        let data = record(std::str::from_utf8(&line).map_err(|_| invalid())?, format)?;
        encoded = encoded.checked_add(data.len() as u64).ok_or_else(invalid)?;
        if encoded > limits.artifact_bytes {
            return Err(invalid());
        }
        emit(&data)?;
    }
    if count != expected {
        return Err(invalid());
    }
    Ok(encoded)
}
/// Два линейных прохода, O(1) памяти. Источник должен оставаться immutable между
/// проходами; публикация writer допустима только после успешного возврата.
pub fn compile_records(
    mut source: impl Read + Seek,
    mut writer: impl Write,
    format: Format,
    expected: u64,
    limits: Limits,
) -> io::Result<u64> {
    let hard = Limits::default();
    if expected == 0
        || expected > limits.records
        || limits.records > hard.records
        || limits.source_bytes > hard.source_bytes
        || limits.artifact_bytes > hard.artifact_bytes
    {
        return Err(invalid());
    }
    let start = source.stream_position()?;
    let size = visit(&mut source, format, expected, limits, |_| Ok(()))?;
    let sing = matches!(format, Format::SingDomains | Format::SingCidrs);
    let mut header = Vec::new();
    let tail: &[u8] = if sing { b"]}" } else { b"" };
    if sing {
        header.extend_from_slice(b"{\"version\":3,\"rules\":[");
    } else {
        header.push(10);
        varint(size + 7, &mut header);
        field(10, b"YONIK", &mut header);
    }
    let total =
        size + header.len() as u64 + tail.len() as u64 + if sing { expected - 1 } else { 0 };
    if total > limits.artifact_bytes {
        return Err(invalid());
    }
    source.seek(std::io::SeekFrom::Start(start))?;
    writer.write_all(&header)?;
    let mut first = true;
    visit(&mut source, format, expected, limits, |data| {
        if sing && !first {
            writer.write_all(b",")?;
        }
        first = false;
        writer.write_all(data)
    })?;
    writer.write_all(tail)?;
    Ok(total)
}
