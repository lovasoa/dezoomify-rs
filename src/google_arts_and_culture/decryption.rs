use aes::cipher::{block_padding::NoPadding, KeyIvInit, BlockDecryptMut};
use custom_error::custom_error;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// Decrypt an encrypted image
pub fn decrypt(encrypted: Vec<u8>) -> Result<Vec<u8>, InvalidEncryptedImage> {
    let mut c = Cursor::new(encrypted);

    let marker = read_u32_as_u64_le(&mut c)?;
    if marker != 0x0A_0A_0A_0A {
        // The file is not encrypted
        return Ok(c.into_inner());
    }

    let end_position = c.seek(SeekFrom::End(-4))?;
    let header_size = read_u32_as_u64_le(&mut c)?;
    if 4 + header_size > end_position {
        return Err(InvalidEncryptedImage::BadHeaderSize { header_size });
    }

    let mut decrypted = Vec::new();

    c.seek(SeekFrom::Start(4))?;
    c = read_size(c, &mut decrypted, header_size)?;

    let encrypted_size = read_u32_as_u64_le(&mut c)?;
    if 4 + header_size + 4 + encrypted_size > end_position {
        return Err(InvalidEncryptedImage::BadEncryptedSize { encrypted_size });
    }
    let mut encrypted = Vec::new();
    c = read_size(c, &mut encrypted, encrypted_size)?;
    decrypted.write_all(aes_decrypt_buffer(&mut encrypted)?)?;

    let footer_size = end_position - encrypted_size - 4 - header_size - 4;
    read_size(c, &mut decrypted, footer_size)?;

    Ok(decrypted)
}

fn aes_decrypt_buffer(encrypted: &mut[u8]) -> Result<&[u8], InvalidEncryptedImage> {
    let key = [
        91, 99, 219, 17, 59, 122, 243, 224, 177, 67, 85, 86, 200, 249, 83, 12,
    ];
    let iv = [
        113, 231, 4, 5, 53, 58, 119, 139, 250, 111, 188, 48, 50, 27, 149, 146,
    ];
    Aes128CbcDec::new(&key.into(), &iv.into())
        .decrypt_padded_mut::<NoPadding>(encrypted)
        .map_err(|_| InvalidEncryptedImage::DecryptError)
}

#[inline]
fn read_u32_as_u64_le<T: Read>(buf: &mut T) -> std::io::Result<u64> {
    let mut bytes = [0u8; 4];
    buf.read_exact(&mut bytes)?;
    let result = u32::from_le_bytes(bytes);
    Ok(u64::from(result))
}

/// Reads "size" bytes from "c" and writes them to "dest".
fn read_size<T: Read>(c: T, dest: &mut Vec<u8>, size: u64) -> Result<T, std::io::Error> {
    let mut wrapper = c.take(size);
    wrapper.read_to_end(dest)?;

    Ok(wrapper.into_inner())
}

custom_error! {pub InvalidEncryptedImage
    BadHeaderSize{header_size:u64} = "The size of the unencrypted header ({header_size}) is invalid.",
    BadEncryptedSize{encrypted_size:u64} = "The size of the encrypted data ({encrypted_size}) is invalid.",
    DecryptError = "Unable to decrypt the encrypted data",
    IO{source: std::io::Error} = "Unable to read from the buffer: {source}",
}

#[test]
fn test_decrypt_dummy() {
    let encrypted: Vec<u8> = vec![
        10, 10, 10, 10, // magic bytes
        186, 186, 192, 192, // unencrypted header
        16, 0, 0, 0, // encrypted data length
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // encrypted data
        222, 173, 190, 175, // unencrypted footer
        4, 0, 0, 0, // size of unencrypted header
    ];
    let decrypted: Vec<u8> = vec![
        186, 186, 192, 192, // unencrypted header
        202, 37, 17, 24, 3, 15, 249, 175, 241, 134, 189, 204, 188, 226, 106,
        76, // decrypted data
        222, 173, 190, 175, // unencrypted footer
    ];
    assert_eq!(decrypt(encrypted).unwrap(), decrypted);
}

#[test]
fn test_decrypt_sample_tile() {
    use std::{fs, path::Path};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("google_arts_and_culture");
    let encrypted = fs::read(root.join("tile_encrypted.bin")).unwrap();
    let decrypted: Vec<u8> = fs::read(root.join("tile.jpg")).unwrap();
    assert_eq!(decrypt(encrypted).unwrap(), decrypted);
}
