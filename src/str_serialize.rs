use bincode::deserialize;
use bincode::serialize;
use data_encoding::BASE64;
use serde::{Deserialize, Serialize};
use std::io;

pub trait StrSerialize<T>
where
    T: Serialize + for<'a> Deserialize<'a>,
{
    fn str_serialize(self_obj: &T) -> Result<String, io::Error> {
        let binary_encoding_raw = serialize(self_obj);
        return match binary_encoding_raw {
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, "str_serialize failed")),
            Ok(binary_encoding) => Ok(BASE64.encode(&binary_encoding)),
        };
    }

    fn str_deserialize(s: &String) -> Result<T, io::Error> {
        // Don't understand what's wrong with pattern matching here...
        let dec = BASE64.decode(s.as_bytes());

        match dec {
            Err(ae) => {
                return Err(io::Error::new(io::ErrorKind::Other, "BASE64.decode failed"));
            }
            Ok(_) => {}
        };

        let binary_encoding = dec.unwrap();

        match deserialize(&binary_encoding) {
            Err(e) => Err(io::Error::new(
                io::ErrorKind::Other,
                "str_deserialize failed",
            )),
            Ok(deserialized) => return Ok(deserialized),
        }
    }
}
