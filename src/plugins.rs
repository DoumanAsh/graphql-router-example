//! Plugin repository

use apollo_router_core::{Plugin, SubgraphRequest, SubgraphResponse};
use hyper::http::header::{
    HeaderName, CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, HOST, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TE, TRAILER,
    TRANSFER_ENCODING, UPGRADE,
};
use tower::util::BoxService;
use tower::{BoxError, ServiceExt};

use core::future::{ready, Future};
use core::pin::Pin;
use core::task;

static RESERVED_HEADERS: [HeaderName; 10] = [
    CONNECTION,
    PROXY_AUTHENTICATE,
    PROXY_AUTHORIZATION,
    TE,
    TRAILER,
    TRANSFER_ENCODING,
    UPGRADE,
    CONTENT_LENGTH,
    CONTENT_TYPE,
    HOST,
];

pub struct PropagateHeaders;

impl Plugin for PropagateHeaders {
    type Config = ();

    #[inline(always)]
    fn new<'a>(_: Self::Config) -> Pin<Box<dyn Future<Output = Result<Self, BoxError>> + Send + 'a>>
    where
        Self: 'a,
    {
        Box::pin(ready(Ok(Self)))
    }

    #[inline(always)]
    fn subgraph_service(
        &mut self,
        _subgraph_name: &str,
        service: BoxService<SubgraphRequest, SubgraphResponse, BoxError>,
    ) -> BoxService<SubgraphRequest, SubgraphResponse, BoxError> {
        tower::ServiceBuilder::new().layer(Self).service(service).boxed()
    }
}

impl<S> tower::Layer<S> for PropagateHeaders {
    type Service = PropagateHeadersService<S>;

    #[inline(always)]
    fn layer(&self, inner: S) -> Self::Service {
        PropagateHeadersService { inner }
    }
}

pub struct PropagateHeadersService<S> {
    inner: S,
}

impl<S: tower::Service<SubgraphRequest>> tower::Service<SubgraphRequest> for PropagateHeadersService<S> {
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    #[inline(always)]
    fn poll_ready(&mut self, cx: &mut task::Context<'_>) -> task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    #[inline(always)]
    fn call(&mut self, mut req: SubgraphRequest) -> Self::Future {
        let headers = req.subgraph_request.headers_mut();
        for (key, value) in req.originating_request.headers().iter() {
            if !RESERVED_HEADERS.contains(key) {
                headers.insert(key, value.clone());
            }
        }
        self.inner.call(req)
    }
}
