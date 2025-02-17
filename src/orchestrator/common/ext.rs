use std::future::Future;

pub trait OptionExt<T> {
    /// Maps an [`Option<T>`] to [`Option<U>`] by applying an async function to a value
    /// if `Some` or returns `None`.
    async fn async_map<U, F, Fut>(self, f: F) -> Option<U>
    where
        F: FnOnce(T) -> Fut,
        Fut: Future<Output = U>;
}

impl<T> OptionExt<T> for Option<T> {
    async fn async_map<U, F, Fut>(self, f: F) -> Option<U>
    where
        F: FnOnce(T) -> Fut,
        Fut: Future<Output = U>,
    {
        match self {
            Some(t) => {
                let u = f(t).await;
                Some(u)
            }
            None => None,
        }
    }
}

pub trait VecExt {
    /// Returns true if the vector contains elements.
    fn is_not_empty(&self) -> bool;
}

impl<T> VecExt for Vec<T> {
    fn is_not_empty(&self) -> bool {
        !self.is_empty()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn test_option_ext() {
        let foo = Some(5);
        let bar = foo.async_map(|value| async move { value * 2 }).await;
        assert_eq!(bar.unwrap(), 10);
    }
}
