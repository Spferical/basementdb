use bincode::deserialize;
use bincode::serialize;
use data_encoding::BASE64;
use serde::{Deserialize, Serialize};
use std::io;

/// Objects that are StrSerialize are serialized by bincode -> b64encode,
/// not json.
pub trait StrSerialize<T>
where
    T: Serialize + for<'a> Deserialize<'a>,
{
    /// Converts Object -> bincode's serialize -> String.
    fn str_serialize(self_obj: &T) -> Result<String, io::Error> {
        let binary_encoding_raw = serialize(self_obj);
        return match binary_encoding_raw {
            Err(e) => Err(io::Error::new(
                io::ErrorKind::Other,
                format!("str_serialize failed: {:?}", e),
            )),
            Ok(binary_encoding) => Ok(BASE64.encode(&binary_encoding)),
        };
    }

    /// Converts String -> bincode's deserialize -> Object.
    fn str_deserialize(s: &String) -> Result<T, io::Error> {
        // Don't understand what's wrong with pattern matching here...
        let dec = BASE64.decode(s.as_bytes());

        match dec {
            Err(ae) => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("BASE64.decode failed: {:?}", ae),
                ));
            }
            Ok(_) => {}
        };

        let binary_encoding = dec.unwrap();

        match deserialize(&binary_encoding) {
            Err(e) => Err(io::Error::new(
                io::ErrorKind::Other,
                format!("str_deserialize failed: {:?}", e),
            )),
            Ok(deserialized) => return Ok(deserialized),
        }
    }
}
