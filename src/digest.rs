use bincode::serialize;
use serde::Serialize;
use sodiumoxide::crypto::hash::{Digest, sha512};

/// A 512-byte SHA-512 hash
pub type HashDigest = [u64; 4];

/// The hash chain
pub type HashChain = Vec<HashDigest>;

fn u8_conv(a: u8) -> u64 {
    return (a as u64) & 0xff;
}

fn u8to64(p: [u8; 64], i: usize) -> u64 {
    return u8_conv(p[i]) | u8_conv(p[i + 1]) << 8 | u8_conv(p[i + 2]) << 16
        | u8_conv(p[i + 3]) << 24 | u8_conv(p[i + 4]) << 32 | u8_conv(p[i + 5]) << 40
        | u8_conv(p[i + 6]) << 48 | u8_conv(p[i + 7]) << 56;
}

fn conv(digest: [u8; 64]) -> HashDigest {
    return [
        u8to64(digest, 0),
        u8to64(digest, 8),
        u8to64(digest, 16),
        u8to64(digest, 24),
    ];
}

/// The D() hash function from the Zeno paper. For the case of
/// d(h_n, d(REQ_n+1)), just pass in a array of two HashDigests.
pub fn d<T: Serialize>(obj: T) -> HashDigest {
    let Digest(digest) = sha512::hash(&serialize(&obj).unwrap());
    return conv(digest);
}

#[cfg(test)]
mod tests {
    use super::D;

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
