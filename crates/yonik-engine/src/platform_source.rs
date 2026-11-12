//! Закрытый переносимый DTO; не предоставляет runtime mapping.
use serde::{Deserialize, Serialize};
use yonik_domain::{AndroidPackageId, PlatformSource};
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlatformSourceInput {
    AndroidPackage { package: String },
}
impl PlatformSourceInput {
    pub fn typed(&self) -> Result<PlatformSource, &'static str> {
        match self {
            Self::AndroidPackage { package } => AndroidPackageId::parse(package)
                .map(|package| PlatformSource::AndroidPackage { package })
                .ok_or("package"),
        }
    }
    pub fn bytes(&self) -> usize {
        match self {
            Self::AndroidPackage { package } => package.len(),
        }
    }
}

pub fn deserialize_sources<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<PlatformSourceInput>>, D::Error> {
    Vec::<PlatformSourceInput>::deserialize(deserializer).map(Some)
}
