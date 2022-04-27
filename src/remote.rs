use apollo_router_core::{SubgraphRequest, SubgraphResponse};
use hyper::client::HttpConnector;
use hyper::header::HeaderValue;
use hyper::http::header::{ACCEPT, CONTENT_TYPE};
use hyper_rustls::HttpsConnector;
use tower_service::Service;

use crate::BuildGraph;

use core::future::Future;
use core::pin::Pin;
use core::task;

#[allow(clippy::declare_interior_mutable_const)]
const APPLICATION_JSON: HeaderValue = HeaderValue::from_static("application/json");

#[derive(Clone, Copy)]
struct Config {
    max_retry_num: usize,
    max_redirect_num: usize,
}

///Remote subgraph builder
pub struct RemoteGraphBuilder {
    url: hyper::Uri,
    name: &'static str,
    config: Config,
}

impl RemoteGraphBuilder {
    #[inline(always)]
    pub fn new(name: &'static str, url: hyper::Uri) -> Self {
        Self {
            name,
            url,
            config: Config {
                max_redirect_num: 10,
                max_retry_num: 2,
            },
        }
    }

    ///Sets retry number.
    ///
    ///Retry happens only when there is network issue or service is temp unavailable.
    ///
    ///Default is 2.
    pub fn max_retry_num(mut self, max_retry_num: usize) -> Self {
        self.config.max_retry_num = max_retry_num;
        self
    }

    #[inline(always)]
    ///Builds service
    pub fn build(self) -> RemoteGraphService {
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .https_or_http()
            .enable_http1()
            .build();
        RemoteGraphService {
            url: self.url,
            name: self.name,
            http: hyper::Client::builder().build(https),
            config: self.config,
        }
    }
}

impl BuildGraph for RemoteGraphBuilder {
    type SubgraphSerivce = RemoteGraphService;

    #[inline(always)]
    fn name(&self) -> &str {
        self.name
    }

    #[inline(always)]
    fn build(self) -> Self::SubgraphSerivce {
        self.build()
    }
}

///Remote subgraph service
pub struct RemoteGraphService {
    url: hyper::Uri,
    name: &'static str,
    http: hyper::Client<HttpsConnector<HttpConnector>>,
    config: Config,
}

impl Service<SubgraphRequest> for RemoteGraphService {
    type Response = SubgraphResponse;
    type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    #[inline(always)]
    fn poll_ready(&mut self, ctx: &mut task::Context<'_>) -> task::Poll<Result<(), Self::Error>> {
        self.http
            .poll_ready(ctx)
            .map(|res| res.map_err(|err| Box::new(err) as Self::Error))
    }

    #[inline]
    fn call(&mut self, request: SubgraphRequest) -> Self::Future {
        Box::pin(remote_subgraph(
            self.http.clone(),
            request,
            self.config,
            self.name,
            self.url.clone(),
        ))
    }
}

fn redirect_url(location: Option<hyper::Uri>, original: &hyper::Uri) -> Result<hyper::Uri, &'static str> {
    match location {
        Some(loc) => match loc.scheme().is_some() {
            //We assume that if scheme is present then it is absolute redirect
            //should clear some sensitive headers, but in our case it is unlikely graphql server
            //would redirect to different host, so consider it an error.
            true => {
                if let Some(prev_host) = original.authority().map(|part| part.host()) {
                    match loc.authority().map(|part| part.host() == prev_host).unwrap_or(false) {
                        true => Ok(loc),
                        false => Err("Redirect points to different host"),
                    }
                } else {
                    Ok(loc)
                }
            }
            //relative to current location
            false => {
                use std::path::Path;

                let current = Path::new(original.path());
                let loc = Path::new(loc.path());
                let loc = current.join(loc);
                let loc = loc
                    .to_str()
                    .expect("Valid UTF-8 path")
                    .parse::<hyper::Uri>()
                    .expect("Valid URI");
                let mut loc_parts = loc.into_parts();

                loc_parts.scheme = original.scheme().cloned();
                loc_parts.authority = original.authority().cloned();

                hyper::Uri::from_parts(loc_parts).map_err(|_| "Relative redirect cannot be constructed")
            }
        },
        None => Err("Redirect requested without Location header"),
    }
}

