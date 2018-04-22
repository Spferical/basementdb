extern crate sodiumoxide;

use self::sodiumoxide::crypto::sign::{verify_detached, PublicKey, SecretKey};
use self::sodiumoxide::crypto::sign::{gen_keypair, sign_detached, Signature};
use serde::{Deserialize, Serialize};
use tarpc::bincode::Infinite;
use tarpc::bincode::serialize;

/// A signed object.
///
/// Construct one of these by creating a public-private keypair
/// and then using `Signed::new()`
#[derive(Serialize, Deserialize, Debug)]
pub struct Signed<T: Serialize> {
    base: T,
    signature: Signature,
}

/// A Public-Private KeyPair
pub type KeyPair = (PublicKey, SecretKey);

/// A Public Key
pub type Public = PublicKey;

/// A Private Key
pub type Private = SecretKey;

/// Use this function to create a public-private keypair
pub fn gen_keys() -> KeyPair {
    return gen_keypair();
}

impl<T: Serialize> Signed<T> {
    /// Create new `Signed<>` object; signed with private key `private_key`.
    pub fn new(base: T, private_key: &Private) -> Signed<T> {
        let m: Vec<u8> = serialize(&base, Infinite).unwrap();

        let sig = sign_detached(&m, private_key);
        return Signed {
            base: base,
            signature: sig,
        };
    }

    /// Verify that an object is signed with a public key `public_key`.
    /// If so, return the underlying object, otherwise, don't.
    pub fn verify(self, public_key: &Public) -> Option<T> {
        let m: Vec<u8> = serialize(&(self.base), Infinite).unwrap();

        if verify_detached(&(self.signature), &m, public_key) {
            return Some(self.base);
        } else {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{gen_keys, Signed};

    #[test]
    fn create_verify() {
        let (public_key, private_key) = gen_keys();

        let obj = 3;

        let signed_obj = Signed::new(obj, &private_key);
        assert_eq!(signed_obj.base, obj);

        let read_obj = signed_obj.verify(&public_key);
        assert_eq!(read_obj.unwrap(), obj);
    }
}
