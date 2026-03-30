pub use bytes::Bytes;
pub use http_body_util::Full;
pub use hyper::body::Incoming;
pub use hyper::server::conn::http1;
pub use hyper::service::service_fn;
pub use hyper::{Method, Request, Response, StatusCode, Uri, Version};
pub use hyper_util::rt::TokioIo;

pub fn adapter_name() -> &'static str {
    "hyper_adapter"
}
