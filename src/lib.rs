//! Graphql Router

use std::sync::Arc;

///Error result of graphql router handler
pub type HandleError = Box<dyn std::error::Error + Send + Sync>;
///Alias to plain http request
pub type HttpRequest = hyper::Request<hyper::Body>;
pub use apollo_router_core::Request as GraphqlRequest;
pub use apollo_router_core::Response as GraphqlResponse;
use apollo_router_core::{PluggableRouterServiceBuilder, SubgraphRequest, SubgraphResponse};
pub use apollo_router_core::{RouterRequest, RouterResponse, Schema};

use core::future::Future;
use core::pin::Pin;
use core::task;

mod parser;
pub use parser::{parse_http_request, ParseHttpError};
pub mod local;
pub use local::LocalGraphBuilder;
pub mod remote;
pub use remote::RemoteGraphBuilder;

pub trait BuildGraph: Sized + Send {
    ///Service type
    type SubgraphSerivce: tower_service::Service<
            SubgraphRequest,
            Response = SubgraphResponse,
            Error = Box<dyn std::error::Error + Send + Sync + 'static>,
        > + Send
        + 'static;

    ///Returns service name.
    fn name(&self) -> &str;
    ///Builds service
    fn build(self) -> Self::SubgraphSerivce;
}

#[allow(clippy::large_enum_variant)]
enum GraphqlRouterHandlerState {
    //RouterRequest is relative big, but we don't move it all that much
    Pending(RouterRequest),
    Ongoing(Pin<Box<dyn Future<Output = Result<RouterResponse, HandleError>> + Send + 'static>>),
}

pub struct GraphqlRouterHandler {
    service: tower::util::BoxCloneService<RouterRequest, RouterResponse, HandleError>,
    state: GraphqlRouterHandlerState,
}

impl Future for GraphqlRouterHandler {
    type Output = Result<RouterResponse, HandleError>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        use tower_service::Service;

        let this = Pin::into_inner(self);
        loop {
            this.state = match &mut this.state {
                GraphqlRouterHandlerState::Ongoing(ongoing) => return Future::poll(Pin::new(ongoing), ctx),
                GraphqlRouterHandlerState::Pending(req) => match this.service.poll_ready(ctx) {
                    task::Poll::Ready(Ok(())) => {
                        let mut future_req = RouterRequest::fake_builder()
                            .build()
                            .into();
                        core::mem::swap(&mut future_req, req);
                        //intentionally falling through to poll future
                        GraphqlRouterHandlerState::Ongoing(this.service.call(future_req))
                    }
                    task::Poll::Pending => return task::Poll::Pending,
                    task::Poll::Ready(Err(error)) => return task::Poll::Ready(Err(error)),
                },
            }
        }
    }
}

#[derive(Clone)]
///Router
pub struct GraphqlRouter {
    pub schema: Arc<Schema>,
    service: tower::util::BoxCloneService<RouterRequest, RouterResponse, HandleError>,
}

impl GraphqlRouter {
    #[inline(always)]
    pub fn build(schema: Arc<Schema>) -> GraphqlRouterBuilder {
        GraphqlRouterBuilder {
            builder: PluggableRouterServiceBuilder::new(schema.clone()),
            schema,
        }
    }

    #[inline(always)]
    pub fn handle(&mut self, req: RouterRequest) -> GraphqlRouterHandler {
        GraphqlRouterHandler {
            service: self.service.clone(),
            state: GraphqlRouterHandlerState::Pending(req),
        }
    }
}

///Router builder
pub struct GraphqlRouterBuilder {
    builder: PluggableRouterServiceBuilder,
    schema: Arc<Schema>,
}

impl GraphqlRouterBuilder {
    #[inline]
    ///Adds subgraph
    pub fn add_subgraph<T: BuildGraph>(self, graph: T) -> Self
    where
        <<T as BuildGraph>::SubgraphSerivce as tower_service::Service<SubgraphRequest>>::Future: Send,
    {
        if cfg!(debug_assertions) {
            let mut found = false;

            for (service_name, _) in self.schema.subgraphs() {
                found = graph.name() == service_name;
                if found {
                    break;
                }
            }

            assert!(
                found,
                "Attempt to add subgraph '{}' which is not present in schema",
                graph.name()
            );
        }

        let name = graph.name().to_owned();
        Self {
            schema: self.schema,
            builder: self.builder.with_subgraph_service(&name, graph.build()),
        }
    }

    #[inline(always)]
    ///Finalizes builder
    ///
    ///Not being able to build query likely means that query planner is unable to handle schema
    ///with subgraphs, which is probably means error in schema, so cannot be recovered so treat it
    ///as 500 error
    pub async fn finish(self) -> Result<GraphqlRouter, apollo_router_core::ServiceBuildError> {
        Ok(GraphqlRouter {
            schema: self.schema,
            service: self.builder.build().await?.0,
        })
    }
}
