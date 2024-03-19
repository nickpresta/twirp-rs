//! Support for serving Twirp APIs.
//!
//! There is not much to see in the documentation here. This API is meant to be used with
//! `twirp-build`. See <https://github.com/github/twirp-rs#usage> for details and an example.

use std::fmt::Debug;

use axum::body::Body;
use axum::response::IntoResponse;
use futures::Future;
use http::{Extensions, HeaderMap};
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::time::{Duration, Instant};

use crate::headers::{CONTENT_TYPE_JSON, CONTENT_TYPE_PROTOBUF};
use crate::{error, serialize_proto_message, GenericError, TwirpErrorResponse};

#[derive(Debug, Clone)]
pub struct Request<T> {
    message: T,
    extensions: Extensions,
}

#[derive(Debug, Clone)]
pub struct Response<T> {
    message: T,
    extensions: Extensions,
}

impl<T> Request<T> {
    pub fn new(message: T) -> Self {
        Request {
            message,
            extensions: Extensions::new(),
        }
    }

    pub fn into_inner(self) -> T {
        self.message
    }

    pub fn into_parts(self) -> (T, Extensions) {
        (self.message, self.extensions)
    }

    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.extensions
    }
}

impl<T> Response<T> {
    pub fn new(message: T) -> Self {
        Response {
            message,
            extensions: Extensions::new(),
        }
    }

    pub fn into_inner(self) -> T {
        self.message
    }

    pub fn into_parts(self) -> (T, Extensions) {
        (self.message, self.extensions)
    }

    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.extensions
    }
}

// TODO: Properly implement JsonPb (de)serialization as it is slightly different
// than standard JSON.
#[derive(Debug, Clone, Copy, Default)]
enum BodyFormat {
    #[default]
    JsonPb,
    Pb,
}

impl BodyFormat {
    fn from_content_type(headers: &HeaderMap) -> BodyFormat {
        match headers
            .get(hyper::header::CONTENT_TYPE)
            .map(|x| x.as_bytes())
        {
            Some(CONTENT_TYPE_PROTOBUF) => BodyFormat::Pb,
            _ => BodyFormat::JsonPb,
        }
    }
}

/// Entry point used in code generated by `twirp-build`.
pub(crate) async fn handle_request<S, F, Fut, Req, Resp>(
    service: S,
    req: hyper::Request<Body>,
    f: F,
) -> hyper::Response<Body>
where
    F: FnOnce(S, Request<Req>) -> Fut + Clone + Sync + Send + 'static,
    Fut: Future<Output = Result<Response<Resp>, TwirpErrorResponse>> + Send,
    Req: prost::Message + Default + serde::de::DeserializeOwned,
    Resp: prost::Message + serde::Serialize,
{
    let mut timings = req
        .extensions()
        .get::<Timings>()
        .copied()
        .unwrap_or_else(|| Timings::new(Instant::now()));

    let (req, resp_fmt) = match parse_request(req, &mut timings).await {
        Ok(pair) => pair,
        Err(err) => {
            // This is the only place we use tracing (would be nice to remove)
            // tracing::error!(?err, "failed to parse request");
            // TODO: We don't want to lose the underlying error here, but it might not be safe to
            // include in the response like this always.
            let mut twirp_err = error::malformed("bad request");
            twirp_err.insert_meta("error".to_string(), err.to_string());
            return twirp_err.into_response();
        }
    };

    let res = f(service, req).await;
    timings.set_response_handled();

    let mut resp = match write_response(res, resp_fmt) {
        Ok(resp) => resp,
        Err(err) => {
            let mut twirp_err = error::unknown("error serializing response");
            twirp_err.insert_meta("error".to_string(), err.to_string());
            return twirp_err.into_response();
        }
    };
    timings.set_response_written();

    resp.extensions_mut().insert(timings);
    resp
}

async fn parse_request<T>(
    req: hyper::Request<Body>,
    timings: &mut Timings,
) -> Result<(Request<T>, BodyFormat), GenericError>
where
    T: prost::Message + Default + DeserializeOwned,
{
    let (parts, body) = req.into_parts();
    let headers = &parts.headers;
    let format = BodyFormat::from_content_type(headers);
    let bytes = body.collect().await?.to_bytes();
    timings.set_received();
    let message = match format {
        BodyFormat::Pb => T::decode(&bytes[..])?,
        BodyFormat::JsonPb => serde_json::from_slice(&bytes)?,
    };
    timings.set_parsed();

    let mut request = Request::new(message);
    request.extensions_mut().extend(parts.extensions);
    Ok((request, format))
}

fn write_response<T>(
    response: Result<Response<T>, TwirpErrorResponse>,
    response_format: BodyFormat,
) -> Result<hyper::Response<Body>, GenericError>
where
    T: prost::Message + Serialize,
{
    let res = match response {
        Ok(response) => {
            let (message, response_extensions) = response.into_parts();
            let mut builder = hyper::Response::builder();
            if let Some(extensions_mut) = builder.extensions_mut() {
                extensions_mut.extend(response_extensions);
            }

            match response_format {
                BodyFormat::Pb => builder
                    .header(hyper::header::CONTENT_TYPE, CONTENT_TYPE_PROTOBUF)
                    .body(Body::from(serialize_proto_message(message)))?,
                BodyFormat::JsonPb => {
                    let data = serde_json::to_string(&message)?;
                    builder
                        .header(hyper::header::CONTENT_TYPE, CONTENT_TYPE_JSON)
                        .body(Body::from(data))?
                }
            }
        }
        Err(err) => err.into_response(),
    };
    Ok(res)
}

