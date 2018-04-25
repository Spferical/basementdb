use bincode::serialize;
use serde::{Deserialize, Serialize};
use sodiumoxide::crypto::sign::{verify_detached, PublicKey, SecretKey};
use sodiumoxide::crypto::sign::{gen_keypair, sign_detached, Signature};

use str_serialize::StrSerialize;

/// A signed object.
///
/// Construct one of these by creating a public-private keypair
/// and then using `Signed::new()`
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Hash)]
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
        let m: Vec<u8> = serialize(&base).unwrap();

        let sig = sign_detached(&m, private_key);
        return Signed {
            base: base,
            signature: sig,
        };
    }

    /// Verify that an object is signed with a public key `public_key`.
    /// If so, return the underlying object, otherwise, don't.
    pub fn verify(self, public_key: &Public) -> Option<T> {
        let m: Vec<u8> = serialize(&(self.base)).unwrap();

        if verify_detached(&(self.signature), &m, public_key) {
            return Some(self.base);
        } else {
            return None;
        }
    }
}

/// Signed objects are string serializable too. This is important
/// because we need to be able to send signed messages over the
/// network.
impl<T> StrSerialize<Signed<T>> for Signed<T>
where
    for<'a> T: Serialize + Deserialize<'a>,
{
}

#[cfg(test)]
mod tests {
    use super::{gen_keys, Signed, StrSerialize};

    #[test]
    fn create_verify() {
        let (public_key, private_key) = gen_keys();

        let obj = 3;

        let signed_obj = Signed::new(obj, &private_key);
        assert_eq!(signed_obj.base, obj);

        let read_obj = signed_obj.verify(&public_key);
        assert_eq!(read_obj.unwrap(), obj);
    }

    #[test]
    fn signed_serialization_works() {
        let (public_key, private_key) = gen_keys();

        let obj = 7;

        let signed_obj = Signed::new(obj, &private_key);
        let str_rep: String = Signed::str_serialize(&signed_obj).unwrap();
        let deserialized_signed_obj = Signed::str_deserialize(&str_rep).unwrap();
        assert_eq!(deserialized_signed_obj, signed_obj);
    }
}
