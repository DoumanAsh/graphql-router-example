use graphql_router::{GraphqlRequest, GraphqlResponse, GraphqlRouter, LocalGraphBuilder, RemoteGraphBuilder};

use std::sync::Arc;

use async_graphql::{Context, EmptyMutation, EmptySubscription, Object, SimpleObject, ID};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::Extension;
use hyper::Uri;

const API_PORT: u16 = 9000;

mod user {
    use super::*;

    pub type Schema = async_graphql::Schema<Query, EmptyMutation, EmptySubscription>;

    #[derive(SimpleObject)]
    pub struct User {
        id: ID,
        username: String,
    }

    pub struct Query;

    #[Object(extends)]
    impl Query {
        async fn me(&self) -> User {
            User {
                id: "1234".into(),
                username: "Me".to_string(),
            }
        }

        #[graphql(entity)]
        async fn find_user_by_id(&self, id: ID) -> User {
            let username = if id == "1234" {
                "Me".to_string()
            } else {
                format!("User {:?}", id)
            };
            User { id, username }
        }
    }

    #[allow(unused)]
    pub async fn graphql_handler(schema: Extension<Schema>, req: GraphQLRequest) -> GraphQLResponse {
        schema.execute(req.into_inner()).await.into()
    }

    pub fn schema() -> Schema {
        Schema::new(Query, EmptyMutation, EmptySubscription)
    }
}

mod product {
    use super::*;

    pub type Schema = async_graphql::Schema<Query, EmptyMutation, EmptySubscription>;

    #[derive(SimpleObject)]
    pub struct Product {
        upc: String,
        name: String,
        price: i32,
    }

    pub struct Query;

    #[Object(extends)]
    impl Query {
        async fn top_products<'a>(&self, ctx: &'a Context<'_>) -> &'a Vec<Product> {
            ctx.data_unchecked::<Vec<Product>>()
        }

        #[graphql(entity)]
        async fn find_product_by_upc<'a>(&self, ctx: &'a Context<'_>, upc: String) -> Option<&'a Product> {
            let hats = ctx.data_unchecked::<Vec<Product>>();
            hats.iter().find(|product| product.upc == upc)
        }
    }

    pub async fn graphql_handler(schema: Extension<Schema>, req: GraphQLRequest) -> GraphQLResponse {
        schema.execute(req.into_inner()).await.into()
    }

    pub fn schema() -> Schema {
        let hats = vec![
            Product {
                upc: "top-1".to_string(),
                name: "Trilby".to_string(),
                price: 11,
            },
            Product {
                upc: "top-2".to_string(),
                name: "Fedora".to_string(),
                price: 22,
            },
            Product {
                upc: "top-3".to_string(),
                name: "Boater".to_string(),
                price: 33,
            },
        ];

        Schema::build(Query, EmptyMutation, EmptySubscription)
            .data(hats)
            .finish()
    }
}

mod review {
    use super::*;

    pub struct User {
        id: ID,
    }

    #[Object(extends)]
    impl User {
        #[graphql(external)]
        async fn id(&self) -> &ID {
            &self.id
        }

        async fn reviews<'a>(&self, ctx: &'a Context<'_>) -> Vec<&'a Review> {
            let reviews = ctx.data_unchecked::<Vec<Review>>();
            reviews.iter().filter(|review| review.author.id == self.id).collect()
        }
    }

    pub struct Product {
        upc: String,
    }

    #[Object(extends)]
    impl Product {
        #[graphql(external)]
        async fn upc(&self) -> &String {
            &self.upc
        }

        async fn reviews<'a>(&self, ctx: &'a Context<'_>) -> Vec<&'a Review> {
            let reviews = ctx.data_unchecked::<Vec<Review>>();
            reviews.iter().filter(|review| review.product.upc == self.upc).collect()
        }
    }

    #[derive(SimpleObject)]
    pub struct Review {
        body: String,
        author: User,
        product: Product,
    }

    pub struct Query;

    #[Object]
    impl Query {
        #[graphql(entity)]
        async fn find_user_by_id(&self, id: ID) -> User {
            User { id }
        }

        #[graphql(entity)]
        async fn find_product_by_upc(&self, upc: String) -> Product {
            Product { upc }
        }
    }

    pub type Schema = async_graphql::Schema<Query, EmptyMutation, EmptySubscription>;

    pub async fn graphql_handler(schema: Extension<Schema>, req: GraphQLRequest) -> GraphQLResponse {
        schema.execute(req.into_inner()).await.into()
    }

    pub fn schema() -> Schema {
        let reviews = vec![
            Review {
                body: "A highly effective form of birth control.".into(),
                author: User { id: "1234".into() },
                product: Product {
                    upc: "top-1".to_string(),
                },
            },
            Review {
                body:
                    "Fedoras are one of the most fashionable hats around and can look great with a variety of outfits."
                        .into(),
                author: User { id: "1234".into() },
                product: Product {
                    upc: "top-1".to_string(),
                },
            },
            Review {
                body: "This is the last straw. Hat you will wear. 11/10".into(),
                author: User { id: "7777".into() },
                product: Product {
                    upc: "top-1".to_string(),
                },
            },
        ];

        Schema::build(Query, EmptyMutation, EmptySubscription)
            .data(reviews)
            .finish()
    }
}

