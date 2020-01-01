use bincode::serialize;
use serde::{Deserialize, Serialize};
use sodiumoxide::crypto::sign::{gen_keypair, sign_detached, Signature};
use sodiumoxide::crypto::sign::{verify_detached, PublicKey, SecretKey};

use crate::str_serialize::StrSerialize;

/// A signed object.
///
/// Construct one of these by creating a public-private keypair
/// and then using `Signed::new()`
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Hash, Clone)]
pub struct Signed<T: Serialize> {
    pub base: T,
    pub signature: Signature,
}

/// A Public-Private KeyPair
pub type KeyPair = (PublicKey, SecretKey);

/// A Public Key
pub type Public = PublicKey;

/// A Private Key
pub type Private = SecretKey;

/// Use this function to create a public-private keypair
pub fn gen_keys() -> KeyPair {
    gen_keypair()
}

impl<T: Serialize> Signed<T> {
    /// Create new `Signed<>` object; signed with private key `private_key`.
    pub fn new(base: T, private_key: &Private) -> Signed<T> {
        let m: Vec<u8> = serialize(&base).unwrap();

        let sig = sign_detached(&m, private_key);
        Signed {
            base,
            signature: sig,
        }
    }

    /// Verify that an object is signed with a public key `public_key`.
    pub fn verify(&self, public_key: &Public) -> bool {
        let m: Vec<u8> = serialize(&(self.base)).unwrap();

        verify_detached(&(self.signature), &m, public_key)
    }
}

/// Signed objects are string serializable too. This is important
/// because we need to be able to send signed messages over the
/// network.
impl<T> StrSerialize<Signed<T>> for Signed<T> where for<'a> T: Serialize + Deserialize<'a> {}

#[cfg(test)]
mod tests {
    use super::{gen_keys, Signed, StrSerialize};

    #[test]
    fn create_verify() {
        let (public_key, private_key) = gen_keys();

        let obj = 3;

        let signed_obj = Signed::new(obj, &private_key);
        assert_eq!(signed_obj.base, obj);

        assert_eq!(signed_obj.verify(&public_key), true)
    }

    #[test]
    fn signed_serialization_works() {
        let (_, private_key) = gen_keys();

        let obj = 7;

        let signed_obj = Signed::new(obj, &private_key);
        let str_rep: String = Signed::str_serialize(&signed_obj).unwrap();
        let deserialized_signed_obj = Signed::str_deserialize(&str_rep).unwrap();
        assert_eq!(deserialized_signed_obj, signed_obj);
    }
}
