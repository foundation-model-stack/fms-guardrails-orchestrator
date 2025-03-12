use serde::Serialize;

/// Serialize the given data structure as a String of ND-JSON.
///
/// # Errors
///
/// Serialization can fail if `T`'s implementation of `Serialize` decides to
/// fail, or if `T` contains a map with non-string keys.
#[inline]
pub fn to_nd_string<T>(value: &T) -> Result<String, serde_json::Error>
where
    T: ?Sized + Serialize,
{
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    let string = unsafe { String::from_utf8_unchecked(bytes) };
    Ok(string)
}
