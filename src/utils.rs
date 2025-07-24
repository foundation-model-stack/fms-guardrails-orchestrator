use hyper::Uri;
use serde::{Deserialize, Deserializer, de::DeserializeOwned};
use url::Url;
pub mod json;
pub mod tls;
pub mod trace;

/// Simple trait used to extend `url::Url` with functionality to transform into `hyper::Uri`.
pub trait AsUriExt {
    fn as_uri(&self) -> Uri;
}

impl AsUriExt for Url {
    fn as_uri(&self) -> Uri {
        Uri::try_from(self.to_string()).unwrap()
    }
}

/// Serde helper to deserialize one or many [`T`] to [`Vec<T>`].
pub fn one_or_many<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    T: DeserializeOwned,
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany<T> {
        One(T),
        Many(Vec<T>),
    }
    let v: OneOrMany<T> = Deserialize::deserialize(deserializer)?;
    match v {
        OneOrMany::One(value) => Ok(vec![value]),
        OneOrMany::Many(values) => Ok(values),
    }
}
