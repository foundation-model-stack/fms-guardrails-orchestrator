use hyper::Uri;
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