async fn super_handler(
    supergraph: Extension<Arc<graphql_router::Schema>>,
    parts: http::request::Parts,
    req: axum::Json<GraphqlRequest>,
) -> axum::Json<GraphqlResponse> {
    let req = req.0;

    let user_subgraph = LocalGraphBuilder::new("user".to_owned(), user::schema);
    let product_subgraph =
        RemoteGraphBuilder::new("product".to_owned(), "http://127.0.0.1:9000/product".parse().unwrap());
    let review_subgraph = RemoteGraphBuilder::new("review".to_owned(), "http://127.0.0.1:9000/review".parse().unwrap());
    let mut router = GraphqlRouter::build(supergraph.0)
        .add_subgraph(user_subgraph)
        .add_subgraph(review_subgraph)
        .add_subgraph(product_subgraph)
        .finish()
        .await
        .expect("to create router");

    let request = apollo_router_core::http_compat::Request::from_parts(parts, req);
    match router.handle(request.into()).await {
        Ok(result) => match GraphqlResponse::try_from(result.response.into_body()) {
            Ok(response) => axum::Json(response),
            Err(error) => panic!("Cannot parse response: {}", error),
        },
        Err(error) => panic!("Unexpected error in router: {}", error),
    }
}

#[tokio::test]
async fn should_handle_local_and_remote_graphql() {
    let supergraph = graphql_router::Schema::read("tests/supergraph.graphql").expect("To read supergraph");

    let user_schema = user::schema();
    let product_schema = product::schema();
    let review_schema = review::schema();

    //std::fs::write("user.graphql", user_schema.federation_sdl()).expect("Write user schema");
    //std::fs::write("product.graphql", product_schema.federation_sdl()).expect("Write product schema");
    //std::fs::write("review.graphql", review_schema.federation_sdl()).expect("Write review schema");

    let app = axum::Router::new()
        .route("/review", axum::routing::post(review::graphql_handler))
        .route("/product", axum::routing::post(product::graphql_handler))
        .route("/super", axum::routing::post(super_handler))
        .layer(Extension(user_schema))
        .layer(Extension(product_schema))
        .layer(Extension(review_schema))
        .layer(Extension(Arc::new(supergraph)));

    let server = axum::Server::bind(&([127, 0, 0, 1], API_PORT).into()).serve(app.into_make_service());

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let graceful = server.with_graceful_shutdown(async {
        rx.await.ok();
    });

    let server_task = tokio::spawn(graceful);

    //START

    //query me
    let expected_body = r#"{"data":{"me":{"username":"Me"}}}"#;

    let http = hyper::Client::new();
    let supergraph_uri = Uri::from_static("http://127.0.0.1:9000/super");
    let req = apollo_router_core::Request::builder()
        .query(r#"query Query { me { username } }"#.to_owned())
        .build();
    let req = serde_json::to_vec(&req).expect("Serialize request");
    let req = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri(supergraph_uri)
        .header("Content-Type", "application/json")
        .body(req.into())
        .expect("build request");

    println!("Supergraph request\n=====");
    let response = http.request(req).await.expect("Successfully send supergraph request");
    let (headers, body) = response.into_parts();
    let body = hyper::body::to_bytes(body).await.expect("To read body");
    let body = core::str::from_utf8(&body).expect("To return string back");
    println!("Supergraph response\n=====\n{:?}", headers);
    println!("=====\n{}", body);
    assert_eq!(body, expected_body);

    //query topProducts
    let expected_body = r#"{"data":{"topProducts":[{"name":"Trilby","price":11,"upc":"top-1","reviews":[{"body":"A highly effective form of birth control.","author":{"username":"Me"}},{"body":"Fedoras are one of the most fashionable hats around and can look great with a variety of outfits.","author":{"username":"Me"}},{"body":"This is the last straw. Hat you will wear. 11/10","author":{"username":"User ID(\"7777\")"}}]},{"name":"Fedora","price":22,"upc":"top-2","reviews":[]},{"name":"Boater","price":33,"upc":"top-3","reviews":[]}]}}"#;

    let http = hyper::Client::new();
    let supergraph_uri = Uri::from_static("http://127.0.0.1:9000/super");
    let req = apollo_router_core::Request::builder()
        .query(r#"query Query { topProducts { name, price, upc, reviews { body, author { username } } } }"#.to_owned())
        .build();
    let req = serde_json::to_vec(&req).expect("Serialize request");
    let req = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri(supergraph_uri)
        .header("Content-Type", "application/json")
        .body(req.into())
        .expect("build request");

    println!("Supergraph request\n=====");
    let response = http.request(req).await.expect("Successfully send supergraph request");
    let (headers, body) = response.into_parts();
    let body = hyper::body::to_bytes(body).await.expect("To read body");
    let body = core::str::from_utf8(&body).expect("To return string back");
    println!("Supergraph response\n=====\n{:?}", headers);
    println!("=====\n{}", body);
    assert_eq!(body, expected_body);

    //END

    let _ = tx.send(());

    server_task
        .await
        .expect("Successfully finish server task")
        .expect("Successfully finish server");
}
