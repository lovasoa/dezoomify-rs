use super::EncryptedKrpanoError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KencBranch {
    /// `KENCRUZR` — old engine, Z body transform (modified Base85 + byte-decrypt + LZ4).
    OldZ,
    /// `KENCPUZR` — modern engine, Z body transform (same pipeline, modern key).
    ModernZ,
    /// `KENCRURR` — modern engine, R/R body transform.
    RR,
    /// `KENCPUPR` — modern engine, P/P body transform.
    PP,
    /// `KENC..B.` — any mode with byte-6 key_source `B` (Base64 + byte-decrypt + UTF-8).
    B,
    /// Header bytes not matching any known branch.
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KencHeader {
    /// The full eight-byte `KENC....` marker at the start of the payload.
    pub raw: String,
    /// First mode byte after `KENC`.
    pub mode: char,
    /// Encoding/compression byte after the mode.
    pub encoding: char,
    /// Key/source byte after the encoding.
    pub key_source: char,
    /// Final flag byte. Observed samples end with `R`.
    pub flags: char,
}

impl KencHeader {
    pub const LEN: usize = 8;

    /// The constant subtracted from header byte character codes to derive branch
    /// arithmetic values.  `k = (r << 4) + (r << 2)` where `r = 4`.
    const K: u32 = 80;

    pub fn parse(payload: &str) -> Result<Self, EncryptedKrpanoError> {
        let header = payload
            .get(..Self::LEN)
            .ok_or(EncryptedKrpanoError::HeaderTooShort { len: payload.len() })?;
        if !header.starts_with("KENC") {
            return Err(EncryptedKrpanoError::InvalidHeader {
                header: header.to_string(),
            });
        }
        let raw = header.to_string();
        let fields: Vec<_> = raw.chars().skip(4).collect();
        Ok(Self {
            raw,
            mode: fields[0],
            encoding: fields[1],
            key_source: fields[2],
            flags: fields[3],
        })
    }

    /// Classify the header into a known branch family.
    ///
    /// The branch is determined from byte 6 (`key_source`) using `charCode - 80`:
    /// `10` → Z (Base85 + decrypt + LZ4),
    /// `2`  → R/R,
    /// `0`  → P/P,
    /// `-14`→ B (Base64).
    /// Mode value from byte 4 (`mode`) via `(charCode - 80) >> 1` distinguishes
    /// Old (`1`) from Modern (`0`) within Z.
    pub fn branch(&self) -> KencBranch {
        let byte6_value = (self.key_source as u32).wrapping_sub(Self::K) as i32;
        let mode_value = ((self.mode as u32).wrapping_sub(Self::K) >> 1) as i32;
        match byte6_value {
            10 => {
                if mode_value == 1 {
                    KencBranch::OldZ
                } else {
                    KencBranch::ModernZ
                }
            }
            2 => KencBranch::RR,
            0 => KencBranch::PP,
            -14 => KencBranch::B,
            _ => KencBranch::Unknown,
        }
    }

    pub fn payload<'a>(&self, payload: &'a str) -> &'a str {
        &payload[Self::LEN..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_kenc_headers() {
        let public = KencHeader::parse("KENCPUPRpayload").unwrap();
        assert_eq!(public.payload("KENCPUPRpayload"), "payload");
        assert_eq!(
            public,
            KencHeader {
                raw: "KENCPUPR".to_string(),
                mode: 'P',
                encoding: 'U',
                key_source: 'P',
                flags: 'R',
            }
        );
        assert_eq!(
            KencHeader::parse("KENCRURRpayload").unwrap(),
            KencHeader {
                raw: "KENCRURR".to_string(),
                mode: 'R',
                encoding: 'U',
                key_source: 'R',
                flags: 'R',
            }
        );
    }

    #[test]
    fn rejects_invalid_kenc_headers() {
        assert!(matches!(
            KencHeader::parse("short"),
            Err(EncryptedKrpanoError::HeaderTooShort { len: 5 })
        ));
        assert!(matches!(
            KencHeader::parse("NOTKENC!payload"),
            Err(EncryptedKrpanoError::InvalidHeader { .. })
        ));
    }

    #[test]
    fn classifies_every_header_branch() {
        // Verify KENCRUZR -> OldZ
        let old_z = KencHeader {
            raw: "KENCRUZR".to_string(),
            mode: 'R',
            encoding: 'U',
            key_source: 'Z',
            flags: 'R',
        };
        assert_eq!(old_z.branch(), KencBranch::OldZ);

        // KENCPUZR -> ModernZ
        let modern_z = KencHeader {
            raw: "KENCPUZR".to_string(),
            mode: 'P',
            encoding: 'U',
            key_source: 'Z',
            flags: 'R',
        };
        assert_eq!(modern_z.branch(), KencBranch::ModernZ);

        // KENCRURR -> RR
        assert_eq!(
            KencHeader {
                raw: "KENCRURR".to_string(),
                mode: 'R',
                encoding: 'U',
                key_source: 'R',
                flags: 'R',
            }
            .branch(),
            KencBranch::RR
        );

        // KENCPUPR -> PP
        assert_eq!(
            KencHeader {
                raw: "KENCPUPR".to_string(),
                mode: 'P',
                encoding: 'U',
                key_source: 'P',
                flags: 'R',
            }
            .branch(),
            KencBranch::PP
        );

        // KENCXXBZ -> B
        assert_eq!(
            KencHeader {
                raw: "KENCXXBZ".to_string(),
                mode: 'X',
                encoding: 'X',
                key_source: 'B',
                flags: 'Z',
            }
            .branch(),
            KencBranch::B
        );

        // Unknown byte-6 value
        assert_eq!(
            KencHeader {
                raw: "KENCXXXZ".to_string(),
                mode: 'X',
                encoding: 'X',
                key_source: 'X',
                flags: 'Z',
            }
            .branch(),
            KencBranch::Unknown
        );
    }
}
