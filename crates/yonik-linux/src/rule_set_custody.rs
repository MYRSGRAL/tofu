//! Зашифрованный immutable контейнер ресурсов; filesystem/FD authority задаёт host.
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use sha2::{Digest, Sha256};
use std::io;
use zeroize::Zeroizing;

const MAX_BYTES: usize = 128 * 1024 * 1024;
const MAX_RECORDS: u64 = 1_500_000;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleSetFormat {
    XrayDomainsV1,
    XrayCidrsV1,
    SingSourceV3,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSetRef {
    digest: [u8; 32],
    bytes: u64,
    records: u64,
    format: RuleSetFormat,
}
impl RuleSetRef {
    pub fn new(
        digest: [u8; 32],
        bytes: u64,
        records: u64,
        format: RuleSetFormat,
    ) -> io::Result<Self> {
        if bytes == 0 || bytes > MAX_BYTES as u64 || records == 0 || records > MAX_RECORDS {
            return Err(invalid());
        }
        Ok(Self {
            digest,
            bytes,
            records,
            format,
        })
    }
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn records(&self) -> u64 {
        self.records
    }
    pub fn format(&self) -> RuleSetFormat {
        self.format
    }
    fn aad(&self) -> Vec<u8> {
        let mut aad = b"yonik-rule-set-v1".to_vec();
        aad.extend_from_slice(&self.digest);
        aad.extend_from_slice(&self.bytes.to_be_bytes());
        aad.extend_from_slice(&self.records.to_be_bytes());
        aad.push(match self.format {
            RuleSetFormat::XrayDomainsV1 => 1,
            RuleSetFormat::XrayCidrsV1 => 2,
            RuleSetFormat::SingSourceV3 => 3,
        });
        aad
    }
    fn verify(&self, body: &[u8]) -> io::Result<()> {
        if body.len() as u64 != self.bytes || Sha256::digest(body).as_slice() != self.digest {
            return Err(invalid());
        }
        Ok(())
    }
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Повреждённый ресурс или неверная ссылка",
    )
}
/// Запись только ciphertext через явно переданный writer. Ключ принадлежит host.
/// Ошибка не разрешает публиковать staging вместо предыдущего snapshot.
pub fn seal_resource(
    reference: &RuleSetRef,
    body: &[u8],
    key: &[u8; 32],
    mut writer: impl io::Write,
) -> io::Result<()> {
    reference.verify(body)?;
    let mut nonce = [0u8; 24];
    getrandom::getrandom(&mut nonce).map_err(|_| invalid())?;
    let ciphertext = XChaCha20Poly1305::new(key.into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: body,
                aad: &reference.aad(),
            },
        )
        .map_err(|_| invalid())?;
    writer.write_all(&nonce)?;
    writer.write_all(&ciphertext)
}
/// Размер проверяется до чтения; bounded reader не читает неограниченный хвост.
pub fn open_resource(
    reference: &RuleSetRef,
    key: &[u8; 32],
    reader: impl io::Read,
) -> io::Result<Zeroizing<Vec<u8>>> {
    use std::io::Read;
    let expected = reference.bytes as usize + 40;
    let mut ciphertext = Vec::new();
    reader
        .take(expected as u64 + 1)
        .read_to_end(&mut ciphertext)?;
    if ciphertext.len() != expected {
        return Err(invalid());
    }
    let body = Zeroizing::new(
        XChaCha20Poly1305::new(key.into())
            .decrypt(
                XNonce::from_slice(&ciphertext[..24]),
                Payload {
                    msg: &ciphertext[24..],
                    aad: &reference.aad(),
                },
            )
            .map_err(|_| invalid())?,
    );
    reference.verify(&body)?;
    Ok(body)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_roundtrip_and_corruption_refusal() {
        let body = b"private.example.org";
        let reference = RuleSetRef::new(
            Sha256::digest(body).into(),
            body.len() as u64,
            1,
            RuleSetFormat::XrayDomainsV1,
        )
        .unwrap();
        let mut sealed = Vec::new();
        seal_resource(&reference, body, &[7; 32], &mut sealed).unwrap();
        assert!(!sealed.windows(body.len()).any(|part| part == body));
        assert_eq!(
            &*open_resource(&reference, &[7; 32], &sealed[..]).unwrap(),
            body
        );
        sealed[25] ^= 1;
        assert!(open_resource(&reference, &[7; 32], &sealed[..]).is_err());
        assert!(seal_resource(&reference, b"different", &[7; 32], Vec::new()).is_err());
        assert!(
            RuleSetRef::new(
                [0; 32],
                MAX_BYTES as u64 + 1,
                1,
                RuleSetFormat::SingSourceV3
            )
            .is_err()
        );
    }
}
