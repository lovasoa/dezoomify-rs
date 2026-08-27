//! Application of processing recipes to fetched tile payloads.

use custom_error::custom_error;

use super::model::ProcessingRecipe;
use crate::google_arts_and_culture::decryption;

custom_error! {pub ProcessingError
    GoogleArtsDecrypt{source: decryption::InvalidEncryptedImage} =
        "unable to decrypt a Google Arts & Culture tile: {source}",
}

impl ProcessingRecipe {
    /// Apply this recipe to a fetched tile payload, returning the bytes that
    /// should be decoded as an image.
    pub fn apply(&self, bytes: Vec<u8>) -> Result<Vec<u8>, ProcessingError> {
        match self {
            Self::None => Ok(bytes),
            Self::GoogleArtsDecrypt => Ok(decryption::decrypt(bytes)?),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn none_is_the_identity() {
        let bytes = vec![1, 2, 3];
        assert_eq!(ProcessingRecipe::None.apply(bytes.clone()).unwrap(), bytes);
    }

    #[test]
    fn google_arts_decryption_passes_through_unencrypted_payloads() {
        // Payloads without the encryption marker come back unchanged.
        let bytes = vec![1, 2, 3, 4];
        assert_eq!(
            ProcessingRecipe::GoogleArtsDecrypt
                .apply(bytes.clone())
                .unwrap(),
            bytes
        );
    }
}
