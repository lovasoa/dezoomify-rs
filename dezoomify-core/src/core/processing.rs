//! Application of processing recipes to fetched tile payloads.

use custom_error::custom_error;

use super::model::ProcessingRecipe;
use crate::google_arts_and_culture::decryption;

custom_error! {pub ProcessingError
    GoogleArtsDecrypt{source: decryption::InvalidEncryptedImage} =
        "unable to decrypt a Google Arts & Culture tile: {source}",
    Unsupported{name: String} = "unsupported tile processing recipe: {name}",
}

impl ProcessingRecipe {
    /// Apply this recipe to a fetched tile payload, returning the bytes that
    /// should be decoded as an image.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessingError::Unsupported`] for [`ProcessingRecipe::Named`]
    /// values this crate does not know; applications that inject their own
    /// recipes must dispatch on the name themselves before calling this.
    pub fn apply(&self, bytes: Vec<u8>) -> Result<Vec<u8>, ProcessingError> {
        match self {
            Self::None => Ok(bytes),
            Self::GoogleArtsDecrypt => Ok(decryption::decrypt(bytes)?),
            Self::Named(name) => Err(ProcessingError::Unsupported {
                name: name.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::StableId;

    #[test]
    fn none_is_the_identity() {
        let bytes = vec![1, 2, 3];
        assert_eq!(ProcessingRecipe::None.apply(bytes.clone()).unwrap(), bytes);
    }

    #[test]
    fn unknown_names_are_rejected_with_the_name() {
        let error = ProcessingRecipe::Named(StableId::from("custom-recipe"))
            .apply(vec![])
            .unwrap_err();
        assert!(matches!(
            error,
            ProcessingError::Unsupported { name } if name == "custom-recipe"
        ));
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
