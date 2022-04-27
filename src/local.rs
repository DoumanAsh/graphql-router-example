use apollo_router_core::{SubgraphRequest, SubgraphResponse};
use async_graphql::{ObjectType, Schema, SubscriptionType};

use crate::BuildGraph;

use core::any::Any;
use core::future::Future;
use core::pin::Pin;
use core::{mem, task};

///Builder to create local graphql service
pub struct LocalGraphBuilder<Q, M, S> {
    schema: Schema<Q, M, S>,
    name: &'static str,
    data: async_graphql::context::Data,
}

impl<Q: ObjectType + 'static, M: ObjectType + 'static, S: SubscriptionType + 'static> LocalGraphBuilder<Q, M, S> {
    #[inline]
    ///Starts building subgraph
    pub fn new(name: &'static str, schema: Schema<Q, M, S>) -> Self {
        Self {
            schema,
            name,
            data: Default::default(),
        }
    }

    #[inline(always)]
    ///Inserts context data for request processing.
    pub fn data<D: Any + Send + Sync>(&mut self, data: D) -> &mut Self {
        self.data.insert(data);
        self
    }

    #[inline(always)]
    ///Builds service
    pub fn build(self) -> LocalGraphService<Q, M, S> {
        LocalGraphService {
            name: self.name,
            inner: self.schema,
            data: self.data,
        }
    }
}

impl<Q: ObjectType + 'static, M: ObjectType + 'static, S: SubscriptionType + 'static> BuildGraph
    for LocalGraphBuilder<Q, M, S>
{
    type SubgraphSerivce = LocalGraphService<Q, M, S>;

    #[inline(always)]
    fn name(&self) -> &str {
        self.name
    }

    #[inline(always)]
    fn build(self) -> Self::SubgraphSerivce {
        self.build()
    }
}

/// Local graphql service.
pub struct LocalGraphService<Q, M, S> {
    name: &'static str,
    inner: Schema<Q, M, S>,
    data: async_graphql::context::Data,
}

impl<Q: ObjectType + 'static, M: ObjectType + 'static, S: SubscriptionType + 'static>
    tower_service::Service<SubgraphRequest> for LocalGraphService<Q, M, S>
{
    type Response = SubgraphResponse;
    type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    #[inline(always)]
    fn poll_ready(&mut self, _: &mut task::Context<'_>) -> task::Poll<Result<(), Self::Error>> {
        //Local graph should be always ready
        task::Poll::Ready(Ok(()))
    }

    #[inline]
    fn call(&mut self, request: SubgraphRequest) -> Self::Future {
        tracing::info!("{}: Local subgraph request", self.name);

        let (_http, graphql) = request.subgraph_request.into_parts();
        let context = request.context;
        let service_name = self.name;
        let mut variables = async_graphql::Variables::default();

        for (key, val) in graphql.variables.iter() {
            let key = async_graphql::Name::new(key.as_str());
            //This is a similar approach to how `async-graphql` does conversion
            let val = serde_json_bytes::from_value(val.clone()).unwrap_or_default();
            variables.insert(key, val);
        }

        let mut transformed_req = async_graphql::Request::new(graphql.query.unwrap_or_default())
            .operation_name(graphql.operation_name.unwrap_or_default())
            .variables(variables);
        for (key, val) in graphql.extensions.into_iter() {
            let key = key.as_str().to_owned();
            let val = serde_json_bytes::from_value(val.clone()).unwrap_or_default();
            transformed_req.extensions.insert(key, val);
        }
        //TODO: This subgraph is valid once if we insert data.
        //      Consider if we need it to be re-used
        mem::swap(&mut self.data, &mut transformed_req.data);

        let schema = self.inner.clone();
        let res = async move {
            let res = schema.execute(transformed_req).await;
            let bytes = serde_json::to_vec(&res)?;
            let res = apollo_router_core::Response::from_bytes(service_name, bytes.into())?;
            let res = apollo_router_core::SubgraphResponse {
                //It shouldn't fail here actually but just in case propagate error
                response: http::Response::builder().body(res)?.into(),
                context,
            };
            Ok(res)
        };

        Box::pin(res)
    }
}