#[tracing::instrument(skip(http, req, config))]
async fn remote_subgraph(
    mut http: hyper::Client<HttpsConnector<HttpConnector>>,
    req: SubgraphRequest,
    config: Config,
    service_name: &'static str,
    mut url: hyper::Uri,
) -> Result<SubgraphResponse, Box<dyn std::error::Error + Send + Sync + 'static>> {
    tracing::info!("{}: Remote subgraph request towards {}", service_name, url);

    let mut http_request = req.subgraph_request;
    let context = req.context;

    http_request.headers_mut().insert(CONTENT_TYPE, APPLICATION_JSON);
    http_request.headers_mut().insert(ACCEPT, APPLICATION_JSON);
    let (parts, body) = http_request.into_parts();
    let body = serde_json::to_string(&body).expect("JSON serialization should not fail");
    let headers = parts.headers.clone();
    let method = parts.method.clone();

    let mut fetch_error_reason = String::new();
    let mut retry_remain = config.max_retry_num;
    let mut redirect_remain = config.max_redirect_num;
    while retry_remain > 0 {
        let (mut parts, _) = hyper::Request::<()>::new(()).into_parts();
        parts.headers = headers.clone();
        parts.method = method.clone();
        parts.uri = url.clone();

        let request = hyper::Request::from_parts(parts, body.clone().into());
        match http.call(request).await {
            Ok(response) => {
                let status = response.status().as_u16();
                tracing::debug!("Response status={}", status);

                //Since we act as proxy here, we only propagate response back
                //Unless we can retry.
                match status {
                    //We're redirected, let's follow it up, if we allow.
                    301 | 302 | 303 | 307 | 308 if redirect_remain > 0 => {
                        redirect_remain -= 1;
                        let location = response
                            .headers()
                            .get(hyper::header::LOCATION)
                            .and_then(|loc| loc.to_str().ok())
                            .and_then(|loc| loc.parse::<hyper::Uri>().ok());
                        match redirect_url(location, &url) {
                            Ok(new_url) => {
                                //Successful redirection, try again
                                url = new_url;
                                continue;
                            }
                            Err(error) => {
                                //Error here means we do not have valid location for redirect so
                                //give up
                                fetch_error_reason = error.to_string();
                                break;
                            }
                        }
                    }
                    //Temp unavailable, retry later
                    503 => {
                        tracing::info!("Server temp unavail. Retry");
                        retry_remain -= 1;
                        continue;
                    }
                    //We're good to return response
                    _ => {
                        let body = match hyper::body::to_bytes(response.into_body()).await {
                            Ok(body) => body,
                            //This case might be due to sudden loss of connection,
                            //but it is a bit unlikely to happen during reading body so
                            //let's assume error.
                            Err(error) => {
                                tracing::info!("Failed to read body: {}", error);
                                fetch_error_reason = error.to_string();
                                break;
                            }
                        };

                        let response = match apollo_router_core::Response::from_bytes(service_name, body) {
                            Ok(response) => response,
                            //This should not happen
                            Err(error) => {
                                return Err(apollo_router_core::FetchError::SubrequestMalformedResponse {
                                    service: service_name.to_owned(),
                                    reason: error.to_string(),
                                }
                                .into());
                            }
                        };

                        let response = hyper::Response::builder()
                            .body(response)
                            .expect("no argument can fail to parse or converted to the internal representation here")
                            .into();
                        return Ok(SubgraphResponse { response, context });
                    }
                }
            }
            Err(error) => {
                tracing::info!("failed: {}", error);

                fetch_error_reason = error.to_string();
                retry_remain -= 1;
            }
        };
    }

    let fetch_error = apollo_router_core::FetchError::SubrequestHttpError {
        service: service_name.to_owned(),
        reason: fetch_error_reason,
    };
    Err(fetch_error.into())
}
