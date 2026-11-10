 -  Moved GraphiQL to `/graphiql` in the CLI starters and maintained examples,
    leaving `/graphql` for GET and POST GraphQL requests. Open `/graphiql` to
    use the UI. Existing applications should register
    `graphiql_html("/graphql")` on a separate GET route and register their
    GraphQL handler for GET as well as POST. GraphiQL remains opt-in and its
    endpoint argument still points to the API route.  [[#30]]