/// Axum handler function that returns 404 Not Found with a Twirp JSON payload.
///
/// `axum::Router`'s default fallback handler returns a 404 Not Found with no body content.
/// Use this fallback instead for full Twirp compliance.
///
/// # Usage
///
/// ```
/// use axum::Router;
///
/// # fn build_app(twirp_routes: Router) -> Router {
/// let app = Router::new()
///     .nest("/twirp", twirp_routes)
///     .fallback(twirp::server::not_found_handler);
/// # app }
/// ```
pub async fn not_found_handler() -> hyper::Response<Body> {
    error::bad_route("not found").into_response()
}

/// Contains timing information associated with a request.
/// To access the timings in a given request, use the [extensions](Request::extensions)
/// method and specialize to `Timings` appropriately.
#[derive(Debug, Clone, Copy)]
pub struct Timings {
    // When the request started.
    pub start: Instant,
    // When the request was received (headers and body).
    pub request_received: Option<Instant>,
    // When the request body was parsed.
    pub request_parsed: Option<Instant>,
    // When the response handler returned.
    pub response_handled: Option<Instant>,
    // When the response was written.
    pub response_written: Option<Instant>,
}

impl Timings {
    #[allow(clippy::new_without_default)]
    pub fn new(start: Instant) -> Self {
        Self {
            start,
            request_received: None,
            request_parsed: None,
            response_handled: None,
            response_written: None,
        }
    }

    fn set_received(&mut self) {
        self.request_received = Some(Instant::now());
    }

    fn set_parsed(&mut self) {
        self.request_parsed = Some(Instant::now());
    }

    fn set_response_handled(&mut self) {
        self.response_handled = Some(Instant::now());
    }

    fn set_response_written(&mut self) {
        self.response_written = Some(Instant::now());
    }

    pub fn received(&self) -> Option<Duration> {
        self.request_received.map(|x| x - self.start)
    }

    pub fn parsed(&self) -> Option<Duration> {
        match (self.request_parsed, self.request_received) {
            (Some(parsed), Some(received)) => Some(parsed - received),
            _ => None,
        }
    }

    pub fn response_handled(&self) -> Option<Duration> {
        match (self.response_handled, self.request_parsed) {
            (Some(handled), Some(parsed)) => Some(handled - parsed),
            _ => None,
        }
    }

    pub fn response_written(&self) -> Option<Duration> {
        match (self.response_written, self.response_handled) {
            (Some(written), Some(handled)) => Some(written - handled),
            (Some(written), None) => {
                if let Some(parsed) = self.request_parsed {
                    Some(written - parsed)
                } else {
                    self.request_received.map(|received| written - received)
                }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::test::*;

    use tower::Service;

    fn timings() -> Timings {
        Timings::new(Instant::now())
    }

    #[tokio::test]
    async fn test_bad_route() {
        let mut router = test_api_router();
        let req = hyper::Request::get("/nothing")
            .extension(timings())
            .body(Body::empty())
            .unwrap();

        let resp = router.call(req).await.unwrap();
        let data = read_err_body(resp.into_body()).await;
        assert_eq!(data, error::bad_route("not found"));
    }

    #[tokio::test]
    async fn test_ping_success() {
        let mut router = test_api_router();
        let resp = router.call(gen_ping_request("hi")).await.unwrap();
        assert!(resp.status().is_success(), "{:?}", resp);
        let data: PingResponse = read_json_body(resp.into_body()).await;
        assert_eq!(&data.name, "hi");
    }

    #[tokio::test]
    async fn test_ping_invalid_request() {
        let mut router = test_api_router();
        let req = hyper::Request::post("/twirp/test.TestAPI/Ping")
            .extension(timings())
            .body(Body::empty()) // not a valid request
            .unwrap();
        let resp = router.call(req).await.unwrap();
        assert!(resp.status().is_client_error(), "{:?}", resp);
        let data = read_err_body(resp.into_body()).await;

        // TODO: I think malformed should return some info about what was wrong
        // with the request, but we don't want to leak server errors that have
        // other details.
        let mut expected = error::malformed("bad request");
        expected.insert_meta(
            "error".to_string(),
            "EOF while parsing a value at line 1 column 0".to_string(),
        );
        assert_eq!(data, expected);
    }

    #[tokio::test]
    async fn test_boom() {
        let mut router = test_api_router();
        let req = serde_json::to_string(&PingRequest {
            name: "hi".to_string(),
        })
        .unwrap();
        let req = hyper::Request::post("/twirp/test.TestAPI/Boom")
            .extension(timings())
            .body(Body::from(req))
            .unwrap();
        let resp = router.call(req).await.unwrap();
        assert!(resp.status().is_server_error(), "{:?}", resp);
        let data = read_err_body(resp.into_body()).await;
        assert_eq!(data, error::internal("boom!"));
    }
}
