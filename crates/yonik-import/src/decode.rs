use crate::{FatalError, MAX_DECODED_BYTES, MAX_JSON_DEPTH};
use base64::{
    Engine, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};

pub(crate) fn base64_text(text: &str) -> Result<String, FatalError> {
    base64_text_checked(
        text,
        &crate::Checkpoint {
            proceed: &|| true,
            cancelled: std::cell::Cell::new(false),
        },
    )
}
pub(crate) fn base64_text_checked(
    text: &str,
    checkpoint: &crate::Checkpoint<'_>,
) -> Result<String, FatalError> {
    let mut cleaned = Vec::with_capacity(text.len());
    for chunk in text.as_bytes().chunks(4096) {
        checkpoint.check()?;
        cleaned.extend(chunk.iter().copied().filter(|b| !b.is_ascii_whitespace()));
    }
    let estimated = base64::decoded_len_estimate(cleaned.len());
    if estimated > MAX_DECODED_BYTES {
        return Err(FatalError::TooLarge);
    }
    let config =
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent);
    let decode = |alphabet| -> Result<Vec<u8>, FatalError> {
        let engine = GeneralPurpose::new(alphabet, config);
        let mut decoded = Vec::with_capacity(estimated);
        let chunks = cleaned.chunks(4096);
        let count = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            checkpoint.check()?;
            if index + 1 < count && chunk.contains(&b'=') {
                return Err(FatalError::InvalidEncoding);
            }
            engine
                .decode_vec(chunk, &mut decoded)
                .map_err(|_| FatalError::InvalidEncoding)?;
        }
        Ok(decoded)
    };
    let decoded = decode(&alphabet::STANDARD).or_else(|_| decode(&alphabet::URL_SAFE))?;
    checkpoint.check()?;
    String::from_utf8(decoded).map_err(|_| FatalError::InvalidUtf8)
}

/// До JSON-парсера ограничиваем стек; скобки внутри строк не считаются.
pub(crate) fn json_depth(text: &str) -> Result<(), FatalError> {
    json_depth_checked(
        text,
        &crate::Checkpoint {
            proceed: &|| true,
            cancelled: std::cell::Cell::new(false),
        },
    )
}
pub(crate) fn json_depth_checked(
    text: &str,
    checkpoint: &crate::Checkpoint<'_>,
) -> Result<(), FatalError> {
    let mut depth = 0usize;
    let mut string = false;
    let mut escaped = false;
    for (index, byte) in text.bytes().enumerate() {
        if index % 4096 == 0 {
            checkpoint.check()?;
        }
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
