 -  Changed the `actix-web` dependency to require 4.19 or later and to disable
    its default features. `necrassrs-actix` no longer enables Actix Web's
    HTTP/2, compression, cookie, WebSocket, or macro features on the
    application's behalf, so applications that disable them no longer pull in
    `h2` 0.3, `brotli`, or `zstd`. An application that uses one of these
    features must enable it on its own `actix-web` dependency; applications
    that declare `actix-web = "4"` keep the defaults. Run
    `cargo update -p actix-web -p actix-http` to pick up the fixed Actix Web
    and Actix HTTP releases for RUSTSEC-2026-0446, RUSTSEC-2026-0447,
    RUSTSEC-2026-0448, RUSTSEC-2026-0455, RUSTSEC-2026-0552,
    RUSTSEC-2026-0553, and RUSTSEC-2026-0554; `actix-http` 3.21.3 or later
    includes every fix. [[#101]]
