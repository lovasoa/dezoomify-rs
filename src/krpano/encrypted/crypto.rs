use super::EncryptedKrpanoError;

pub fn decrypt_bytes(
    input: &[u8],
    key: &[u8],
    widened_key_index: bool,
) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let key_mask_base = 15usize;
    let prefix_len = 1usize << (key_mask_base / 2);
    if input.len() < prefix_len || key.is_empty() {
        return Err(EncryptedKrpanoError::InvalidByteCipherInput);
    }
    // encrypted_start is always computed with key_mask=15 (before widening),
    // matching the JS engine: c=k+(a[ob(Ma[1],0)]&f>>1) where f=15.
    let encrypted_start =
        prefix_len + (usize::from(input[usize::from(b'A')]) & (key_mask_base >> 1));
    if encrypted_start > input.len() {
        return Err(EncryptedKrpanoError::InvalidByteCipherInput);
    }
    let key_mask = if widened_key_index {
        key_mask_base | (key_mask_base << 3)
    } else {
        key_mask_base
    };

    let mut mixed_key = vec![0u8; prefix_len * 2];
    let mut out = 0;
    for idx in 0..prefix_len {
        mixed_key[out] = input[idx];
        mixed_key[out + 1] = key[(idx & key_mask) % key.len()];
        out += 2;
    }

    let mut state = [0u8; 256];
    for (idx, value) in state.iter_mut().enumerate() {
        *value = idx as u8;
    }
    let mut j = 0usize;
    for idx in 0..256 {
        j = (j + usize::from(state[idx]) + usize::from(mixed_key[idx])) & 255;
        state.swap(idx, j);
    }

    // krpano discards the first 256 bytes of the stream.
    let mut i = 0usize;
    j = 0;
    for _ in 0..256 {
        i = (i + 1) & 255;
        j = (j + usize::from(state[i])) & 255;
        state.swap(i, j);
    }

    let mut decrypted = Vec::with_capacity(input.len() - encrypted_start);
    for &byte in &input[encrypted_start..] {
        i = (i + 1) & 255;
        j = (j + usize::from(state[i])) & 255;
        let key_byte = state[(usize::from(state[i]) + usize::from(state[j])) & 255];
        decrypted.push(byte ^ key_byte);
        state.swap(i, j);
    }
    Ok(decrypted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypts_byte_cipher_round_trip() {
        let key = b"test-key";
        let plaintext = b"plain krpano bytes";
        let prefix_len = 128;

        let mut encrypted = vec![0u8; prefix_len];
        encrypted[usize::from(b'A')] = 0;

        // Generate keystream and encrypt.
        let mut src = encrypted.clone();
        src.extend(std::iter::repeat_n(0, plaintext.len()));
        let keystream = decrypt_bytes(&src, key, true).unwrap();
        let ciphertext: Vec<_> = plaintext
            .iter()
            .zip(keystream.iter())
            .map(|(&p, &k)| p ^ k)
            .collect();

        encrypted.extend_from_slice(&ciphertext);
        assert_eq!(decrypt_bytes(&encrypted, key, true).unwrap(), plaintext);
    }
}
