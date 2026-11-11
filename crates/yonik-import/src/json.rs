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
pub(crate) fn validate_unique(
    text: &str,
    checkpoint: &crate::Checkpoint<'_>,
) -> Result<(), serde_json::Error> {
    use serde::de::DeserializeSeed;
    #[derive(Clone, Copy)]
    struct Unique<'a>(&'a crate::Checkpoint<'a>);
    impl<'de> DeserializeSeed<'de> for Unique<'_> {
        type Value = ();
        fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
            self.0
                .check()
                .map_err(|_| D::Error::custom("Отмена импорта"))?;
            struct AnyVisitor<'a>(Unique<'a>);
            impl<'de> Visitor<'de> for AnyVisitor<'_> {
                type Value = ();
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("JSON с уникальными ключами")
                }
                fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
                    let mut keys = std::collections::HashSet::new();
                    while let Some(key) = map.next_key::<String>()? {
                        self.0
                            .0
                            .check()
                            .map_err(|_| M::Error::custom("Отмена импорта"))?;
                        if !keys.insert(key) {
                            return Err(M::Error::custom("Повторяющееся поле JSON"));
                        }
                        map.next_value_seed(self.0)?;
                    }
                    Ok(())
                }
                fn visit_seq<S: serde::de::SeqAccess<'de>>(
                    self,
                    mut seq: S,
                ) -> Result<(), S::Error> {
                    while seq.next_element_seed(self.0)?.is_some() {}
                    Ok(())
                }
            }
            let raw = <&serde_json::value::RawValue>::deserialize(deserializer)?;
            self.0
                .check()
                .map_err(|_| D::Error::custom("Отмена импорта"))?;
            if !raw.get().starts_with(['{', '[']) {
                return Ok(());
            }
            // Числа и строки остаются RawValue: opaque JSON не ограничен f64.
            let mut nested = serde_json::Deserializer::from_str(raw.get());
            nested
                .deserialize_any(AnyVisitor(self))
                .map_err(D::Error::custom)
        }
    }
    let mut deserializer = serde_json::Deserializer::from_str(text);
    Unique(checkpoint).deserialize(&mut deserializer)?;
    deserializer.end()
}
