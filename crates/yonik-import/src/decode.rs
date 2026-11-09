use crate::{FatalError, MAX_DECODED_BYTES, MAX_JSON_DEPTH};
use base64::{
    Engine, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};

pub(crate) fn base64_text(text: &str) -> Result<String, FatalError> {
    let cleaned: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let estimated = base64::decoded_len_estimate(cleaned.len());
    if estimated > MAX_DECODED_BYTES {
        return Err(FatalError::TooLarge);
    }
    let config =
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent);
    let decoded = GeneralPurpose::new(&alphabet::STANDARD, config)
        .decode(&cleaned)
        .or_else(|_| GeneralPurpose::new(&alphabet::URL_SAFE, config).decode(&cleaned))
        .map_err(|_| FatalError::InvalidEncoding)?;
    String::from_utf8(decoded).map_err(|_| FatalError::InvalidUtf8)
}

/// До JSON-парсера ограничиваем стек; скобки внутри строк не считаются.
pub(crate) fn json_depth(text: &str) -> Result<(), FatalError> {
    let mut depth = 0usize;
    let mut string = false;
    let mut escaped = false;
    for byte in text.bytes() {
        if string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                string = false;
            }
            continue;
        }
        match byte {
            b'"' => string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_JSON_DEPTH {
                    return Err(FatalError::JsonTooDeep);
                }
            }
            b'}' | b']' => depth = depth.checked_sub(1).ok_or(FatalError::InvalidJson)?,
            _ => {}
        }
    }
    Ok(())
}
