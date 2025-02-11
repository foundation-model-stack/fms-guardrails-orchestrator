use std::pin::Pin;

use futures::Stream;

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;
