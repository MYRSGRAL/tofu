//! Условия исходного destination; witness предоставляет доверенный адаптер.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationPorts(Vec<(u16, u16)>);
impl DestinationPorts {
    pub fn parse(input: &[(u16, u16)]) -> Result<Self, &'static str> {
        if input.is_empty() || input.len() > 16 {
            return Err("ports");
        }
        let mut ranges = input.to_vec();
        ranges.sort_unstable();
        let mut normalized: Vec<(u16, u16)> = Vec::with_capacity(ranges.len());
        for (first, last) in ranges {
            if first == 0 || first > last {
                return Err("ports");
            }
            if let Some(previous) = normalized.last_mut() {
                if first <= previous.1 {
                    return Err("ports_overlap");
                }
                if u32::from(first) == u32::from(previous.1) + 1 {
                    previous.1 = last;
                    continue;
                }
            }
            normalized.push((first, last));
        }
        Ok(Self(normalized))
    }
    pub fn ranges(&self) -> &[(u16, u16)] {
        &self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportWitness {
    KnownTcp(u16),
    KnownUdp(u16),
    KnownOther,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TransportConditions {
    protocols: Option<Vec<String>>,
    ports: Option<DestinationPorts>,
}
impl TransportConditions {
    pub fn parse(
        protocols: Option<&[&str]>,
        ports: Option<DestinationPorts>,
    ) -> Result<Self, &'static str> {
        if let Some(values) = protocols {
            if values.is_empty()
                || values.len() > 2
                || values.iter().any(|v| !["tcp", "udp"].contains(v))
                || (values.len() == 2 && values[0] == values[1])
            {
                return Err("protocols");
            }
        }
        Ok(Self {
            protocols: protocols.map(|v| v.iter().map(|s| (*s).to_owned()).collect()),
            ports,
        })
    }
    pub fn constrained(&self) -> bool {
        self.protocols.is_some() || self.ports.is_some()
    }
    pub fn protocols(&self) -> Option<&[String]> {
        self.protocols.as_deref()
    }
    pub fn ports(&self) -> Option<&DestinationPorts> {
        self.ports.as_ref()
    }
    pub fn matches(&self, witness: TransportWitness) -> Result<bool, &'static str> {
        if !self.constrained() {
            return Ok(true);
        }
        let (protocol, port) = match witness {
            TransportWitness::KnownTcp(p) => ("tcp", p),
            TransportWitness::KnownUdp(p) => ("udp", p),
            TransportWitness::KnownOther => return Ok(false),
            TransportWitness::Unavailable => return Err("transport_unavailable"),
        };
        Ok(self
            .protocols
            .as_ref()
            .is_none_or(|v| v.iter().any(|s| s == protocol))
            && self
                .ports
                .as_ref()
                .is_none_or(|v| v.0.iter().any(|&(a, b)| a <= port && port <= b)))
    }
}
