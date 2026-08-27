//! Formatting of byte slices for logs (Python-like `b'...'` display).

use std::fmt;

/// A wrapper struct to display binary data in a Python-like format
///
/// This displays binary data as b'...' with printable ASCII characters shown as-is
/// and non-printable characters shown as escape sequences like \x00, \x01, etc.
pub struct BinaryDisplay<'a>(pub &'a [u8]);

impl fmt::Display for BinaryDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "b'")?;

        for &byte in self.0 {
            match byte {
                // Printable ASCII characters (space to tilde)
                0x20..=0x7e => write!(f, "{}", byte as char)?,
                // Non-printable characters as hex escape sequences
                _ => write!(f, "\\x{byte:02x}")?,
            }
        }

        write!(f, "'")
    }
}

impl fmt::Debug for BinaryDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Convenience function to create a `BinaryDisplay` wrapper
pub fn display_bytes<T: AsRef<[u8]> + ?Sized>(bytes: &T) -> BinaryDisplay<'_> {
    BinaryDisplay(bytes.as_ref())
}
