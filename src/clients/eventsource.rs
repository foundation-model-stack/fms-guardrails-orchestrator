use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use eventsource_stream::Eventsource as _;
use futures::{future::BoxFuture, stream::BoxStream, Stream, TryStreamExt};
use futures_timer::Delay;
use hyper::{
    header::{HeaderName, HeaderValue, CONTENT_TYPE},
    HeaderMap, StatusCode,
};
use pin_project_lite::pin_project;
use url::Url;

use crate::clients::{
    http::{RequestLike, Response},
    Error, HttpClient,
};

const DEFAULT_RETRY: u64 = 5000;

type ResponseFuture = BoxFuture<'static, Result<Response, Error>>;
type EventStream = BoxStream<'static, Result<eventsource_stream::Event, Error>>;

pin_project! {
    #[project = EventSourceProjection]
    pub struct EventSource<T: RequestLike> {
        client: HttpClient,
        method: hyper::Method,
        url: Url,
        headers: HeaderMap,
        request_body: T,
        #[pin]
        next_response: Option<ResponseFuture>,
        #[pin]
        cur_stream: Option<EventStream>,
        #[pin]
        delay: Option<Delay>,
        is_closed: bool,
        retry: Duration,
        last_event_id: String,
        last_retry: Option<(usize, Duration)>,
        max_duration: Option<Duration>,
        max_retries: Option<usize>,
        factor: f64,
    }
}

impl<'a, T: RequestLike + Send + Sync + 'static> EventSourceProjection<'a, T> {
    fn clear_fetch(&mut self) {
        self.next_response.take();
        self.cur_stream.take();
    }

    fn retry_fetch(&mut self) -> Result<(), Error> {
        self.cur_stream.take();
        self.headers.insert(
            HeaderName::from_static("last-event-id"),
            HeaderValue::from_str(self.last_event_id).unwrap(),
        );
        match *self.method {
            hyper::Method::GET => {
                let res_fut = Box::pin(self.client.clone().get(
                    self.url.clone(),
                    self.headers.clone(),
                    self.request_body.clone(),
                ));
                self.next_response.replace(res_fut);
            }
            hyper::Method::POST => {
                let res_fut = Box::pin(self.client.clone().post(
                    self.url.clone(),
                    self.headers.clone(),
                    self.request_body.clone(),
                ));
                self.next_response.replace(res_fut);
            }
            _ => panic!("unsupported HTTP streaming method"),
        };
        Ok(())
    }

    fn handle_response(&mut self, res: Response) {
        self.last_retry.take();
        let mut stream = res.bytes_stream().eventsource();
        stream.set_last_event_id(self.last_event_id.clone());
        self.cur_stream.replace(Box::pin(
            stream.map_err(|e| Error::internal("client stream response error", e)),
        ));
    }

    fn handle_event(&mut self, event: &eventsource_stream::Event) {
        *self.last_event_id = event.id.clone();
        if let Some(duration) = event.retry {
            *self.retry = duration;
            if let Some(max_duration) = *self.max_duration {
                *self.max_duration = Some(max_duration.max(duration))
            }
        }
    }

    fn handle_error(&mut self, _error: &Error) {
        self.clear_fetch();
        let retry_delay = if let Some((retry_num, last_duration)) = *self.last_retry {
            if self.max_retries.is_none() || retry_num < self.max_retries.unwrap() {
                let duration = last_duration.mul_f64(*self.factor);
                if let Some(max_duration) = *self.max_duration {
                    Some(duration.min(max_duration))
                } else {
                    Some(duration)
                }
            } else {
                None
            }
        } else {
            Some(*self.retry)
        };

        match retry_delay {
            Some(retry_delay) => {
                let retry_num = self.last_retry.map(|retry| retry.0).unwrap_or(1);
                *self.last_retry = Some((retry_num, retry_delay));
                self.delay.replace(Delay::new(retry_delay));
            }
            None => {
                *self.is_closed = true;
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Event {
    Open,
    Message(eventsource_stream::Event),
}

impl From<eventsource_stream::Event> for Event {
    fn from(event: eventsource_stream::Event) -> Self {
        Event::Message(event)
    }
}

impl<T: RequestLike + Send + Sync + 'static> Stream for EventSource<T> {
    type Item = Result<Event, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        if *this.is_closed {
            return Poll::Ready(None);
        }

        if let Some(delay) = this.delay.as_mut().as_pin_mut() {
            match delay.poll(cx) {
                Poll::Ready(_) => {
                    this.delay.take();
                    if let Err(err) = this.retry_fetch() {
                        *this.is_closed = true;
                        return Poll::Ready(Some(Err(err)));
                    }
                }
                Poll::Pending => return Poll::Pending,
            }
        }

        if let Some(response_future) = this.next_response.as_mut().as_pin_mut() {
            match response_future.poll(cx) {
                Poll::Ready(Ok(res)) => {
                    this.clear_fetch();
                    match check_response(res) {
                        Ok(res) => {
                            this.handle_response(res);
                            return Poll::Ready(Some(Ok(Event::Open)));
                        }
                        Err(err) => {
                            *this.is_closed = true;
                            return Poll::Ready(Some(Err(err)));
                        }
                    }
                }
                Poll::Ready(Err(err)) => {
                    this.handle_error(&err);
                    return Poll::Ready(Some(Err(err)));
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }

        match this
            .cur_stream
            .as_mut()
            .as_pin_mut()
            .unwrap()
            .as_mut()
            .poll_next(cx)
        {
            Poll::Ready(Some(Err(err))) => {
                this.handle_error(&err);
                Poll::Ready(Some(Err(err)))
            }
            Poll::Ready(Some(Ok(event))) => {
                this.handle_event(&event);
                Poll::Ready(Some(Ok(event.into())))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

fn check_response(response: Response) -> Result<Response, Error> {
    match response.status() {
        StatusCode::OK => {}
        code => {
            return Err(Error::Http {
                code,
                message: "".to_string(),
            });
        }
    }
    let content_type = if let Some(content_type) = response.headers().get(&CONTENT_TYPE) {
        content_type
    } else {
        return Err(Error::Internal {
            error: "client response missing Content-Type".to_string(),
        });
    };
    if content_type
        .to_str()
        .map_err(|_| ())
        .and_then(|s| s.parse::<mime::Mime>().map_err(|_| ()))
        .map(|mime_type| {
            matches!(
                (mime_type.type_(), mime_type.subtype()),
                (mime::TEXT, mime::EVENT_STREAM)
            )
        })
        .unwrap_or(false)
    {
        Ok(response)
    } else {
        Err(Error::internal(
            "client response has invalid Content-Type",
            content_type.clone().to_str().unwrap(),
        ))
    }
}

impl<T: RequestLike + Send + Sync + 'static> EventSource<T> {
    pub async fn from_client(
        client: HttpClient,
        url: Url,
        headers: HeaderMap,
        request_body: T,
    ) -> Pin<Box<EventSource<T>>> {
        Box::pin(EventSource {
            client,
            method: Default::default(),
            url,
            headers,
            request_body,
            next_response: None,
            cur_stream: None,
            delay: None,
            is_closed: false,
            last_event_id: String::new(),
            retry: Duration::from_millis(300),
            last_retry: None,
            max_duration: Some(Duration::from_secs(5)),
            max_retries: None,
            factor: 2.,
        })
    }
}
