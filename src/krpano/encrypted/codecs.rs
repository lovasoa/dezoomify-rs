use super::EncryptedKrpanoError;

pub const PACKED_VIEWER_HEADER_LEN: usize = 8;
pub const MIN_PACKED_VIEWER_PAYLOAD_LEN: usize = 100;
pub const MAX_DECODED_VIEWER_JS_LEN: usize = 8 * 1024 * 1024;

pub fn decode_modified_base85(input: &str) -> Result<Vec<u8>, EncryptedKrpanoError> {
    decode_modified_base85_with_order(input, u32::to_be_bytes)
}

pub fn decode_modified_base85_little_endian(input: &str) -> Result<Vec<u8>, EncryptedKrpanoError> {
    decode_modified_base85_with_order(input, u32::to_le_bytes)
}

fn decode_modified_base85_with_order(
    input: &str,
    byte_order: fn(u32) -> [u8; 4],
) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let complete_len = input.len() / 5 * 5;
    let mut decoded = Vec::with_capacity(complete_len / 5 * 4);
    for chunk in input.as_bytes()[..complete_len].chunks_exact(5) {
        let mut value = 0u64;
        for &byte in chunk {
            let mut digit = byte
                .checked_sub(35)
                .ok_or(EncryptedKrpanoError::InvalidBase85Byte { byte })?;
            if digit > 56 {
                digit -= 1;
            }
            if digit >= 85 {
                return Err(EncryptedKrpanoError::InvalidBase85Byte { byte });
            }
            value = value * 85 + u64::from(digit);
        }
        decoded.extend_from_slice(&byte_order(value as u32));
    }
    Ok(decoded)
}

pub fn lz4_decompress_block(
    input: &[u8],
    decompressed_len: usize,
    compressed_end: usize,
) -> Result<Vec<u8>, EncryptedKrpanoError> {
    if compressed_end > input.len() {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }
    let mut src = 0;
    let mut output = Vec::with_capacity(decompressed_len);
    while src < compressed_end {
        let token = input[src];
        src += 1;

        let literal_len = read_lz4_len(token >> 4, input, &mut src, compressed_end)?;
        if src + literal_len > compressed_end {
            return Err(EncryptedKrpanoError::InvalidLz4Block);
        }
        output.extend_from_slice(&input[src..src + literal_len]);
        src += literal_len;
        if src == compressed_end {
            break;
        }

        if src + 2 > compressed_end {
            return Err(EncryptedKrpanoError::InvalidLz4Block);
        }
        let offset = usize::from(input[src]) | (usize::from(input[src + 1]) << 8);
        src += 2;
        if offset == 0 || offset > output.len() {
            return Err(EncryptedKrpanoError::InvalidLz4Block);
        }
        let match_len = read_lz4_len(token & 0x0f, input, &mut src, compressed_end)? + 4;
        for _ in 0..match_len {
            let byte = output[output.len() - offset];
            output.push(byte);
        }
    }
    if output.len() != decompressed_len {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }
    Ok(output)
}

fn read_lz4_len(
    nibble: u8,
    input: &[u8],
    src: &mut usize,
    compressed_end: usize,
) -> Result<usize, EncryptedKrpanoError> {
    let mut len = usize::from(nibble);
    if nibble == 15 {
        loop {
            if *src >= compressed_end {
                return Err(EncryptedKrpanoError::InvalidLz4Block);
            }
            let next = input[*src];
            *src += 1;
            len += usize::from(next);
            if next != 255 {
                break;
            }
        }
    }
    Ok(len)
}

pub fn decode_packed_viewer_js_payload(input: &str) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let packed = decode_modified_base85(input)?;
    decode_packed_lz4_payload(&packed)
}

pub fn decode_packed_viewer_js_payload_little_endian(
    input: &str,
) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let packed = decode_modified_base85_little_endian(input)?;
    decode_packed_lz4_payload(&packed)
}

fn decode_packed_lz4_payload(packed: &[u8]) -> Result<Vec<u8>, EncryptedKrpanoError> {
    if packed.len() < PACKED_VIEWER_HEADER_LEN {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }

    let decompressed_len = read_u24_le(&packed[0..3]);
    if decompressed_len > MAX_DECODED_VIEWER_JS_LEN {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }
    let compressed_end = PACKED_VIEWER_HEADER_LEN + read_u24_le(&packed[4..7]);
    if compressed_end > packed.len() {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }

    lz4_decompress_block(
        &packed[PACKED_VIEWER_HEADER_LEN..],
        decompressed_len,
        compressed_end - PACKED_VIEWER_HEADER_LEN,
    )
}

fn read_u24_le(input: &[u8]) -> usize {
    usize::from(input[0]) | (usize::from(input[1]) << 8) | (usize::from(input[2]) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_modified_base85_chunks() {
        assert_eq!(decode_modified_base85("7vgt.").unwrap(), b"ABCD");
    }

    #[test]
    fn decodes_lz4_literal_only_block() {
        assert_eq!(
            lz4_decompress_block(&[0x30, b'a', b'b', b'c'], 3, 4).unwrap(),
            b"abc"
        );
    }

    #[test]
    fn decodes_lz4_back_reference_block() {
        assert_eq!(
            lz4_decompress_block(&[0x32, b'a', b'b', b'c', 3, 0], 9, 6).unwrap(),
            b"abcabcabc"
        );
    }
}
