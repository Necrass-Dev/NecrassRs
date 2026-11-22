 -  Changed the Actix Web project created by `necrass init --framework actix`
    to depend on `actix-web` 4.19 or later with default features disabled and
    only the `macros` feature enabled. New projects no longer resolve Actix Web
    and Actix HTTP releases affected by recent advisories, and no longer pull in
    `h2` 0.3, which is affected by RUSTSEC-2026-0258, or `brotli` and `zstd`.
    Enable features such as `http2` or the `compress-*` features on the
    project's `actix-web` dependency when the application needs them.
    [[#101]]
