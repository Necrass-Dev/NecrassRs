 -  Fixed a request within the default token limit aborting the server
    process by overflowing the stack of a Tokio worker thread during
    validation, even with introspection disabled. Requests whose composed
    selection depth exceeds 500 now fail as validation errors with a single
    `too much recursion` error, and the maintained adapters return 422.
