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

/// Проверка всех ключей без построения дерева и без last-wins для credentials.
pub(crate) fn validate_unique(text: &str) -> Result<(), serde_json::Error> {
    struct Unique;
    impl<'de> Deserialize<'de> for Unique {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct AnyVisitor;
            impl<'de> Visitor<'de> for AnyVisitor {
                type Value = Unique;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("JSON с уникальными ключами")
                }
                fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Unique, M::Error> {
                    let mut keys = std::collections::HashSet::new();
                    while let Some(key) = map.next_key::<String>()? {
                        if !keys.insert(key) {
                            return Err(M::Error::custom("Повторяющееся поле JSON"));
                        }
                        map.next_value::<Unique>()?;
                    }
                    Ok(Unique)
                }
                fn visit_seq<S: serde::de::SeqAccess<'de>>(
                    self,
                    mut seq: S,
                ) -> Result<Unique, S::Error> {
                    while seq.next_element::<Unique>()?.is_some() {}
                    Ok(Unique)
                }
            }
            let raw = <&serde_json::value::RawValue>::deserialize(deserializer)?;
            if !raw.get().starts_with(['{', '[']) {
                return Ok(Unique);
            }
            // Числа и строки остаются RawValue: opaque JSON не ограничен f64.
            let mut nested = serde_json::Deserializer::from_str(raw.get());
            nested
                .deserialize_any(AnyVisitor)
                .map_err(|_| D::Error::custom("Повторяющееся поле JSON"))
        }
    }
    serde_json::from_str::<Unique>(text).map(|_| ())
}
