use std::fmt::Display;

use super::id::ID;

pub fn ok_or_missing_field<T>(
    opt: Option<T>,
    field_name: impl Display,
) -> Result<T, bendy::decoding::Error> {
    opt.ok_or_else(|| bendy::decoding::Error::missing_field(field_name))
}

pub fn vec_to_id(v: Vec<u8>) -> ID {
    // TODO should not unwrap, improve error handling
    ID(v.try_into().unwrap())
}
