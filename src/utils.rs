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

/// Serde helper to deserialize value from environment variable.
pub fn from_env<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let env_name: Option<String> = Option::deserialize(deserializer)?;
    if let Some(env_name) = env_name {
        let value = std::env::var(&env_name)
            .map_err(|_| serde::de::Error::custom(format!("env var `{env_name}` not found")))?;
        Ok(Some(value))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::from_env;

    #[derive(Debug, Deserialize)]
    pub struct Config {
        #[serde(default, deserialize_with = "from_env")]
        pub api_token: Option<String>,
    }

    #[test]
    fn test_from_env() -> Result<(), Box<dyn std::error::Error>> {
        // Test no value
        let config: Config = serde_json::from_value(json!({}))?;
        assert_eq!(config.api_token, None);

        // Test invalid value
        let config: Result<Config, serde_json::error::Error> = serde_json::from_value(json!({
            "api_token": "DOES_NOT_EXIST"
        }));
        assert!(config.is_err_and(|err| err.to_string() == "env var `DOES_NOT_EXIST` not found"));

        // Test valid value
        unsafe {
            std::env::set_var("CLIENT_API_TOKEN", "token");
        }
        let config: Config = serde_json::from_value(json!({
            "api_token": "CLIENT_API_TOKEN"
        }))?;
        assert_eq!(config.api_token, Some("token".into()));

        Ok(())
    }
}
