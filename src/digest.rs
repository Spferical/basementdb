use bincode::serialize;
use serde::Serialize;
use sodiumoxide::crypto::hash::{sha512, Digest};

/// A 512-byte SHA-512 hash
pub type HashDigest = [u64; 4];

/// The hash chain
pub type HashChain = Vec<HashDigest>;

fn u8to64(p: [u8; 64], i: usize) -> u64 {
    u64::from(p[i])
        | u64::from(p[i + 1]) << 8
        | u64::from(p[i + 2]) << 16
        | u64::from(p[i + 3]) << 24
        | u64::from(p[i + 4]) << 32
        | u64::from(p[i + 5]) << 40
        | u64::from(p[i + 6]) << 48
        | u64::from(p[i + 7]) << 56
}

fn conv(digest: [u8; 64]) -> HashDigest {
    [
        u8to64(digest, 0),
        u8to64(digest, 8),
        u8to64(digest, 16),
        u8to64(digest, 24),
    ]
}

/// The D() hash function from the Zeno paper. For the case of
/// d(h_n, d(REQ_n+1)), just pass in a tuple of two HashDigests.
pub fn d<T: Serialize>(obj: T) -> HashDigest {
    let Digest(digest) = sha512::hash(&serialize(&obj).unwrap());
    conv(digest)
}

#[cfg(test)]
mod tests {
    use super::d as D;

    #[test]
    fn simple_hash_digest() {
        assert!(D(3) != [0; 4]);
        assert!(D(3) != D(2));
        assert!(D(3) == D(3));
    }

    #[test]
    fn pair_hash_digest() {
        assert!(D([D(3), D(2)]) != [0; 4]);
        assert!(D([D(3), D(2)]) != D([D(2), D(3)]));
        assert!(D([D(3), D(2)]) == D([D(3), D(2)]));
    }
}
