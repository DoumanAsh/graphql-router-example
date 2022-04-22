use core::fmt;

use crate::{HttpRequest, RouterRequest};

#[derive(Debug)]
pub enum ParseHttpError {
    ///Unable to read HTTP request's body.
    Http(hyper::Error),
    ///Body contains invalid Graphql Request.
    Invalid(serde_json::Error),
}

impl From<hyper::Error> for ParseHttpError {
    #[inline(always)]
    fn from(error: hyper::Error) -> Self {
        ParseHttpError::Http(error)
    }
}

impl From<serde_json::Error> for ParseHttpError {
    #[inline(always)]
    fn from(error: serde_json::Error) -> Self {
        ParseHttpError::Invalid(error)
    }
}

impl fmt::Display for ParseHttpError {
    #[inline(always)]
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseHttpError::Http(error) => fmt.write_fmt(format_args!("Failed to read Graphql request: {}", error)),
            ParseHttpError::Invalid(error) => fmt.write_fmt(format_args!("Invalid Graphql Request: {}", error)),
        }
    }
}

///Parses raw HTTP Request into GraphqlRouter's request.
pub async fn parse_http_request(req: HttpRequest) -> Result<RouterRequest, ParseHttpError> {
    let (http, body) = req.into_parts();
    let bytes = hyper::body::to_bytes(body).await?;
    let graphql = apollo_router_core::Request::from_bytes(bytes)?;
    let graphql = apollo_router_core::http_compat::Request::from_parts(http, graphql);
    Ok(graphql.into())
}
