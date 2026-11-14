use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportInput {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub transport_protocols: Option<Vec<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub destination_ports: Option<Vec<(u16, u16)>>,
}
impl TransportInput {
    pub fn typed(&self) -> Result<yonik_domain::TransportConditions, &'static str> {
        let protocols = self
            .transport_protocols
            .as_ref()
            .map(|v| v.iter().map(String::as_str).collect::<Vec<_>>());
        let ports = self
            .destination_ports
            .as_ref()
            .map(|v| yonik_domain::DestinationPorts::parse(v))
            .transpose()?;
        yonik_domain::TransportConditions::parse(protocols.as_deref(), ports)
    }
    pub fn from_typed(value: &yonik_domain::TransportConditions) -> Self {
        Self {
            transport_protocols: value.protocols().map(<[String]>::to_vec),
            destination_ports: value.ports().map(|p| p.ranges().to_vec()),
        }
    }
}

fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use super::TransportInput;
    #[test]
    fn null_empty_unknown_are_not_any() {
        for raw in [
            r#"{"transport_protocols":null}"#,
            r#"{"destination_ports":null}"#,
            r#"{"future":true}"#,
        ] {
            assert!(serde_json::from_str::<TransportInput>(raw).is_err());
        }
        for raw in [
            r#"{"transport_protocols":[]}"#,
            r#"{"destination_ports":[]}"#,
            r#"{"destination_ports":[[0,1]]}"#,
        ] {
            assert!(
                serde_json::from_str::<TransportInput>(raw)
                    .unwrap()
                    .typed()
                    .is_err()
            );
        }
    }
}
