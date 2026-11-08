use serde::de::{Deserialize, Deserializer, Error, MapAccess, Visitor};
use serde_json::Value;
use std::{collections::BTreeMap, fmt};

struct UniqueObject(BTreeMap<String, Value>);
impl<'de> Deserialize<'de> for UniqueObject {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = UniqueObject;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON-объект с уникальными полями")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut source: M) -> Result<Self::Value, M::Error> {
                let mut fields = BTreeMap::new();
                while let Some((key, value)) = source.next_entry::<String, Value>()? {
                    if fields.insert(key, value).is_some() {
                        return Err(M::Error::custom("Повторяющееся поле JSON"));
                    }
                }
                Ok(UniqueObject(fields))
            }
        }
        deserializer.deserialize_map(ObjectVisitor)
    }
}
pub(crate) fn unique_object(text: &str) -> Result<BTreeMap<String, Value>, serde_json::Error> {
    serde_json::from_str::<UniqueObject>(text).map(|object| object.0)
}
