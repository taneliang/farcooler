//! UUIDv7 values are encoded as validated 16-byte fields, never strings.

use bytes::Bytes;
use uuid::Uuid;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("id field must be exactly 16 bytes, got {0}")]
pub struct InvalidId(pub usize);

/// Fresh time-ordered id.
pub fn new_id() -> Bytes {
    Bytes::copy_from_slice(Uuid::now_v7().as_bytes())
}

pub fn to_bytes(id: Uuid) -> Bytes {
    Bytes::copy_from_slice(id.as_bytes())
}

/// Validate a wire id field before it is used as an identity.
pub fn from_bytes(raw: &[u8]) -> Result<Uuid, InvalidId> {
    let arr: [u8; 16] = raw.try_into().map_err(|_| InvalidId(raw.len()))?;
    Ok(Uuid::from_bytes(arr))
}

/// Short, stable display form for logs and CLI output.
pub fn short(raw: &[u8]) -> String {
    match from_bytes(raw) {
        Ok(u) => u.simple().to_string()[..8].to_string(),
        Err(_) => "????????".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let b = new_id();
        let u = from_bytes(&b).unwrap();
        assert_eq!(to_bytes(u), b);
    }

    #[test]
    fn wrong_length_rejected() {
        assert_eq!(from_bytes(&[1, 2, 3]), Err(InvalidId(3)));
        assert_eq!(from_bytes(&[]), Err(InvalidId(0)));
    }

    #[test]
    fn ids_are_time_ordered() {
        let a = from_bytes(&new_id()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = from_bytes(&new_id()).unwrap();
        assert!(a < b, "uuidv7 must sort by creation time");
    }
}
