 -  Changed the Actix Web project created by `necrass init --framework actix`
    to require `actix-web` 4.19 or later, so a new project's lockfile does not
    resolve releases affected by recent Actix Web and Actix HTTP advisories.
    The template keeps Actix Web's default features, which include HTTP/2
    through `h2` 0.3 and are affected by RUSTSEC-2026-0258; disable
    the `http2` feature if the application does not need it. [[#101]]
