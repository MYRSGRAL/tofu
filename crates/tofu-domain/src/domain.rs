use std::{fmt, net::IpAddr, str::FromStr};

#[derive(Clone, PartialEq, Eq)]
pub struct DomainName(String);
impl DomainName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for DomainName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DomainName(<скрыто>)")
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidDomain;
impl fmt::Display for InvalidDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Некорректное DNS-имя")
    }
}
impl std::error::Error for InvalidDomain {}
impl FromStr for DomainName {
    type Err = InvalidDomain;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() > 1024 || value.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(InvalidDomain);
        }
        let mut name = idna::uts46::Uts46::new()
            .to_ascii(
                value.as_bytes(),
                idna::AsciiDenyList::STD3,
                idna::uts46::Hyphens::Allow,
                idna::uts46::DnsLength::VerifyAllowRootDot,
            )
            .map_err(|_| InvalidDomain)?
            .into_owned();
        name.make_ascii_lowercase();
        if name.ends_with('.') {
            name.pop();
        }
        if name.is_empty()
            || name.len() > 253
            || name.parse::<IpAddr>().is_ok()
            || name.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return Err(InvalidDomain);
        }
        Ok(Self(name))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainPattern {
    Exact(DomainName),
    Suffix(DomainName),
}
impl DomainPattern {
    pub fn name(&self) -> &DomainName {
        match self {
            Self::Exact(name) | Self::Suffix(name) => name,
        }
    }
    pub fn matches(&self, candidate: &DomainName) -> bool {
        let expected = self.name().as_str();
        candidate.as_str() == expected
            || matches!(self, Self::Suffix(_))
                && candidate
                    .as_str()
                    .strip_suffix(expected)
                    .is_some_and(|prefix| prefix.ends_with('.'))
    }
}
