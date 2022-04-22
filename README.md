## graphql-router-example

This example uses [apollo-router](https://github.com/apollographql/router) with custom implementation of subgraphs:
- LocalGraph - this one takes `async-graphql` schema using builder function and creates schema with it each time request comes in. It handles conversion between `apollo-router` types and `async-graphql` types with as little overhead as possible
- RemoteGraphql - This is remote fetcher, which is based on `apollo-router-core`, but provides few extra functions: allow to specify URL where to fetch (instead of using URL in schema) and retry mechanism
